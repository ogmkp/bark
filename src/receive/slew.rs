use std::time::Duration;

use cpal::SampleRate;
use derive_more::From;

use crate::buffer::AudioBuffer;
use crate::protocol::{self, SAMPLES_PER_PACKET};
use crate::resample::{Resampler, SpeexError};
use crate::receive::output::Output;
use crate::time::{Timestamp, TimestampDelta, SampleDuration};

// these are u16 so we can always cast them into i64 and usize,
// which is what we actually need
const MIN_PLAYBACK_RATE_PERCENT: u16 = 98;
const MAX_PLAYBACK_RATE_PERCENT: u16 = 200;

pub struct Slew {
    output: Output,
    resample: Resampler,
    rate: RateAdjust,
}

#[derive(Debug, From)]
pub enum SlewError {
    Speex(SpeexError),
}

impl Slew {
    pub fn new(output: Output) -> Self {
        Slew {
            output,
            resample: Resampler::new(),
            rate: RateAdjust::new(),
        }
    }

    pub fn output(&mut self) -> &mut Output {
        &mut self.output
    }

    pub fn write(&mut self, mut pts: Timestamp, mut audio: AudioBuffer) -> Result<(), SlewError> {
        // calculate playback rate based on current output offset
        let rate = self.output.offset()
            .and_then(|offset| self.rate.calculate(offset))
            .unwrap_or(protocol::SAMPLE_RATE);

        let mut buffer = [0f32; SAMPLES_PER_PACKET];

        while !audio.is_empty() {
            // resample
            let _ = self.resample.set_input_rate(rate.0);
            let process = self.resample.process_interleaved(audio.samples(), &mut buffer)?;

            // write out
            self.output.write(pts, &buffer[0..process.output_written.as_buffer_offset()]);

            // advance
            let duration = process.input_read;
            pts += duration;
            audio.consume_duration(duration);
        }

        Ok(())
    }
}

pub struct RateAdjust {
    slew: bool,
}

impl RateAdjust {
    pub fn new() -> Self {
        RateAdjust {
            slew: false
        }
    }

    pub fn slew(&self) -> bool {
        self.slew
    }

    pub fn calculate(&mut self, offset: TimestampDelta) -> Option<SampleRate> {
        // parameters, maybe these could be cli args?
        let start_slew_threshold = Duration::from_micros(2000);
        let stop_slew_threshold = Duration::from_micros(100);
        let slew_target_duration = Duration::from_millis(500);

        // turn them into native units
        let start_slew_threshold = SampleDuration::from_std_duration_lossy(start_slew_threshold);
        let stop_slew_threshold = SampleDuration::from_std_duration_lossy(stop_slew_threshold);

        if offset.abs() < stop_slew_threshold {
            self.slew = false;
            return None;
        }

        if offset.abs() < start_slew_threshold && !self.slew {
            return None;
        }

        let slew_duration_duration = i64::try_from(slew_target_duration.as_micros()).unwrap();
        let base_sample_rate = i64::from(protocol::SAMPLE_RATE.0);
        let rate_offset = offset.as_frames() * 1_000_000 / slew_duration_duration;
        let rate = base_sample_rate + rate_offset;

        // clamp any potential slow down to 2%, we shouldn't ever get too far
        // ahead of the stream
        let rate = std::cmp::max(base_sample_rate * i64::from(MIN_PLAYBACK_RATE_PERCENT) / 100, rate);

        // let the speed up run much higher, but keep it reasonable still
        let rate = std::cmp::min(base_sample_rate * i64::from(MAX_PLAYBACK_RATE_PERCENT) / 100, rate);

        self.slew = true;
        Some(SampleRate(u32::try_from(rate).unwrap()))
    }
}
