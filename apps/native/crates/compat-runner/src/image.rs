//! Encoded images an extension holds (`new Image()`, `<img>`, `createImageBitmap`).
//! Decoded once on the realm's worker into straight-alpha sRGB, colour-managed and
//! oriented the way WebKit shows the file. SVG stays vector: it is rasterized by
//! resvg at the size it is drawn, never stretched from its intrinsic size.
use crate::{Error, Result};
use pm_recorder::ImageData;
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, OnceLock};

/// The recorder's canvas limit, so any decoded image can be drawn.
const MAX_PIXELS: u64 = 67_108_864;
/// The largest SVG raster side, as for SVG stills in the engine.
const MAX_SVG_SIDE: f64 = 8192.;

pub(crate) struct Encoded {
    pub bytes: Arc<[u8]>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    /// Bytes this image holds (encoded and, for a raster, decoded).
    pub cost: u64,
    kind: Kind,
}
enum Kind {
    Raster(Arc<ImageData>),
    Svg(Box<usvg::Tree>),
}

fn is_svg(bytes: &[u8], mime: &str) -> bool {
    if mime == "image/svg+xml" {
        return true;
    }
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with('<') && text.contains("<svg")
}

/// `width × height × 4` bytes, refusing more pixels than a canvas can hold.
pub(crate) fn rgba_bytes(width: u64, height: u64) -> Result<u64> {
    width
        .checked_mul(height)
        .filter(|pixels| *pixels <= MAX_PIXELS)
        .map(|pixels| pixels * 4)
        .ok_or_else(|| Error::Host("The image is too large to draw.".into()))
}

impl Encoded {
    /// Decodes `bytes`. `admit` sees what the image will hold before anything
    /// large is allocated, and may refuse it.
    pub fn load(bytes: &[u8], mime: &str, admit: impl FnOnce(u64) -> Result<()>) -> Result<Self> {
        let invalid = || Error::Host("The source image cannot be decoded.".into());
        if bytes.is_empty() {
            return Err(invalid());
        }
        let encoded = bytes.len() as u64;
        if is_svg(bytes, mime) {
            let tree = usvg::Tree::from_data(bytes, &svg_options()).map_err(|_| invalid())?;
            let size = tree.size();
            let side = |v: f32| {
                let v = f64::from(v).round();
                (v.is_finite() && v <= f64::from(u32::MAX)).then_some(v as u32)
            };
            let (Some(width), Some(height)) = (side(size.width()), side(size.height())) else {
                return Err(invalid());
            };
            // Rasters of it are charged when drawn (`raster_size`).
            admit(encoded)?;
            return Ok(Self {
                bytes: bytes.into(),
                mime: "image/svg+xml".into(),
                width,
                height,
                cost: encoded,
                kind: Kind::Svg(Box::new(tree)),
            });
        }
        let (width, height) = raster_size(bytes).ok_or_else(invalid)?;
        let decoded = rgba_bytes(width, height)?;
        let cost = decoded + encoded;
        admit(cost)?;
        let pixels = decode_raster(bytes, width, height).ok_or_else(invalid)?;
        Ok(Self {
            bytes: bytes.into(),
            mime: sniff(bytes).unwrap_or(mime).into(),
            width: pixels.width(),
            height: pixels.height(),
            cost,
            kind: Kind::Raster(Arc::new(pixels)),
        })
    }
    /// The raster an SVG gets at `scale` times its intrinsic size (capped at
    /// 8192 px a side), as `(width, height)`; its bytes are `rgba_bytes`.
    pub fn raster_size(&self, scale: f64) -> Option<(u32, u32)> {
        let Kind::Svg(tree) = &self.kind else {
            return None;
        };
        let size = tree.size();
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let scale = scale.max(1. / w.min(h).max(1.));
        let scale = scale.min(MAX_SVG_SIDE / w.max(h));
        Some((
            (w * scale).round().clamp(1., MAX_SVG_SIDE) as u32,
            (h * scale).round().clamp(1., MAX_SVG_SIDE) as u32,
        ))
    }
    /// The intrinsic size in CSS pixels, unrounded.
    pub fn natural(&self) -> (f64, f64) {
        match &self.kind {
            Kind::Svg(tree) => (
                f64::from(tree.size().width()),
                f64::from(tree.size().height()),
            ),
            Kind::Raster(_) => (f64::from(self.width), f64::from(self.height)),
        }
    }
    /// Pixels to draw: a raster's own, or an SVG rendered at `size` (from
    /// `raster_size`, already admitted by the caller).
    pub fn pixels(&self, size: Option<(u32, u32)>) -> Result<Arc<ImageData>> {
        match (&self.kind, size) {
            (Kind::Raster(pixels), _) => Ok(pixels.clone()),
            (Kind::Svg(tree), Some((pw, ph))) => {
                let (w, h) = self.natural();
                let mut pixmap = tiny_skia::Pixmap::new(pw, ph)
                    .ok_or(Error::Host("SVG raster is too large".into()))?;
                let transform = tiny_skia::Transform::from_scale(
                    (f64::from(pw) / w) as f32,
                    (f64::from(ph) / h) as f32,
                );
                resvg::render(tree, transform, &mut pixmap.as_mut());
                let mut bytes = pixmap.take();
                demultiply(&mut bytes);
                Ok(Arc::new(
                    ImageData::from_rgba(pw, ph, bytes).map_err(crate::recorder::host_error)?,
                ))
            }
            (Kind::Svg(_), None) => Err(Error::Invalid("SVG raster size")),
        }
    }
    pub fn is_vector(&self) -> bool {
        matches!(self.kind, Kind::Svg(_))
    }
}

