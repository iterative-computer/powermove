//! PNG, JPEG, TIFF, BMP, ICO, GIF, and WebP through the `image` crate, at
//! their stored depth: 8-bit formats to RGBA8, 16-bit to RGBA16, float TIFF
//! to RGBA32F. No color management; the ICC profile is classified.

use super::{ImageBackend, ImageColor, ImageFormat, Rgba, StillImage, icc, orient, pool_rgba};
use crate::error::{MediaError, Result};
use crate::pool::FramePool;
use image::{DynamicImage, ImageDecoder, metadata::Orientation};
use pm_protocol::color::{AlphaMode, Transfer};
use std::path::Path;

fn exif_number(o: Orientation) -> u8 {
    match o {
        Orientation::NoTransforms => 1,
        Orientation::FlipHorizontal => 2,
        Orientation::Rotate180 => 3,
        Orientation::FlipVertical => 4,
        Orientation::Rotate90FlipH => 5,
        Orientation::Rotate90 => 6,
        Orientation::Rotate270FlipH => 7,
        Orientation::Rotate270 => 8,
    }
}

/// The first image's `PhotometricInterpretation` (tag 262) of a classic or
/// Big TIFF, None when the file has no such tag or is not a TIFF.
pub(crate) fn tiff_photometric(path: &Path) -> Result<Option<u16>> {
    tiff_short(path, 262)
}

pub(crate) fn tiff_short(path: &Path, tag: u16) -> Result<Option<u16>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let mut head = [0u8; 16];
    if f.read(&mut head)? < 8 {
        return Ok(None);
    }
    let le = match &head[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(None),
    };
    let u16_at = |b: &[u8]| {
        let b = [b[0], b[1]];
        if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    };
    let u32_at = |b: &[u8]| {
        let b = [b[0], b[1], b[2], b[3]];
        if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let u64_at = |b: &[u8]| {
        let b: [u8; 8] = b[..8].try_into().unwrap();
        if le {
            u64::from_le_bytes(b)
        } else {
            u64::from_be_bytes(b)
        }
    };
    // (IFD offset, entry-count size, entry size)
    let (ifd, count_size, entry_size) = match u16_at(&head[2..]) {
        42 => (u64::from(u32_at(&head[4..])), 2, 12),
        43 => (u64_at(&head[8..]), 8, 20),
        _ => return Ok(None),
    };
    f.seek(SeekFrom::Start(ifd))?;
    let mut count = [0u8; 8];
    f.read_exact(&mut count[..count_size])?;
    let n = if count_size == 2 {
        u64::from(u16_at(&count))
    } else {
        u64_at(&count)
    };
    // Tags are sorted, and 262 comes early; a bounded read covers it.
    let mut entries = vec![0u8; n.min(64) as usize * entry_size];
    f.read_exact(&mut entries)?;
    Ok(entries
        .chunks_exact(entry_size)
        .find(|e| u16_at(e) == tag)
        .map(|e| u16_at(&e[if entry_size == 12 { 8 } else { 12 }..])))
}

