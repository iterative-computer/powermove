//! Glyph masks as Chromium's Canvas2D gets them on macOS (level 1).
//!
//! Skia's CoreText scaler (`SkScalerContext_Mac`) does not rasterize glyph
//! outlines itself: for an A8 mask with font smoothing (`kGenA8FromLCD`), it
//! draws the glyph with CoreGraphics into a 32-bit RGB bitmap, black on
//! white, with antialiasing and smoothing on, subpixel positioning on and
//! quantization off, at the glyph's quarter-pixel horizontal offset, then
//! turns each pixel's luminance into coverage through the paint's
//! "preblend" table (`SkMaskGamma`, chosen by the paint's luminance). The
//! glyph atlas then draws the mask 1:1 at a whole device pixel.
//!
//! This module does the same with the same system's CoreGraphics, so the
//! masks are CoreGraphics' own (its smoothing, which no outline dilation
//! reproduces at small sizes), and the measured tables (`mask_tables`) are
//! the four the paint's luminance bucket selects ([`luminance_bucket`]).
//! Masks are cached by face, glyph, size, variation location, and
//! quarter-pixel offset.

use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::Arc;

use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_core_text::CTFont;

use super::coretext::{CoreTextReference, RefRun};
use super::outline::FaceRef;

pub use super::scene::GlyphMask;

impl GlyphMask {
    pub fn is_empty(&self) -> bool {
        self.coverage.iter().all(|&c| c == 0)
    }
}

/// Face buffer, face index, glyph, size bits, variation (tag, value bits),
/// quarter-pixel offset.
type MaskKey = (usize, u32, u16, u32, Vec<(u32, u32)>, u8);

/// CoreGraphics glyph masks, cached.
#[derive(Default)]
pub struct CgMasks {
    fonts: CoreTextReference,
    masks: HashMap<MaskKey, Option<Arc<GlyphMask>>>,
    colors: HashMap<ColorKey, ColorGlyph>,
    bytes: usize,
}

/// Face buffer, face index, glyph, size bits, em bits, quarter offset.
type ColorKey = (usize, u32, u16, u32, u32, u8);
type ColorGlyph = Option<(Arc<super::bitmap::Rgba8>, i32, i32)>;

// SAFETY: CoreText fonts are immutable and thread-safe (CTFont is
// documented as safe to use from any thread); every bitmap context is
// created, drawn, and released within one `mask` call. The masks move with
// the renderer to its render thread and are never shared.
unsafe impl Send for CgMasks {}

impl super::scene::MaskSource for CgMasks {
    fn mask(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        quarter: u8,
        bounds: kurbo::Rect,
    ) -> Option<Arc<GlyphMask>> {
        CgMasks::mask(self, face, coords, glyph, size, quarter, bounds)
    }

    fn preblend(&self, rgb: [f64; 3]) -> [u8; 256] {
        *mask_table(rgb)
    }

    fn color(
        &mut self,
        face: FaceRef<'_>,
        glyph: u16,
        size: f32,
        quarter: u8,
        em: f32,
    ) -> Option<(Arc<super::bitmap::Rgba8>, i32, i32)> {
        let key = (
            face.id,
            face.index,
            glyph,
            size.to_bits(),
            em.to_bits(),
            quarter,
        );
        if let Some(hit) = self.colors.get(&key) {
            return hit.clone();
        }
        let run = RefRun {
            data: face.data,
            index: face.index,
            size,
            coords: &[],
            glyphs: Vec::new(),
            color: [0.0, 0.0, 0.0, 1.0],
        };
        let built = self
            .fonts
            .font(&run)
            .and_then(|f| draw_color(&f, glyph, f64::from(quarter) / 4.0, em.max(size)));
        if self.colors.len() > 4096 {
            self.colors.clear();
        }
        self.colors.insert(key, built.clone());
        built
    }
}

/// Masks kept before the cache starts over.
const CACHE_BYTES: usize = 64 << 20;

impl CgMasks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.masks.clear();
        self.colors.clear();
        self.bytes = 0;
    }

    /// The mask of `glyph` at `size` device px and variation `coords`
    /// (user space, `opsz` included), drawn at `quarter`/4 px right of a
    /// whole pixel. `bounds` is the glyph's outline box at that size (device
    /// px, y down, relative to the pen), used to size the bitmap; it is
    /// grown for CoreGraphics' smoothing. `None` when the font does not load.
    pub fn mask(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        quarter: u8,
        bounds: kurbo::Rect,
    ) -> Option<Arc<GlyphMask>> {
        let key = (
            face.id,
            face.index,
            glyph,
            size.to_bits(),
            coords.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
            quarter,
        );
        if let Some(m) = self.masks.get(&key) {
            return m.clone();
        }
        let run = RefRun {
            data: face.data,
            index: face.index,
            size,
            coords,
            glyphs: Vec::new(),
            color: [0.0, 0.0, 0.0, 1.0],
        };
        let font = self.fonts.font(&run);
        let mask =
            font.and_then(|f| draw(&f, glyph, f64::from(quarter) / 4.0, bounds).map(Arc::new));
        if self.bytes > CACHE_BYTES {
            self.clear();
        }
        self.bytes += mask.as_ref().map_or(0, |m| m.coverage.len() + 64);
        self.masks.insert(key, mask.clone());
        mask
    }
}

