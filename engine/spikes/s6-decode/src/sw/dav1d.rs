//! dav1d decoding straight into Metal-shared pool slots through
//! `Dav1dPicAllocator` (FFmpeg's libdav1d wrapper uses its own buffer pool and
//! would need a copy).

use super::SwDecoder;
use super::sys::*;
use crate::ff::Packet;
use crate::pool::{MetalPool, PoolFrame, PoolPlane, SlotRef};
use anyhow::{Result, bail, ensure};
use std::ffi::{c_int, c_void};
use std::sync::Arc;

const EAGAIN: c_int = -(libc::EAGAIN);
/// Metal's linear-texture alignment is 16 bytes on Apple silicon; 256 keeps
/// every plane offset and stride valid on any Metal device.
const ALIGN: usize = 256;

struct Cookie {
    pool: Arc<MetalPool>,
}

unsafe extern "C" fn alloc_picture(pic: *mut Dav1dPicture, cookie: *mut c_void) -> c_int {
    unsafe {
        let c = &*(cookie as *const Cookie);
        let p = &(*pic).p;
        let hbd = (p.bpc > 8) as usize;
        let aligned_w = (p.w as usize + 127) & !127;
        let aligned_h = (p.h as usize + 127) & !127;
        let has_chroma = p.layout != DAV1D_PIXEL_LAYOUT_I400;
        let ss_hor = (p.layout != DAV1D_PIXEL_LAYOUT_I444) as usize;
        let ss_ver = (p.layout == DAV1D_PIXEL_LAYOUT_I420) as usize;
        let round = |v: usize| {
            let v = v.div_ceil(ALIGN) * ALIGN;
            // As dav1d's default allocator: avoid strides that are multiples of
            // 1024, which alias cache sets across superblock rows.
            if v.is_multiple_of(1024) { v + ALIGN } else { v }
        };
        let y_stride = round(aligned_w << hbd);
        let uv_stride = if has_chroma { round((aligned_w >> ss_hor) << hbd) } else { 0 };
        let y_sz = y_stride * aligned_h;
        let uv_sz = uv_stride * (aligned_h >> ss_ver);
        let Some(slot) = c.pool.acquire(y_sz + 2 * uv_sz + DAV1D_PICTURE_ALIGNMENT as usize) else {
            return -(libc::ENOMEM);
        };
        let base = slot.ptr();
        (*pic).data[0] = base as *mut c_void;
        (*pic).data[1] = if has_chroma { base.add(y_sz) as *mut c_void } else { std::ptr::null_mut() };
        (*pic).data[2] = if has_chroma { base.add(y_sz + uv_sz) as *mut c_void } else { std::ptr::null_mut() };
        (*pic).stride[0] = y_stride as isize;
        (*pic).stride[1] = uv_stride as isize;
        (*pic).allocator_data = Box::into_raw(Box::new(slot)) as *mut c_void;
        0
    }
}

unsafe extern "C" fn release_picture(pic: *mut Dav1dPicture, _cookie: *mut c_void) {
    unsafe {
        let a = (*pic).allocator_data as *mut SlotRef;
        if !a.is_null() {
            drop(Box::from_raw(a));
            (*pic).allocator_data = std::ptr::null_mut();
        }
    }
}

/// Holds a dav1d picture reference while its planes are in use.
struct PicOwner(Dav1dPicture);

unsafe impl Send for PicOwner {}

impl Drop for PicOwner {
    fn drop(&mut self) {
        unsafe { dav1d_picture_unref(&mut self.0) }
    }
}

pub struct Dav1dDecoder {
    ctx: *mut Dav1dContext,
    _cookie: Box<Cookie>,
}

unsafe impl Send for Dav1dDecoder {}

impl Dav1dDecoder {
    pub fn new(pool: Arc<MetalPool>, threads: i32) -> Result<Self> {
        unsafe {
            let cookie = Box::new(Cookie { pool });
            let mut s: Dav1dSettings = std::mem::zeroed();
            dav1d_default_settings(&mut s);
            s.n_threads = threads;
            s.allocator = Dav1dPicAllocator {
                cookie: &*cookie as *const Cookie as *mut c_void,
                alloc_picture_callback: Some(alloc_picture),
                release_picture_callback: Some(release_picture),
            };
            let mut ctx: *mut Dav1dContext = std::ptr::null_mut();
            let r = dav1d_open(&mut ctx, &s);
            ensure!(r == 0 && !ctx.is_null(), "dav1d_open: {r}");
            Ok(Dav1dDecoder { ctx, _cookie: cookie })
        }
    }

