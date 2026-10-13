//! AV1 through dav1d directly (section 26 amendment 6), decoding straight
//! into pool slots through `Dav1dPicAllocator`. FFmpeg's libdav1d wrapper
//! decodes into dav1d's own buffer pool and would need a copy per frame (S6:
//! 1,980 and 2,970 copies on the corpus).

use crate::error::{MediaError, Result};
use crate::ff::Packet;
use crate::frame::{
    DecodedFrame, Layout, PixelFormat, Pixels, PlaneDesc, PlaneFormat, PoolFrame, PoolPlane,
};
use crate::pool::{FramePool, SlotRef, ceil_rshift};
use crate::sys::dav1d::*;
use pm_protocol::color::{Range, SampleAlignment};
use std::ffi::{c_int, c_void};
use std::sync::Arc;

const EAGAIN: c_int = -libc::EAGAIN;
/// Plane alignment: Metal's linear-texture alignment is 16 bytes on Apple
/// silicon; 256 keeps every offset and stride valid on any Metal device.
const ALIGN: usize = 256;

struct Cookie {
    pool: FramePool,
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
            // As dav1d's default allocator: strides that are multiples of
            // 1024 alias cache sets across superblock rows.
            if v.is_multiple_of(1024) { v + ALIGN } else { v }
        };
        let y_stride = round(aligned_w << hbd);
        let uv_stride = if has_chroma {
            round((aligned_w >> ss_hor) << hbd)
        } else {
            0
        };
        let y_sz = y_stride * aligned_h;
        let uv_sz = uv_stride * (aligned_h >> ss_ver);
        let Ok(slot) = c
            .pool
            .acquire(y_sz + 2 * uv_sz + DAV1D_PICTURE_ALIGNMENT as usize)
        else {
            return -libc::ENOMEM;
        };
        let base = slot.ptr();
        (*pic).data[0] = base as *mut c_void;
        (*pic).data[1] = if has_chroma {
            base.add(y_sz) as *mut c_void
        } else {
            std::ptr::null_mut()
        };
        (*pic).data[2] = if has_chroma {
            base.add(y_sz + uv_sz) as *mut c_void
        } else {
            std::ptr::null_mut()
        };
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
unsafe impl Sync for PicOwner {}

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
    pub fn new(pool: FramePool, threads: i32) -> Result<Self> {
        unsafe {
            let cookie = Box::new(Cookie { pool });
            let mut s: Dav1dSettings = std::mem::zeroed();
            dav1d_default_settings(&mut s);
            s.n_threads = threads.max(0);
            s.allocator = Dav1dPicAllocator {
                cookie: &*cookie as *const Cookie as *mut c_void,
                alloc_picture_callback: Some(alloc_picture),
                release_picture_callback: Some(release_picture),
            };
            let mut ctx: *mut Dav1dContext = std::ptr::null_mut();
            let r = dav1d_open(&mut ctx, &s);
            if r != 0 || ctx.is_null() {
                return Err(MediaError::Dav1d {
                    what: "open".into(),
                    code: r,
                });
            }
            Ok(Dav1dDecoder {
                ctx,
                _cookie: cookie,
            })
        }
    }

    /// Outputs every picture dav1d has ready. With no input pending, dav1d
    /// drains its frame threads, so after a `send` the sent frame is output.
    fn drain(&mut self, out: &mut Vec<DecodedFrame>) -> Result<()> {
        loop {
            let mut pic: Dav1dPicture = unsafe { std::mem::zeroed() };
            let r = unsafe { dav1d_get_picture(self.ctx, &mut pic) };
            if r == EAGAIN {
                return Ok(());
            }
            if r != 0 {
                return Err(MediaError::Dav1d {
                    what: "get picture".into(),
                    code: r,
                });
            }
            out.push(adopt(pic)?);
        }
    }

    pub fn send(&mut self, p: Option<&Packet>, out: &mut Vec<DecodedFrame>) -> Result<()> {
        let Some(p) = p else { return self.drain(out) };
        unsafe {
            let mut data: Dav1dData = std::mem::zeroed();
            let buf = dav1d_data_create(&mut data, p.data.len());
            if buf.is_null() {
                return Err(MediaError::OutOfMemory(p.data.len()));
            }
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
                    return Err(MediaError::Dav1d {
                        what: "send data".into(),
                        code: r,
                    });
                }
            }
        }
        self.drain(out)
    }

    pub fn flush(&mut self) {
        unsafe { dav1d_flush(self.ctx) }
    }
}

impl Drop for Dav1dDecoder {
    fn drop(&mut self) {
        unsafe { dav1d_close(&mut self.ctx) }
    }
}

fn adopt(pic: Dav1dPicture) -> Result<DecodedFrame> {
    unsafe {
        let owner = PicOwner(pic);
        let a = pic.allocator_data as *const SlotRef;
        if a.is_null() {
            return Err(MediaError::decode(
                "dav1d picture not from the pool allocator",
            ));
        }
        let slot = (*a).clone();
        let p = pic.p;
        let bits = p.bpc as u8;
        let (lw, lh) = match p.layout {
            DAV1D_PIXEL_LAYOUT_I420 => (1u8, 1u8),
            DAV1D_PIXEL_LAYOUT_I422 => (1, 0),
            DAV1D_PIXEL_LAYOUT_I444 => (0, 0),
            _ => return Err(MediaError::unsupported("monochrome AV1")),
        };
        let (w, h) = (p.w as usize, p.h as usize);
        let plane = |i: usize, stride: isize| {
            let ptr = pic.data[i] as *const u8;
            PoolPlane {
                slot: slot.clone(),
                offset: ptr.offset_from(slot.ptr()) as usize,
                stride: stride as usize,
                decoder_ptr: ptr as usize,
            }
        };
        let (cw, ch) = (ceil_rshift(w, lw), ceil_rshift(h, lh));
        let pf = if bits > 8 {
            PlaneFormat::R16
        } else {
            PlaneFormat::R8
        };
        let full = !pic.seq_hdr.is_null() && (*pic.seq_hdr).color_range != 0;
        let sub = match (lw, lh) {
            (1, 1) => "420",
            (1, 0) => "422",
            _ => "444",
        };
        let format = PixelFormat {
            layout: Layout::Planar,
            bits,
            alignment: SampleAlignment::Lsb,
            range: if full { Range::Full } else { Range::Limited },
            log2_chroma_w: lw,
            log2_chroma_h: lh,
            alpha_bits: 0,
            planes: vec![
                PlaneDesc {
                    format: pf,
                    width: w,
                    height: h,
                },
                PlaneDesc {
                    format: pf,
                    width: cw,
                    height: ch,
                },
                PlaneDesc {
                    format: pf,
                    width: cw,
                    height: ch,
                },
            ],
            width: w,
            height: h,
            label: format!(
                "yuv{sub}p{}",
                if bits > 8 {
                    format!("{bits}le")
                } else {
                    String::new()
                }
            ),
            rgba_premultiplied: false,
        };
        Ok(DecodedFrame {
            pts: pic.m.timestamp,
            format: Arc::new(format),
            pixels: Pixels::Pool(Arc::new(PoolFrame {
                planes: vec![
                    plane(0, pic.stride[0]),
                    plane(1, pic.stride[1]),
                    plane(2, pic.stride[1]),
                ],
                owner: Some(Box::new(owner)),
                copied: false,
            })),
            color: None,
        })
    }
}
