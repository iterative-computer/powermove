//! The still-image classes this renderer draws (docs/engine/pm-render.md,
//! "Image classes").
//!
//! Capability routing must be conservative (plan section 21): a still is
//! drawn only when its container, encoding, depth, and color tagging fall in
//! a class compared against the legacy renderer on the parity corpus
//! (`render/still-*` and the fixture stills). Everything else is routed to
//! the legacy engine, even where decoding would very likely match, because
//! the legacy pipeline (Chromium's decoders, and the PNG conversion it
//! applies to TIFF and HEIF on import) differs from ours in ways only a
//! comparison shows: IDCT and chroma upsampling for JPEG and lossy WebP,
//! 16 to 8-bit reduction, gamma and chromaticity chunks, color management.
//!
//! The class is decided from the decoded [`StillImage`] facts plus a header
//! parse of the container for what the decoder does not report (JPEG frame
//! type and sampling factors, PNG color chunks, WebP chunk layout).

use std::path::Path;

use pm_media::image::icc::IccClass;
use pm_media::image::{ImageFormat, StillImage};
use pm_protocol::color::AlphaMode;

use crate::capability::Feature;

fn unsupported(what: impl Into<String>) -> Result<(), Feature> {
    Err(Feature::Media(what.into()))
}

/// Whether a decoded still is in a tested class.
pub fn check(path: &Path, still: &StillImage) -> Result<(), Feature> {
    if still.float {
        return unsupported("floating-point stills");
    }
    let bytes =
        std::fs::read(path).map_err(|e| Feature::Media(format!("{}: {e}", path.display())))?;
    let icc = still.color.icc_class;
    match still.format {
        ImageFormat::Png => png(&bytes)?,
        ImageFormat::Jpeg => jpeg(&bytes)?,
        ImageFormat::WebP => webp(&bytes)?,
        ImageFormat::Avif | ImageFormat::Heif => {
            if still.bit_depth != 8 {
                return unsupported("AVIF and HEIF stills deeper than 8 bits");
            }
            if still.color.alpha != AlphaMode::Opaque {
                return unsupported("AVIF and HEIF stills with alpha");
            }
            if still.format == ImageFormat::Avif && icc == Some(IccClass::DisplayP3) {
                return unsupported("Display P3 AVIF stills");
            }
        }
        ImageFormat::Tiff => {
            if still.bit_depth != 8 {
                return unsupported("TIFF stills deeper than 8 bits");
            }
            if still.color.alpha != AlphaMode::Opaque {
                return unsupported("TIFF stills with alpha");
            }
            if icc.is_some() {
                return unsupported("TIFF stills with an ICC profile");
            }
        }
        ImageFormat::Bmp => return unsupported("BMP stills"),
        ImageFormat::Ico => return unsupported("ICO stills"),
        ImageFormat::Gif => return unsupported("GIF stills"),
        ImageFormat::Exr => return unsupported("OpenEXR stills"),
        ImageFormat::Dpx => return unsupported("DPX stills"),
        ImageFormat::Other => return unsupported("stills of an unrecognized format"),
    }
    // Display P3 is tested in PNG, JPEG, and HEIC.
    if icc == Some(IccClass::DisplayP3)
        && !matches!(
            still.format,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Heif
        )
    {
        return unsupported(format!("Display P3 {:?} stills", still.format));
    }
    Ok(())
}

/// Whether an image asset's recorded format (or file name extension) is a
/// container this renderer may draw, for the static check that opens no
/// media. An asset with neither is unsupported until its file is probed.
pub fn static_format(format: &str, name: &str) -> Result<(), Feature> {
    let ext = if format.is_empty() {
        name.rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default()
    } else {
        format.to_ascii_lowercase()
    };
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "jpe" | "jfif" | "webp" | "avif" | "heic" | "heif" | "tif"
        | "tiff" => Ok(()),
        "" => unsupported("an image of unrecorded format"),
        other => unsupported(format!("{other} stills")),
    }
}

