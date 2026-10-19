//! Engine-owned stem-darkening and positioning profiles (amendment 10).
//!
//! Vello rasterizes plain outline coverage. Today's Canvas2D text and the
//! macOS look (CoreText with font smoothing) are both heavier than plain
//! coverage: CoreGraphics font smoothing dilates glyphs, more at small
//! sizes, and grows them upward. A profile reproduces that on top of plain
//! coverage with levers the engine owns, tabulated by device font size
//! (linear between points, clamped at the ends):
//!
//! 1. **Outline expansion** per side in device pixels, separately in x and
//!    y (`kurbo::expand_path` over the glyph outline; bitmap glyphs are never
//!    expanded).
//! 2. **A vertical ink offset** (CoreGraphics smoothing moves the ink
//!    centroid up).
//! 3. **A coverage curve** the float paint pass applies to Vello's 8-bit
//!    coverage, `c' = 1 − (1 − c)^gamma`. Canvas2D's glyph masks also depend
//!    on the paint's luminance (Skia's mask gamma "preblend"): dark paint
//!    gets heavier coverage. `gamma` is the white-paint value and
//!    `dark_gamma` scales it toward black.
//! 4. **Glyph positioning**: exact (CoreText positions glyphs continuously)
//!    or Skia's subpixel positioning for horizontal text (x to 1/4 px, y
//!    rounded), which is what Canvas2D draws.
//!
//! Stroke passes (caption strokes) are glyph outlines stroked as paths:
//! Skia strokes the path with its own scan converter and no font smoothing,
//! so only positioning and the stroke profile's own adjustments apply.
//!
//! The fitted values and how they were measured are in
//! `docs/engine/text-render.md`.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// How glyph origins are placed in device space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Snap {
    /// Exact layout positions.
    #[default]
    Exact,
    /// x rounded to the nearest 1/4 px, y rounded to whole pixels (Skia's
    /// `SkPackedGlyphID` subpixel positioning for axis-aligned horizontal
    /// text; the rounding bias is half a step).
    QuarterRound,
    /// x and y rounded to whole pixels (CoreText draws color bitmap glyphs
    /// at the nearest device pixel: measured on Apple Color Emoji at 24 to
    /// 64 px and quarter-pixel phases).
    Round,
}

impl Snap {
    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        match self {
            Snap::Exact => (x, y),
            Snap::QuarterRound => ((x * 4.0 + 0.5).floor() / 4.0, (y + 0.5).floor()),
            Snap::Round => ((x + 0.5).floor(), (y + 0.5).floor()),
        }
    }
}

/// The profile at one device font size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Device font size in px.
    pub size: f64,
    /// Outline expansion per side, device px.
    pub expand_x: f64,
    pub expand_y: f64,
    /// Vertical ink offset in device px (negative is up).
    pub dy: f64,
    /// Coverage curve exponent for white paint (1 is linear).
    pub gamma: f64,
}

/// Points of a coverage curve: the value at coverage `i / 16`.
pub const CURVE_POINTS: usize = 17;

/// A coverage curve the paint pass applies (linear between points).
pub type Curve = [f32; CURVE_POINTS];

/// The identity curve.
pub const LINEAR: Curve = {
    let mut c = [0.0f32; CURVE_POINTS];
    let mut i = 0;
    while i < CURVE_POINTS {
        c[i] = i as f32 / 16.0;
        i += 1;
    }
    c
};

/// Evaluates a curve at coverage `c`.
pub fn curve_at(curve: &[f32], c: f64) -> f64 {
    let x = c.clamp(0.0, 1.0) * (curve.len() - 1) as f64;
    let i = (x.floor() as usize).min(curve.len() - 2);
    let t = x - i as f64;
    f64::from(curve[i]) + (f64::from(curve[i + 1]) - f64::from(curve[i])) * t
}

