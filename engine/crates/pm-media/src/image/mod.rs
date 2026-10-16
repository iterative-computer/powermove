//! Still and animated image readers (section 13 "Images"): libjpeg-turbo for
//! JPEG (the decoder today's viewer uses); the `image` crate for PNG (8 and
//! 16-bit), TIFF (8, 16, 32f), BMP, ICO, GIF, and WebP; ImageIO for
//! HEIF/HEIC, AVIF, CMYK JPEG, animation frames, and anything else macOS
//! reads; OpenEXR's C library for EXR; FFmpeg for DPX; resvg for SVG.
//!
//! Pixels are returned as they are stored, never color managed: an RGBA
//! plane in pool memory at the file's depth (8 or 16-bit integer, half or
//! float), with the ICC profile classified for section 9 (sRGB-equivalent
//! profiles pass through exactly). EXIF orientation is applied, as the
//! browser applies it today.

pub mod exr;
pub mod icc;
pub mod imageio;
pub mod jpeg;
pub mod raster;
pub mod sequence;
pub mod svg;

use crate::error::{MediaError, Result};
use crate::frame::{
    DecodedFrame, Layout, PixelFormat, Pixels, PlaneDesc, PlaneFormat, PoolFrame, PoolPlane,
};
use crate::pool::{FramePool, ROW_ALIGN};
use icc::IccClass;
use pm_protocol::color::{AlphaMode, Range, SampleAlignment, Transfer};
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageBackend {
    LibjpegTurbo,
    ImageCrate,
    ImageIo,
    OpenExr,
    Ffmpeg,
    Resvg,
}

/// Formats recognized from the first bytes of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Tiff,
    Bmp,
    Ico,
    Gif,
    WebP,
    Exr,
    Dpx,
    Heif,
    Avif,
    Svg,
    Other,
}

pub fn sniff(head: &[u8]) -> ImageFormat {
    let at = |o: usize, m: &[u8]| head.get(o..o + m.len()) == Some(m);
    if at(0, b"\x89PNG\r\n\x1a\n") {
        ImageFormat::Png
    } else if at(0, &[0xFF, 0xD8, 0xFF]) {
        ImageFormat::Jpeg
    } else if at(0, b"II*\0") || at(0, b"MM\0*") || at(0, b"II+\0") || at(0, b"MM\0+") {
        ImageFormat::Tiff
    } else if at(0, b"BM") {
        ImageFormat::Bmp
    } else if at(0, &[0, 0, 1, 0]) {
        ImageFormat::Ico
    } else if at(0, b"GIF87a") || at(0, b"GIF89a") {
        ImageFormat::Gif
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        ImageFormat::WebP
    } else if at(0, &[0x76, 0x2F, 0x31, 0x01]) {
        ImageFormat::Exr
    } else if at(0, b"SDPX") || at(0, b"XPDS") {
        ImageFormat::Dpx
    } else if at(4, b"ftyp") {
        // Only image brands: MOV and MP4 movies carry `ftyp` too.
        let size = head
            .get(..4)
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
        let brand = head.get(8..12).unwrap_or(&[]);
        let compat: Vec<&[u8]> = head
            .get(16..size.clamp(16, head.len()))
            .unwrap_or(&[])
            .chunks_exact(4)
            .collect();
        let any = |set: &[&[u8; 4]]| {
            set.iter()
                .any(|b| brand == &b[..] || compat.iter().any(|c| *c == &b[..]))
        };
        if any(&[b"avif", b"avis"]) {
            ImageFormat::Avif
        } else if any(&[
            b"mif1", b"msf1", b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis",
        ]) {
            ImageFormat::Heif
        } else {
            ImageFormat::Other
        }
    } else if svg::sniff(head) {
        ImageFormat::Svg
    } else {
        ImageFormat::Other
    }
}

/// How a still's values relate to light and to its alpha.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageColor {
    #[serde(skip)]
    pub icc: Option<Vec<u8>>,
    pub icc_bytes: Option<usize>,
    /// The embedded profile's class; None without a profile (read as sRGB).
    pub icc_class: Option<IccClass>,
    pub transfer: Transfer,
    pub alpha: AlphaMode,
    /// OpenEXR `chromaticities` (red, green, blue, white x/y), when present;
    /// absent means Rec.709 primaries.
    pub chromaticities: Option<[f32; 8]>,
}

