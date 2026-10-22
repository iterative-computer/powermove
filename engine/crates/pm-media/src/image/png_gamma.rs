//! PNG `gAMA` without a higher-precedence color chunk, read the way
//! ImageIO reads it (compat level 2 and Interpret Footage). Level 1 follows
//! Chromium instead (`compat.rs`, `png_color`).
//!
//! A `gAMA` before `IDAT` applies when no `sRGB`, `iCCP` or `cICP` chunk
//! precedes `IDAT`. Its value is the file (encoding) gamma `g`: stored
//! samples are linear light raised to `g`, so decoding raises them to
//! `1 / g` with sRGB primaries. ImageIO (measured on macOS 27 with
//! `CGImageSourceCreateImageAtIndex` drawn into extended linear sRGB) reads
//! a gamma whose decoding exponent is within 0.0005 of 2.2 (stored values
//! 45445 through 45464) as sRGB itself and every other gamma, 1.0
//! included, as that pure power curve. Chromium's window is wider (0.95 to
//! 1.05 of 2.2) and applies only at level 1.

use super::{ImageColor, Rgba, StillImage, pool_rgba};
use crate::error::{MediaError, Result};
use crate::pool::FramePool;
use pm_protocol::color::{AlphaMode, Transfer};
use std::io::Read;
use std::path::Path;

/// The file gamma of a PNG whose `gAMA` applies, or `None` (not a PNG,
/// no `gAMA`, a zero value, or a `sRGB`, `iCCP` or `cICP` chunk before
/// `IDAT`). A `cHRM` chunk does not change the answer; stills with one are
/// a separate class.
pub fn png_gamma(path: &Path) -> Result<Option<f32>> {
    let mut file = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut signature = [0; 8];
    if file.read_exact(&mut signature).is_err() || signature != *b"\x89PNG\r\n\x1a\n" {
        return Ok(None);
    }
    let mut gamma = None;
    loop {
        let mut header = [0; 8];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let len = u32::from_be_bytes(header[..4].try_into().unwrap());
        match &header[4..] {
            b"IDAT" | b"IEND" => break,
            b"sRGB" | b"iCCP" | b"cICP" => return Ok(None),
            b"gAMA" if len == 4 => {
                let mut b = [0; 4];
                file.read_exact(&mut b)?;
                gamma = Some(u32::from_be_bytes(b));
                file.seek_relative(4)?;
                continue;
            }
            _ => {}
        }
        file.seek_relative(i64::from(len) + 4)?;
    }
    Ok(gamma.filter(|&g| g > 0).map(|g| g as f32 / 100_000.0))
}

/// Whether ImageIO reads a file gamma as sRGB.
pub fn reads_as_srgb(file_gamma: f32) -> bool {
    (1.0 / f64::from(file_gamma) - 2.2).abs() < 0.0005
}

/// The gamma ImageIO applies to `path` beyond reading it as sRGB, if any.
pub fn imageio_gamma(path: &Path) -> Result<Option<f32>> {
    Ok(png_gamma(path)?.filter(|&g| !reads_as_srgb(g)))
}

