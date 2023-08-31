use std::collections::VecDeque;

pub struct PacketQueue {
    queue: VecDeque<PacketSlot>,
}

struct PacketSlot {
    seq: u64,
    pts: Option<Timestamp>,
    consumed: SampleDuration,
    audio: Option<AudioBuffer>,
}