/// The stroke pass of stroked text (`strokeText`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeProfile {
    pub snap: Snap,
    /// Added to the stroke's half width, in device px.
    pub widen: f64,
    /// Coverage curve exponent of the stroke pass (1 is linear).
    pub gamma: f64,
}

/// A stem-darkening and positioning profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// By ascending size.
    pub points: Cow<'static, [Point]>,
    pub snap: Snap,
    /// Glyphs whose device font size, or whose device bounding box in
    /// either dimension, exceeds this draw as plain paths: Skia renders
    /// glyphs beyond its 256 px atlas limit as paths, without CoreGraphics
    /// smoothing (measured on Menlo's H and full block, 150 to 300 px:
    /// the block, 1.03 em tall, switches at 250 px, the H at 260 px).
    pub path_above: Option<f64>,
    /// Glyphs above this device size (and not drawn as paths) are drawn
    /// the way Chromium's GPU canvas draws large text: from the plain
    /// outline at exact positions (no rounding of the baseline), with the
    /// expansion and exponent of `large` and the linear curve.
    #[serde(default)]
    pub large_above: Option<f64>,
    #[serde(default)]
    pub large: Point,
    /// Coverage curves for white and for black paint (`CURVE_POINTS`
    /// values each). Paint in between mixes them by `luma^mix` (Rec. 709
    /// luma of the E-encoded color). Canvas2D's glyph masks are CoreText's
    /// smoothed masks through Skia's luminance-dependent "preblend".
    pub light: Cow<'static, [f32]>,
    pub dark: Cow<'static, [f32]>,
    pub mix: f64,
    /// Synthesized bold: the outline grows by `fake_bold × size` per side
    /// horizontally and `fake_bold_y` times that vertically.
    #[serde(default)]
    pub fake_bold: f64,
    #[serde(default = "one")]
    pub fake_bold_y: f64,
    pub stroke: StrokeProfile,
    /// Under a rotated, skewed or non-uniform matrix the smoothing's
    /// upward ink offset (`Point::dy`) stays in device space, as CoreText
    /// grows glyphs toward the top of the bitmap whatever the text's
    /// orientation (measured on rotations of 90° and 180°); otherwise it
    /// turns with the glyph.
    #[serde(default)]
    pub device_space_offset: bool,
    /// Where axis-aligned bitmap glyphs are placed, when not as `snap`.
    #[serde(default)]
    pub bitmap_snap: Option<Snap>,
}

fn one() -> f64 {
    1.0
}

const fn p(size: f64, expand_x: f64, expand_y: f64, dy: f64, gamma: f64) -> Point {
    Point {
        size,
        expand_x,
        expand_y,
        dy,
        gamma,
    }
}

/// S4's CoreGraphics fit: expansion `min(0.0175 × size, 0.34)`, the ink up
/// by `min(expansion, 0.30)`, exact positions.
const CORETEXT_POINTS: &[Point] = &[
    p(0.0, 0.0, 0.0, 0.0, 1.0),
    p(17.142857, 0.30, 0.30, -0.30, 1.0),
    p(19.428571, 0.34, 0.34, -0.30, 1.0),
];

/// Canvas2D fit (`docs/engine/text-render.md`, "Canvas2D profile"): per
/// device size, horizontal expansion, vertical expansion with the ink moved
/// up by the same amount (CoreGraphics smoothing grows glyphs upward and
/// leaves their bottoms in place; measured on Menlo's full block), and the
/// exponent applied after the measured coverage curve. Fitted on
/// `text-fit` (10 families, 2 weights, 24 sizes), smoothed.
const CANVAS2D_POINTS: &[Point] = &[
    p(8.0, 0.05, 0.05, -0.05, 1.15),
    p(12.0, 0.075, 0.09, -0.09, 1.12),
    p(16.0, 0.088, 0.11, -0.11, 1.14),
    p(20.0, 0.125, 0.13, -0.13, 1.1),
    p(24.0, 0.135, 0.145, -0.145, 1.11),
    p(28.0, 0.158, 0.168, -0.168, 1.125),
    p(32.0, 0.18, 0.192, -0.192, 1.145),
    p(40.0, 0.228, 0.235, -0.235, 1.19),
    p(48.0, 0.272, 0.282, -0.282, 1.24),
    p(64.0, 0.305, 0.31, -0.31, 1.18),
    p(160.0, 0.305, 0.32, -0.32, 1.16),
];

