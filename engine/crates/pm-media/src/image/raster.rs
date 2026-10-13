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

pub fn read(path: &Path, pool: &FramePool, format: ImageFormat) -> Result<StillImage> {
    let err = |e: image::ImageError| MediaError::Image(e.to_string());
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder().map_err(err)?;
    let profile = decoder.icc_profile().map_err(err)?;
    let orientation = decoder.orientation().map(exif_number).unwrap_or(1);
    let color_type = decoder.color_type();
    let img = DynamicImage::from_decoder(decoder).map_err(err)?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let label = format!("{format:?} {color_type:?}").to_lowercase();
    let (frame, bits, float) = match &img {
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => {
            let px = img.to_rgba32f().into_raw();
            let (px, ow, oh) = orient(&px, w, h, 4, orientation);
            (pool_rgba(pool, ow, oh, Rgba::F32(&px), false, 32, label)?, 32, true)
        }
        DynamicImage::ImageLuma16(_) | DynamicImage::ImageLumaA16(_) | DynamicImage::ImageRgb16(_) | DynamicImage::ImageRgba16(_) => {
            let px = img.to_rgba16().into_raw();
            let (px, ow, oh) = orient(&px, w, h, 4, orientation);
            (pool_rgba(pool, ow, oh, Rgba::U16(&px), false, 16, label)?, 16, false)
        }
        _ => {
            let px = img.to_rgba8().into_raw();
            let (px, ow, oh) = orient(&px, w, h, 4, orientation);
            (pool_rgba(pool, ow, oh, Rgba::U8(&px), false, 8, label)?, 8, false)
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
            transfer: if float { Transfer::Linear } else { Transfer::DisplayE },
            alpha: if color_type.has_alpha() { AlphaMode::Straight } else { AlphaMode::Opaque },
            chromaticities: None,
        },
        orientation,
    })
}
