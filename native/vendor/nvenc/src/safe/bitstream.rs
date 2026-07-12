use std::{ffi::c_void, sync::Arc};

use crate::{safe::encoder::EncoderInternal, sys::result::NVencError};

pub struct BitStream {
    pub(crate) buffer: *mut c_void,
    pub(crate) encoder: Arc<EncoderInternal>,
}

impl Drop for BitStream {
    fn drop(&mut self) {
        self.encoder.destroy_bitstream_buffer(self.buffer).unwrap();
    }
}

unsafe impl Send for BitStream {}

impl BitStream {
    /// Attempts to lock the bit stream, if `wait` is true it will wait
    /// otherwise a `LockBusy` Error may be returned, in which case the
    /// client should retry in a few milliseconds
    pub fn try_lock(&self, wait: bool) -> Result<BitStreamLockGuard<'_>, NVencError> {
        let lock = self.encoder.lock_bit_stream_buffer(self.buffer, wait)?;
        Ok(BitStreamLockGuard {
            buffer: self,
            data_ptr: lock.bitstream_buffer,
            data_len: lock.bitstream_size_in_bytes,
            output_time_stamp: lock.output_time_stamp,
            output_duration: lock.output_duration,
            frame_idx: lock.frame_idx,
            frame_idx_display: lock.frame_idx_display,
            picture_type: lock.picture_type,
        })
    }
}

/// Holds a reference to the `BitStream` and holds the data and associated fields
pub struct BitStreamLockGuard<'a> {
    buffer: &'a BitStream,
    data_ptr: *mut c_void,
    data_len: u32,
    output_time_stamp: u64,
    output_duration: u64,
    frame_idx: u32,
    frame_idx_display: u32,
    picture_type: crate::sys::enums::NVencPicType,
}

impl BitStreamLockGuard<'_> {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.data_ptr as _, self.data_len as _) }
    }
    pub fn output_time_stamp(&self) -> u64 {
        self.output_time_stamp
    }
    pub fn output_duration(&self) -> u64 {
        self.output_duration
    }
    pub fn frame_idx(&self) -> u32 {
        self.frame_idx
    }
    pub fn frame_idx_display(&self) -> u32 {
        self.frame_idx_display
    }
    pub fn picture_type(&self) -> crate::sys::enums::NVencPicType {
        self.picture_type
    }
}

impl<'a> Drop for BitStreamLockGuard<'a> {
    fn drop(&mut self) {
        self.buffer
            .encoder
            .unlock_bit_stream_buffer(self.buffer.buffer)
            .unwrap();
    }
}
