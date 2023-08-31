use std::array;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::buffer::AudioBuffer;
use crate::protocol::packet::{Packet, Audio, Time};
use crate::protocol::types::{SessionId, TimestampMicros};
use crate::stats::receiver::ReceiverStats;
use crate::time::{Timestamp, SampleDuration, TimestampDelta, ClockDelta};

use super::output::{Output, OutputConfig};
use super::slew::Slew;

pub struct Session {
    sid: SessionId,
    start_seq: u64,
    shared: Arc<Mutex<Shared>>,
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<QueuedPacket>,
    sync: bool,
    quit: bool,
    timing: Timing,
    stats: ReceiverStats,
}

impl Session {
    pub fn start(&self, first_packet: &Audio) -> Self {
        let header = first_packet.header();
        let shared = Mutex::default();

        std::thread::spawn({
            move || {
                // no real-time priority for session processing thread
                crate::thread::set_name("bark/session");


            }
        });
        run_session(shared.clone());

        Session {
            sid: first_packet.header().sid,
            start_seq: first_packet.header().seq,
            shared: shared,
        }
    }

    pub fn send_packet(&self, packet: Audio) {
        let now = TimestampMicros::now();

        assert!(self.sid == packet.header().sid);

        let mut shared = self.shared.lock().unwrap();

        if let Some(latency) = shared.timing.network_latency() {
            if let Some(clock_delta) = shared.timing.clock_delta.median() {
                let latency_usec = u64::try_from(latency.as_micros()).unwrap();
                let delta_usec = clock_delta.as_micros();
                let predict_dts = (now.0 - latency_usec).checked_add_signed(-delta_usec).unwrap();
                let predict_diff = predict_dts as i64 - packet.header().dts.0 as i64;

                shared.stats.set_predict_offset(predict_diff)
            }
        }

        // INVARIANT: at this point we are guaranteed that, if there are
        // packets in the queue, the seq of the incoming packet is less than
        // back.seq + max_seq_gap

        // expand queue to make space for new packet
        if let Some(back) = shared.queue.back() {
            if packet.header().seq > back.seq {
                // extend queue from back to make space for new packet
                // this also allows for out of order packets
                for seq in (back.seq + 1)..=packet.header().seq {
                    shared.queue.push_back(QueuedPacket {
                        seq,
                        pts: None,
                        consumed: SampleDuration::zero(),
                        audio: None,
                    })
                }
            }
        } else {
            // queue is empty, insert missing packet slot for the packet we are about to receive
            shared.queue.push_back(QueuedPacket {
                seq: packet.header().seq,
                pts: None,
                consumed: SampleDuration::zero(),
                audio: None,
            });
        }

        // INVARIANT: at this point queue is non-empty and contains an
        // allocated slot for the packet we just received
        let front_seq = shared.queue.front().unwrap().seq;
        let idx_for_packet = (packet.header().seq - front_seq) as usize;

        let slot = shared.queue.get_mut(idx_for_packet).unwrap();
        assert!(slot.seq == packet.header().seq);
        slot.pts = shared.timing.adjust_pts(Timestamp::from_micros_lossy(packet.header().pts));
        slot.audio = Some(packet.into_audio_buffer());
    }
}

struct QueuedPacket {
    seq: u64,
    pts: Option<Timestamp>,
    consumed: SampleDuration,
    audio: Option<AudioBuffer>,
}

#[derive(Default)]
struct Timing {
    latency: Aggregate<Duration>,
    clock_delta: Aggregate<ClockDelta>,
}

impl Timing {
    pub fn adjust_pts(&self, pts: Timestamp) -> Option<Timestamp> {
        self.clock_delta.median().map(|delta| {
            pts.adjust(TimestampDelta::from_clock_delta_lossy(delta))
        })
    }

    pub fn network_latency(&self) -> Option<Duration> {
        self.latency.median()
    }
}

struct Aggregate<T> {
    samples: [T; 64],
    count: usize,
    index: usize,
}

impl<T: Default> Default for Aggregate<T> {
    fn default() -> Self {
        let samples = array::from_fn(|_| Default::default());
        Aggregate { samples, count: 0, index: 0 }
    }
}

impl<T: Copy + Default + Ord> Aggregate<T> {
    pub fn observe(&mut self, value: T) {
        self.samples[self.index] = value;

        if self.count < self.samples.len() {
            self.count += 1;
        }

        self.index += 1;
        self.index %= self.samples.len();
    }

    pub fn median(&self) -> Option<T> {
        let mut samples = self.samples;
        let samples = &mut samples[0..self.count];
        samples.sort();
        samples.get(self.count / 2).copied()
    }
}

fn run_session(shared: Arc<Mutex<Shared>>, config: OutputConfig) {
    let output = Output::new(&config)
        .expect("open output stream in run_session");

    let slew = Slew::new(output);

    'next: loop {
        let mut shared = shared.lock().unwrap();

        if shared.quit {
            break;
        }

        // sync up to stream if necessary:
        if !shared.sync {
            loop {
                let Some(front) = shared.queue.front_mut() else {
                    continue 'next;
                };

                let Some(front_pts) = front.pts else {
                    // haven't received enough info to adjust pts of queue
                    // front yet, just pop and ignore it
                    shared.queue.pop_front();
                    // and output silence for this part:
                    data.fill(0f32);
                    return;
                };

                if pts > front_pts {
                    // frame has already begun, we are late
                    let late = pts.duration_since(front_pts);

                    if late >= SampleDuration::ONE_PACKET {
                        // we are late by more than a packet, skip to the next
                        self.queue.pop_front();
                        continue;
                    }

                    // partially consume this packet to sync up
                    front.consumed = late;

                    // we are synced
                    stream.sync = true;
                    self.stats.set_stream(StreamStatus::Sync);
                    break;
                }

                // otherwise we are early
                let early = front_pts.duration_since(pts);

                if early >= SampleDuration::from_buffer_offset(data.len()) {
                    // we are early by more than what was asked of us in this
                    // call, fill with zeroes and return
                    data.fill(0f32);
                    return;
                }

                // we are early, but not an entire packet timing's early
                // partially output some zeroes
                let zero_count = early.as_buffer_offset();
                data[0..zero_count].fill(0f32);
                data = &mut data[zero_count..];

                // then mark ourselves as synced and fall through to regular processing
                stream.sync = true;
                self.stats.set_stream(StreamStatus::Sync);
                break;
            }
        }

    }
}