/// A snapshot's JPEG at `quality` (main's snapshot uses 0.74), through ImageIO.
#[cfg(target_os = "macos")]
pub(crate) fn jpeg(pixels: &ImageData, quality: f64) -> Result<Vec<u8>> {
    use objc2_core_foundation::{CFData, CFDictionary, CFMutableData, CFNumber, CFString, CFType};
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGImageAlphaInfo,
        kCGColorSpaceSRGB,
    };
    use objc2_image_io::{CGImageDestination, kCGImageDestinationLossyCompressionQuality};
    let failed = || Error::Host("The snapshot could not be encoded".into());
    let (width, height) = (pixels.width() as usize, pixels.height() as usize);
    // JPEG has no alpha: composite over black, as a canvas's toDataURL does.
    let mut rgb: Vec<u8> = pixels.bytes().to_vec();
    for px in rgb.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
        px[3] = 255;
    }
    // SAFETY: a constant colour space name.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).ok_or_else(failed)?;
    // SAFETY: `rgb` is width*height*4 bytes and outlives the context.
    let context = unsafe {
        CGBitmapContextCreate(
            rgb.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            Some(&space),
            CGImageAlphaInfo::NoneSkipLast.0,
        )
    }
    .ok_or_else(failed)?;
    let image = CGBitmapContextCreateImage(Some(&context)).ok_or_else(failed)?;
    let data = CFMutableData::new(None, 0).ok_or_else(failed)?;
    let kind = CFString::from_static_str("public.jpeg");
    // SAFETY: a fresh mutable data, one image, documented option types.
    unsafe {
        let destination =
            CGImageDestination::with_data(&data, &kind, 1, None).ok_or_else(failed)?;
        let quality = CFNumber::new_f64(quality);
        let options = CFDictionary::<CFString, CFType>::from_slices(
            &[kCGImageDestinationLossyCompressionQuality],
            &[quality.as_ref()],
        );
        destination.add_image(&image, Some(options.as_ref()));
        if !destination.finalize() {
            return Err(failed());
        }
    }
    let data: &CFData = &data;
    Ok(data.to_vec())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn jpeg(_: &ImageData, _: f64) -> Result<Vec<u8>> {
    Err(Error::Host("JPEG snapshots need ImageIO".into()))
}

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    Some(if at(0, b"\x89PNG") {
        "image/png"
    } else if at(0, b"\xff\xd8\xff") {
        "image/jpeg"
    } else if at(0, b"GIF8") {
        "image/gif"
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        "image/webp"
    } else if at(0, b"BM") {
        "image/bmp"
    } else if at(4, b"ftypavif") {
        "image/avif"
    } else if at(4, b"ftypheic") {
        "image/heic"
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        "image/tiff"
    } else {
        return None;
    })
}

