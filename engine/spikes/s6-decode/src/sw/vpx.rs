//! libvpx VP9 decoding into Metal-shared pool slots through external frame
//! buffers (`vpx_codec_set_frame_buffer_functions`). VP9 with alpha (Matroska
//! BlockAdditional id 1) runs a second decoder for the alpha plane, as
//! FFmpeg's libvpx wrapper does; FFmpeg's wrapper allocates from its own pool,
//! so through FFmpeg those frames need a copy.
//!
//! libvpx supports external frame buffers for VP9 only (VP8 returns
//! `VPX_CODEC_INCAPABLE`). VP8 with alpha goes through two FFmpeg native VP8
//! decoders instead (`ffalpha`), which render into the pool.

use super::SwDecoder;
use super::sys::*;
use crate::ff::Packet;
use crate::pool::{MetalPool, PoolFrame, PoolPlane, SlotRef};
use anyhow::{Context, Result, bail, ensure};
use std::ffi::{c_int, c_void};
use std::sync::Arc;

unsafe extern "C" fn get_fb(priv_: *mut c_void, min_size: usize, fb: *mut vpx_codec_frame_buffer_t) -> c_int {
    unsafe {
        let pool = &*(priv_ as *const MetalPool);
        // A pool made with new_zeroed: libvpx requires zeroed buffers; fresh
        // slots are zero-filled, and reused slots only ever held libvpx frames
        // (FFmpeg's wrapper behaves the same way).
        let Some(slot) = pool.acquire(min_size) else { return -1 };
        (*fb).data = slot.ptr();
        (*fb).size = slot.len();
        (*fb).priv_ = Box::into_raw(Box::new(slot)) as *mut c_void;
        0
    }
}

