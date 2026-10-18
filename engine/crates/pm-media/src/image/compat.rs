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
    (!still.float
        || (still.format == ImageFormat::Tiff
            && still.bit_depth == 32
            && matches!(still.color.icc_class, None | Some(IccClass::Srgb))))
        && matches!(
            still.format,
            ImageFormat::Png
                | ImageFormat::WebP
                | ImageFormat::Jpeg
                | ImageFormat::Heif
                | ImageFormat::Tiff
                | ImageFormat::Bmp
                | ImageFormat::Ico
                | ImageFormat::Gif
        )
        && (matches!(
            still.color.icc_class,
            None | Some(
                IccClass::Srgb
                    | IccClass::GraySrgb
                    | IccClass::DisplayP3
                    | IccClass::Other
                    | IccClass::ConvertedToSrgb
            )
        ))
}

/// Build a level-1 surface for a class accepted by [`applies`].
/// Calling this on an unmeasured class does not establish pixel parity.
pub fn bitmap(still: &StillImage, pool: &FramePool) -> Result<StillImage> {
    bitmap_impl(still, pool, None)
}

/// As [`bitmap`], including color information PNG decoders do not expose
/// through the generic image trait. This changes only the level-1 bitmap.
pub fn bitmap_at(
    path: &std::path::Path,
    still: &StillImage,
    pool: &FramePool,
) -> Result<StillImage> {
    if still.format == ImageFormat::Tiff && super::raster::tiff_photometric(path)? == Some(3) {
        return bitmap(&super::compat_palette::read(path, still, pool)?, pool);
    }
    let png = if still.format == ImageFormat::Png {
        png_color(path)?
    } else {
        None
    };
    bitmap_impl(still, pool, png)
}

