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
}

impl Snap {
    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        match self {
            Snap::Exact => (x, y),
            Snap::QuarterRound => ((x * 4.0 + 0.5).floor() / 4.0, (y + 0.5).floor()),
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
    /// Glyphs above this device size draw as plain paths: Skia renders
    /// glyphs larger than its 256 px atlas limit as paths, without
    /// CoreGraphics smoothing.
    pub path_above: Option<f64>,
    /// The coverage exponent for black paint is `gamma × dark_gamma`; paint
    /// in between interpolates by `1 − luminance` (Rec. 709 luma of the
    /// E-encoded color).
    pub dark_gamma: f64,
    pub stroke: StrokeProfile,
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

/// Canvas2D fit (`docs/engine/text-render.md`, "Canvas2D profile").
const CANVAS2D_POINTS: &[Point] = &[
    p(0.0, 0.11, 0.11, -0.0715, 1.0),
    p(29.333333, 0.33, 0.33, -0.2145, 1.0),
    p(32.0, 0.35, 0.35, -0.2275, 1.0),
    p(46.153846, 0.4561538, 0.4561538, -0.2965, 1.0),
    p(46.666667, 0.46, 0.46, -0.30, 1.0),
];

impl Profile {
    /// Plain outline coverage at exact positions.
    pub const PLAIN: Profile = Profile {
        points: Cow::Borrowed(&[p(0.0, 0.0, 0.0, 0.0, 1.0)]),
        snap: Snap::Exact,
        path_above: None,
        dark_gamma: 1.0,
        stroke: StrokeProfile {
            snap: Snap::Exact,
            widen: 0.0,
            gamma: 1.0,
        },
    };

    /// `compat` level 2: CoreText with font smoothing (the macOS look),
    /// fitted in spike S4 (`docs/engine/spikes/S4.md`).
    pub const CORETEXT: Profile = Profile {
        points: Cow::Borrowed(CORETEXT_POINTS),
        snap: Snap::Exact,
        path_above: None,
        dark_gamma: 1.0,
        stroke: StrokeProfile {
            snap: Snap::Exact,
            widen: 0.0,
            gamma: 1.0,
        },
    };

    /// `compat` level 1: today's Canvas2D (Chromium's Skia over CoreText's
    /// smoothed glyph masks).
    pub const CANVAS2D: Profile = Profile {
        points: Cow::Borrowed(CANVAS2D_POINTS),
        snap: Snap::QuarterRound,
        path_above: Some(256.0),
        dark_gamma: 1.0,
        stroke: StrokeProfile {
            snap: Snap::QuarterRound,
            widen: 0.0,
            gamma: 1.0,
        },
    };

    fn as_path(&self, size: f64) -> bool {
        self.path_above.is_some_and(|p| size > p)
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

    /// A stroked glyph's origin in device space.
    pub fn stroke_position(&self, x: f64, y: f64) -> (f64, f64) {
        self.stroke.snap.apply(x, y)
    }

    /// The fill pass's coverage curve exponent at `size` device px for
    /// paint `rgb` (E-encoded).
    pub fn gamma(&self, size: f64, rgb: [f64; 3]) -> f64 {
        if self.as_path(size) {
            return 1.0;
        }
        let white = self.at(size).gamma;
        let luma = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]).clamp(0.0, 1.0);
        (white * (1.0 + (self.dark_gamma - 1.0) * (1.0 - luma))).max(0.05)
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
    fn dark_paint_scales_gamma() {
        let mut p = Profile::CANVAS2D;
        p.dark_gamma = 1.5;
        assert!((p.gamma(20.0, [1.0; 3]) - 1.0).abs() < 1e-12);
        assert!((p.gamma(20.0, [0.0; 3]) - 1.5).abs() < 1e-12);
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