    fn drain(&mut self, out: &mut Vec<(i64, PoolFrame)>) -> Result<()> {
        loop {
            let mut pic: Dav1dPicture = unsafe { std::mem::zeroed() };
            let r = unsafe { dav1d_get_picture(self.ctx, &mut pic) };
            if r == EAGAIN {
                return Ok(());
            }
            ensure!(r == 0, "dav1d_get_picture: {r}");
            out.push((pic.m.timestamp, adopt(pic)?));
        }
    }
}

fn adopt(pic: Dav1dPicture) -> Result<PoolFrame> {
    unsafe {
        let a = pic.allocator_data as *const SlotRef;
        if a.is_null() {
            bail!("dav1d picture not from the pool allocator");
        }
        let slot = (*a).clone();
        let p = pic.p;
        let hbd = p.bpc > 8;
        let bpp = if hbd { 2 } else { 1 };
        let (lw, lh) = match p.layout {
            DAV1D_PIXEL_LAYOUT_I420 => (1u8, 1u8),
            DAV1D_PIXEL_LAYOUT_I422 => (1, 0),
            DAV1D_PIXEL_LAYOUT_I444 => (0, 0),
            _ => bail!("monochrome AV1 not handled by the spike"),
        };
        let (w, h) = (p.w as usize, p.h as usize);
        let plane = |i: usize, pw: usize, ph: usize, stride: isize| {
            let ptr = pic.data[i] as *const u8;
            PoolPlane {
                slot: slot.clone(),
                offset: ptr.offset_from(slot.ptr()) as usize,
                stride: stride as usize,
                width: pw,
                height: ph,
                bytes_per_px: bpp,
                decoder_ptr: ptr as usize,
            }
        };
        let (cw, ch) = ((w + (1 << lw) - 1) >> lw, (h + (1 << lh) - 1) >> lh);
        let planes =
            vec![plane(0, w, h, pic.stride[0]), plane(1, cw, ch, pic.stride[1]), plane(2, cw, ch, pic.stride[1])];
        let full_range = !pic.seq_hdr.is_null() && (*pic.seq_hdr).color_range != 0;
        let sub = match (lw, lh) {
            (1, 1) => "420",
            (1, 0) => "422",
            _ => "444",
        };
        Ok(PoolFrame {
            planes,
            depth: p.bpc as u8,
            log2_chroma_w: lw,
            log2_chroma_h: lh,
            full_range,
            label: format!("yuv{sub}p{}", if hbd { format!("{}le", p.bpc) } else { String::new() }),
            copied: false,
            owner: Some(Box::new(PicOwner(pic))),
        })
    }
}

impl SwDecoder for Dav1dDecoder {
    fn send(&mut self, p: Option<&Packet>, out: &mut Vec<(i64, PoolFrame)>) -> Result<()> {
        let Some(p) = p else { return self.drain(out) };
        unsafe {
            let mut data: Dav1dData = std::mem::zeroed();
            let buf = dav1d_data_create(&mut data, p.data.len());
            ensure!(!buf.is_null(), "dav1d_data_create");
            std::ptr::copy_nonoverlapping(p.data.as_ptr(), buf, p.data.len());
            data.m.timestamp = p.pts;
            data.m.duration = p.duration;
            while data.sz > 0 {
                let r = dav1d_send_data(self.ctx, &mut data);
                if r == EAGAIN {
                    // Output first, then the rest of this packet.
                    self.drain(out)?;
                    continue;
                }
                if r != 0 {
                    dav1d_data_unref(&mut data);
                    bail!("dav1d_send_data: {r}");
                }
            }
        }
        self.drain(out)
    }
    fn name(&self) -> String {
        "dav1d".into()
    }
    fn flush(&mut self) {
        unsafe { dav1d_flush(self.ctx) }
    }
    fn direct(&self) -> bool {
        true
    }
    fn copies(&self) -> usize {
        0
    }
}

impl Drop for Dav1dDecoder {
    fn drop(&mut self) {
        unsafe { dav1d_close(&mut self.ctx) }
    }
}