fn bitmap_impl(still: &StillImage, pool: &FramePool, png: Option<PngColor>) -> Result<StillImage> {
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
                    PlaneFormat::Rgba32Float if still.format == ImageFormat::Tiff => {
                        for c in 0..4 {
                            let v =
                                unsafe { std::ptr::read_unaligned(ptr.add(c * 4).cast::<f32>()) };
                            out.push(((v * 65535.0).round().clamp(0.0, 65535.0) as u16 >> 8) as u8);
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
    let transformed = png.is_some()
        || matches!(
            still.color.icc_class,
            Some(IccClass::DisplayP3 | IccClass::Other)
        );
    let mut premultiplied = still.color.alpha == AlphaMode::Premultiplied;
    if transformed {
        use crate::sys::skcms as s;
        let profile = still.color.icc.as_deref().unwrap_or_default();
        let mut padded = profile.to_vec();
        padded.extend_from_slice(&[0; 4]);
        // skcms retains pointers into the padded profile only for this call.
        unsafe {
            let mut source: s::skcms_ICCProfile = std::mem::zeroed();
            let parsed = if let Some(color) = png {
                source = *s::skcms_sRGB_profile();
                match color {
                    PngColor::Gamma(gamma) => {
                        for curve in &mut source.trc {
                            curve.__bindgen_anon_1 = s::skcms_Curve__bindgen_ty_1 {
                                alias_of_table_entries: 0,
                                parametric: s::skcms_TransferFunction {
                                    g: 1.0 / gamma,
                                    a: 1.0,
                                    ..Default::default()
                                },
                            };
                        }
                    }
                    PngColor::DisplayP3 => {
                        source.toXYZD50.vals = [
                            [0.515102, 0.291965, 0.157153],
                            [0.241182, 0.692236, 0.0665819],
                            [-0.00104941, 0.0418818, 0.784378],
                        ]
                    }
                }
                true
            } else {
                s::skcms_ParseWithA2BPriority(
                    padded.as_ptr().cast(),
                    profile.len(),
                    [0, 1].as_ptr(),
                    2,
                    &mut source,
                )
            };
            if parsed {
                canonicalize_trc(&mut source);
                // ImageBitmap first quantizes a premultiplied source-space
                // bitmap. Skia readPixels then unpremultiplies, converts color
                // without clipping, and premultiplies before the final store.
                // RGBA8 skcms output clamps unpremultiplied RGB internally;
                // use float output so out-of-gamut RGB survives until after
                // multiplication. Valid legacy RGB can exceed its alpha.
                if !premultiplied && still.color.alpha != AlphaMode::Opaque {
                    for p in rgba.chunks_exact_mut(4) {
                        let a = u32::from(p[3]);
                        for c in &mut p[..3] {
                            *c = ((u32::from(*c) * a * 257 + 257 * 128) >> 16) as u8;
                        }
                    }
                }
                let mut converted = vec![0.0f32; rgba.len()];
                if !super::compat_color::transform(&source, &rgba, &mut converted)
                    && !s::skcms_Transform(
                        rgba.as_ptr().cast(),
                        s::skcms_PixelFormat_RGBA_8888,
                        s::skcms_AlphaFormat_PremulAsEncoded,
                        &source,
                        converted.as_mut_ptr().cast(),
                        s::skcms_PixelFormat_RGBA_ffff,
                        s::skcms_AlphaFormat_PremulAsEncoded,
                        s::skcms_sRGB_profile(),
                        w * h,
                    )
                {
                    return Err(MediaError::invalid(
                        "ICC cannot transform the compatibility bitmap",
                    ));
                }
                for (byte, value) in rgba.iter_mut().zip(converted) {
                    *byte = (value * 255.0).round_ties_even().clamp(0.0, 255.0) as u8;
                }
                premultiplied = true;
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

/// PNG's `gAMA`-only profile selection from SkPngRustCodec. Higher
/// precedence color chunks and cHRM are handled by their measured classes.
#[derive(Clone, Copy)]
enum PngColor {
    Gamma(f32),
    DisplayP3,
}

fn png_color(path: &std::path::Path) -> Result<Option<PngColor>> {
    use std::io::Read;
    let mut file = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut signature = [0; 8];
    file.read_exact(&mut signature)?;
    if signature != *b"\x89PNG\r\n\x1a\n" {
        return Err(MediaError::invalid("PNG signature"));
    }
    let mut gamma = None;
    let mut overridden = false;
    let mut p3 = false;
    loop {
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let len = u32::from_be_bytes(header[..4].try_into().unwrap());
        match &header[4..] {
            b"IDAT" | b"IEND" => break,
            b"sRGB" | b"iCCP" | b"cHRM" => overridden = true,
            b"cICP" if len == 4 => {
                let mut b = [0; 4];
                file.read_exact(&mut b)?;
                p3 = b == [12, 13, 0, 1];
                overridden = true;
                file.seek_relative(4)?;
                continue;
            }
            b"gAMA" if len == 4 => {
                let mut b = [0; 4];
                file.read_exact(&mut b)?;
                gamma = Some(u32::from_be_bytes(b) as f32 / 100000.0);
                file.seek_relative(4)?;
                continue;
            }
            _ => {}
        }
        file.seek_relative(i64::from(len) + 4)?;
    }
    Ok(if p3 {
        Some(PngColor::DisplayP3)
    } else if overridden {
        None
    } else {
        gamma
            .filter(|g| *g > 0.0 && !(0.95 < *g * 2.2 && *g * 2.2 < 1.05))
            .map(PngColor::Gamma)
    })
}

/// `SkColorSpace::Make` / `MakeRGB` canonicalize near-sRGB and gamma-2.2
/// curves before readPixels constructs its color transform. Profile curves
/// use 16.16 coefficients; applying those literally changes byte rounding.
unsafe fn canonicalize_trc(profile: &mut crate::sys::skcms::skcms_ICCProfile) {
    use crate::sys::skcms as s;
    unsafe {
        if !profile.has_trc || profile.has_A2B {
            return;
        }
        let values = |t: s::skcms_TransferFunction| [t.g, t.a, t.b, t.c, t.d, t.e, t.f];
        let srgb = *s::skcms_sRGB_TransferFunction();
        let parametric = profile
            .trc
            .iter()
            .all(|c| c.__bindgen_anon_2.table_entries == 0);
        let replacement = if parametric {
            let t = profile.trc[0].__bindgen_anon_1.parametric;
            if !profile
                .trc
                .iter()
                .all(|c| values(c.__bindgen_anon_1.parametric) == values(t))
            {
                return;
            }
            if values(t)
                .iter()
                .zip(values(srgb))
                .all(|(a, b)| (a - b).abs() < 0.001)
            {
                Some(srgb)
            } else if (t.a - 1.0).abs() < 0.001
                && t.b.abs() < 0.001
                && t.e.abs() < 0.001
                && (t.g - 2.2).abs() < 0.001
                && t.d <= 0.0
            {
                Some(s::skcms_TransferFunction {
                    g: 2.2,
                    a: 1.0,
                    ..Default::default()
                })
            } else {
                None
            }
        } else if s::skcms_TRCs_AreApproximateInverse(
            profile,
            s::skcms_sRGB_Inverse_TransferFunction(),
        ) {
            Some(srgb)
        } else {
            None
        };
        if let Some(t) = replacement {
            for c in &mut profile.trc {
                c.__bindgen_anon_1 = s::skcms_Curve__bindgen_ty_1 {
                    alias_of_table_entries: 0,
                    parametric: t,
                };
            }
        }
    }
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
