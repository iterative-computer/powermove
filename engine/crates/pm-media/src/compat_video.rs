//! Chromium's software full-range BT.709 I420 Canvas upload.
//! libyuv is already pinned and linked by the native media build. This
//! conversion deliberately precedes spatial filtering and keeps its integer
//! coefficients, nearest chroma, clipping and rounding intact.
use crate::frame::{DecodedFrame, Layout, PlaneFormat};
use crate::image::{Rgba, pool_rgba};
use crate::pool::FramePool;
use crate::{MediaError, Result};
use pm_protocol::color::Range;

#[repr(C)]
struct YuvConstants {
    _private: [u8; 0],
}
unsafe extern "C" {
    static kYuvF709Constants: YuvConstants;
    fn I420ToARGBMatrix(
        y: *const u8,
        ys: i32,
        u: *const u8,
        us: i32,
        v: *const u8,
        vs: i32,
        dst: *mut u8,
        ds: i32,
        constants: *const YuvConstants,
        w: i32,
        h: i32,
    ) -> i32;
    fn NV12ToARGBMatrix(
        y: *const u8,
        ys: i32,
        uv: *const u8,
        uvs: i32,
        dst: *mut u8,
        ds: i32,
        constants: *const YuvConstants,
        w: i32,
        h: i32,
    ) -> i32;
}

/// The caller selects Chromium's software Canvas path from codec and tags.
/// Accept native planar I420 or VideoToolbox's equivalent biplanar 420f.
pub fn full_range_709_bitmap(frame: &DecodedFrame, pool: &FramePool) -> Result<DecodedFrame> {
    let f = &frame.format;
    let planar = f.layout == Layout::Planar
        && f.planes.len() == 3
        && f.planes.iter().all(|p| p.format == PlaneFormat::R8);
    let biplanar = f.layout == Layout::Biplanar
        && f.planes.len() == 2
        && f.planes[0].format == PlaneFormat::R8
        && f.planes[1].format == PlaneFormat::Rg8;
    if !(planar || biplanar)
        || f.bits != 8
        || f.range != Range::Full
        || f.log2_chroma_w != 1
        || f.log2_chroma_h != 1
        || f.has_alpha()
    {
        return Err(MediaError::invalid(
            "Chromium full-range BT.709 bitmap requires opaque 8-bit 4:2:0",
        ));
    }
    let w = i32::try_from(f.width).map_err(|_| MediaError::invalid("video bitmap width"))?;
    let h = i32::try_from(f.height).map_err(|_| MediaError::invalid("video bitmap height"))?;
    let stride = w
        .checked_mul(4)
        .ok_or_else(|| MediaError::invalid("video bitmap stride"))?;
    let length = (stride as usize)
        .checked_mul(h as usize)
        .filter(|&n| n <= 1 << 30)
        .ok_or_else(|| MediaError::invalid("video bitmap size"))?;
    let mut rgba = vec![0; length];
    frame.with_planes(|p| -> Result<()> {
        let strides = p
            .iter()
            .map(|p| {
                i32::try_from(p.stride)
                    .map_err(|_| MediaError::invalid("video bitmap plane stride"))
            })
            .collect::<Result<Vec<_>>>()?;
        let rc = unsafe {
            if planar {
                I420ToARGBMatrix(
                    p[0].ptr,
                    strides[0],
                    p[1].ptr,
                    strides[1],
                    p[2].ptr,
                    strides[2],
                    rgba.as_mut_ptr(),
                    stride,
                    &kYuvF709Constants,
                    w,
                    h,
                )
            } else {
                NV12ToARGBMatrix(
                    p[0].ptr,
                    strides[0],
                    p[1].ptr,
                    strides[1],
                    rgba.as_mut_ptr(),
                    stride,
                    &kYuvF709Constants,
                    w,
                    h,
                )
            }
        };
        if rc != 0 {
            return Err(MediaError::decode(format!(
                "libyuv full-range BT.709 bitmap: {rc}"
            )));
        }
        Ok(())
    })??;
    // libyuv ARGB is BGRA in little-endian memory. Keep alpha (always 255).
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let mut bitmap = pool_rgba(
        pool,
        f.width,
        f.height,
        Rgba::U8(&rgba),
        true,
        8,
        "Chromium full-range BT.709 rgba8".into(),
    )?;
    bitmap.pts = frame.pts;
    Ok(bitmap)
}
