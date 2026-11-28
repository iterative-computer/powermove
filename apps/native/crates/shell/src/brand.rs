//! Official Powermove & Aerolite brand artwork (wordmarks in Geist Width and
//! the macOS App Icon from `bundle/Powermove.icon`), rasterised at device scale
//! with `resvg` and cached for both GPUI and AppKit surfaces.
use gpui::{prelude::*, *};
use resvg::{tiny_skia, usvg};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

pub const WORDMARK_SVG: &str = include_str!("../assets/brand/wordmark.svg");
pub const AEROLITE_WORDMARK_SVG: &str = include_str!("../assets/brand/aerolite-wordmark.svg");
pub const APP_ICON_DARK_SVG: &str = include_str!("../assets/brand/app-icon-dark.svg");
pub const APP_ICON_LIGHT_SVG: &str = include_str!("../assets/brand/app-icon-light.svg");

/// Wordmark viewBox dimensions (`assets/brand/wordmark.svg`).
pub const WORDMARK_VB_W: f32 = 296.79;
pub const WORDMARK_VB_H: f32 = 115.20;
/// Playhead head bounds inside `wordmark.svg`'s viewBox:
/// `translate(150.80, 7.96) scale(0.06)` applied to `x ∈ [282, 708], y ∈ [150, 518]`.
pub const PLAYHEAD_HEAD_CX: f32 = 180.50;
pub const PLAYHEAD_HEAD_CY: f32 = 28.00;
pub const PLAYHEAD_HEAD_H: f32 = 22.08;