/// Canvas2D's coverage curve for white paint, measured on the bottom edge
/// of Menlo's full block at 8 to 40 px in 1/4 px steps (an edge CoreGraphics
/// smoothing does not move; `text-probe`): the same values at every size.
const CANVAS2D_LIGHT: Curve = [
    0.0, 0.10, 0.20, 0.29, 0.39, 0.48, 0.56, 0.64, 0.71, 0.78, 0.84, 0.88, 0.93, 0.96, 0.98, 1.0,
    1.0,
];

/// The same for black paint (measured at coverage 1/4, 1/2, 3/4: 0.22,
/// 0.46, 0.72; in between `1 − (1 − c)^0.89`).
const CANVAS2D_DARK: Curve = [
    0.0, 0.0558, 0.1121, 0.1687, 0.22, 0.2836, 0.3418, 0.4007, 0.46, 0.5209, 0.5823, 0.6448, 0.72,
    0.7746, 0.8429, 0.9152, 1.0,
];

/// CoreText's coverage for black paint relative to plain coverage:
/// `1 − (1 − c)^0.55`.
const CORETEXT_DARK: Curve = [
    0.0, 0.0349, 0.0708, 0.1079, 0.1463, 0.1862, 0.2278, 0.2713, 0.317, 0.3653, 0.4169, 0.4726,
    0.5335, 0.6018, 0.6814, 0.7824, 1.0,
];

impl Profile {
    /// Plain outline coverage at exact positions.
    pub const PLAIN: Profile = Profile {
        points: Cow::Borrowed(&[p(0.0, 0.0, 0.0, 0.0, 1.0)]),
        snap: Snap::Exact,
        path_above: None,
        large_above: None,
        large: p(0.0, 0.0, 0.0, 0.0, 1.0),
        light: Cow::Borrowed(&LINEAR),
        dark: Cow::Borrowed(&LINEAR),
        mix: 1.0,
        fake_bold: 0.00875,
        fake_bold_y: 0.925,
        stroke: StrokeProfile {
            snap: Snap::Exact,
            widen: 0.0,
            gamma: 1.0,
        },
        device_space_offset: false,
        bitmap_snap: None,
    };

    /// `compat` level 2: CoreText with font smoothing (the macOS look),
    /// fitted in spike S4 (`docs/engine/spikes/S4.md`).
    pub const CORETEXT: Profile = Profile {
        points: Cow::Borrowed(CORETEXT_POINTS),
        snap: Snap::Exact,
        path_above: None,
        large_above: None,
        large: p(0.0, 0.0, 0.0, 0.0, 1.0),
        light: Cow::Borrowed(&LINEAR),
        // Dark paint is lighter under CoreGraphics smoothing: measured
        // against CoreText on the `colors` cases (`tests/text_level2.rs`).
        dark: Cow::Borrowed(&CORETEXT_DARK),
        mix: 1.4,
        fake_bold: 0.00875,
        fake_bold_y: 0.925,
        stroke: StrokeProfile {
            snap: Snap::Exact,
            widen: 0.0,
            gamma: 1.0,
        },
        device_space_offset: true,
        bitmap_snap: Some(Snap::Round),
    };

