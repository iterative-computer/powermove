//! Chromium's software full-range I420 Canvas upload.
//! libyuv is already pinned and linked by the native media build. This
//! conversion deliberately precedes spatial filtering and keeps its integer
//! coefficients, nearest chroma, clipping and rounding intact.
use crate::frame::{DecodedFrame, Layout, PlaneFormat};
use crate::image::{Rgba, pool_rgba};
use crate::pool::FramePool;
use crate::{MediaError, Result};
use pm_protocol::color::{MatrixCoefficients, Range};

#[repr(C)]
struct YuvConstants {
    _private: [u8; 0],
}
unsafe extern "C" {
    static kYuvF709Constants: YuvConstants;
    static kYuvJPEGConstants: YuvConstants;
    static kYuvV2020Constants: YuvConstants;
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
pub fn full_range_bitmap(
    frame: &DecodedFrame,
    pool: &FramePool,
    matrix: MatrixCoefficients,
) -> Result<DecodedFrame> {
    let constants = match matrix {
        MatrixCoefficients::Bt601 => &raw const kYuvJPEGConstants,
        MatrixCoefficients::Bt709 => &raw const kYuvF709Constants,
        MatrixCoefficients::Bt2020Ncl => &raw const kYuvV2020Constants,
        _ => return Err(MediaError::invalid("unsupported full-range video matrix")),
    };
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
            "Chromium full-range bitmap requires opaque 8-bit 4:2:0",
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
                    constants,
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
                    constants,
                    w,
                    h,
                )
            }
        };
        if rc != 0 {
            return Err(MediaError::decode(format!(
                "libyuv full-range bitmap: {rc}"
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
        "Chromium full-range rgba8".into(),
    )?;
    bitmap.pts = frame.pts;
    Ok(bitmap)
}

/// Chromium's software alpha video bitmap, including playback renditions.
/// Matrix conversion and alpha attenuation happen together in libyuv before
/// Canvas or the layer's filter samples the premultiplied RGBA8 pixels.
pub fn alpha_bitmap(
    frame: &DecodedFrame,
    pool: &FramePool,
    matrix: pm_protocol::color::MatrixCoefficients,
) -> Result<Option<DecodedFrame>> {
    use pm_protocol::color::MatrixCoefficients;
    let f = &frame.format;
    if f.layout != Layout::Planar
        || f.bits != 8
        || f.alpha_bits != 8
        || f.range != Range::Limited
        || f.log2_chroma_w != 1
        || f.log2_chroma_h != 1
        || f.planes.len() != 4
        || !f.planes.iter().all(|p| p.format == PlaneFormat::R8)
    {
        return Ok(None);
    }
    unsafe extern "C" {
        static kYuvI601Constants: YuvConstants;
        static kYuvH709Constants: YuvConstants;
        static kYuv2020Constants: YuvConstants;
        fn I420AlphaToARGBMatrix(
            y: *const u8,
            ys: i32,
            u: *const u8,
            us: i32,
            v: *const u8,
            vs: i32,
            a: *const u8,
            as_: i32,
            dst: *mut u8,
            ds: i32,
            constants: *const YuvConstants,
            w: i32,
            h: i32,
            attenuate: i32,
        ) -> i32;
    }
    let constants = match matrix {
        MatrixCoefficients::Bt601 => &raw const kYuvI601Constants,
        MatrixCoefficients::Bt709 => &raw const kYuvH709Constants,
        MatrixCoefficients::Bt2020Ncl => &raw const kYuv2020Constants,
        _ => return Ok(None),
    };
    let w = i32::try_from(f.width).map_err(|_| MediaError::invalid("alpha video width"))?;
    let h = i32::try_from(f.height).map_err(|_| MediaError::invalid("alpha video height"))?;
    let stride = w
        .checked_mul(4)
        .ok_or_else(|| MediaError::invalid("alpha video stride"))?;
    let length = (stride as usize)
        .checked_mul(h as usize)
        .filter(|&n| n <= 1 << 30)
        .ok_or_else(|| MediaError::invalid("alpha video size"))?;
    let mut rgba = vec![0; length];
    frame.with_planes(|p| -> Result<()> {
        let s = p
            .iter()
            .map(|p| {
                i32::try_from(p.stride).map_err(|_| MediaError::invalid("alpha video plane stride"))
            })
            .collect::<Result<Vec<_>>>()?;
        let status = unsafe {
            I420AlphaToARGBMatrix(
                p[0].ptr,
                s[0],
                p[1].ptr,
                s[1],
                p[2].ptr,
                s[2],
                p[3].ptr,
                s[3],
                rgba.as_mut_ptr(),
                stride,
                constants,
                w,
                h,
                1,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(MediaError::decode(format!(
                "libyuv alpha video bitmap: {status}"
            )))
        }
    })??;
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
        "Chromium alpha video rgba8".into(),
    )?;
    bitmap.pts = frame.pts;
    Ok(Some(bitmap))
}

/// Place planar 10-bit codes in the same unorm storage as Chromium's P010
/// hardware surfaces, before the texture's interpolation precision is applied.
pub fn aligned_planar_10(frame: &DecodedFrame, pool: &FramePool) -> Result<DecodedFrame> {
    use crate::frame::{Pixels, PoolFrame, PoolPlane};
    use pm_protocol::color::SampleAlignment;
    use std::sync::Arc;
    let f = &frame.format;
    if f.layout != Layout::Planar
        || f.bits != 10
        || f.planes.len() != 3
        || f.alignment != SampleAlignment::Lsb
        || f.has_alpha()
        || !f.planes.iter().all(|p| p.format == PlaneFormat::R16)
    {
        return Err(MediaError::invalid("planar 10-bit alignment"));
    }
    let planes = frame.with_planes(|src| -> Result<Vec<PoolPlane>> {
        src.iter()
            .map(|p| {
                let stride = (p.desc.width * 2).div_ceil(256) * 256;
                let slot = pool.acquire(stride * p.desc.height)?;
                for y in 0..p.desc.height {
                    for x in 0..p.desc.width {
                        unsafe {
                            let code = p
                                .ptr
                                .add(y * p.stride + x * 2)
                                .cast::<u16>()
                                .read_unaligned();
                            slot.ptr()
                                .add(y * stride + x * 2)
                                .cast::<u16>()
                                .write_unaligned(code << 6);
                        }
                    }
                }
                Ok(PoolPlane {
                    slot,
                    offset: 0,
                    stride,
                    decoder_ptr: 0,
                })
            })
            .collect()
    })??;
    let mut format = (**f).clone();
    format.alignment = SampleAlignment::MsbIn16;
    Ok(DecodedFrame {
        pts: frame.pts,
        format: Arc::new(format),
        pixels: Pixels::Pool(Arc::new(PoolFrame {
            planes,
            owner: None,
            copied: true,
        })),
        color: frame.color,
    })
}