unsafe extern "C" fn release_fb(_priv: *mut c_void, fb: *mut vpx_codec_frame_buffer_t) -> c_int {
    unsafe {
        let s = (*fb).priv_ as *mut SlotRef;
        if !s.is_null() {
            drop(Box::from_raw(s));
            (*fb).priv_ = std::ptr::null_mut();
        }
        0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Vp8,
    Vp9,
}

struct Ctx {
    ctx: Box<vpx_codec_ctx_t>,
}

impl Ctx {
    fn new(codec: Codec, pool: &Arc<MetalPool>, threads: u32) -> Result<Ctx> {
        unsafe {
            let mut ctx: Box<vpx_codec_ctx_t> = Box::new(std::mem::zeroed());
            let cfg = vpx_codec_dec_cfg_t { threads, w: 0, h: 0 };
            let iface = match codec {
                Codec::Vp8 => vpx_codec_vp8_dx(),
                Codec::Vp9 => vpx_codec_vp9_dx(),
            };
            let r = vpx_codec_dec_init_ver(&mut *ctx, iface, &cfg, 0, VPX_DECODER_ABI_VERSION as c_int);
            ensure!(r == VPX_CODEC_OK, "vpx_codec_dec_init: {r}");
            let r = vpx_codec_set_frame_buffer_functions(
                &mut *ctx,
                Some(get_fb),
                Some(release_fb),
                Arc::as_ptr(pool) as *mut c_void,
            );
            if r != VPX_CODEC_OK {
                vpx_codec_destroy(&mut *ctx);
                bail!("libvpx {codec:?} rejects external frame buffers (vpx_codec_set_frame_buffer_functions: {r})");
            }
            Ok(Ctx { ctx })
        }
    }

    /// Decodes one compressed frame and returns the shown image, if any.
    fn decode(&mut self, data: &[u8]) -> Result<Option<*mut vpx_image_t>> {
        unsafe {
            let r = vpx_codec_decode(&mut *self.ctx, data.as_ptr(), data.len() as u32, std::ptr::null_mut(), 0);
            ensure!(r == VPX_CODEC_OK, "vpx_codec_decode: {r}");
            let mut iter: vpx_codec_iter_t = std::ptr::null();
            let img = vpx_codec_get_frame(&mut *self.ctx, &mut iter);
            Ok((!img.is_null()).then_some(img))
        }
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        unsafe {
            vpx_codec_destroy(&mut *self.ctx);
        }
    }
}

pub struct VpxDecoder {
    codec: Codec,
    main: Ctx,
    alpha: Option<Ctx>,
    // Declared last: dropped after both decoders have released their buffers.
    pool: Arc<MetalPool>,
}

unsafe impl Send for VpxDecoder {}

impl VpxDecoder {
    /// `pool` must come from `MetalPool::new_zeroed` and serve only libvpx.
    pub fn new(codec: Codec, pool: Arc<MetalPool>, alpha: bool, threads: u32) -> Result<Self> {
        let main = Ctx::new(codec, &pool, threads)?;
        let alpha = if alpha { Some(Ctx::new(codec, &pool, threads)?) } else { None };
        Ok(VpxDecoder { codec, main, alpha, pool })
    }
}

/// The pool lease behind an image's frame buffer, cloned so the frame stays
/// valid after libvpx releases the buffer.
unsafe fn lease(img: *const vpx_image_t) -> Result<SlotRef> {
    unsafe {
        let s = (*img).fb_priv as *const SlotRef;
        ensure!(!s.is_null(), "vpx image without an external frame buffer");
        Ok((*s).clone())
    }
}

unsafe fn plane(img: *const vpx_image_t, slot: &SlotRef, i: usize, w: usize, h: usize, bpp: usize) -> PoolPlane {
    unsafe {
        let ptr = (*img).planes[i] as *const u8;
        PoolPlane {
            slot: slot.clone(),
            offset: ptr.offset_from(slot.ptr()) as usize,
            stride: (*img).stride[i] as usize,
            width: w,
            height: h,
            bytes_per_px: bpp,
            decoder_ptr: ptr as usize,
        }
    }
}

impl SwDecoder for VpxDecoder {
    fn send(&mut self, p: Option<&Packet>, out: &mut Vec<(i64, PoolFrame)>) -> Result<()> {
        // No reordering delay in libvpx's serial decoder: nothing to drain.
        let Some(p) = p else { return Ok(()) };
        let Some(img) = self.main.decode(&p.data)? else { return Ok(()) };
        let alpha_img = match (&mut self.alpha, &p.additional) {
            (Some(a), Some(add)) => Some(a.decode(add)?.context("alpha decoder produced no frame")?),
            (Some(_), None) => bail!("VP9 alpha stream packet without BlockAdditional"),
            _ => None,
        };
        unsafe {
            let im = &*img;
            let hbd = im.fmt & VPX_IMG_FMT_HIGHBITDEPTH != 0;
            let bpp = if hbd { 2 } else { 1 };
            let (w, h) = (im.d_w as usize, im.d_h as usize);
            let (lw, lh) = (im.x_chroma_shift as u8, im.y_chroma_shift as u8);
            let (cw, ch) = ((w + (1 << lw) - 1) >> lw, (h + (1 << lh) - 1) >> lh);
            let slot = lease(img)?;
            let mut planes = vec![plane(img, &slot, 0, w, h, bpp), plane(img, &slot, 1, cw, ch, bpp), plane(img, &slot, 2, cw, ch, bpp)];
            if let Some(a) = alpha_img {
                ensure!((*a).d_w as usize == w && (*a).d_h as usize == h, "alpha plane size differs");
                ensure!((*a).fmt & VPX_IMG_FMT_HIGHBITDEPTH == im.fmt & VPX_IMG_FMT_HIGHBITDEPTH, "alpha depth differs");
                let aslot = lease(a)?;
                planes.push(plane(a, &aslot, 0, w, h, bpp));
            }
            let sub = match (lw, lh) {
                (1, 1) => "420",
                (1, 0) => "422",
                (0, 1) => "440",
                _ => "444",
            };
            let depth = im.bit_depth as u8;
            out.push((
                p.pts,
                PoolFrame {
                    label: format!(
                        "yuv{}{sub}p{}",
                        if planes.len() == 4 { "a" } else { "" },
                        if hbd { format!("{depth}le") } else { String::new() }
                    ),
                    planes,
                    depth,
                    log2_chroma_w: lw,
                    log2_chroma_h: lh,
                    full_range: im.range == VPX_CR_FULL_RANGE,
                    copied: false,
                    owner: None,
                },
            ));
        }
        Ok(())
    }
    fn name(&self) -> String {
        format!("libvpx-{}{}", if self.codec == Codec::Vp9 { "vp9" } else { "vp8" }, if self.alpha.is_some() { " + alpha decoder" } else { "" })
    }
    fn direct(&self) -> bool {
        let _ = &self.pool;
        true
    }
    fn copies(&self) -> usize {
        0
    }
    fn flush(&mut self) {
        // libvpx has no flush: its serial decoder holds no pending output, and
        // every random-access point is a keyframe, which resets all reference
        // state. The warm random-access pass verifies this.
    }
}