fn demultiply(bytes: &mut [u8]) {
    for px in bytes.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

fn svg_options() -> usvg::Options<'static> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let fontdb = FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone();
    usvg::Options {
        // As an SVG drawn through `<img>`: embedded images render, nothing external loads.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        fontdb,
        ..usvg::Options::default()
    }
}

/// ImageIO decodes every format WebKit shows, applies EXIF orientation and the
/// embedded colour profile, and draws into sRGB as a canvas sees it.
/// A raster's displayed size from its header, before any pixel is decoded
/// (orientations 5–8 swap the sides).
#[cfg(target_os = "macos")]
fn raster_size(bytes: &[u8]) -> Option<(u64, u64)> {
    use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFString, CFType};
    use objc2_image_io::{
        CGImageSource, kCGImagePropertyOrientation, kCGImagePropertyPixelHeight,
        kCGImagePropertyPixelWidth,
    };
    let data = CFData::from_bytes(bytes);
    // SAFETY: no options; the data outlives the source.
    let source = unsafe { CGImageSource::with_data(&data, None) }?;
    // SAFETY: index 0, no options.
    let properties = unsafe { source.properties_at_index(0, None) }?;
    // SAFETY: ImageIO's property dictionaries have CFString keys.
    let properties: &CFDictionary<CFString, CFType> = unsafe { properties.cast_unchecked() };
    let number = |key: &CFString| -> Option<i64> {
        properties.get(key)?.downcast_ref::<CFNumber>()?.as_i64()
    };
    // SAFETY: constant keys.
    let (w, h, orientation) = unsafe {
        (
            number(kCGImagePropertyPixelWidth)?,
            number(kCGImagePropertyPixelHeight)?,
            number(kCGImagePropertyOrientation).unwrap_or(1),
        )
    };
    let (w, h) = (u64::try_from(w).ok()?, u64::try_from(h).ok()?);
    (w > 0 && h > 0).then_some(if (5..=8).contains(&orientation) {
        (h, w)
    } else {
        (w, h)
    })
}
#[cfg(not(target_os = "macos"))]
fn raster_size(_: &[u8]) -> Option<(u64, u64)> {
    None
}

#[cfg(target_os = "macos")]
fn decode_raster(bytes: &[u8], expected_width: u64, expected_height: u64) -> Option<ImageData> {
    use objc2_core_foundation::{
        CFBoolean, CFData, CFDictionary, CFString, CGPoint, CGRect, CGSize,
    };
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
        kCGColorSpaceSRGB,
    };
    use objc2_image_io::{
        CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways,
        kCGImageSourceCreateThumbnailWithTransform,
    };
    let data = CFData::from_bytes(bytes);
    // SAFETY: no options dictionary; the data outlives the source.
    let source = unsafe { CGImageSource::with_data(&data, None) }?;
    // SAFETY: CFString keys and CFBoolean values, as ImageIO documents.
    let yes = CFBoolean::new(true);
    let options = unsafe {
        let keys: [&CFString; 2] = [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceCreateThumbnailWithTransform,
        ];
        CFDictionary::<CFString, CFBoolean>::from_slices(&keys, &[yes, yes])
    };
    // A "thumbnail" with no size limit is the whole image with its orientation applied.
    // SAFETY: the options dictionary has the documented key and value types.
    let image = unsafe { source.thumbnail_at_index(0, Some(options.as_ref())) }?;
    let (width, height) = (CGImage::width(Some(&image)), CGImage::height(Some(&image)));
    // The header admitted this size; a stream that decodes larger is refused.
    if width == 0
        || height == 0
        || width as u64 > expected_width.max(expected_height)
        || height as u64 > expected_width.max(expected_height)
    {
        return None;
    }
    let mut pixels = vec![0u8; width * height * 4];
    // SAFETY: a constant colour space name.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    // SAFETY: `pixels` is width*height*4 bytes and outlives the context.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }?;
    let rect = CGRect::new(
        CGPoint::new(0., 0.),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::draw_image(Some(&context), rect, Some(&image));
    drop(context);
    demultiply(&mut pixels);
    ImageData::from_rgba(width as u32, height as u32, pixels).ok()
}

#[cfg(not(target_os = "macos"))]
fn decode_raster(_: &[u8], _: u64, _: u64) -> Option<ImageData> {
    None
}