/// A color glyph drawn by CoreGraphics into a premultiplied RGBA bitmap
/// (Skia's `kARGB32` glyph images), in a box sized from the em.
fn draw_color(font: &CFRetained<CTFont>, glyph: u16, sub_x: f64, em: f32) -> ColorGlyph {
    let e = f64::from(em);
    let left = -(PAD + 2);
    let top = -((e * 1.25).ceil() as i32 + PAD);
    let right = (e * 1.5).ceil() as i32 + PAD;
    let bottom = (e * 0.5).ceil() as i32 + PAD;
    let (w, h) = ((right - left) as usize, (bottom - top) as usize);
    if w > 4096 || h > 4096 {
        return None;
    }
    let mut pixels = vec![0u32; w * h];
    let space = CGColorSpace::new_device_rgb()?;
    let info = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;
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
    CGContext::set_should_antialias(c, true);
    CGContext::set_allows_font_smoothing(c, true);
    CGContext::set_should_smooth_fonts(c, true);
    let pos = CGPoint {
        x: f64::from(-left) + sub_x,
        y: (h as i32 + top) as f64,
    };
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
    if pixels.iter().all(|&p| p == 0) {
        return None;
    }
    let mut data = Vec::with_capacity(w * h * 4);
    for p in &pixels {
        // BGRA in memory (little-endian ARGB): to RGBA.
        let (a, r, g, b) = ((p >> 24) as u8, (p >> 16) as u8, (p >> 8) as u8, *p as u8);
        data.extend_from_slice(&[r, g, b, a]);
    }
    Some((
        Arc::new(super::bitmap::Rgba8 {
            width: w as u32,
            height: h as u32,
            data,
        }),
        left,
        top,
    ))
}

/// Room around the outline box for smoothing (Skia outsets its CoreGraphics
/// bounds by one pixel; two keep every smoothed pixel).
const PAD: i32 = 3;

fn draw(
    font: &CFRetained<CTFont>,
    glyph: u16,
    sub_x: f64,
    bounds: kurbo::Rect,
) -> Option<GlyphMask> {
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
    CGContext::set_should_antialias(c, true);
    CGContext::set_allows_font_smoothing(c, true);
    CGContext::set_should_smooth_fonts(c, true);
    CGContext::set_gray_fill_color(c, 0.0, 1.0);
    // The pen: `-left + sub_x` from the bitmap's left edge, on the baseline
    // `-top` rows below its top (CoreGraphics' y runs up from the bottom).
    let pos = CGPoint {
        x: f64::from(-left) + sub_x,
        y: (h as i32 + top) as f64,
    };
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

    #[test]
    fn luminance_buckets_follow_the_mac_scaler() {
        let hex = |h: u32| {
            [
                f64::from(h >> 16) / 255.0,
                f64::from((h >> 8) & 0xff) / 255.0,
                f64::from(h & 0xff) / 255.0,
            ]
        };
        // The fit capture's colors and the table each used.
        for (color, bucket) in [
            (0x000000, 0),
            (0x0000ff, 0),
            (0xff0000, 0),
            (0xff3b30, 0),
            (0x151515, 0),
            (0x404040, 1),
            (0x00ff00, 2),
            (0x00ffff, 2),
            (0x808080, 2),
            (0x24d5ff, 2),
            (0xc0c0c0, 3),
            (0xffff00, 3),
            (0xffd43b, 3),
            (0xffffff, 3),
        ] {
            assert_eq!(luminance_bucket(hex(color)), bucket, "{color:06x}");
        }
        // The tables are monotonic and fix both ends.
        for t in &super::super::mask_tables::TABLES {
            assert_eq!((t[0], t[255]), (0, 255));
            assert!(t.windows(2).all(|w| w[0] <= w[1]));
        }
    }

    #[test]
    fn a_glyph_masks_like_coregraphics() {
        let data = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("Arial");
        let face = FaceRef {
            data: &data,
            index: 0,
            id: data.as_ptr() as usize,
        };
        let gid = {
            use read_fonts::TableProvider;
            read_fonts::FontRef::new(&data)
                .unwrap()
                .cmap()
                .unwrap()
                .map_codepoint('H' as u32)
                .unwrap()
                .to_u32() as u16
        };
        let mut masks = CgMasks::new();
        let bounds = kurbo::Rect::new(1.0, -11.5, 9.0, 0.0);
        let m = masks.mask(face, &[], gid, 16.0, 0, bounds).unwrap();
        assert!(!m.is_empty());
        // The H's left stem starts about one pixel right of the pen.
        let col = |x: i32| -> u32 {
            (0..m.height as usize)
                .map(|y| u32::from(m.coverage[y * m.width as usize + (x - m.left) as usize]))
                .sum()
        };
        assert!(col(-1) == 0 && col(2) > 1000);
        // Offsets move the ink; the cache returns the same mask.
        let q = masks.mask(face, &[], gid, 16.0, 2, bounds).unwrap();
        assert_ne!(q.coverage, m.coverage);
        assert!(Arc::ptr_eq(
            &masks.mask(face, &[], gid, 16.0, 0, bounds).unwrap(),
            &m
        ));
    }
}