/// Just the white "Powermove" letterforms in the exact `wordmark.svg` viewBox.
const WORDMARK_TEXT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="296.79" height="115.20" viewBox="0 0 296.79 115.20" fill="none">
  <path fill="#FFFFFF" d="M6.52 91.20L6.52 45.76L20.68 45.76Q27.26 45.76 30.91 49.40Q34.55 53.05 34.55 59.49Q34.55 63.81 32.90 66.96Q31.26 70.11 28.15 71.77Q25.04 73.43 20.68 73.43L11.82 73.43L11.82 91.20ZM11.82 68.25L20.58 68.25Q24.77 68.25 26.92 66.04Q29.06 63.84 29.06 59.49Q29.06 55.25 26.92 53.09Q24.77 50.94 20.58 50.94L11.82 50.94ZM51.49 91.97Q47.41 91.97 44.37 89.79Q41.33 87.62 39.65 83.62Q37.97 79.62 37.97 74.24Q37.97 68.80 39.65 64.83Q41.33 60.86 44.37 58.69Q47.41 56.51 51.49 56.51Q55.55 56.51 58.59 58.69Q61.62 60.86 63.29 64.83Q64.95 68.80 64.95 74.24Q64.95 79.62 63.28 83.62Q61.61 87.62 58.58 89.79Q55.55 91.97 51.49 91.97ZM51.51 87.03Q55.32 87.03 57.43 83.62Q59.55 80.20 59.55 74.24Q59.55 68.28 57.43 64.86Q55.32 61.45 51.51 61.45Q47.70 61.45 45.55 64.86Q43.40 68.28 43.40 74.24Q43.40 80.20 45.55 83.62Q47.70 87.03 51.51 87.03ZM75.89 91.20L66.97 57.28L72.54 57.28L78.99 85.78L85.61 57.28L90.74 57.28L97.36 85.78L103.87 57.28L109.40 57.28L100.52 91.20L94.44 91.20L88.17 65.11L81.91 91.20ZM124.86 91.97Q120.69 91.97 117.67 89.78Q114.65 87.58 113.00 83.58Q111.36 79.58 111.36 74.24Q111.36 68.90 113.00 64.92Q114.63 60.95 117.61 58.73Q120.59 56.51 124.67 56.51Q128.54 56.51 131.50 58.61Q134.46 60.71 136.08 64.67Q137.71 68.64 137.71 74.27L137.71 75.83L116.79 75.83Q117.04 81.37 119.16 84.20Q121.27 87.03 124.88 87.03Q127.60 87.03 129.35 85.43Q131.11 83.82 131.79 81.20L137.27 81.62Q136.12 86.16 132.84 89.06Q129.57 91.97 124.86 91.97ZM116.79 71.26L132.14 71.26Q131.83 66.24 129.81 63.84Q127.79 61.45 124.66 61.45Q121.45 61.45 119.37 63.93Q117.30 66.41 116.79 71.26ZM143.84 91.20L143.84 57.28L148.50 57.28L148.76 65.98L148.28 65.76Q148.85 61.38 150.69 59.33Q152.53 57.28 155.61 57.28L158.37 57.28L158.37 62.23L155.63 62.23Q153.44 62.23 151.98 63.19Q150.52 64.15 149.78 66.01Q149.03 67.87 149.03 70.69L149.03 91.20ZM162.59 91.20L162.59 57.28L167.34 57.28L167.49 65.95L166.86 65.65Q167.33 62.84 168.55 60.81Q169.78 58.77 171.59 57.64Q173.40 56.51 175.57 56.51Q179.26 56.51 181.60 58.98Q183.94 61.45 184.55 65.86L183.66 65.78Q184.09 62.82 185.27 60.77Q186.45 58.72 188.33 57.62Q190.20 56.51 192.56 56.51Q195.61 56.51 197.74 58.03Q199.87 59.54 200.97 62.42Q202.06 65.30 202.06 69.39L202.06 91.20L196.88 91.20L196.88 70.56Q196.88 65.88 195.45 63.48Q194.01 61.09 191.16 61.09Q189.22 61.09 187.80 62.27Q186.38 63.45 185.61 65.65Q184.83 67.84 184.83 70.95L184.83 91.20L179.78 91.20L179.78 70.91Q179.78 66.30 178.48 63.69Q177.18 61.09 174.15 61.09Q172.24 61.09 170.81 62.27Q169.37 63.45 168.57 65.66Q167.77 67.87 167.77 70.91L167.77 91.20ZM221.59 91.97Q217.52 91.97 214.47 89.79Q211.43 87.62 209.75 83.62Q208.07 79.62 208.07 74.24Q208.07 68.80 209.75 64.83Q211.43 60.86 214.47 58.69Q217.52 56.51 221.59 56.51Q225.65 56.51 228.69 58.69Q231.72 60.86 233.39 64.83Q235.05 68.80 235.05 74.24Q235.05 79.62 233.38 83.62Q231.71 87.62 228.68 89.79Q225.65 91.97 221.59 91.97ZM221.61 87.03Q225.42 87.03 227.53 83.62Q229.65 80.20 229.65 74.24Q229.65 68.28 227.53 64.86Q225.42 61.45 221.61 61.45Q217.80 61.45 215.65 64.86Q213.50 68.28 213.50 74.24Q213.50 80.20 215.65 83.62Q217.80 87.03 221.61 87.03ZM247.62 91.20L237.07 57.28L242.68 57.28L250.82 85.90L258.94 57.28L264.58 57.28L254.03 91.20ZM280.03 91.97Q275.87 91.97 272.85 89.78Q269.83 87.58 268.18 83.58Q266.53 79.58 266.53 74.24Q266.53 68.90 268.17 64.92Q269.81 60.95 272.79 58.73Q275.77 56.51 279.85 56.51Q283.72 56.51 286.68 58.61Q289.64 60.71 291.26 64.67Q292.88 68.64 292.88 74.27L292.88 75.83L271.97 75.83Q272.22 81.37 274.33 84.20Q276.45 87.03 280.05 87.03Q282.78 87.03 284.53 85.43Q286.28 83.82 286.96 81.20L292.44 81.62Q291.30 86.16 288.02 89.06Q284.74 91.97 280.03 91.97ZM271.97 71.26L287.32 71.26Q287.01 66.24 284.99 63.84Q282.97 61.45 279.84 61.45Q276.62 61.45 274.55 63.93Q272.48 66.41 271.97 71.26Z"/>
</svg>"##;