pub fn read(path: &Path, pool: &FramePool, format: ImageFormat) -> Result<StillImage> {
    let err = |e: image::ImageError| MediaError::Image(e.to_string());
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder().map_err(err)?;
    let profile = decoder.icc_profile().map_err(err)?;
    let profile = if format == ImageFormat::Tiff && profile.is_none() {
        tiff_icc(path)?
    } else {
        profile
    };
    // Blink's WebP decoder reads no EXIF: a WebP is shown as stored, in
    // the box today's import measured. Other formats apply it as Blink does.
    let orientation = if format == ImageFormat::WebP {
        1
    } else {
        decoder.orientation().map(exif_number).unwrap_or(1)
    };
    let color_type = decoder.color_type();
    let img = DynamicImage::from_decoder(decoder).map_err(err)?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let label = format!("{format:?} {color_type:?}").to_lowercase();
    let (frame, bits, float) = match &img {
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => {
            let px = img.to_rgba32f().into_raw();
            let (px, ow, oh) = orient(&px, w, h, 4, orientation);
            (
                pool_rgba(pool, ow, oh, Rgba::F32(&px), false, 32, label)?,
                32,
                true,
            )
        }
        DynamicImage::ImageLuma16(_)
        | DynamicImage::ImageLumaA16(_)
        | DynamicImage::ImageRgb16(_)
        | DynamicImage::ImageRgba16(_) => {
            let px = img.to_rgba16().into_raw();
            let (px, ow, oh) = orient(&px, w, h, 4, orientation);
            (
                pool_rgba(pool, ow, oh, Rgba::U16(&px), false, 16, label)?,
                16,
                false,
            )
        }
        _ => {
            let px = img.to_rgba8().into_raw();
            let (px, ow, oh) = orient(&px, w, h, 4, orientation);
            (
                pool_rgba(pool, ow, oh, Rgba::U8(&px), false, 8, label)?,
                8,
                false,
            )
        }
    };
    let icc_class = profile.as_deref().map(icc::classify);
    Ok(StillImage {
        frame,
        format,
        backend: ImageBackend::ImageCrate,
        bit_depth: bits,
        float,
        color: ImageColor {
            icc_bytes: profile.as_ref().map(|p| p.len()),
            icc: profile,
            icc_class,
            // Float TIFF is scene-linear by convention; 8 and 16-bit stills
            // are display-encoded.
            transfer: if float {
                Transfer::Linear
            } else {
                Transfer::DisplayE
            },
            alpha: if format == ImageFormat::Tiff
                && color_type.has_alpha()
                && tiff_short(path, 338)? == Some(1)
            {
                AlphaMode::Premultiplied
            } else if color_type.has_alpha() {
                AlphaMode::Straight
            } else {
                AlphaMode::Opaque
            },
            chromaticities: None,
        },
        orientation,
    })
}

/// ICC's TIFF tag is UNDEFINED, which some versions of the TIFF decoder's
/// `get_tag_u8_vec` reject. Read that bounded first-IFD payload directly so
/// native-depth/level-2 color management retains the original profile.
fn tiff_icc(path: &Path) -> Result<Option<Vec<u8>>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let mut h = [0u8; 16];
    f.read_exact(&mut h)?;
    let le = &h[..2] == b"II";
    let n = |b: &[u8]| -> u64 {
        if le {
            b.iter().rev().fold(0, |v, b| (v << 8) | u64::from(*b))
        } else {
            b.iter().fold(0, |v, b| (v << 8) | u64::from(*b))
        }
    };
    let (ifd, count_size, entry_size, inline) = match n(&h[2..4]) {
        42 => (n(&h[4..8]), 2, 12, 4),
        43 => (n(&h[8..16]), 8, 20, 8),
        _ => return Ok(None),
    };
    f.seek(SeekFrom::Start(ifd))?;
    let mut count = [0u8; 8];
    f.read_exact(&mut count[..count_size])?;
    let count = n(&count[..count_size]);
    if count > 65536 {
        return Err(MediaError::invalid("TIFF IFD exceeds metadata limit"));
    }
    let mut entry = vec![0u8; entry_size];
    for _ in 0..count {
        f.read_exact(&mut entry)?;
        if n(&entry[..2]) != 34675 {
            continue;
        }
        if !matches!(n(&entry[2..4]), 1 | 7) {
            return Err(MediaError::invalid("TIFF ICC tag type"));
        }
        let size = n(&entry[4..entry_size - inline]);
        if size > 1 << 20 {
            return Err(MediaError::invalid("TIFF ICC profile exceeds 1 MiB"));
        }
        let value = &entry[entry_size - inline..];
        if size <= inline as u64 {
            return Ok(Some(value[..size as usize].to_vec()));
        }
        f.seek(SeekFrom::Start(n(value)))?;
        let mut bytes = vec![0; size as usize];
        f.read_exact(&mut bytes)?;
        return Ok(Some(bytes));
    }
    Ok(None)
}
