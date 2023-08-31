use std::sync::Arc;

use crate::protocol::packet::{Audio, AudioWriter};
use crate::protocol::Protocol;
use crate::protocol::types::{AudioPacketHeader, SessionId, TimestampMicros};
use crate::time::{Timestamp, SampleDuration};
use crate::util::Sequence;

pub trait Encoder {
    fn write(&mut self, data: &[f32], pts: Timestamp);
}

pub struct PcmFloat32 {
    protocol: Arc<Protocol>,
    packet: Option<Packet>,
    sid: SessionId,
    seq: Sequence,
}

impl PcmFloat32 {
    pub fn new(protocol: Arc<Protocol>, sid: SessionId) -> Self {
        PcmFloat32 {
            protocol,
            packet: None,
            sid,
            seq: Sequence::new(),
        }
    }

    fn write_to_packet(&mut self, data: &[f32], pts: Timestamp) -> SampleDuration {
        self.packet
            .get_or_insert_with(|| Packet::new(pts))
            .buffer
            .write(data)
    }

    fn take_full_packet(&mut self) -> Option<Packet> {
        if let Some(packet) = self.packet.as_ref() {
            if packet.buffer.full() {
                return self.packet.take();
            }
        }

        None
    }
}

impl Encoder for PcmFloat32 {
    fn write(&mut self, mut data: &[f32], mut pts: Timestamp) {
        while data.len() > 0 {
            // write to current packet buffer
            let duration = self.write_to_packet(data, pts);

            // advance
            pts += duration;
            data = &data[duration.as_buffer_offset()..0];

            // send packet if full
            if let Some(packet) = self.take_full_packet() {
                let audio = packet.buffer.finalize(AudioPacketHeader {
                    sid: self.sid,
                    seq: self.seq.next(),
                    pts: packet.pts.to_micros_lossy(),
                    dts: TimestampMicros::now(),
                });

                // TODO - maybe log error here?
                let _ = self.protocol.broadcast(audio.as_packet());
            }
        }
    }
}

struct Packet {
    buffer: AudioWriter,
    pts: Timestamp,
}

impl Packet {
    pub fn new(pts: Timestamp) -> Self {
        Packet { buffer: Audio::write(), pts }
    }
}