    /// `compat` level 1: today's Canvas2D (Chromium's Skia over CoreText's
    /// smoothed glyph masks).
    pub const CANVAS2D: Profile = Profile {
        points: Cow::Borrowed(CANVAS2D_POINTS),
        snap: Snap::QuarterRound,
        path_above: Some(256.0),
        // Measured on Menlo's H at 150 to 300 px (`text-probe`): masks up
        // to 160 px; from 170 px glyphs sit at exact positions with this
        // treatment (mean SSIM 0.991 at 170 to 250 px).
        large_above: Some(162.0),
        large: p(200.0, 0.425, 0.325, -0.303, 0.962),
        light: Cow::Borrowed(&CANVAS2D_LIGHT),
        dark: Cow::Borrowed(&CANVAS2D_DARK),
        mix: 1.0,
        fake_bold: 0.00875,
        fake_bold_y: 0.925,
        // Fitted on `fit-stroke` (7 sizes × 6 widths): mean SSIM 0.992.
        stroke: StrokeProfile {
            snap: Snap::QuarterRound,
            widen: -0.025,
            gamma: 0.612,
        },
        device_space_offset: false,
        bitmap_snap: None,
    };

    fn as_path(&self, size: f64) -> bool {
        // `SubRunContainer`: masks and distance fields only below the
        // maximum size (`approximateDeviceTextSize < maxMaskSize`).
        self.path_above.is_some_and(|p| size >= p)
    }

    /// Whether a glyph at `size` device px is in the large-text regime.
    pub fn is_large(&self, size: f64) -> bool {
        self.large_above.is_some_and(|l| size > l)
    }

    /// Whether a glyph at `size` device px with plain outline `bounds`
    /// (device px) draws as a path.
    pub fn glyph_as_path(&self, size: f64, bounds: kurbo::Rect) -> bool {
        // Skia compares the glyph mask's size, the bounds rounded out (one
        // pixel more than the outline's extent at most phases).
        self.as_path(size)
            || self
                .path_above
                .is_some_and(|p| bounds.width() + 1.0 > p || bounds.height() + 1.0 > p)
    }

    /// The profile at `size` device px (linear between points).
    pub fn at(&self, size: f64) -> Point {
        let pts = &self.points;
        let Some(first) = pts.first() else {
            return p(size, 0.0, 0.0, 0.0, 1.0);
        };
        if size <= first.size {
            return *first;
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if size <= b.size {
                let t = if b.size > a.size {
                    (size - a.size) / (b.size - a.size)
                } else {
                    1.0
                };
                let l = |x: f64, y: f64| x + (y - x) * t;
                return Point {
                    size,
                    expand_x: l(a.expand_x, b.expand_x),
                    expand_y: l(a.expand_y, b.expand_y),
                    dy: l(a.dy, b.dy),
                    gamma: l(a.gamma, b.gamma),
                };
            }
        }
        *pts.last().expect("not empty")
    }

    /// Outline expansion per side `(x, y)` in device px for a glyph at
    /// `size` device px.
    pub fn expansion(&self, size: f64, bitmap: bool) -> (f64, f64) {
        if bitmap || self.as_path(size) {
            return (0.0, 0.0);
        }
        let p = self.at(size);
        (p.expand_x.max(0.0), p.expand_y.max(0.0))
    }

    /// The vertical ink offset in device px (negative is up).
    pub fn dy(&self, size: f64, bitmap: bool) -> f64 {
        if bitmap || self.as_path(size) {
            return 0.0;
        }
        self.at(size).dy
    }

    /// A fill glyph's origin in device space.
    pub fn position(&self, x: f64, y: f64, size: f64, bitmap: bool) -> (f64, f64) {
        let (x, y) = self.snap.apply(x, y);
        (x, y + self.dy(size, bitmap))
    }

    /// An axis-aligned bitmap glyph's origin in device space.
    pub fn bitmap_position(&self, x: f64, y: f64, size: f64) -> (f64, f64) {
        match self.bitmap_snap {
            Some(snap) => {
                let (x, y) = snap.apply(x, y);
                (x, y + self.dy(size, true))
            }
            None => self.position(x, y, size, true),
        }
    }

