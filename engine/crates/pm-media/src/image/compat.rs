//! The 8-bit premultiplied bitmap Blink exposes to existing image layers.
//! Source readers keep their native precision; this compatibility transform
//! happens before any filtering, exactly where image decoders quantize it.

use super::{ImageColor, ImageFormat, Rgba, StillImage, icc::IccClass, pool_rgba};
use crate::frame::PlaneFormat;
use crate::pool::FramePool;
use crate::{MediaError, Result};
use pm_protocol::color::{AlphaMode, Transfer};

/// Native pixels for which the decoder's old 8-bit surface is reproduced.
pub fn applies(still: &StillImage) -> bool {
    !still.float
        && !unmatched_icc_alpha(still)
        && matches!(
            still.format,
            ImageFormat::Png
                | ImageFormat::WebP
                | ImageFormat::Jpeg
                | ImageFormat::Heif
                | ImageFormat::Tiff
        )
        && (matches!(
            still.color.icc_class,
            None | Some(
                IccClass::Srgb | IccClass::GraySrgb | IccClass::DisplayP3 | IccClass::Other
            )
        ))
}

/// Wide-gamut translucent ImageBitmaps still miss the legacy alpha class.
/// An alpha channel containing only full coverage is an opaque bitmap and
/// remains in the measured ICC class, regardless of the container's flags.
pub fn unmatched_icc_alpha(still: &StillImage) -> bool {
    if !matches!(
        still.color.icc_class,
        Some(IccClass::DisplayP3 | IccClass::Other)
    ) || still.color.alpha == AlphaMode::Opaque
    {
        return false;
    }
    still
        .frame
        .with_planes(|planes| {
            let p = &planes[0];
            for y in 0..p.desc.height {
                for x in 0..p.desc.width {
                    let ptr = unsafe { p.ptr.add(y * p.stride + x * p.desc.format.bytes()) };
                    let opaque = match p.desc.format {
                        PlaneFormat::Rgba8 => unsafe { *ptr.add(3) == 255 },
                        PlaneFormat::Rgba16 => unsafe {
                            std::ptr::read_unaligned(ptr.add(6).cast::<u16>()) == u16::MAX
                        },
                        _ => false,
                    };
                    if !opaque {
                        return true;
                    }
                }
            }
            false
        })
        .unwrap_or(true)
}

/// Build a level-1 surface for a class accepted by [`applies`].
/// Calling this on an unmeasured class does not establish pixel parity.
pub fn bitmap(still: &StillImage, pool: &FramePool) -> Result<StillImage> {
    let (w, h) = (still.frame.format.width, still.frame.format.height);
    let mut rgba = still.frame.with_planes(|planes| {
        let p = &planes[0];
        let mut out = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let ptr = unsafe { p.ptr.add(y * p.stride + x * p.desc.format.bytes()) };
                match p.desc.format {
                    PlaneFormat::Rgba8 => {
                        out.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr, 4) })
                    }
                    // Skia's PNG swizzler drops the low byte before color
                    // conversion and alpha multiplication on an 8888 surface.
                    PlaneFormat::Rgba16 => {
                        for c in 0..4 {
                            let v =
                                unsafe { std::ptr::read_unaligned(ptr.add(c * 2).cast::<u16>()) };
                            out.push((v >> 8) as u8);
                        }
                    }
                    _ => {
                        return Err(MediaError::invalid(
                            "compatibility bitmap needs integer RGBA",
                        ));
                    }
                }
            }
        }
        Ok(out)
    })??;
    let transformed = matches!(
        still.color.icc_class,
        Some(IccClass::DisplayP3 | IccClass::Other)
    );
    let mut premultiplied = still.color.alpha == AlphaMode::Premultiplied;
    if transformed && let Some(profile) = &still.color.icc {
        use crate::sys::skcms as s;
        let mut padded = profile.clone();
        padded.extend_from_slice(&[0; 4]);
        // skcms retains pointers into the padded profile only for this call.
        unsafe {
            let mut source: s::skcms_ICCProfile = std::mem::zeroed();
            if s::skcms_ParseWithA2BPriority(
                padded.as_ptr().cast(),
                profile.len(),
                [0, 1].as_ptr(),
                2,
                &mut source,
            ) {
                let alpha = if premultiplied {
                    s::skcms_AlphaFormat_PremulAsEncoded
                } else {
                    s::skcms_AlphaFormat_Unpremul
                };
                if !s::skcms_Transform(
                    rgba.as_ptr().cast(),
                    s::skcms_PixelFormat_RGBA_8888,
                    alpha,
                    &source,
                    rgba.as_mut_ptr().cast(),
                    s::skcms_PixelFormat_RGBA_8888,
                    s::skcms_AlphaFormat_Unpremul,
                    s::skcms_sRGB_profile(),
                    w * h,
                ) {
                    return Err(MediaError::invalid(
                        "ICC cannot transform the compatibility bitmap",
                    ));
                }
                premultiplied = false;
            }
            // Blink ignores an invalid embedded profile and reads sRGB.
        }
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        if still.color.alpha == AlphaMode::Opaque {
            px[3] = 255;
        } else if !premultiplied {
            for c in px.iter_mut().take(3) {
                let v = u32::from(*c);
                *c = if still.format == ImageFormat::WebP && !transformed {
                    // libwebp MODE_bgrA: MULTIPLIER(a) = a * 32897,
                    // followed by the high-byte multiply (truncation).
                    if a == 255 {
                        *c
                    } else {
                        ((v * a * 32897) >> 23) as u8
                    }
                } else {
                    // ImageFrame::SetRGBAPremultiply, Chromium 152.
                    ((v * a * 257 + 257 * 128) >> 16) as u8
                };
            }
        }
    }
    Ok(StillImage {
        frame: pool_rgba(
            pool,
            w,
            h,
            Rgba::U8(&rgba),
            true,
            8,
            "Blink compatibility rgba8".into(),
        )?,
        color: ImageColor {
            icc: None,
            icc_bytes: None,
            icc_class: None,
            transfer: Transfer::DisplayE,
            alpha: if still.color.alpha == AlphaMode::Opaque {
                AlphaMode::Opaque
            } else {
                AlphaMode::Premultiplied
            },
            chromaticities: None,
        },
        bit_depth: 8,
        float: false,
        ..still.clone()
    })
}

