mod encode;

use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{HostTrait, DeviceTrait, StreamTrait};
use cpal::InputCallbackInfo;
use structopt::StructOpt;

use crate::protocol::{self, Protocol};
use crate::protocol::packet::{self, StatsReply, PacketKind};
use crate::protocol::types::{TimestampMicros, SessionId, ReceiverId, TimePhase};
use crate::socket::{Socket, SocketOpt};
use crate::source::encode::{PcmFloat32, Encoder};
use crate::stats::node::NodeStats;
use crate::time::{SampleDuration, Timestamp};
use crate::util;
use crate::RunError;

#[derive(StructOpt)]
pub struct StreamOpt {
    #[structopt(flatten)]
    pub socket: SocketOpt,

    #[structopt(
        long,
        env = "BARK_SOURCE_DEVICE",
    )]
    pub device: Option<String>,

    #[structopt(
        long,
        env = "BARK_SOURCE_DELAY_MS",
        default_value = "20",
    )]
    pub delay_ms: u64,
}

pub fn run(opt: StreamOpt) -> Result<(), RunError> {
    let host = cpal::default_host();

    if let Some(device) = &opt.device {
        crate::audio::set_source_env(device);
    }

    let device = host.default_input_device()
        .ok_or(RunError::NoDeviceAvailable)?;

    let config = util::config_for_device(&device)?;

    let socket = Socket::open(opt.socket)
        .map_err(RunError::Listen)?;

    let protocol = Arc::new(Protocol::new(socket));

    let delay = Duration::from_millis(opt.delay_ms);
    let delay = SampleDuration::from_std_duration_lossy(delay);

    let sid = SessionId::generate();
    let node = NodeStats::get();

    let stream = device.build_input_stream(&config,
        {
            let protocol = Arc::clone(&protocol);
            let mut encoder = PcmFloat32::new(protocol, sid);

            let mut initialized_thread = false;
            move |data: &[f32], _: &InputCallbackInfo| {
                if !initialized_thread {
                    crate::thread::set_name("bark/audio");
                    crate::thread::set_realtime_priority();
                    initialized_thread = true;
                }

                // assert data only contains complete frames:
                assert!(data.len() % usize::from(protocol::CHANNELS) == 0);

                let pts = Timestamp::now() + delay;
                encoder.write(data, pts);
            }
        },
        move |err| {
            eprintln!("stream error! {err:?}");
        },
        None
    ).map_err(RunError::BuildStream)?;

    // set up t1 sender thread
    std::thread::spawn({
        crate::thread::set_name("bark/clock");
        crate::thread::set_realtime_priority();

        let protocol = Arc::clone(&protocol);
        move || {
            let mut time = packet::Time::allocate();

            // set up packet
            let data = time.data_mut();
            data.sid = sid;
            data.rid = ReceiverId::broadcast();

            loop {
                time.data_mut().stream_1 = TimestampMicros::now();

                protocol.broadcast(time.as_packet())
                    .expect("broadcast time");

                std::thread::sleep(Duration::from_millis(200));
            }
        }
    });

    stream.play().map_err(RunError::Stream)?;

    crate::thread::set_name("bark/network");
    crate::thread::set_realtime_priority();

    loop {
        let (packet, peer) = protocol.recv_from().expect("protocol.recv_from");

        match packet.parse() {
            Some(PacketKind::Audio(audio)) => {
                // we should only ever receive an audio packet if another
                // stream is present. check if it should take over
                if audio.header().sid > sid {
                    eprintln!("Peer {peer} has taken over stream, exiting");
                    break;
                }
            }
            Some(PacketKind::Time(mut time)) => {
                // only handle packet if it belongs to our stream:
                if time.data().sid != sid {
                    continue;
                }

                match time.data().phase() {
                    Some(TimePhase::ReceiverReply) => {
                        time.data_mut().stream_3 = TimestampMicros::now();

                        protocol.send_to(time.as_packet(), peer)
                            .expect("protocol.send_to responding to time packet");
                    }
                    _ => {
                        // any other packet here must be destined for
                        // another instance on the same machine
                    }
                }

            }
            Some(PacketKind::StatsRequest(_)) => {
                let reply = StatsReply::source(sid, node);
                let _ = protocol.send_to(reply.as_packet(), peer);
            }
            Some(PacketKind::StatsReply(_)) => {
                // ignore
            }
            None => {
                // unknown packet, ignore
            }
        }
    }

    Ok(())
}