/// A decoded still (or one animation frame).
#[derive(Clone, Debug)]
pub struct StillImage {
    /// One RGBA plane in pool memory (`Layout::Rgba`).
    pub frame: DecodedFrame,
    pub format: ImageFormat,
    pub backend: ImageBackend,
    /// Bits per stored component (8, 16, 32 for float; 16 for half).
    pub bit_depth: u8,
    pub float: bool,
    pub color: ImageColor,
    /// EXIF orientation (1 to 8) that was applied.
    pub orientation: u8,
}

/// Animation facts.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Animation {
    pub frames: usize,
    /// Each frame's delay in seconds.
    pub delays: Vec<f64>,
    pub loop_count: Option<u32>,
}

/// Reads a still (the first frame of an animation).
pub fn read(path: &Path, pool: &FramePool) -> Result<StillImage> {
    let mut head = [0u8; 512];
    let n = {
        use std::io::Read;
        std::fs::File::open(path)?.read(&mut head)?
    };
    let format = sniff(&head[..n]);
    match format {
        ImageFormat::Exr => exr::read(path, pool),
        ImageFormat::Jpeg => jpeg::read(path, pool),
        ImageFormat::Svg => svg::read(path, pool),
        ImageFormat::Dpx => ffmpeg_still(path, pool),
        ImageFormat::Heif | ImageFormat::Avif | ImageFormat::Other => {
            imageio::read_frame(path, 0, pool, format)
        }
        _ => raster::read(path, pool, format),
    }
}

/// Animation frame count and delays (GIF, APNG, animated WebP and AVIF,
/// HEIF sequences), through ImageIO. None for stills.
pub fn animation(path: &Path) -> Result<Option<Animation>> {
    imageio::animation(path)
}

/// Frame `index` of an animated image, fully composited.
pub fn read_animation_frame(path: &Path, index: usize, pool: &FramePool) -> Result<StillImage> {
    let mut head = [0u8; 64];
    let n = {
        use std::io::Read;
        std::fs::File::open(path)?.read(&mut head)?
    };
    imageio::read_frame(path, index, pool, sniff(&head[..n]))
}

/// Packed RGBA samples ready to place in pool memory.
pub(crate) enum Rgba<'a> {
    U8(&'a [u8]),
    U16(&'a [u16]),
    F16(&'a [u16]),
    F32(&'a [f32]),
}

/// Copies packed RGBA rows into a pool frame (Metal-aligned rows).
pub(crate) fn pool_rgba(
    pool: &FramePool,
    w: usize,
    h: usize,
    px: Rgba<'_>,
    premultiplied: bool,
    bits: u8,
    label: String,
) -> Result<DecodedFrame> {
    let (pf, bpp) = match px {
        Rgba::U8(_) => (PlaneFormat::Rgba8, 4),
        Rgba::U16(_) => (PlaneFormat::Rgba16, 8),
        Rgba::F16(_) => (PlaneFormat::Rgba16Float, 8),
        Rgba::F32(_) => (PlaneFormat::Rgba32Float, 16),
    };
    let stride = (w * bpp).div_ceil(ROW_ALIGN) * ROW_ALIGN;
    let slot = pool.acquire(stride * h.max(1))?;
    let src: &[u8] = match px {
        Rgba::U8(v) => v,
        Rgba::U16(v) | Rgba::F16(v) => unsafe {
            std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2)
        },
        Rgba::F32(v) => unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) },
    };
    if src.len() < w * h * bpp {
        return Err(MediaError::Image(format!(
            "{} bytes for a {w}×{h} image",
            src.len()
        )));
    }
    for y in 0..h {
        unsafe {
            std::ptr::copy_nonoverlapping(
                src.as_ptr().add(y * w * bpp),
                slot.ptr().add(y * stride),
                w * bpp,
            )
        };
    }
    let float = matches!(pf, PlaneFormat::Rgba16Float | PlaneFormat::Rgba32Float);
    Ok(DecodedFrame {
        pts: 0,
        format: Arc::new(PixelFormat {
            layout: Layout::Rgba,
            bits: if float { 16 } else { bits },
            alignment: SampleAlignment::Lsb,
            range: Range::Full,
            log2_chroma_w: 0,
            log2_chroma_h: 0,
            alpha_bits: if float { 16 } else { bits },
            planes: vec![PlaneDesc {
                format: pf,
                width: w,
                height: h,
            }],
            width: w,
            height: h,
            label,
            rgba_premultiplied: premultiplied,
        }),
        pixels: Pixels::Pool(Arc::new(PoolFrame {
            planes: vec![PoolPlane {
                decoder_ptr: slot.ptr() as usize,
                slot,
                offset: 0,
                stride,
            }],
            owner: None,
            copied: true,
        })),
        color: None,
    })
}