/// PNG: 8 or 16-bit samples; no HDR or CICP chunks; gamma and
/// chromaticity chunks only where an sRGB chunk or ICC profile overrides
/// them (as in Chromium and libpng).
fn png(bytes: &[u8]) -> Result<(), Feature> {
    let mut at = 8usize;
    let (mut depth, mut srgb, mut iccp, mut gama, mut chrm) = (0u8, false, false, false, false);
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let ty = &bytes[at + 4..at + 8];
        let body = bytes.get(at + 8..at + 8 + len).unwrap_or(&[]);
        match ty {
            b"IHDR" if body.len() >= 13 => depth = body[8],
            b"sRGB" => srgb = true,
            b"iCCP" => iccp = true,
            b"gAMA" => gama = true,
            b"cHRM" => chrm = true,
            b"cICP" | b"mDCv" | b"cLLi" | b"mDCV" | b"cLLI" => {
                return unsupported("PNG stills with CICP or HDR metadata");
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    if !matches!(depth, 8 | 16) {
        return unsupported(format!("{depth}-bit PNG stills"));
    }
    if (gama || chrm) && !srgb && !iccp {
        return unsupported("PNG stills with gamma or chromaticity chunks (gAMA, cHRM)");
    }
    Ok(())
}

/// The Y'CbCr sampling-factor layouts compared against the legacy renderer
/// ((luma h, v), (chroma h, v)): 4:4:4, 4:2:2, and 4:2:0 as libjpeg writes
/// them, and 4:4:4 and 4:2:2 as FFmpeg writes them (two-block-tall MCUs).
const TESTED_JPEG_LAYOUTS: [((u8, u8), (u8, u8)); 5] = [
    ((1, 1), (1, 1)),
    ((2, 1), (1, 1)),
    ((2, 2), (1, 1)),
    ((1, 2), (1, 2)),
    ((2, 2), (1, 2)),
];

/// JPEG: baseline or progressive Huffman, 8-bit, gray or Y'CbCr in a tested
/// sampling layout; not Adobe RGB-transform or CMYK.
fn jpeg(bytes: &[u8]) -> Result<(), Feature> {
    let mut at = 2usize;
    let mut adobe_transform: Option<u8> = None;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF {
            return unsupported("malformed JPEG headers");
        }
        let marker = bytes[at + 1];
        if marker == 0xFF {
            at += 1;
            continue;
        }
        let len = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        let body = bytes.get(at + 4..at + 2 + len).unwrap_or(&[]);
        match marker {
            0xEE if body.len() >= 12 && body.starts_with(b"Adobe") => {
                adobe_transform = Some(body[11]);
            }
            0xC0 | 0xC2 => return jpeg_frame(body, adobe_transform),
            0xC1 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return unsupported("JPEG stills other than baseline or progressive Huffman");
            }
            0xDA => break,
            _ => {}
        }
        at += 2 + len;
    }
    unsupported("malformed JPEG headers")
}

fn jpeg_frame(sof: &[u8], adobe_transform: Option<u8>) -> Result<(), Feature> {
    if sof.len() < 6 {
        return unsupported("malformed JPEG headers");
    }
    if sof[0] != 8 {
        return unsupported(format!("{}-bit JPEG stills", sof[0]));
    }
    let n = sof[5] as usize;
    let comps: Vec<(u8, u8, u8)> = sof[6..]
        .chunks_exact(3)
        .take(n)
        .map(|c| (c[0], c[1] >> 4, c[1] & 15))
        .collect();
    if comps.len() != n {
        return unsupported("malformed JPEG headers");
    }
    match n {
        1 => Ok(()),
        3 => {
            let ids: Vec<u8> = comps.iter().map(|c| c.0).collect();
            if adobe_transform == Some(0) || ids == b"RGB" {
                return unsupported("RGB-encoded JPEG stills");
            }
            let factors = |c: (u8, u8, u8)| (c.1, c.2);
            let layout = (factors(comps[0]), factors(comps[1]));
            if factors(comps[1]) != factors(comps[2]) || !TESTED_JPEG_LAYOUTS.contains(&layout) {
                return unsupported(format!(
                    "JPEG sampling factors {}x{}, {}x{}, {}x{} (untested layout)",
                    comps[0].1, comps[0].2, comps[1].1, comps[1].2, comps[2].1, comps[2].2
                ));
            }
            Ok(())
        }
        _ => unsupported("CMYK and other JPEG stills with neither 1 nor 3 components"),
    }
}

