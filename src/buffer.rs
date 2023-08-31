use std::fmt::Debug;
use std::ops::{Deref, DerefMut};

use crate::protocol;
use crate::time::SampleDuration;

pub struct ByteBuffer {
    alloc: Box<[u8]>,
    offset: usize,
    length: usize,
}

impl ByteBuffer {
    pub fn allocate(capacity: usize) -> Self {
        let alloc = bytemuck::allocation::zeroed_slice_box(capacity);
        ByteBuffer { alloc, offset: 0, length: 0 }
    }

    pub fn len(&self) -> usize {
        self.length
    }

    pub fn capacity(&self) -> usize {
        self.alloc.len()
            .checked_sub(self.offset)
            .expect("offset > capacity in ByteBuffer")
    }

    pub fn set_len(&mut self, len: usize) {
        let cap = self.capacity();
        if len > cap {
            panic!("would set byte buffer length greater than capacity: {len} > {cap}");
        }

        self.length = len;
    }

    pub fn offset(mut self, offset: usize) -> ByteBuffer {
        self.offset_in_place(offset);
        self
    }

    pub fn offset_in_place(&mut self, offset: usize) {
        let length = self.len()
            .checked_sub(offset)
            .expect("offset > length in ByteBuffer::offset");

        self.offset += offset;
        self.length = length;
    }

    pub fn as_full_buffer_mut(&mut self) -> &mut [u8] {
        &mut self.alloc[self.offset..]
    }
}

impl Debug for ByteBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

impl Deref for ByteBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.alloc[self.offset..][0..self.length]
    }
}

impl DerefMut for ByteBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.alloc[self.offset..][0..self.length]
    }
}

pub struct AudioBuffer(ByteBuffer);

impl AudioBuffer {
    pub fn allocate(duration: SampleDuration) -> Self {
        AudioBuffer(ByteBuffer::allocate(duration_to_byte_offset(duration)))
    }

    pub fn from_buffer(buffer: ByteBuffer) -> Self {
        let samples = match bytemuck::try_cast_slice::<u8, f32>(&buffer) {
            Ok(samples) => samples,
            Err(e) => { panic!("failed to convert ByteBuffer to AudioBuffer: {e:?}") }
        };

        assert_whole_frames(samples);

        AudioBuffer(buffer)
    }

    pub fn empty() -> Self {
        // there is underlying logic in the rust allocator that skips
        // allocation for empty slices:
        let buffer = ByteBuffer::allocate(0);
        AudioBuffer(buffer)
    }

    pub fn is_empty(&self) -> bool {
        self.samples().len() == 0
    }

    pub fn duration(&self) -> SampleDuration {
        SampleDuration::from_buffer_offset(self.samples().len())
    }

    pub fn samples(&self) -> &[f32] {
        bytemuck::cast_slice(&self.0)
    }

    pub fn consume_duration(&mut self, duration: SampleDuration) {
        self.0.offset_in_place(duration_to_byte_offset(duration));
    }

    /// Consumes from the front of this buffer into the output buffer,
    /// returning the duration of audio copied.
    pub fn drain_to(&mut self, output: &mut [f32]) -> SampleDuration {
        assert_whole_frames(output);

        let output_duration = SampleDuration::from_buffer_offset(output.len());
        let copy_duration = std::cmp::min(self.duration(), output_duration);
        let copy_length = copy_duration.as_buffer_offset();

        output[0..copy_length].copy_from_slice(&self.samples()[0..copy_length]);

        self.consume_duration(copy_duration);
        copy_duration
    }
}

fn assert_whole_frames(buffer: &[f32]) {
    if (buffer.len() % protocol::CHANNELS as usize) != 0 {
        panic!("sample buffer length not divisible by channel count");
    }
}

fn duration_to_byte_offset(duration: SampleDuration) -> usize {
    duration.as_buffer_offset() * std::mem::size_of::<f32>()
}