/// Applies EXIF orientation (1 to 8) to packed pixels of `c` components.
pub(crate) fn orient<T: Copy>(
    px: &[T],
    w: usize,
    h: usize,
    c: usize,
    orientation: u8,
) -> (Vec<T>, usize, usize) {
    let swap = matches!(orientation, 5..=8);
    let (ow, oh) = if swap { (h, w) } else { (w, h) };
    let mut out = Vec::with_capacity(px.len());
    for oy in 0..oh {
        for ox in 0..ow {
            // Source coordinates for each output pixel (EXIF definitions).
            let (sx, sy) = match orientation {
                2 => (w - 1 - ox, oy),
                3 => (w - 1 - ox, h - 1 - oy),
                4 => (ox, h - 1 - oy),
                5 => (oy, ox),
                6 => (oy, h - 1 - ox),
                7 => (w - 1 - oy, h - 1 - ox),
                8 => (w - 1 - oy, ox),
                _ => (ox, oy),
            };
            let i = (sy * w + sx) * c;
            out.extend_from_slice(&px[i..i + c]);
        }
    }
    (out, ow, oh)
}

/// DPX and other single-image formats FFmpeg reads: the first frame,
/// decoded into pool memory (planar RGB or Y′CbCr at native depth).
pub fn ffmpeg_still(path: &Path, pool: &FramePool) -> Result<StillImage> {
    use crate::ff::{Input, PacketReader};
    let input = Input::open(path)?;
    let v = input
        .best_video()
        .ok_or_else(|| MediaError::invalid("no image stream"))?;
    let idx = crate::index::build(path, &input, v)?;
    let mut dec = crate::ff::decoder::FfDecoder::new(&input, v, None, pool.clone(), 1)?;
    let mut reader = PacketReader::new(path, v)?;
    let pkts = reader.read(&idx, &idx.decode_plan_for_frame(0))?;
    let mut out = Vec::new();
    for p in &pkts {
        dec.send(Some(p), &mut out)?;
    }
    dec.send(None, &mut out)?;
    let frame = out
        .into_iter()
        .next()
        .ok_or_else(|| MediaError::decode("FFmpeg output no image"))?;
    let bits = frame.format.bits;
    Ok(StillImage {
        format: ImageFormat::Dpx,
        backend: ImageBackend::Ffmpeg,
        bit_depth: bits,
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
        frame,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_rotates_and_flips() {
        // 3×2 image, one component: 1 2 3 / 4 5 6.
        let px = [1, 2, 3, 4, 5, 6];
        assert_eq!(orient(&px, 3, 2, 1, 1), (px.to_vec(), 3, 2));
        assert_eq!(orient(&px, 3, 2, 1, 2).0, vec![3, 2, 1, 6, 5, 4]);
        assert_eq!(orient(&px, 3, 2, 1, 3).0, vec![6, 5, 4, 3, 2, 1]);
        // 6: rotate 90° clockwise to display.
        assert_eq!(orient(&px, 3, 2, 1, 6), (vec![4, 1, 5, 2, 6, 3], 2, 3));
        // 8: rotate 90° counterclockwise.
        assert_eq!(orient(&px, 3, 2, 1, 8), (vec![3, 6, 2, 5, 1, 4], 2, 3));
    }

    #[test]
    fn sniffs_formats() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), ImageFormat::Png);
        assert_eq!(
            sniff(b"\0\0\0\x1cftypheic\0\0\0\0mif1heic"),
            ImageFormat::Heif
        );
        assert_eq!(sniff(b"\0\0\0\x1cftypavif\0\0\0\0"), ImageFormat::Avif);
        // Movies are not images.
        assert_eq!(
            sniff(b"\0\0\0\x14ftypqt  \0\0\x02\0qt  \0\0\0\x08wide"),
            ImageFormat::Other
        );
        assert_eq!(
            sniff(b"\0\0\0\x20ftypisom\0\0\x02\0isomiso2avc1mp41"),
            ImageFormat::Other
        );
        assert_eq!(
            sniff(&[0x76, 0x2F, 0x31, 0x01, 2, 0, 0, 0]),
            ImageFormat::Exr
        );
    }
}