/// Just the orange playhead stem + motion trail in the exact `wordmark.svg` viewBox.
const WORDMARK_STEM_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="296.79" height="115.20" viewBox="0 0 296.79 115.20" fill="none">
  <defs>
    <linearGradient id="pmTrailX" x1="166.33" y1="0" x2="182.33" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#F0580A" stop-opacity="0"/>
      <stop offset="0.38" stop-color="#F0580A" stop-opacity="0.16"/>
      <stop offset="0.78" stop-color="#F0580A" stop-opacity="0.48"/>
      <stop offset="1" stop-color="#F0580A" stop-opacity="0.72"/>
    </linearGradient>
    <linearGradient id="pmTrailMaskG" x1="0" y1="41.92" x2="0" y2="91.52" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.95"/>
      <stop offset="0.62" stop-color="#FFFFFF" stop-opacity="0.85"/>
      <stop offset="1" stop-color="#FFFFFF" stop-opacity="0.06"/>
    </linearGradient>
    <mask id="pmTrailMask">
      <rect x="166.33" y="41.92" width="16.00" height="49.60" fill="url(#pmTrailMaskG)"/>
    </mask>
  </defs>
  <rect x="166.33" y="41.92" width="16.00" height="49.60" fill="url(#pmTrailX)" mask="url(#pmTrailMask)"/>
  <rect x="180.92" y="41.92" width="2.82" height="49.60" rx="0.99" fill="#F0580A"/>
</svg>"##;

/// Just the orange playhead head in the exact `wordmark.svg` viewBox.
const WORDMARK_HEAD_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="296.79" height="115.20" viewBox="0 0 296.79 115.20" fill="none">
  <g transform="translate(150.80 7.96) scale(0.06000)">
    <path d="M 346 150 L 654 150 C 690 150 708 176 708 206 C 708 226 702 238 692 251 L 628 327 C 612 346 606 364 603 386 L 592 465 C 587 498 560 518 527 518 C 501 518 485 504 472 486 L 297 248 C 286 233 282 220 282 206 C 282 174 306 150 346 150 Z" fill="#F0580A"/>
  </g>
</svg>"##;

/// Pure white ink for brand surfaces.
pub const WHITE: u32 = 0xffffff;

/// Paper `Powermove Aerolite` Sky Aurora palette.
pub const AEROLITE_NIGHT: u32 = 0x061322;
pub const AEROLITE_DEEP: u32 = 0x0c2d4d;
pub const AEROLITE_CERULEAN: u32 = 0x0284c7;
pub const AEROLITE_SKY: u32 = 0x38bdf8;
pub const AEROLITE_LIGHT: u32 = 0x7dd3fc;
pub const AEROLITE_TINT: u32 = 0xd6efff;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Asset {
    Wordmark,
    WordmarkText,
    WordmarkStem,
    WordmarkHead,
    AeroliteWordmark,
    AppIconDark,
    AppIconLight,
}

impl Asset {
    pub fn svg(self) -> &'static str {
        match self {
            Self::Wordmark => WORDMARK_SVG,
            Self::WordmarkText => WORDMARK_TEXT_SVG,
            Self::WordmarkStem => WORDMARK_STEM_SVG,
            Self::WordmarkHead => WORDMARK_HEAD_SVG,
            Self::AeroliteWordmark => AEROLITE_WORDMARK_SVG,
            Self::AppIconDark => APP_ICON_DARK_SVG,
            Self::AppIconLight => APP_ICON_LIGHT_SVG,
        }
    }
}

