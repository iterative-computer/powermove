//! TIFF palette lookup before ColorSync, as in the app's FFmpeg PNG.
//! The native-depth source reader remains unchanged.
use super::{ImageColor, Rgba, StillImage, icc, orient, pool_rgba};
use crate::{MediaError, Result, pool::FramePool};
use pm_protocol::color::{AlphaMode, Transfer};

pub(super) fn read(
    path: &std::path::Path,
    original: &StillImage,
    pool: &FramePool,
) -> Result<StillImage> {
    let mut bytes = std::fs::read(path)?;
    let invalid = || MediaError::invalid("TIFF palette structure");
    let le = bytes.starts_with(b"II");
    let u16_at = |b: &[u8]| -> Option<u16> {
        let a = b.get(..2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(a)
        } else {
            u16::from_be_bytes(a)
        })
    };
    let u32_at = |b: &[u8]| -> Option<u32> {
        let a = b.get(..4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    };
    let u64_at = |b: &[u8]| -> Option<u64> {
        let a = b.get(..8)?.try_into().ok()?;
        Some(if le {
            u64::from_le_bytes(a)
        } else {
            u64::from_be_bytes(a)
        })
    };
    let number = |n: u64| usize::try_from(n).map_err(|_| invalid());
    let (ifd, count_size, entry_size, value_at) = match u16_at(bytes.get(2..).ok_or_else(invalid)?)
    {
        Some(42) => (
            number(u64::from(
                u32_at(bytes.get(4..).ok_or_else(invalid)?).ok_or_else(invalid)?,
            ))?,
            2,
            12,
            8,
        ),
        Some(43) => (
            number(u64_at(bytes.get(8..).ok_or_else(invalid)?).ok_or_else(invalid)?)?,
            8,
            20,
            12,
        ),
        _ => return Err(invalid()),
    };
    let count_bytes = bytes.get(ifd..).ok_or_else(invalid)?;
    let count = if count_size == 2 {
        usize::from(u16_at(count_bytes).ok_or_else(invalid)?)
    } else {
        number(u64_at(count_bytes).ok_or_else(invalid)?)?
    };
    if count > 4096 {
        return Err(invalid());
    }
    let entries = bytes.get(ifd + count_size..).ok_or_else(invalid)?;
    let mut photometric_at = None;
    let mut colors = None;
    for i in 0..count {
        let entry = entries
            .get(i * entry_size..(i + 1) * entry_size)
            .ok_or_else(invalid)?;
        match u16_at(entry).ok_or_else(invalid)? {
            262 => photometric_at = Some(ifd + count_size + i * entry_size + value_at),
            320 => {
                if u16_at(&entry[2..]) != Some(3) {
                    return Err(invalid());
                }
                let n = if entry_size == 12 {
                    u64::from(u32_at(&entry[4..]).ok_or_else(invalid)?)
                } else {
                    u64_at(&entry[4..]).ok_or_else(invalid)?
                };
                if n != 768 {
                    return Err(MediaError::invalid("TIFF palette needs 8-bit indices"));
                }
                let offset = if entry_size == 12 {
                    u64::from(u32_at(&entry[value_at..]).ok_or_else(invalid)?)
                } else {
                    u64_at(&entry[value_at..]).ok_or_else(invalid)?
                };
                let offset = number(offset)?;
                let map = bytes
                    .get(offset..offset.checked_add(1536).ok_or_else(invalid)?)
                    .ok_or_else(invalid)?;
                colors = Some(
                    map.chunks_exact(2)
                        .map(|b| (u16_at(b).unwrap() >> 8) as u8)
                        .collect::<Vec<_>>(),
                );
            }
            _ => {}
        }
    }
    let colors = colors.ok_or_else(invalid)?;
    let at = photometric_at.ok_or_else(invalid)?;
    // Decode the same strip/index stream as unsigned grayscale, avoiding
    // unsupported palette expansion in image-rs. Only this in-memory tag
    // changes. The source and native-depth reader keep their original data.
    bytes[at..at + 2].copy_from_slice(&if le {
        1u16.to_le_bytes()
    } else {
        1u16.to_be_bytes()
    });
    let indices = image::load_from_memory_with_format(&bytes, image::ImageFormat::Tiff)
        .map_err(|e| MediaError::Image(e.to_string()))?
        .to_luma8();
    let mut rgba = Vec::with_capacity(indices.len() * 4);
    for &i in indices.as_raw() {
        let i = usize::from(i);
        rgba.extend([colors[i], colors[256 + i], colors[512 + i], 255]);
    }
    let (rgba, w, h) = orient(
        &rgba,
        indices.width() as usize,
        indices.height() as usize,
        4,
        original.orientation,
    );
    let profile = original.color.icc.clone();
    Ok(StillImage {
        frame: pool_rgba(
            pool,
            w,
            h,
            Rgba::U8(&rgba),
            false,
            8,
            "TIFF palette before ColorSync".into(),
        )?,
        bit_depth: 8,
        color: ImageColor {
            icc_class: profile.as_deref().map(icc::classify),
            icc: profile,
            icc_bytes: original.color.icc_bytes,
            transfer: Transfer::DisplayE,
            alpha: AlphaMode::Opaque,
            chromaticities: None,
        },
        ..original.clone()
    })
}
