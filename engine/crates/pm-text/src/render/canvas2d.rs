//! Canvas2D's macOS A8 masks: CoreGraphics smoothing followed by Skia's
//! measured paint-luminance preblend. Kept local to legacy, axis-aligned
//! glyph strikes; native level 2 and Exact outlines keep float area coverage.
use super::{
    coretext::{CoreTextReference, RefRun},
    outline::FaceRef,
};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_core_text::CTFont;
use std::{collections::HashMap, ptr::NonNull, sync::Arc};

#[derive(Clone, Copy)]
struct MaskStyle {
    quarter: u8,
    skew: f32,
}
pub(crate) struct GlyphMask {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub coverage: Vec<u8>,
}
type Key = (u64, u32, u16, u32, Vec<(u32, u32)>, u8, u32, Option<u64>);
#[derive(Default)]
pub(crate) struct CanvasMasks {
    fonts: CoreTextReference,
    masks: HashMap<Key, Arc<GlyphMask>>,
    bytes: usize,
}
// SAFETY: immutable CTFont objects are thread-safe; contexts are created and
// destroyed within one call. The cache moves with its owning render thread.
unsafe impl Send for CanvasMasks {}
impl CanvasMasks {
    pub fn clear(&mut self) {
        self.masks.clear();
        self.fonts = CoreTextReference::default();
        self.bytes = 0;
    }
    #[allow(clippy::too_many_arguments)]
    pub fn get(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        quarter: u8,
        skew: f32,
        bounds: kurbo::Rect,
        bold_path: Option<(&kurbo::BezPath, f64)>,
    ) -> Option<Arc<GlyphMask>> {
        let key = (
            face.id,
            face.index,
            glyph,
            size.to_bits(),
            coords.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
            quarter,
            skew.to_bits(),
            // Fake-bold stroke width depends on the source font size before
            // its device transform; equal strike sizes can have other widths.
            bold_path.map(|(_, scale)| scale.to_bits()),
        );
        if let Some(mask) = self.masks.get(&key) {
            return Some(mask.clone());
        }
        if self.bytes >= 8 << 20 {
            self.clear();
        }
        let run = RefRun {
            data: face.data,
            index: face.index,
            size,
            coords,
            glyphs: vec![],
            color: [0., 0., 0., 1.],
        };
        let mask = Arc::new(match bold_path {
            Some((path, scale)) => draw_path(path, scale, quarter, bounds)?,
            None => {
                let font = self.fonts.font_of(face.id, &run)?;
                draw(&font, glyph, MaskStyle { quarter, skew }, bounds)?
            }
        });
        self.bytes += mask.coverage.len();
        self.masks.insert(key, mask.clone());
        Some(mask)
    }
}
const PAD: i32 = 4;
/// SkTypeface_Mac::onFilterRec uses useStrokeForFakeBold. Such strikes go
/// through SkScalerContext::GenerateImageFromPath, without CoreGraphics' font
/// dilation. Their geometric A8 coverage still takes the same preblend table.
fn draw_path(
    path: &kurbo::BezPath,
    scale: f64,
    quarter: u8,
    bounds: kurbo::Rect,
) -> Option<GlyphMask> {
    if !scale.is_finite()
        || scale <= 0.
        || ![bounds.x0, bounds.y0, bounds.x1, bounds.y1]
            .iter()
            .all(|v| v.is_finite())
    {
        return None;
    }
    let left = bounds.x0.floor() as i32 - PAD;
    let top = bounds.y0.floor() as i32 - PAD;
    let w = (bounds.x1.ceil() as i32 + PAD + 1 - left).max(1) as u32;
    let h = (bounds.y1.ceil() as i32 + PAD - top).max(1) as u32;
    if w > 4096 || h > 4096 {
        return None;
    }
    let mut context = vello_cpu::RenderContext::new(w as u16, h as u16);
    context.set_paint(vello_cpu::color::palette::css::WHITE);
    context.set_transform(
        kurbo::Affine::translate((f64::from(-left) + f64::from(quarter) / 4., f64::from(-top)))
            * kurbo::Affine::scale(scale),
    );
    context.fill_path(path);
    context.flush();
    let mut pixels = vello_cpu::Pixmap::new(w as u16, h as u16);
    context.render(&mut pixels, &mut vello_cpu::Resources::new());
    Some(GlyphMask {
        width: w,
        height: h,
        left,
        top,
        coverage: pixels.data().iter().map(|p| p.a).collect(),
    })
}
fn draw(
    font: &CFRetained<CTFont>,
    glyph: u16,
    style: MaskStyle,
    bounds: kurbo::Rect,
) -> Option<GlyphMask> {
    let sub_x = f64::from(style.quarter) / 4.0;
    if !(bounds.x0.is_finite()
        && bounds.y0.is_finite()
        && bounds.x1.is_finite()
        && bounds.y1.is_finite())
    {
        return None;
    }
    let left = bounds.x0.floor() as i32 - PAD;
    let top = bounds.y0.floor() as i32 - PAD;
    let right = bounds.x1.ceil() as i32 + PAD + 1;
    let bottom = bounds.y1.ceil() as i32 + PAD;
    let (w, h) = (
        (right - left).max(1) as usize,
        (bottom - top).max(1) as usize,
    );
    if w > 4096 || h > 4096 {
        return None;
    }
    // Skia's offscreen: device RGB, 32 bits, no alpha, host byte order.
    let mut pixels = vec![0xffff_ffffu32; w * h];
    let space = CGColorSpace::new_device_rgb()?;
    let info = CGImageAlphaInfo::NoneSkipFirst.0 | CGImageByteOrderInfo::Order32Little.0;
    let ctx = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            w,
            h,
            8,
            w * 4,
            Some(&space),
            info,
        )
    }?;
    let c = Some(&*ctx);
    CGContext::set_allows_font_subpixel_quantization(c, false);
    CGContext::set_should_subpixel_quantize_fonts(c, false);
    CGContext::set_allows_font_subpixel_positioning(c, true);
    CGContext::set_should_subpixel_position_fonts(c, true);
    CGContext::set_text_drawing_mode(c, objc2_core_graphics::CGTextDrawingMode::Fill);

    // The pen is the CTM's origin, so the text matrix (synthetic italic)
    // shears the glyph about its own origin.
    CGContext::translate_ctm(c, f64::from(-left) + sub_x, (h as i32 + top) as f64);
    if style.skew != 0.0 {
        // CoreGraphics' y runs up: y-down skew `x + s·y` is `c = −s`.
        CGContext::set_text_matrix(
            c,
            objc2_core_foundation::CGAffineTransform {
                a: 1.0,
                b: 0.0,
                c: -f64::from(style.skew),
                d: 1.0,
                tx: 0.0,
                ty: 0.0,
            },
        );
    }
    CGContext::set_should_antialias(c, true);
    CGContext::set_allows_font_smoothing(c, true);
    CGContext::set_should_smooth_fonts(c, true);
    CGContext::set_gray_fill_color(c, 0.0, 1.0);
    // The pen: `-left + sub_x` from the bitmap's left edge, on the baseline
    // `-top` rows below its top (CoreGraphics' y runs up from the bottom);
    // the CTM is translated there.
    let pos = CGPoint { x: 0.0, y: 0.0 };
    unsafe {
        font.draw_glyphs(
            NonNull::new(&glyph as *const u16 as *mut u16).expect("non-null"),
            NonNull::new(&pos as *const CGPoint as *mut CGPoint).expect("non-null"),
            1,
            &ctx,
        )
    };
    CGContext::flush(c);
    drop(ctx);
    let coverage = pixels
        .iter()
        .map(|p| {
            let (r, g, b) = ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff);
            // SkComputeLuminance of the inverted pixel.
            let (r, g, b) = (255 - r, 255 - g, 255 - b);
            ((r * 54 + g * 183 + b * 19) >> 8) as u8
        })
        .collect();
    Some(GlyphMask {
        width: w as u32,
        height: h as u32,
        left,
        top,
        coverage,
    })
}

