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

/// Whether today's RGBA8 WebGL upload of this still fails (`INVALID_VALUE`):
/// JPEGs deeper than 8 bits or arithmetic-coded, which Chromium cannot keep
/// as an 8-bit `ImageBitmap` (the importer falls back to an image element
/// whose upload fails). [`bitmap_at`] gives image layers' incomplete
/// texture (opaque black); a `texStorage2D` texture keeps its zeroed
/// storage instead (3D material maps).
pub fn upload_fails(path: &std::path::Path, still: &StillImage) -> Result<bool> {
    Ok(if still.format == ImageFormat::Jpeg {
        let info = super::jpeg::info(&std::fs::read(path)?)?;
        info.precision > 8 || info.arithmetic
    } else {
        false
    })
}

/// As [`bitmap`], including color information PNG decoders do not expose
/// through the generic image trait. This changes only the level-1 bitmap.
pub fn bitmap_at(
    path: &std::path::Path,
    still: &StillImage,
    pool: &FramePool,
) -> Result<StillImage> {
    if upload_fails(path, still)? {
        // Chromium imports these as float ImageBitmaps, but today's RGBA8
        // texImage2D fails INVALID_VALUE. Its incomplete sampler is opaque
        // black, including in completed exports. Preserve that picture at
        // level 1; the native-depth JPEG remains available at level 2.
        let (w, h) = (still.frame.format.width, still.frame.format.height);
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [0, 0, 0, 255]).collect();
        return Ok(StillImage {
            frame: pool_rgba(
                pool,
                w,
                h,
                Rgba::U8(&rgba),
                true,
                8,
                "Chromium incomplete JPEG texture".into(),
            )?,
            color: ImageColor {
                icc: None,
                icc_bytes: None,
                icc_class: None,
                transfer: Transfer::DisplayE,
                alpha: AlphaMode::Opaque,
                chromaticities: None,
            },
            bit_depth: 8,
            float: false,
            ..still.clone()
        });
    }
    if still.format == ImageFormat::Jpeg
        && let Some(bitmap) = super::jpeg::cmyk_bitmap(path, pool)?
    {
        return Ok(bitmap);
    }
    // TIFF (palettes included): the shipped FFmpeg PNG rendition, read
    // here when the caller passed another reader's still.
    if still.format == ImageFormat::Tiff && still.backend == super::ImageBackend::Ffmpeg {
        return bitmap(still, pool);
    }
    if still.format == ImageFormat::Tiff {
        return bitmap(
            &super::compat_tiff::read(path, still.orientation, pool)?,
            pool,
        );
    }
    let png = if still.format == ImageFormat::Png {
        png_color(path)?
    } else {
        None
    };
    bitmap_impl(still, pool, png)
}

/// Stored integer samples of the TIFF rendition the shipped app imports.
/// The native-depth source API is deliberately separate.
pub fn read_tiff_source(path: &std::path::Path, pool: &FramePool) -> Result<StillImage> {
    let orientation = super::raster::tiff_short(path, 274)?
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1) as u8;
    super::compat_tiff::read(path, orientation, pool)
}

/// The shipped TIFF PNG rendition's samples at their original integer
/// depth, before Chromium's bitmap conversion (Blender passes it through).
pub fn read_tiff_png_source(path: &std::path::Path, pool: &FramePool) -> Result<StillImage> {
    let orientation = super::raster::tiff_short(path, 274)?
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1) as u8;
    super::compat_tiff::read_source(path, orientation, pool, true)
}

/// AVIF CICP pairs Chromium's bitmap draws through Skia's PQ/HLG/primaries
/// transform: primaries BT.709, unspecified, BT.2020 or P3-D65 with every
/// SDR sRGB-family transfer, PQ or HLG. Every one of these 32 pairs was
/// compared byte-exact with the app's upload, 8 and 10-bit (media.md,
/// "Level 1 HDR and chromaticity stills"); other pairs stay named.
pub(super) fn cicp_supported(primaries: u16, transfer: u16) -> bool {
    matches!(primaries, 1 | 2 | 9 | 12) && matches!(transfer, 1 | 2 | 6 | 13 | 14 | 15 | 16 | 18)
}

