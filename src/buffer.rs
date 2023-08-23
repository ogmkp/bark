use std::fmt::Debug;
use std::ops::{Deref, DerefMut};

use crate::protocol;
use crate::protocol::packet::Audio;
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

    pub fn offset(self, offset: usize) -> ByteBuffer {
        let length = self.len()
            .checked_sub(offset)
            .expect("offset > length in ByteBuffer::offset");

        ByteBuffer {
            alloc: self.alloc,
            offset: self.offset + offset,
            length,
        }
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
    pub fn from_buffer(buffer: ByteBuffer) -> Self {
        let samples = match bytemuck::try_cast_slice::<u8, f32>(&buffer) {
            Ok(samples) => samples,
            Err(e) => { panic!("failed to convert ByteBuffer to AudioBuffer: {e:?}") }
        };

        if (samples.len() % protocol::CHANNELS as usize) != 0 {
            panic!("sample buffer length not divisible by channel count");
        }

        AudioBuffer(buffer)
    }

    pub fn duration(&self) -> SampleDuration {
        SampleDuration::from_buffer_offset(self.samples().len())
    }

    pub fn samples(&self) -> &[f32] {
        bytemuck::cast_slice(&self.0)
    }
}
