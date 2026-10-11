//! Metal-shared frame memory for software decoders.
//!
//! Each slot is page-aligned host memory wrapped once in an `MTLBuffer`
//! (`newBufferWithBytesNoCopy`, shared storage). FFmpeg decoders with direct
//! rendering write their output planes straight into a slot through
//! `get_buffer2`; each plane is then a linear `MTLTexture` over that buffer. On
//! Apple silicon's unified memory the GPU samples the bytes the decoder wrote:
//! no upload and no copy.

use ffmpeg_sys_the_third as ff;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLBuffer, MTLDevice, MTLResourceOptions};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, Weak};

pub const PAGE: usize = 16384;
/// Row and plane alignment; covers Metal's linear-texture alignment and every
/// FFmpeg `linesize_align`.
pub const ROW_ALIGN: usize = 256;

struct Slot {
    ptr: *mut u8,
    len: usize,
    buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
}

unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

impl Drop for Slot {
    fn drop(&mut self) {
        unsafe { libc::free(self.ptr as *mut libc::c_void) }
    }
}

struct Lease {
    slot: Option<Arc<Slot>>,
    pool: Weak<PoolInner>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let slot = self.slot.take().unwrap();
        if let Some(p) = self.pool.upgrade() {
            let mut g = p.state.lock().unwrap();
            g.live.remove(&(slot.ptr as usize));
            g.free.push(slot);
        }
    }
}

/// A leased slot; the slot returns to the pool when the last clone drops.
#[derive(Clone)]
pub struct SlotRef(Arc<Lease>);

impl SlotRef {
    pub fn ptr(&self) -> *mut u8 {
        self.0.slot.as_ref().unwrap().ptr
    }
    pub fn len(&self) -> usize {
        self.0.slot.as_ref().unwrap().len
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn buffer(&self) -> &ProtocolObject<dyn MTLBuffer> {
        &self.0.slot.as_ref().unwrap().buffer
    }
}

#[derive(Default)]
struct State {
    free: Vec<Arc<Slot>>,
    live: HashMap<usize, Weak<Lease>>,
    allocated: usize,
    bytes: usize,
}

struct PoolInner {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    state: Mutex<State>,
    /// Zero slots when first allocated (libvpx requires zeroed frame buffers;
    /// like FFmpeg's wrapper, reused slots are not cleared again).
    zero_new: bool,
}

unsafe impl Send for PoolInner {}
unsafe impl Sync for PoolInner {}

pub struct MetalPool(Arc<PoolInner>);

impl MetalPool {
    pub fn new(device: Retained<ProtocolObject<dyn MTLDevice>>) -> Arc<Self> {
        Arc::new(MetalPool(Arc::new(PoolInner { device, state: Mutex::new(State::default()), zero_new: false })))
    }

    /// A pool whose fresh slots are zero-filled.
    pub fn new_zeroed(device: Retained<ProtocolObject<dyn MTLDevice>>) -> Arc<Self> {
        Arc::new(MetalPool(Arc::new(PoolInner { device, state: Mutex::new(State::default()), zero_new: true })))
    }

    pub fn device(&self) -> Retained<ProtocolObject<dyn MTLDevice>> {
        self.0.device.clone()
    }

    pub fn acquire(&self, len: usize) -> Option<SlotRef> {
        let len = len.div_ceil(PAGE) * PAGE;
        let mut g = self.0.state.lock().unwrap();
        let slot = match g.free.iter().position(|s| s.len == len) {
            Some(i) => g.free.swap_remove(i),
            None => {
                let mut p: *mut libc::c_void = std::ptr::null_mut();
                if unsafe { libc::posix_memalign(&mut p, PAGE, len) } != 0 {
                    return None;
                }
                if self.0.zero_new {
                    unsafe { std::ptr::write_bytes(p as *mut u8, 0, len) };
                }
                let buffer = unsafe {
                    self.0.device.newBufferWithBytesNoCopy_length_options_deallocator(
                        NonNull::new(p)?,
                        len,
                        MTLResourceOptions::StorageModeShared,
                        None,
                    )
                };
                let Some(buffer) = buffer else {
                    unsafe { libc::free(p) };
                    return None;
                };
                g.allocated += 1;
                g.bytes += len;
                Arc::new(Slot { ptr: p as *mut u8, len, buffer })
            }
        };
        let key = slot.ptr as usize;
        let lease = Arc::new(Lease { slot: Some(slot), pool: Arc::downgrade(&self.0) });
        g.live.insert(key, Arc::downgrade(&lease));
        Some(SlotRef(lease))
    }

    /// The live lease whose slot starts at `base`, if any.
    pub fn find(&self, base: *const u8) -> Option<SlotRef> {
        let g = self.0.state.lock().unwrap();
        g.live.get(&(base as usize)).and_then(|w| w.upgrade()).map(SlotRef)
    }