/// Decodes an integer RGBA still stored with file gamma `file_gamma` into
/// premultiplied linear light with Rec.709 primaries in float, the form
/// [`super::imageio::convert_to_linear_srgb`] returns: each color sample
/// `v` becomes `v^(1/file_gamma)`, alpha is kept, and straight color is
/// premultiplied in linear light.
pub fn gamma_to_linear(
    still: &StillImage,
    file_gamma: f32,
    pool: &FramePool,
) -> Result<StillImage> {
    let f = &still.frame;
    if f.format.layout != crate::frame::Layout::Rgba
        || f.format.is_float()
        || f.format.rgba_premultiplied
        || !matches!(still.color.alpha, AlphaMode::Straight | AlphaMode::Opaque)
        || file_gamma <= 0.0
    {
        return Err(MediaError::unsupported(
            "PNG gamma on a float, planar or premultiplied still",
        ));
    }
    let (w, h) = (f.format.width, f.format.height);
    let sixteen = f.format.planes[0].format == crate::frame::PlaneFormat::Rgba16;
    let max = if sixteen {
        ((1u32 << f.format.bits.clamp(1, 16)) - 1) as f64
    } else {
        255.0
    };
    let exponent = 1.0 / f64::from(file_gamma);
    let opaque = still.color.alpha == AlphaMode::Opaque;
    let out: Vec<f32> = f.with_planes(|pl| {
        let p = &pl[0];
        let mut out = Vec::with_capacity(w * h * 4);
        let mut push = |px: [f64; 4]| {
            let a = if opaque { 1.0 } else { px[3] / max };
            for &v in &px[..3] {
                out.push(((v / max).powf(exponent) * a) as f32);
            }
            out.push(a as f32);
        };
        for y in 0..h {
            // SAFETY: the plane holds `height` rows of `stride` bytes.
            let row = unsafe {
                std::slice::from_raw_parts(p.ptr.add(y * p.stride), w * if sixteen { 8 } else { 4 })
            };
            if sixteen {
                for c in row.chunks_exact(8) {
                    push([0, 2, 4, 6].map(|i| f64::from(u16::from_le_bytes([c[i], c[i + 1]]))));
                }
            } else {
                for c in row.chunks_exact(4) {
                    push([0, 1, 2, 3].map(|i| f64::from(c[i])));
                }
            }
        }
        out
    })?;
    let frame = pool_rgba(
        pool,
        w,
        h,
        Rgba::F32(&out),
        !opaque,
        32,
        format!(
            "linear light from PNG gamma {file_gamma} ({})",
            f.format.label
        ),
    )?;
    Ok(StillImage {
        frame,
        format: still.format,
        backend: still.backend,
        bit_depth: 32,
        float: true,
        color: ImageColor {
            icc: None,
            icc_bytes: None,
            icc_class: None,
            transfer: Transfer::Linear,
            alpha: if opaque {
                AlphaMode::Opaque
            } else {
                AlphaMode::Premultiplied
            },
            chromaticities: None,
        },
        orientation: still.orientation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = (body.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(ty);
        c.extend_from_slice(body);
        c.extend_from_slice(&[0; 4]); // The CRC is not read.
        c
    }

    fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]));
        for (ty, body) in chunks {
            v.extend(chunk(ty, body));
        }
        v.extend(chunk(b"IEND", &[]));
        v
    }

    #[test]
    fn gamma_applies_only_without_higher_precedence_chunks() {
        let dir = tempfile::Builder::new()
            .prefix("pm-png-gamma-")
            .tempdir()
            .unwrap();
        let g = |v: u32| v.to_be_bytes();
        type Chunks<'a> = &'a [(&'a [u8; 4], &'a [u8])];
        let cases: [(Chunks, Option<f32>); 8] = [
            (&[], None),
            (&[(b"gAMA", &g(70_000))], Some(0.7)),
            (&[(b"gAMA", &g(0))], None),
            (&[(b"sRGB", &[0]), (b"gAMA", &g(70_000))], None),
            (&[(b"gAMA", &g(70_000)), (b"iCCP", b"x\0\0")], None),
            (&[(b"cICP", &[1, 13, 0, 1]), (b"gAMA", &g(70_000))], None),
            (&[(b"cHRM", &[0; 32]), (b"gAMA", &g(70_000))], Some(0.7)),
            // After IDAT: ignored.
            (&[(b"IDAT", &[]), (b"gAMA", &g(70_000))], None),
        ];
        for (i, (chunks, want)) in cases.iter().enumerate() {
            let path = dir.path().join(format!("{i}.png"));
            std::fs::write(&path, png(chunks)).unwrap();
            assert_eq!(png_gamma(&path).unwrap(), *want, "case {i}");
        }
        std::fs::write(dir.path().join("not.png"), b"GIF89a").unwrap();
        assert_eq!(png_gamma(&dir.path().join("not.png")).unwrap(), None);
    }

    /// ImageIO's sRGB window, measured: 45445..=45464 read as sRGB; 45444,
    /// 45465, 45000, 46000 and 100000 as pure power curves.
    #[test]
    fn imageio_srgb_window() {
        for v in [45445, 45450, 45455, 45460, 45464] {
            assert!(reads_as_srgb(v as f32 / 100_000.0), "{v}");
        }
        for v in [45444, 45465, 45000, 46000, 100_000, 70_000] {
            assert!(!reads_as_srgb(v as f32 / 100_000.0), "{v}");
        }
    }

    #[test]
    fn decodes_pure_power_premultiplied_linear() {
        let pool = FramePool::new(0);
        let px: [u8; 8] = [0, 64, 128, 255, 192, 255, 32, 128];
        let frame = pool_rgba(&pool, 2, 1, Rgba::U8(&px), false, 8, "t".into()).unwrap();
        let still = StillImage {
            frame,
            format: super::super::ImageFormat::Png,
            backend: super::super::ImageBackend::ImageCrate,
            bit_depth: 8,
            float: false,
            color: ImageColor {
                icc: None,
                icc_bytes: None,
                icc_class: None,
                transfer: Transfer::DisplayE,
                alpha: AlphaMode::Straight,
                chromaticities: None,
            },
            orientation: 1,
        };
        let linear = gamma_to_linear(&still, 0.7, &pool).unwrap();
        assert!(linear.float && linear.color.alpha == AlphaMode::Premultiplied);
        let got: Vec<f32> = linear
            .frame
            .with_planes(|p| unsafe {
                std::slice::from_raw_parts(p[0].ptr as *const f32, 8).to_vec()
            })
            .unwrap();
        let e = |v: f64, a: f64| ((v / 255.0).powf(1.0 / 0.7) * a) as f32;
        let a1 = 128.0 / 255.0;
        let want = [
            0.0,
            e(64.0, 1.0),
            e(128.0, 1.0),
            1.0,
            e(192.0, a1),
            e(255.0, a1),
            e(32.0, a1),
            a1 as f32,
        ];
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-6, "{got:?} {want:?}");
        }
        // ImageIO's measured reading of gAMA 0.7 (8-bit 64, 128, 192):
        // 0.13867, 0.37329, 0.66650: ColorSync's table precision, within
        // 0.0003 (0.08 of an 8-bit code value) of the exact curve.
        for (v, imageio) in [
            (64.0, 0.138_671_88),
            (128.0, 0.373_291),
            (192.0, 0.666_503_9),
        ] {
            assert!((e(v, 1.0) - imageio).abs() < 3e-4, "{v}");
        }
    }
}