pub(super) fn bitmap_cicp(
    still: &StillImage,
    pool: &FramePool,
    primaries: u16,
    transfer: u16,
) -> Result<StillImage> {
    bitmap_impl(still, pool, Some(PngColor::Cicp(primaries, transfer)))
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
                    PngColor::Gamma(gamma) | PngColor::Chromaticity { gamma, .. } => {
                        if let PngColor::Chromaticity { matrix, .. } = color {
                            source.toXYZD50.vals = matrix;
                        }
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
                    PngColor::Cicp(primaries, transfer) => {
                        source.toXYZD50.vals = match primaries {
                            9 => [
                                [0.673459, 0.165661, 0.125100],
                                [0.279033, 0.675338, 0.0456288],
                                [-0.00193139, 0.0299794, 0.797162],
                            ],
                            12 => [
                                [0.515102, 0.291965, 0.157153],
                                [0.241182, 0.692236, 0.0665819],
                                [-0.00104941, 0.0418818, 0.784378],
                            ],
                            _ => source.toXYZD50.vals,
                        };
                        let t = if transfer == 16 {
                            s::skcms_TransferFunction {
                                g: super::compat_color::PQ_TF,
                                a: 203.0,
                                ..Default::default()
                            }
                        } else if transfer == 18 {
                            s::skcms_TransferFunction {
                                g: super::compat_color::HLG_TF,
                                a: 203.0,
                                b: 1000.0,
                                c: 1.2,
                                ..Default::default()
                            }
                        } else {
                            *s::skcms_sRGB_TransferFunction()
                        };
                        for curve in &mut source.trc {
                            curve.__bindgen_anon_1 = s::skcms_Curve__bindgen_ty_1 {
                                alias_of_table_entries: 0,
                                parametric: t,
                            };
                        }
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

/// PNG color-profile selection from the pinned SkPngRustCodec. CICP
/// overrides ICC/sRGB; a valid cHRM is used only with a valid gAMA (alone
/// it is ignored), and a cHRM whose primaries do not form a matrix discards
/// the gAMA too, so the image reads as sRGB. Each case was compared with
/// the app's upload (media.md, "Level 1 HDR and chromaticity stills").
#[derive(Clone, Copy)]
enum PngColor {
    Gamma(f32),
    Cicp(u16, u16),
    Chromaticity { gamma: f32, matrix: [[f32; 3]; 3] },
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
    let mut hdr = None;
    let mut chromaticity = None;
    let mut has_chrm = false;
    loop {
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let len = u32::from_be_bytes(header[..4].try_into().unwrap());
        match &header[4..] {
            b"IDAT" | b"IEND" => break,
            b"sRGB" | b"iCCP" => overridden = true,
            b"cHRM" if len == 32 => {
                has_chrm = true;
                let mut b = [0; 32];
                file.read_exact(&mut b)?;
                let v: Vec<f32> = b
                    .chunks_exact(4)
                    .map(|c| u32::from_be_bytes(c.try_into().unwrap()) as f32 / 100000.0)
                    .collect();
                let mut matrix = crate::sys::skcms::skcms_Matrix3x3::default();
                if unsafe {
                    crate::sys::skcms::skcms_PrimariesToXYZD50(
                        v[2],
                        v[3],
                        v[4],
                        v[5],
                        v[6],
                        v[7],
                        v[0],
                        v[1],
                        &mut matrix,
                    )
                } {
                    chromaticity = Some(matrix.vals);
                }
                file.seek_relative(4)?;
                continue;
            }
            b"cICP" if len == 4 => {
                let mut b = [0; 4];
                file.read_exact(&mut b)?;
                if matches!(b[0], 1 | 9 | 12) && matches!(b[1], 13 | 16 | 18) && b[2..] == [0, 1] {
                    hdr = Some((u16::from(b[0]), u16::from(b[1])));
                }
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
    Ok(if let Some((primaries, transfer)) = hdr {
        Some(PngColor::Cicp(primaries, transfer))
    } else if overridden || (has_chrm && chromaticity.is_none()) {
        None
    } else if let (Some(matrix), Some(gamma)) = (chromaticity, gamma.filter(|g| *g > 0.0)) {
        Some(PngColor::Chromaticity { gamma, matrix })
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
/// videos and VP9 browser renditions are 8-bit limited-range I420,
/// optionally with straight alpha.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// An opaque 8-bit RGB ramp (gray 0, 8, ..., 248 and 255) with
    /// `chunks` inserted after IHDR.
    fn ramp_png(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        use image::ImageEncoder;
        let values: Vec<u8> = (0..32).map(|i| (i * 8) as u8).chain([255]).collect();
        let rgb: Vec<u8> = values.iter().flat_map(|&v| [v, v, v]).collect();
        let mut base = Vec::new();
        image::codecs::png::PngEncoder::new(&mut base)
            .write_image(&rgb, values.len() as u32, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        // Signature (8) and IHDR (8 + 13 + 4).
        let mut out = base[..33].to_vec();
        for (ty, body) in chunks {
            out.extend((body.len() as u32).to_be_bytes());
            let mut typed = ty.to_vec();
            typed.extend(body);
            out.extend(&typed);
            out.extend(crc32(&typed).to_be_bytes());
        }
        out.extend(&base[33..]);
        out
    }

    fn gama(v: u32) -> (&'static [u8; 4], Vec<u8>) {
        (b"gAMA", v.to_be_bytes().to_vec())
    }

    /// SkPngRustCodec: a `gAMA` alone selects a pure power curve unless it
    /// is within 5% of 1/2.2; `sRGB`, `iCCP`, `cHRM` take precedence, and
    /// a Display P3 `cICP` selects P3.
    #[test]
    fn png_color_selects_gamma_like_blink() {
        let dir = tempfile::Builder::new()
            .prefix("pm-compat-png-")
            .tempdir()
            .unwrap();
        let check = |name: &str, chunks: &[(&[u8; 4], Vec<u8>)]| {
            let path = dir.path().join(format!("{name}.png"));
            std::fs::write(&path, ramp_png(chunks)).unwrap();
            png_color(&path).unwrap()
        };
        assert!(check("none", &[]).is_none());
        for (v, applied) in [
            (43_181, true),
            (43_190, false),
            (45_455, false),
            (47_720, false),
            (47_730, true),
            (70_000, true),
            (100_000, true),
            (0, false),
        ] {
            let got = check(&format!("g{v}"), &[gama(v)]);
            match got {
                Some(PngColor::Gamma(g)) => {
                    assert!(applied, "{v}");
                    assert_eq!(g, v as f32 / 100_000.0);
                }
                None => assert!(!applied, "{v}"),
                Some(_) => panic!("{v}"),
            }
        }
        assert!(check("srgb", &[(b"sRGB", vec![0]), gama(70_000)]).is_none());
        // Measured: Chromium reads both as sRGB (fixtures/stills2,
        // png-chrm-invalid-gama-*.png and png-chrm-p3-nogama-*.png).
        assert!(check("chrm-invalid", &[(b"cHRM", vec![0; 32]), gama(70_000)]).is_none());
        let p3 = [31270u32, 32900, 68000, 32000, 26500, 69000, 15000, 6000]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect::<Vec<_>>();
        assert!(check("chrm-no-gama", &[(b"cHRM", p3.clone())]).is_none());
        assert!(matches!(
            check("chrm-gama", &[(b"cHRM", p3), gama(70_000)]),
            Some(PngColor::Chromaticity { .. })
        ));
        assert!(matches!(
            check("p3", &[(b"cICP", vec![12, 13, 0, 1]), gama(70_000)]),
            Some(PngColor::Cicp(12, 13))
        ));
    }

    /// The level 1 bitmap of a gamma-only PNG decodes the stored samples
    /// with the pure `1 / gamma` power and encodes them to sRGB bytes,
    /// within skcms rounding (1 code value) of the exact curve; a neutral
    /// gamma leaves the bytes unchanged.
    #[test]
    fn gamma_png_bitmap_applies_the_inverse_power() {
        let dir = tempfile::Builder::new()
            .prefix("pm-compat-gamma-")
            .tempdir()
            .unwrap();
        let pool = FramePool::new(0);
        let bytes = |name: &str, chunks: &[(&[u8; 4], Vec<u8>)]| -> Vec<u8> {
            let path = dir.path().join(format!("{name}.png"));
            std::fs::write(&path, ramp_png(chunks)).unwrap();
            let still = super::super::read(&path, &pool).unwrap();
            let bitmap = bitmap_at(&path, &still, &pool).unwrap();
            let w = bitmap.frame.format.width;
            bitmap
                .frame
                .with_planes(|p| unsafe { std::slice::from_raw_parts(p[0].ptr, w * 4).to_vec() })
                .unwrap()
        };
        let plain = bytes("plain", &[]);
        assert_eq!(bytes("neutral", &[gama(45_455)]), plain);
        let oetf = |l: f64| {
            if l <= 0.003_130_8 {
                12.92 * l
            } else {
                1.055 * l.powf(1.0 / 2.4) - 0.055
            }
        };
        for gamma in [70_000u32, 100_000, 30_000] {
            let got = bytes(&format!("g{gamma}"), &[gama(gamma)]);
            let mut changed = false;
            for (px, stored) in got.chunks_exact(4).zip(plain.chunks_exact(4)) {
                let v = f64::from(stored[0]) / 255.0;
                let want = oetf(v.powf(100_000.0 / f64::from(gamma))) * 255.0;
                for &c in &px[..3] {
                    assert!((f64::from(c) - want).abs() <= 1.0, "{gamma}: {c} vs {want}");
                }
                assert_eq!(px[3], 255);
                changed |= px[0] != stored[0];
            }
            assert!(changed, "{gamma}");
        }
    }
}
