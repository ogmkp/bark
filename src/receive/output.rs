use std::time::Duration;

use cpal::{Stream, BuildStreamError};
use cpal::{traits::DeviceTrait, StreamConfig, OutputCallbackInfo, StreamError};

use crate::time::{Timestamp, SampleDuration, TimestampDelta};
use crate::receive::buffer::{self, StreamWriter};

pub struct Output {
    tx: StreamWriter,
    stream: Stream,
}

pub struct OutputConfig {
    pub device: cpal::Device,
    pub stream: StreamConfig,
    pub buffer_delay: Duration,
}

impl Output {
    pub fn new(config: &OutputConfig) -> Result<Output, BuildStreamError> {
        let buffer_size = SampleDuration::from_std_duration_lossy(config.buffer_delay);
        let (tx, mut rx) = buffer::create(buffer_size);

        let stream = config.device.build_output_stream(&config.stream,
            {
                let mut initialized_thread = false;

                move |output: &mut [f32], info: &OutputCallbackInfo| {
                    if !initialized_thread {
                        crate::thread::set_name("bark/audio");
                        crate::thread::set_realtime_priority();
                        initialized_thread = true;
                    }

                    let output_ts = Timestamp::now() + output_latency(info);

                    rx.read(output_ts, output);
                }
            },
            {
                move |err: StreamError| {
                    panic!("stream error! {err:?}");
                }
            },
            None,
        )?;

        Ok(Output {
            tx,
            stream,
        })
    }

    pub fn write(&mut self, pts: Timestamp, samples: &[f32]) {
        self.tx.write(pts, samples);
    }

    pub fn offset(&self) -> Option<TimestampDelta> {
        self.tx.offset()
    }
}

fn output_latency(info: &OutputCallbackInfo) -> SampleDuration {
    let timing = info.timestamp();

    let latency = timing.playback
        .duration_since(&timing.callback)
        .unwrap_or_default();

    SampleDuration::from_std_duration_lossy(latency)
}
