//! Bounded image packing shared by the mesh importers.
use crate::{Error, Result};
use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage, imageops::FilterType};
use std::io::Cursor;

pub const MAX_EDGE: u32 = 2048;

pub fn decode(bytes: &[u8]) -> Result<RgbaImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(image_error)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map(|image| image.to_rgba8())
        .map_err(image_error)
}

fn image_error(error: impl std::fmt::Display) -> Error {
    Error(format!("Cannot read or encode model texture: {error}"))
}

pub fn capped(image: RgbaImage) -> RgbaImage {
    let (w, h) = image.dimensions();
    let edge = w.max(h);
    if edge <= MAX_EDGE {
        return image;
    }
    let scale = |size| {
        ((u64::from(size) * u64::from(MAX_EDGE) + u64::from(edge / 2)) / u64::from(edge)).max(1)
            as u32
    };
    image::imageops::resize(&image, scale(w), scale(h), FilterType::Triangle)
}

pub fn encode(image: RgbaImage, color: bool) -> Result<(&'static str, Vec<u8>)> {
    let image = capped(image);
    let (w, h) = image.dimensions();
    let mut bytes = Vec::new();
    let opaque = image.pixels().all(|p| p[3] == 255);
    if color && opaque {
        let rgb = image::DynamicImage::ImageRgba8(image).into_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90)
            .encode(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
            .map_err(image_error)?;
        return Ok(("image/jpeg", bytes));
    }
    let encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    if opaque && image.pixels().all(|p| p[0] == p[1] && p[1] == p[2]) {
        let gray: Vec<_> = image.pixels().map(|p| p[0]).collect();
        encoder.write_image(&gray, w, h, ExtendedColorType::L8)
    } else if opaque {
        let rgb = image::DynamicImage::ImageRgba8(image).into_rgb8();
        encoder.write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
    } else {
        encoder.write_image(image.as_raw(), w, h, ExtendedColorType::Rgba8)
    }
    .map_err(image_error)?;
    Ok(("image/png", bytes))
}

fn fitted(image: RgbaImage, size: (u32, u32)) -> RgbaImage {
    if image.dimensions() == size {
        image
    } else {
        image::imageops::resize(&image, size.0, size.1, FilterType::Triangle)
    }
}

pub fn metallic_roughness(
    roughness: Option<RgbaImage>,
    metal: Option<RgbaImage>,
) -> Option<RgbaImage> {
    let size = roughness
        .iter()
        .chain(&metal)
        .map(|image| image.dimensions())
        .max_by_key(|&(w, h)| u64::from(w) * u64::from(h))?;
    let roughness = roughness.map(|image| fitted(image, size));
    let metal = metal.map(|image| fitted(image, size));
    Some(RgbaImage::from_fn(size.0, size.1, |x, y| {
        Rgba([
            255,
            roughness
                .as_ref()
                .map_or(255, |image| image.get_pixel(x, y)[0]),
            metal.as_ref().map_or(255, |image| image.get_pixel(x, y)[0]),
            255,
        ])
    }))
}

/// Multiply a scalar opacity map into the base color's existing alpha.
pub fn with_opacity(base: Option<RgbaImage>, opacity: RgbaImage) -> RgbaImage {
    let size = base
        .as_ref()
        .map_or(opacity.dimensions(), RgbaImage::dimensions);
    let opacity = fitted(opacity, size);
    let mut base = base.unwrap_or_else(|| RgbaImage::from_pixel(size.0, size.1, Rgba([255; 4])));
    for (color, alpha) in base.pixels_mut().zip(opacity.pixels()) {
        color[3] = ((u16::from(color[3]) * u16::from(alpha[0]) + 127) / 255) as u8;
    }
    base
}

pub fn resolve(uri: &str, read: &mut super::ResolveCompanion<'_>) -> Result<Option<RgbaImage>> {
    if let Some((metadata, payload)) = uri
        .strip_prefix("data:")
        .and_then(|data| data.split_once(','))
    {
        use base64::Engine;
        if metadata.ends_with(";base64") {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(payload)
                .map_err(image_error)?;
            return decode(&bytes).map(capped).map(Some);
        }
    }
    read(uri)?
        .map(|bytes| decode(&bytes).map(capped))
        .transpose()
}