/// Chromium's software I420 video upload uses libyuv's nearest-chroma,
/// clipped RGBA8 bitmap before the canvas filters it. Existing sequence
/// renditions are 8-bit limited-range I420, optionally with straight alpha.
/// Other source readings keep the native video conversion.
pub fn video_bitmap(
    frame: &crate::frame::DecodedFrame,
    pool: &FramePool,
) -> Result<Option<crate::frame::DecodedFrame>> {
    use crate::frame::Layout;
    use pm_protocol::color::Range;
    let f = &frame.format;
    if f.layout != Layout::Planar
        || f.bits != 8
        || f.range != Range::Limited
        || f.log2_chroma_w != 1
        || f.log2_chroma_h != 1
        || !f.planes.iter().all(|p| p.format == PlaneFormat::R8)
        || !matches!(f.planes.len(), 3 | 4)
    {
        return Ok(None);
    }
    // Signatures from Chromium 152's pinned libyuv/convert_argb.h. Those
    // C entry points are included in CrabbyAvif's static dependency archive.
    unsafe extern "C" {
        fn I420ToABGR(
            y: *const u8,
            ys: i32,
            u: *const u8,
            us: i32,
            v: *const u8,
            vs: i32,
            dst: *mut u8,
            ds: i32,
            w: i32,
            h: i32,
        ) -> i32;
        fn I420AlphaToABGR(
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
            w: i32,
            h: i32,
            attenuate: i32,
        ) -> i32;
    }
    let w = i32::try_from(f.width).map_err(|_| MediaError::invalid("video bitmap width"))?;
    let h = i32::try_from(f.height).map_err(|_| MediaError::invalid("video bitmap height"))?;
    let stride = w
        .checked_mul(4)
        .ok_or_else(|| MediaError::invalid("video bitmap stride"))?;
    let mut rgba = vec![
        0u8;
        (stride as usize)
            .checked_mul(h as usize)
            .filter(|n| *n <= 1 << 30)
            .ok_or_else(|| MediaError::invalid("video bitmap size"))?
    ];
    frame.with_planes(|p| -> Result<()> {
        let s: Vec<i32> = p
            .iter()
            .map(|p| {
                i32::try_from(p.stride)
                    .map_err(|_| MediaError::invalid("video bitmap plane stride"))
            })
            .collect::<Result<_>>()?;
        let rc = unsafe {
            if p.len() == 4 {
                I420AlphaToABGR(
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
                    w,
                    h,
                    1,
                )
            } else {
                I420ToABGR(
                    p[0].ptr,
                    s[0],
                    p[1].ptr,
                    s[1],
                    p[2].ptr,
                    s[2],
                    rgba.as_mut_ptr(),
                    stride,
                    w,
                    h,
                )
            }
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(MediaError::decode(format!("libyuv video bitmap {rc}")))
        }
    })??;
    Ok(Some(pool_rgba(
        pool,
        f.width,
        f.height,
        Rgba::U8(&rgba),
        true,
        8,
        "Chromium I420 rgba8".into(),
    )?))
}