/// Rasterises `asset` to exact `height` points (preserving aspect ratio) at
/// `scale` device pixels per point, returning `(image, width_pt, height_pt)`.
pub fn render_image(asset: Asset, height: f32, scale: f32) -> Option<(Arc<RenderImage>, f32, f32)> {
    thread_local! {
        static CACHE: RefCell<HashMap<(Asset, u32, u32), Option<(Arc<RenderImage>, f32, f32)>>> =
            RefCell::default();
    }
    let key = (
        asset,
        (height * 10.).round() as u32,
        (scale * 100.).round() as u32,
    );
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let built = (|| {
        let tree = usvg::Tree::from_str(asset.svg(), &usvg::Options::default()).ok()?;
        let size = tree.size();
        let aspect = size.width() / size.height().max(1.);
        let width_pt = height * aspect;
        let pw = (width_pt * scale).round().max(1.) as u32;
        let ph = (height * scale).round().max(1.) as u32;
        let mut pixmap = tiny_skia::Pixmap::new(pw, ph)?;
        let transform =
            tiny_skia::Transform::from_scale(pw as f32 / size.width(), ph as f32 / size.height());
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        // GPUI RenderImage expects straight BGRA bytes in RgbaImage.
        let mut bgra = Vec::with_capacity(pixmap.data().len());
        for p in pixmap.pixels() {
            let c = p.demultiply();
            bgra.extend_from_slice(&[c.blue(), c.green(), c.red(), c.alpha()]);
        }
        let buffer = image::RgbaImage::from_raw(pw, ph, bgra)?;
        Some((
            Arc::new(RenderImage::new([image::Frame::new(buffer)])),
            width_pt,
            height,
        ))
    })();
    CACHE.with(|c| c.borrow_mut().insert(key, built.clone()));
    built
}

/// Official Powermove Geist Width wordmark at `height` points.
pub fn wordmark(height: f32, scale: f32) -> AnyElement {
    render_asset(Asset::Wordmark, height, scale)
}

/// Just the white "Powermove" letterforms in the wordmark's viewBox.
pub fn wordmark_text(height: f32, scale: f32) -> AnyElement {
    render_asset(Asset::WordmarkText, height, scale)
}

/// Just the orange playhead stem + trail in the wordmark's viewBox.
pub fn wordmark_stem(height: f32, scale: f32) -> AnyElement {
    render_asset(Asset::WordmarkStem, height, scale)
}

/// Just the orange playhead head in the wordmark's viewBox.
pub fn wordmark_head(height: f32, scale: f32) -> AnyElement {
    render_asset(Asset::WordmarkHead, height, scale)
}

fn render_asset(asset: Asset, height: f32, scale: f32) -> AnyElement {
    if let Some((image, w, h)) = render_image(asset, height, scale) {
        img(ImageSource::Render(image))
            .w(px(w))
            .h(px(h))
            .into_any_element()
    } else {
        div().into_any_element()
    }
}

/// Official Powermove Aerolite Geist Width wordmark at `height` points.
pub fn aerolite_wordmark(height: f32, scale: f32) -> AnyElement {
    if let Some((image, w, h)) = render_image(Asset::AeroliteWordmark, height, scale) {
        img(ImageSource::Render(image))
            .w(px(w))
            .h(px(h))
            .into_any_element()
    } else {
        div().into_any_element()
    }
}

/// Official Powermove App Icon at `side` × `side` points.
pub fn app_icon(side: f32, dark: bool, scale: f32) -> AnyElement {
    let asset = if dark {
        Asset::AppIconDark
    } else {
        Asset::AppIconLight
    };
    if let Some((image, w, h)) = render_image(asset, side, scale) {
        img(ImageSource::Render(image))
            .w(px(w))
            .h(px(h))
            .into_any_element()
    } else {
        div().into_any_element()
    }
}

/// Official Powermove Aerolite wordmark as a cached AppKit `NSImage` at
/// `height` points (1× and 2× representations), for Settings › Account cards.
pub fn aerolite_wordmark_ns(
    height: f64,
) -> Option<(objc2::rc::Retained<objc2_app_kit::NSImage>, f64)> {
    thread_local! {
        static CACHE: RefCell<HashMap<u32, Option<(objc2::rc::Retained<objc2_app_kit::NSImage>, f64)>>> =
            RefCell::default();
    }
    let key = (height * 10.).round() as u32;
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let built = (|| {
        let tree = usvg::Tree::from_str(AEROLITE_WORDMARK_SVG, &usvg::Options::default()).ok()?;
        let size = tree.size();
        let width = height * (size.width() as f64 / size.height().max(1.) as f64);
        let image = crate::settings_window::svg_image(
            AEROLITE_WORDMARK_SVG,
            objc2_foundation::NSSize::new(width, height),
        )?;
        Some((image, width))
    })();
    CACHE.with(|c| c.borrow_mut().insert(key, built.clone()));
    built
}