/// WebP: lossy (`VP8 `), lossless (`VP8L`), or extended with alpha; no ICC
/// profile.
fn webp(bytes: &[u8]) -> Result<(), Feature> {
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        match &bytes[at..at + 4] {
            b"ICCP" => return unsupported("WebP stills with an ICC profile"),
            b"ANIM" | b"ANMF" => return unsupported("animated WebP"),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_with(depth: u8, chunks: &[&[u8; 4]]) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |ty: &[u8; 4], body: &[u8]| {
            b.extend_from_slice(&(body.len() as u32).to_be_bytes());
            b.extend_from_slice(ty);
            b.extend_from_slice(body);
            b.extend_from_slice(&[0; 4]);
        };
        chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, depth, 6, 0, 0, 0]);
        for c in chunks {
            chunk(c, &[0; 4]);
        }
        chunk(b"IDAT", &[]);
        b
    }

    #[test]
    fn png_classes() {
        assert!(png(&png_with(8, &[])).is_ok());
        assert!(png(&png_with(16, &[])).is_ok());
        assert!(png(&png_with(4, &[])).is_err());
        assert!(png(&png_with(8, &[b"gAMA"])).is_err());
        assert!(png(&png_with(8, &[b"cHRM", b"gAMA"])).is_err());
        assert!(png(&png_with(8, &[b"sRGB", b"gAMA", b"cHRM"])).is_ok());
        assert!(png(&png_with(8, &[b"iCCP", b"gAMA"])).is_ok());
        assert!(png(&png_with(8, &[b"cICP"])).is_err());
        assert!(png(&png_with(16, &[b"mDCv"])).is_err());
    }

    fn jpeg_with(sof: u8, precision: u8, comps: &[(u8, u8)], adobe: Option<u8>) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        if let Some(t) = adobe {
            let body = [b'A', b'd', b'o', b'b', b'e', 0, 100, 0, 0, 0, 0, t];
            b.extend_from_slice(&[0xFF, 0xEE, 0, (body.len() + 2) as u8]);
            b.extend_from_slice(&body);
        }
        let mut body = vec![precision, 0, 8, 0, 8, comps.len() as u8];
        for (id, hv) in comps {
            body.extend_from_slice(&[*id, *hv, 0]);
        }
        b.extend_from_slice(&[0xFF, sof, 0, (body.len() + 2) as u8]);
        b.extend_from_slice(&body);
        b.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
        b
    }

    #[test]
    fn jpeg_classes() {
        let ycc = |y: u8| [(1, y), (2, 0x11), (3, 0x11)];
        for y in [0x11, 0x21, 0x22] {
            assert!(jpeg(&jpeg_with(0xC0, 8, &ycc(y), None)).is_ok(), "{y:x}");
            assert!(jpeg(&jpeg_with(0xC2, 8, &ycc(y), None)).is_ok(), "{y:x}");
        }
        assert!(jpeg(&jpeg_with(0xC0, 8, &[(1, 0x11)], None)).is_ok());
        // FFmpeg's layouts; mismatched chroma factors.
        let layout = |y, cb, cr| jpeg(&jpeg_with(0xC0, 8, &[(1, y), (2, cb), (3, cr)], None));
        assert!(layout(0x12, 0x12, 0x12).is_ok());
        assert!(layout(0x22, 0x12, 0x12).is_ok());
        assert!(layout(0x22, 0x12, 0x11).is_err());
        // 4:4:0, 4:1:1, 12-bit, arithmetic, lossless, CMYK, RGB.
        assert!(jpeg(&jpeg_with(0xC0, 8, &ycc(0x12), None)).is_err());
        assert!(jpeg(&jpeg_with(0xC0, 8, &ycc(0x41), None)).is_err());
        assert!(jpeg(&jpeg_with(0xC1, 12, &ycc(0x11), None)).is_err());
        assert!(jpeg(&jpeg_with(0xC0, 12, &ycc(0x11), None)).is_err());
        assert!(jpeg(&jpeg_with(0xC9, 8, &ycc(0x11), None)).is_err());
        assert!(jpeg(&jpeg_with(0xC3, 8, &ycc(0x11), None)).is_err());
        let cmyk = [(1, 0x11), (2, 0x11), (3, 0x11), (4, 0x11)];
        assert!(jpeg(&jpeg_with(0xC0, 8, &cmyk, Some(2))).is_err());
        assert!(jpeg(&jpeg_with(0xC0, 8, &ycc(0x11), Some(0))).is_err());
        assert!(jpeg(&jpeg_with(0xC0, 8, &ycc(0x11), Some(1))).is_ok());
        let rgb = [(b'R', 0x11), (b'G', 0x11), (b'B', 0x11)];
        assert!(jpeg(&jpeg_with(0xC0, 8, &rgb, None)).is_err());
        assert!(jpeg(&[0xFF, 0xD8, 0x00]).is_err());
    }

    #[test]
    fn webp_classes() {
        let riff = |chunks: &[(&[u8; 4], usize)]| {
            let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
            for (ty, len) in chunks {
                b.extend_from_slice(*ty);
                b.extend_from_slice(&(*len as u32).to_le_bytes());
                b.extend(std::iter::repeat_n(0u8, len + (len & 1)));
            }
            b
        };
        assert!(webp(&riff(&[(b"VP8 ", 11)])).is_ok());
        assert!(webp(&riff(&[(b"VP8X", 10), (b"ALPH", 5), (b"VP8 ", 8)])).is_ok());
        assert!(webp(&riff(&[(b"VP8X", 10), (b"ICCP", 7), (b"VP8L", 8)])).is_err());
        assert!(webp(&riff(&[(b"VP8X", 10), (b"ANIM", 6), (b"ANMF", 8)])).is_err());
    }

    #[test]
    fn static_formats() {
        assert!(static_format("png", "").is_ok());
        assert!(static_format("", "photo.JPG").is_ok());
        assert!(static_format("HEIC", "x").is_ok());
        assert!(static_format("bmp", "x.bmp").is_err());
        assert!(static_format("gif", "x.gif").is_err());
        assert!(static_format("exr", "x.exr").is_err());
        assert!(static_format("", "noextension").is_err());
    }
}