    /// A stroked glyph's origin in device space.
    pub fn stroke_position(&self, x: f64, y: f64) -> (f64, f64) {
        self.stroke.snap.apply(x, y)
    }

    /// The fill pass's coverage curve at `size` device px for paint `rgb`
    /// (E-encoded): the light and dark curves mixed by luma, then the
    /// size's exponent `1 − (1 − v)^gamma`.
    pub fn curve(&self, size: f64, rgb: [f64; 3]) -> Curve {
        if self.as_path(size) {
            return LINEAR;
        }
        let gamma = self.at(size).gamma;
        let luma = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]).clamp(0.0, 1.0);
        let w = luma.powf(self.mix.max(0.0));
        let mut out = LINEAR;
        for (i, o) in out.iter_mut().enumerate() {
            let c = i as f64 / 16.0;
            let v = curve_at(&self.dark, c) * (1.0 - w) + curve_at(&self.light, c) * w;
            *o = coverage_curve(v, gamma) as f32;
        }
        out
    }
}

/// `c' = 1 − (1 − c)^gamma`, the curve the paint pass applies.
pub fn coverage_curve(c: f64, gamma: f64) -> f64 {
    if gamma == 1.0 {
        c
    } else {
        1.0 - (1.0 - c.clamp(0.0, 1.0)).powf(gamma)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_follows_skia() {
        assert_eq!(Snap::QuarterRound.apply(10.124, 3.49), (10.0, 3.0));
        assert_eq!(Snap::QuarterRound.apply(10.126, 3.5), (10.25, 4.0));
        assert_eq!(Snap::Exact.apply(1.3, 2.7), (1.3, 2.7));
    }

    #[test]
    fn expansion_stops_at_the_path_size() {
        let p = Profile::CANVAS2D;
        assert!(p.expansion(200.0, false).0 > 0.0);
        assert_eq!(p.expansion(300.0, false), (0.0, 0.0));
        assert_eq!(p.expansion(20.0, true), (0.0, 0.0));
        assert_eq!(p.dy(20.0, true), 0.0);
        assert!(p.dy(48.0, false) < 0.0);
    }

    #[test]
    fn points_interpolate_and_clamp() {
        let p = Profile::CORETEXT;
        assert_eq!(p.expansion(0.0, false), (0.0, 0.0));
        assert!((p.expansion(10.0, false).0 - 0.175).abs() < 1e-6);
        assert!((p.expansion(100.0, false).0 - 0.34).abs() < 1e-12);
        assert!((p.dy(10.0, false) + 0.175).abs() < 1e-6);
    }

    #[test]
    fn paint_luminance_mixes_the_curves() {
        let mut p = Profile::CANVAS2D;
        p.points = Cow::Owned(vec![Point {
            size: 0.0,
            expand_x: 0.0,
            expand_y: 0.0,
            dy: 0.0,
            gamma: 1.0,
        }]);
        let white = p.curve(20.0, [1.0; 3]);
        let black = p.curve(20.0, [0.0; 3]);
        assert!((white[4] - 0.39).abs() < 1e-6);
        assert!((black[4] - 0.22).abs() < 1e-6);
        let gray = p.curve(20.0, [0.5; 3]);
        assert!(gray[4] > black[4] && gray[4] < white[4]);
        assert_eq!(p.curve(300.0, [1.0; 3]), LINEAR);
        assert_eq!(Profile::CORETEXT.curve(20.0, [1.0; 3]), LINEAR);
        assert!((curve_at(&LINEAR, 0.3) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn the_curve_is_monotone_and_fixes_the_ends() {
        for g in [0.6, 1.0, 1.4] {
            assert_eq!(coverage_curve(0.0, g), 0.0);
            assert_eq!(coverage_curve(1.0, g), 1.0);
            let mut last = 0.0;
            for i in 0..=255 {
                let v = coverage_curve(f64::from(i) / 255.0, g);
                assert!(v >= last);
                last = v;
            }
        }
    }
}
