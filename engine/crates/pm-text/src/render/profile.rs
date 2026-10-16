//! Engine-owned stem-darkening and positioning profiles (amendment 10).
//!
//! Vello rasterizes plain outline coverage. Today's Canvas2D text and the
//! macOS look (CoreText with font smoothing) are both heavier than plain
//! coverage: CoreGraphics font smoothing dilates glyphs, more at small
//! sizes, and grows them upward. A profile reproduces that on top of plain
//! coverage, with four levers the engine owns:
//!
//! 1. **Outline expansion** per side in device pixels, a function of the
//!    device font size, separately in x and y (`kurbo::expand_path` over the
//!    glyph outline; bitmap glyphs are never expanded).
//! 2. **A vertical ink offset** (CoreGraphics smoothing moves the ink
//!    centroid up by about 0.3 px from 24 px up).
//! 3. **Glyph positioning**: exact (CoreText positions glyphs continuously)
//!    or Skia's subpixel positioning for horizontal text (x to 1/4 px, y
//!    rounded), which is what Canvas2D draws.
//! 4. **A coverage curve** applied by the float paint pass to Vello's 8-bit
//!    coverage: `c' = 1 − (1 − c)^gamma`, with `gamma` a function of the
//!    device size. It models the contrast of CoreText's smoothed masks at
//!    small sizes, which no uniform expansion reproduces.
//!
//! Stroke passes (caption strokes) are glyph outlines stroked as paths:
//! Skia strokes the path with its own scan converter and no font smoothing,
//! so only positioning (and the stroke profile's own small adjustments)
//! apply.
//!
//! The fitted values and how they were measured are in
//! `docs/engine/text-render.md`.

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

/// A value that depends on the device font size: `min(base + k × size,
/// max)`, and `small` below `small_until` px blended linearly into it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BySize {
    pub base: f64,
    pub k: f64,
    pub max: f64,
    /// Value at `small_at` px; blended linearly to the main curve at
    /// `small_until` px. `small_until == 0` disables the small-size branch.
    pub small: f64,
    pub small_at: f64,
    pub small_until: f64,
}

impl BySize {
    pub const fn constant(v: f64) -> BySize {
        BySize {
            base: v,
            k: 0.0,
            max: v,
            small: 0.0,
            small_at: 0.0,
            small_until: 0.0,
        }
    }

    pub fn at(&self, size: f64) -> f64 {
        let main = (self.base + self.k * size).min(self.max);
        if self.small_until > self.small_at && size < self.small_until {
            let t = ((size - self.small_at) / (self.small_until - self.small_at)).clamp(0.0, 1.0);
            let end = (self.base + self.k * self.small_until).min(self.max);
            self.small + (end - self.small) * t
        } else {
            main
        }
    }
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Horizontal outline expansion per side, device px.
    pub expand: BySize,
    /// Vertical expansion as a fraction of the horizontal one.
    pub expand_y: f64,
    /// The ink moves up by `min(dy_k × expansion, dy_max)` px.
    pub dy_k: f64,
    pub dy_max: f64,
    pub snap: Snap,
    /// Glyphs above this device size draw as plain paths: Skia renders
    /// glyphs larger than its 256 px atlas limit as paths, without
    /// CoreGraphics smoothing.
    pub path_above: Option<f64>,
    /// Coverage curve exponent by device size (1 is linear coverage).
    pub gamma: BySize,
    pub stroke: StrokeProfile,
}

impl Profile {
    /// Plain outline coverage at exact positions.
    pub const PLAIN: Profile = Profile {
        expand: BySize::constant(0.0),
        expand_y: 1.0,
        dy_k: 0.0,
        dy_max: 0.0,
        snap: Snap::Exact,
        path_above: None,
        gamma: BySize::constant(1.0),
        stroke: StrokeProfile {
            snap: Snap::Exact,
            widen: 0.0,
            gamma: 1.0,
        },
    };

    /// `compat` level 2: CoreText with font smoothing (the macOS look),
    /// fitted in spike S4 (`docs/engine/spikes/S4.md`).
    pub const CORETEXT: Profile = Profile {
        expand: BySize {
            base: 0.0,
            k: 0.0175,
            max: 0.34,
            small: 0.0,
            small_at: 0.0,
            small_until: 0.0,
        },
        expand_y: 1.0,
        dy_k: 1.0,
        dy_max: 0.30,
        snap: Snap::Exact,
        path_above: None,
        gamma: BySize::constant(1.0),
        stroke: StrokeProfile {
            snap: Snap::Exact,
            widen: 0.0,
            gamma: 1.0,
        },
    };

    /// `compat` level 1: today's Canvas2D (Chromium's Skia over CoreText's
    /// smoothed glyph masks). The S4 fit, refined in Phase 3a for sizes up
    /// to 18 px and for stroked captions (`docs/engine/text-render.md`).
    pub const CANVAS2D: Profile = Profile {
        expand: BySize {
            base: 0.11,
            k: 0.0075,
            max: 0.46,
            small: 0.11,
            small_at: 0.0,
            small_until: 0.0,
        },
        expand_y: 1.0,
        dy_k: 0.65,
        dy_max: 0.30,
        snap: Snap::QuarterRound,
        path_above: Some(256.0),
        gamma: BySize::constant(1.0),
        stroke: StrokeProfile {
            snap: Snap::QuarterRound,
            widen: 0.0,
            gamma: 1.0,
        },
    };

    fn as_path(&self, size: f64) -> bool {
        self.path_above.is_some_and(|p| size > p)
    }

    /// Outline expansion per side `(x, y)` in device px for a glyph at
    /// `size` device px.
    pub fn expansion(&self, size: f64, bitmap: bool) -> (f64, f64) {
        if bitmap || self.as_path(size) {
            return (0.0, 0.0);
        }
        let x = self.expand.at(size).max(0.0);
        (x, x * self.expand_y)
    }

    /// The vertical ink offset in device px (negative is up).
    pub fn dy(&self, size: f64, bitmap: bool) -> f64 {
        if bitmap || self.as_path(size) {
            return 0.0;
        }
        let (x, _) = self.expansion(size, false);
        -(self.dy_k * x).min(self.dy_max)
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

    /// The fill pass's coverage curve exponent at `size` device px.
    pub fn gamma(&self, size: f64, bitmap: bool) -> f64 {
        if bitmap || self.as_path(size) {
            1.0
        } else {
            self.gamma.at(size).max(0.05)
        }
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
    fn by_size_blends_the_small_branch() {
        let c = BySize {
            base: 1.0,
            k: 0.0,
            max: 1.0,
            small: 2.0,
            small_at: 8.0,
            small_until: 16.0,
        };
        assert_eq!(c.at(4.0), 2.0);
        assert_eq!(c.at(12.0), 1.5);
        assert_eq!(c.at(20.0), 1.0);
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