/// `SkMaskGamma::CanonicalColor` for one channel: its top 3 bits,
/// replicated (`sk_t_scale255<3>`).
fn canonical(v: u32) -> u32 {
    let b = v >> 5;
    (b << 5) | (b << 2) | (b >> 1)
}

/// The luminance bucket Skia's Mac scaler picks a glyph mask table by, for
/// straight E-encoded paint `rgb` (0..1): the paint color canonicalized,
/// halved (smoothed text without subpixel coverage,
/// `SkCTFontSmoothBehavior::some`), canonicalized again, reduced to one
/// luminance for A8 masks (`SkComputeLuminance`), and canonicalized; its
/// top bits are the bucket. Halving leaves only buckets 0 to 3. Verified
/// on ten colors of the fit capture (black, primaries, grays, yellow, cyan).
pub fn luminance_bucket(rgb: [f64; 3]) -> usize {
    let c = |v: f64| canonical((v.clamp(0.0, 1.0) * 255.0).round() as u32);
    let halve = |v: u32| canonical(v / 2);
    let (r, g, b) = (halve(c(rgb[0])), halve(c(rgb[1])), halve(c(rgb[2])));
    let lum = (r * 54 + g * 183 + b * 19) >> 8;
    (canonical(lum) >> 5) as usize
}

/// The mask table for paint `rgb` (`mask_tables::TABLES`).
pub fn mask_table(rgb: [f64; 3]) -> &'static [u8; 256] {
    &super::mask_tables::TABLES[luminance_bucket(rgb).min(3)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;
    #[test]
    fn synthetic_bold_paths_have_area_coverage_without_font_dilation() {
        let rect = kurbo::Rect::new(0., 0., 2.25, 8.);
        let mask = draw_path(&rect.to_path(0.01), 1., 1, rect).unwrap();
        let pixel = |x: i32, y: i32| {
            mask.coverage[((y - mask.top) as u32 * mask.width + (x - mask.left) as u32) as usize]
        };
        // A quarter-pixel pen: [0.25, 2.5]. No fitted expansion or vertical
        // offset may thicken this independently specified outline.
        assert!((i32::from(pixel(0, 3)) - 191).abs() <= 1);
        assert_eq!(pixel(1, 3), 255);
        assert!((i32::from(pixel(2, 3)) - 128).abs() <= 1);
        assert_eq!(pixel(0, -1), 0);
        assert_eq!(pixel(0, 8), 0);
        assert!(
            (mask.coverage.iter().map(|v| f64::from(*v)).sum::<f64>() / 255. - 18.).abs() < 0.04
        );
    }
}
