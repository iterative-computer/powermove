//! SF Symbols as GPUI elements. AppKit draws the symbol (template, tinted with a
//! token colour) into a bitmap at the window's backing scale; the bitmap is
//! cached per name, size, weight, colour and scale.
use gpui::{prelude::*, *};
use objc2::AnyThread;
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

/// Symbol weight, matched to the adjacent text (language §8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
}

type Key = (String, u32, Weight, u32, u32);
thread_local! {
    static CACHE: RefCell<HashMap<Key, Option<Arc<RenderImage>>>> = RefCell::new(HashMap::new());
}

/// SF Symbol for a layer kind (language §8: kind badges use symbols, not letters).
pub fn kind_symbol(kind: &str) -> &'static str {
    match kind {
        "text" => "textformat",
        "shape" => "square.on.circle",
        "video" | "precompVideo" => "film",
        "image" => "photo",
        "sequence" => "photo.stack",
        "audio" => "waveform",
        "adjustment" | "effect" => "slider.horizontal.3",
        "null" => "circle.dashed",
        "camera" => "camera",
        "group" | "precomp" => "square.stack",
        "solid" => "square.fill",
        "light" => "lightbulb",
        _ => "square.dashed",
    }
}

/// Renders `name` at `points` in `color` (0xRRGGBB) at `scale` pixels per point.
/// `None` when the symbol does not exist on this system.
pub fn rasterize(
    name: &str,
    points: f32,
    weight: Weight,
    color: u32,
    scale: f32,
) -> Option<Arc<RenderImage>> {
    let key = (
        name.to_string(),
        points.to_bits(),
        weight,
        color,
        scale.to_bits(),
    );
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let image = draw(name, points, weight, color, scale);
    CACHE.with(|c| c.borrow_mut().insert(key, image.clone()));
    image
}

fn draw(
    name: &str,
    points: f32,
    weight: Weight,
    color: u32,
    scale: f32,
) -> Option<Arc<RenderImage>> {
    let symbol = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        None,
    )?;
    let w = match weight {
        Weight::Regular => unsafe { NSFontWeightRegular },
        Weight::Medium => unsafe { NSFontWeightMedium },
        Weight::Semibold => unsafe { NSFontWeightSemibold },
    };
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(points as f64, w);
    let symbol = symbol.imageWithSymbolConfiguration(&config)?;
    // Square canvas of the point size; the symbol is centred in it (SF Symbols
    // are drawn on a square template grid at their point size).
    let side = (points * scale).ceil().max(1.) as isize;
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            side,
            side,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            side * 4,
            32,
        )?
    };
    // Token colours are sRGB; tag the bitmap so they are not colour-matched away.
    let rep = rep.bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    let size = symbol.size();
    let canvas = side as f64;
    // Fit the symbol's own bounds (it may be wider than tall) into the canvas.
    let fit = (canvas / (size.width * scale as f64))
        .min(canvas / (size.height * scale as f64))
        .min(1.);
    let (dw, dh) = (
        size.width * scale as f64 * fit,
        size.height * scale as f64 * fit,
    );
    let r = NSRect::new(
        NSPoint::new((canvas - dw) / 2., (canvas - dh) / 2.),
        NSSize::new(dw, dh),
    );
    symbol.drawInRect(r);
    crate::native::native_color(color).set();
    NSRectFillUsingOperation(
        NSRect::new(NSPoint::ZERO, NSSize::new(canvas, canvas)),
        NSCompositingOperation::SourceAtop,
    );
    ctx.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    let data = rep.bitmapData();
    if data.is_null() {
        return None;
    }
    let len = (side * side * 4) as usize;
    let mut pixels = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    // Premultiplied RGBA → straight BGRA (GPUI's sprite format).
    for p in pixels.chunks_exact_mut(4) {
        let a = p[3];
        if a > 0 && a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            }
        }
        p.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(side as u32, side as u32, pixels)?;
    Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

/// An SF Symbol element `points` square, tinted `color`. Draws nothing (but
/// keeps its size) when the symbol is unavailable.
pub fn symbol(name: &str, points: f32, weight: Weight, color: u32, window: &Window) -> AnyElement {
    match rasterize(name, points, weight, color, window.scale_factor()) {
        Some(image) => img(ImageSource::Render(image))
            .flex_none()
            .size(px(points))
            .into_any_element(),
        None => div().flex_none().size(px(points)).into_any_element(),
    }
}
