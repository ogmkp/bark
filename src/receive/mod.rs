mod buffer;
mod output;
mod slew;
// mod session;
mod queue;
use std::time::Duration;

use cpal::traits::HostTrait;
use structopt::StructOpt;

use crate::protocol::Protocol;
use crate::protocol::packet::PacketKind;
use crate::protocol::types::{TimestampMicros, ReceiverId, TimePhase};
use crate::receive::output::OutputConfig;
use crate::socket::{Socket, SocketOpt};
use crate::stats::node::NodeStats;
use crate::util;
use crate::RunError;

#[derive(Clone, Copy)]
pub struct ClockInfo {
    pub network_latency_usec: i64,
    pub clock_diff_usec: i64,
}

#[derive(StructOpt, Clone)]
pub struct ReceiveOpt {
    #[structopt(flatten)]
    pub socket: SocketOpt,
    #[structopt(long, env = "BARK_RECEIVE_DEVICE")]
    pub device: Option<String>,
    #[structopt(long, default_value="12")]
    pub max_seq_gap: usize,
}

pub fn run(opt: ReceiveOpt) -> Result<(), RunError> {
    let receiver_id = ReceiverId::generate();
    let node = NodeStats::get();

    if let Some(device) = &opt.device {
        crate::device::set_sink_env(device);
    }

    let host = cpal::default_host();

    let device = host.default_output_device()
        .ok_or(RunError::NoDeviceAvailable)?;

    let stream_config = util::config_for_device(&device)?;

    let config = OutputConfig {
        device,
        stream: stream_config,
        buffer_delay: Duration::from_millis(10),
    };

    let _output = output::Output::new(&config)
        .map_err(RunError::BuildStream)?;

    let socket = Socket::open(opt.socket)
        .map_err(RunError::Listen)?;

    let protocol = Protocol::new(socket);

    crate::thread::set_name("bark/network");
    crate::thread::set_realtime_priority();

    loop {
        let (packet, peer) = protocol.recv_from().map_err(RunError::Socket)?;

        match packet.parse() {
            Some(PacketKind::Time(mut time)) => {
                if !time.data().rid.matches(&receiver_id) {
                    // not for us - time packets are usually unicast,
                    // but there can be multiple receivers on a machine
                    continue;
                }

                match time.data().phase() {
                    Some(TimePhase::Broadcast) => {
                        let data = time.data_mut();
                        data.receive_2 = TimestampMicros::now();
                        data.rid = receiver_id;

                        protocol.send_to(time.as_packet(), peer)
                            .expect("reply to time packet");
                    }
                    Some(TimePhase::StreamReply) => {
                        // let mut state = state.lock().unwrap();
                        // state.recv.receive_time(time);
                    }
                    _ => {
                        // not for us - must be destined for another process
                        // on same machine
                    }
                }
            }
            Some(PacketKind::Audio(_packet)) => {
                // let mut state = state.lock().unwrap();
                // state.recv.receive_audio(packet);
            }
            Some(PacketKind::StatsRequest(_)) => {
                // let state = state.lock().unwrap();
                // let sid = state.recv.current_session().unwrap_or(SessionId::zeroed());
                // let receiver = *state.recv.stats();
                // drop(state);

                // let reply = StatsReply::receiver(sid, receiver, node);
                // let _ = protocol.send_to(reply.as_packet(), peer);
            }
            Some(PacketKind::StatsReply(_)) => {
                // ignore
            }
            None => {
                // unknown packet type, ignore
            }
        }
    }
}
