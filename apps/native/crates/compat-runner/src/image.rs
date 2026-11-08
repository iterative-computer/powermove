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

impl Encoded {
    pub fn load(bytes: &[u8], mime: &str) -> Result<Self> {
        let invalid = || Error::Host("The source image cannot be decoded.".into());
        if bytes.is_empty() {
            return Err(invalid());
        }
        if is_svg(bytes, mime) {
            let tree = usvg::Tree::from_data(bytes, &svg_options()).map_err(|_| invalid())?;
            let size = tree.size();
            let (width, height) = (size.width().round() as u32, size.height().round() as u32);
            return Ok(Self {
                bytes: bytes.into(),
                mime: "image/svg+xml".into(),
                width,
                height,
                kind: Kind::Svg(Box::new(tree)),
            });
        }
        let pixels = decode_raster(bytes).ok_or_else(invalid)?;
        Ok(Self {
            bytes: bytes.into(),
            mime: sniff(bytes).unwrap_or(mime).into(),
            width: pixels.width(),
            height: pixels.height(),
            kind: Kind::Raster(Arc::new(pixels)),
        })
    }
    /// Pixels to draw at `scale` times the intrinsic size. Rasters ignore the
    /// scale; an SVG is rendered at it (quantized by the caller), capped.
    pub fn pixels(&self, scale: f64) -> Result<Arc<ImageData>> {
        match &self.kind {
            Kind::Raster(pixels) => Ok(pixels.clone()),
            Kind::Svg(tree) => {
                let size = tree.size();
                let (w, h) = (f64::from(size.width()), f64::from(size.height()));
                let scale = scale.max(1. / w.min(h).max(1.));
                let scale = scale.min(MAX_SVG_SIDE / w.max(h));
                let (pw, ph) = (
                    (w * scale).round().max(1.) as u32,
                    (h * scale).round().max(1.) as u32,
                );
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
        }
    }
    pub fn is_vector(&self) -> bool {
        matches!(self.kind, Kind::Svg(_))
    }
}

/// A snapshot's JPEG (quality 0.92, as a canvas's default), through ImageIO.
#[cfg(target_os = "macos")]
pub(crate) fn jpeg(pixels: &ImageData) -> Result<Vec<u8>> {
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
        let quality = CFNumber::new_f64(0.92);
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
pub(crate) fn jpeg(_: &ImageData) -> Result<Vec<u8>> {
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
#[cfg(target_os = "macos")]
fn decode_raster(bytes: &[u8]) -> Option<ImageData> {
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
    if width == 0 || height == 0 || (width as u64) * (height as u64) > MAX_PIXELS {
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
fn decode_raster(_: &[u8]) -> Option<ImageData> {
    None
}