    pub fn stats(&self) -> (usize, usize) {
        let g = self.0.state.lock().unwrap();
        (g.allocated, g.bytes)
    }
}

/// One decoded plane in pool memory.
#[derive(Clone)]
pub struct PoolPlane {
    pub slot: SlotRef,
    pub offset: usize,
    pub stride: usize,
    /// Visible size in samples.
    pub width: usize,
    pub height: usize,
    pub bytes_per_px: usize,
    /// The address the decoder reported for this plane (AVFrame.data,
    /// Dav1dPicture.data, vpx_image_t.planes), used to verify the GPU texture
    /// aliases what the decoder wrote.
    pub decoder_ptr: usize,
}

impl PoolPlane {
    pub fn ptr(&self) -> *const u8 {
        unsafe { self.slot.ptr().add(self.offset) }
    }
}

/// A decoded planar Y′CbCr(A) frame whose planes live in pool slots (one slot
/// per decoder buffer: libvpx's alpha comes from a second decoder).
pub struct PoolFrame {
    /// Y′, Cb, Cr, then A when present.
    pub planes: Vec<PoolPlane>,
    pub depth: u8,
    pub log2_chroma_w: u8,
    pub log2_chroma_h: u8,
    pub full_range: bool,
    /// Pixel format name for reports (FFmpeg naming).
    pub label: String,
    /// The pixels were copied into the pool by the spike (decoder without
    /// direct rendering): not zero-copy.
    pub copied: bool,
    /// Keeps the decoder's own frame reference alive (AVFrame, Dav1dPicture).
    pub owner: Option<Box<dyn Send>>,
}

#[derive(Clone, Debug)]
pub struct PlaneLayout {
    pub offset: usize,
    pub stride: usize,
    /// Visible size in samples of this plane.
    pub width: usize,
    pub height: usize,
    pub bytes_per_px: usize,
    pub log2_w: u8,
    pub log2_h: u8,
}

/// Planar Y′CbCr(A) layout of one FFmpeg frame (each component on its own plane).
#[derive(Clone, Debug)]
pub struct FrameLayout {
    pub format: i32,
    pub depth: u8,
    pub has_alpha: bool,
    pub planes: Vec<PlaneLayout>,
    pub total: usize,
}

impl FrameLayout {
    pub fn for_format(fmt: ff::AVPixelFormat, width: i32, height: i32) -> Option<FrameLayout> {
        unsafe {
            let d = ff::av_pix_fmt_desc_get(fmt);
            if d.is_null() {
                return None;
            }
            let d = &*d;
            let flags = d.flags as i32;
            if flags & (ff::AV_PIX_FMT_FLAG_RGB | ff::AV_PIX_FMT_FLAG_HWACCEL) != 0 {
                return None;
            }
            let n = d.nb_components as usize;
            let depth = d.comp[0].depth as u8;
            if depth > 8 && flags & ff::AV_PIX_FMT_FLAG_BE != 0 {
                return None;
            }
            if n > 1 && flags & ff::AV_PIX_FMT_FLAG_PLANAR == 0 {
                return None;
            }
            let mut planes = Vec::new();
            for i in 0..n {
                let c = d.comp[i];
                // One component per plane, in order, LSB-aligned, no packing.
                if c.plane as usize != i || c.offset != 0 || c.shift != 0 || c.depth as u8 != depth {
                    return None;
                }
                let chroma = i == 1 || i == 2;
                let (lw, lh) = if chroma { (d.log2_chroma_w, d.log2_chroma_h) } else { (0, 0) };
                planes.push(PlaneLayout {
                    offset: 0,
                    stride: 0,
                    width: ceil_rshift(width as usize, lw),
                    height: ceil_rshift(height as usize, lh),
                    bytes_per_px: c.step as usize,
                    log2_w: lw,
                    log2_h: lh,
                });
            }
            let mut l = FrameLayout {
                format: fmt.0,
                depth,
                has_alpha: flags & ff::AV_PIX_FMT_FLAG_ALPHA != 0,
                planes,
                total: 0,
            };
            l.pad_to(width as usize, height as usize, &[0; 8]);
            Some(l)
        }
    }

    /// Lays planes out for a coded size of `w` × `h` with FFmpeg's per-plane
    /// linesize alignment, keeping every offset and stride Metal-aligned.
    pub fn pad_to(&mut self, w: usize, h: usize, linesize_align: &[i32; 8]) {
        let mut off = 0;
        for (i, p) in self.planes.iter_mut().enumerate() {
            let a = ROW_ALIGN.max(linesize_align[i].max(1) as usize);
            let a = a.div_ceil(ROW_ALIGN) * ROW_ALIGN;
            p.stride = (ceil_rshift(w, p.log2_w) * p.bytes_per_px).div_ceil(a) * a;
            p.offset = off;
            off += p.stride * ceil_rshift(h, p.log2_h);
            off = off.div_ceil(ROW_ALIGN) * ROW_ALIGN;
        }
        // Tail padding for decoders that over-read the last row.
        self.total = (off + ROW_ALIGN * 4).div_ceil(PAGE) * PAGE;
    }
}

pub fn ceil_rshift(v: usize, s: u8) -> usize {
    (v + (1 << s) - 1) >> s
}
