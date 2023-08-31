use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use crate::util;
use crate::time::{SampleDuration, Timestamp, TimestampDelta};

pub fn create(delay: SampleDuration) -> (StreamWriter, StreamReader) {
    let shared = Arc::new(Shared {
        buffer_size: delay.as_buffer_offset(),
        locked: Mutex::new(Locked {
            buffer: VecDeque::with_capacity(delay.as_buffer_offset()),
            spans: VecDeque::new(),
            offset: None,
        }),
        cond: Condvar::new(),
    });

    let writer = StreamWriter {
        shared: shared.clone(),
    };

    let reader = StreamReader {
        shared: shared.clone(),
    };

    (writer, reader)
}

pub struct StreamWriter {
    shared: Arc<Shared>,
}

impl StreamWriter {
    pub fn offset(&self) -> Option<TimestampDelta> {
        let locked = self.shared.locked.lock().unwrap();
        locked.offset
    }
}

pub struct StreamReader {
    shared: Arc<Shared>,
}

struct Shared {
    // we could use buffer.capacity(), but this could lead to hard-to-debug
    // increases in audio latency if we ever accidently expand the buffer
    buffer_size: usize,
    locked: Mutex<Locked>,
    cond: Condvar,
}

impl Shared {
    pub fn buffer_duration(&self) -> SampleDuration {
        SampleDuration::from_buffer_offset(self.buffer_size)
    }
}

struct Locked {
    buffer: VecDeque<f32>,
    spans: VecDeque<Span>,
    offset: Option<TimestampDelta>,
}

impl Locked {
    pub fn front_mut(&mut self) -> Option<&mut Span> {
        loop {
            let span = self.spans.front()?;

            if !span.remaining.is_zero() {
                return self.spans.front_mut();
            }

            self.spans.pop_front();
        }
    }

    pub fn push_audio(&mut self, timestamp: Timestamp, samples: &[f32]) -> SampleDuration {
        let duration = SampleDuration::from_buffer_offset(samples.len());

        self.buffer.extend(samples);
        self.spans.push_back(Span {
            timestamp: Some(timestamp),
            remaining: duration,
        });

        duration
    }

    pub fn push_silence(&mut self, duration: SampleDuration) {
        self.buffer.extend(std::iter::repeat(0f32)
            .take(duration.as_buffer_offset()));

        self.spans.push_back(Span {
            timestamp: None,
            remaining: duration,
        });
    }
}

struct Span {
    timestamp: Option<Timestamp>,
    remaining: SampleDuration,
}

impl Span {
    pub fn end(&self) -> Option<Timestamp> {
        self.timestamp.map(|ts| ts + self.remaining)
    }

    pub fn consume(&mut self, duration: SampleDuration) {
        assert!(duration <= self.remaining);

        self.remaining -= duration;

        if let Some(ts) = &mut self.timestamp {
            *ts += duration;
        }
    }
}

impl StreamWriter {
    pub fn write(&mut self, mut pts: Timestamp, mut samples: &[f32]) {
        assert!(util::frame_aligned_buffer(samples));

        let mut locked = self.shared.locked.lock().unwrap();
        let buffer_size = self.shared.buffer_size;

        while samples.len() > 0 {
            let buffer_free = buffer_size - locked.buffer.len();
            let copy_len = std::cmp::min(samples.len(), buffer_free);
            let (source, next) = samples.split_at(copy_len);

            // copy samples into buffer
            pts += locked.push_audio(pts, &source);

            // this releases the mutex while we wait for the condition:
            locked = self.shared.cond.wait(locked).unwrap();

            // advance around loop
            samples = next;
        }
    }
}

impl StreamReader {
    pub fn read(&mut self, pts: Timestamp, mut samples: &mut [f32]) {
        assert!(util::frame_aligned_buffer(samples));

        let mut locked = self.shared.locked.lock().unwrap();

        // set playback offset
        locked.offset = locked.spans.front()
            .and_then(|span| span.timestamp)
            .map(|span_ts| pts.delta(span_ts));

        // copy audio to output buffer
        while samples.len() > 0 {
            let Some(span) = locked.front_mut() else {
                // underrun! TODO report

                // fill remaining output samples with zeroes
                samples.fill(0f32);

                // and re-fill buffer with silence again to give the stream
                // thread a chance to catch up
                locked.push_silence(self.shared.buffer_duration());

                break;
            };

            let output_duration = SampleDuration::from_buffer_offset(samples.len());
            let copy_duration = std::cmp::min(output_duration, span.remaining);
            let (copy, next) = samples.split_at_mut(copy_duration.as_buffer_offset());

            for dest in copy.iter_mut() {
                *dest = locked.buffer.pop_front().unwrap();
            }

            samples = next;
        }

        // notify writers we've read some data
        self.shared.cond.notify_all();
    }
}
