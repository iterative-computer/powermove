//! Software decoders behind one interface: FFmpeg (direct rendering through
//! `get_buffer2` where the decoder supports it), and dav1d and libvpx driven
//! directly with their own frame allocators pointed at the Metal-shared pool.

pub mod dav1d;
pub mod vpx;

use crate::ff::Packet;
use crate::pool::PoolFrame;
use anyhow::Result;

#[allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code, clippy::all)]
pub mod sys {
    include!(concat!(env!("OUT_DIR"), "/swdec.rs"));
}

pub trait SwDecoder: Send {
    /// Sends one packet (None drains) and appends every frame now available,
    /// with its presentation timestamp.
    fn send(&mut self, p: Option<&Packet>, out: &mut Vec<(i64, PoolFrame)>) -> Result<()>;
    fn name(&self) -> String;
    /// Whether the decoder renders into pool memory itself.
    fn direct(&self) -> bool;
    /// Frames the spike had to copy into pool memory.
    fn copies(&self) -> usize;
}

impl SwDecoder for crate::ff::FfDecoder {
    fn send(&mut self, p: Option<&Packet>, out: &mut Vec<(i64, PoolFrame)>) -> Result<()> {
        crate::ff::FfDecoder::send(self, p, out)
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn direct(&self) -> bool {
        self.dr1
    }
    fn copies(&self) -> usize {
        self.copies
    }
}
