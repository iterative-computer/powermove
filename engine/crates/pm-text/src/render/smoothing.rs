//! Text weight: how heavy glyph fills draw (TEXT-SMOOTH-1).
//!
//! Exact outline coverage is the glyph as designed. Today's app draws text
//! through Canvas2D on macOS, where CoreGraphics font smoothing dilates
//! glyphs (more at small sizes, growing them upward) and Skia maps coverage
//! through a luminance-dependent curve: the result is about 15 % heavier
//! (Geist Regular "i" stem width/height 0.183 against the outlines' 0.158).
//! Every video users have posted carries that weight, so frames reproduce
//! it on top of exact coverage with levers the engine owns:
//!
//! 1. an outline expansion per side, in device pixels, x and y (round
//!    joins: a true dilation);
//! 2. the ink moved up by the vertical expansion (smoothing leaves glyph
//!    bottoms in place);
//! 3. a coverage curve in the paint pass, `1 − (1 − c)^e`, with `e` for
//!    white paint and for black paint mixed by the paint's Rec. 709 luma;
//! 4. for Canvas2D, Skia's subpixel glyph placement (x to 1/4 px, the
//!    baseline to whole pixels) on axis-aligned text.
//!
//! The tables are the fits measured before TEXT-RASTER-1 replaced them
//! (`git show 7b3a2962e^:engine/crates/pm-text/src/render/profile.rs`): the
//! Canvas2D fit on 10 families, 2 weights and 24 sizes, the CoreText fit from
//! spike S4. The coverage curves are those fits' measured 17-point curves
//! reduced to one exponent each (largest error 0.018 for white, 0.008 for
//! black). Layout positions are unchanged (TEXT layout gates); stroke
//! passes, bitmap and COLR glyphs are never smoothed. `pm-render`'s
//! `text_weight` gate holds the result to today's measured stem widths
//! (`tools/visual-parity/text-weight-probe.mjs`).

/// How glyph fills are weighted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Smoothing {
    /// Plain outline coverage.
    #[default]
    Exact,
    /// Today's Canvas2D text on macOS (`compat` level 1, captions).
    Canvas2d,
    /// CoreText with font smoothing, the macOS look (`compat` level 2).
    CoreText,
}

/// The smoothing at one device font size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weight {
    /// Outline expansion per side, device px.
    pub expand_x: f64,
    pub expand_y: f64,
    /// Vertical ink offset, device px (negative is up).
    pub dy: f64,
    /// Coverage exponents for white and black paint, and the luma power
    /// that mixes them.
    pub light: f64,
    pub dark: f64,
    pub mix: f64,
    /// Glyph origins on Skia's subpixel grid for axis-aligned text: x to the
    /// nearest 1/4 px, the baseline to whole pixels (`SkPackedGlyphID`;
    /// layout positions are unchanged, only where the ink lands).
    pub snap: bool,
}

impl Weight {
    pub const EXACT: Weight = Weight {
        expand_x: 0.0,
        expand_y: 0.0,
        dy: 0.0,
        light: 1.0,
        dark: 1.0,
        mix: 1.0,
        snap: false,
    };

    /// A device-space glyph origin as this weight places it.
    pub fn place(&self, x: f64, y: f64) -> (f64, f64) {
        if self.snap {
            ((x * 4.0 + 0.5).floor() / 4.0, (y + 0.5).floor())
        } else {
            (x, y)
        }
    }

    /// The paint pass's curve for straight E-encoded `color`:
    /// `[weight of the white curve, white exponent, black exponent]`, or the
    /// identity `[0, 1, 1]`.
    pub fn curve(&self, color: [f64; 4]) -> [f32; 3] {
        if self.light == 1.0 && self.dark == 1.0 {
            return IDENTITY;
        }
        let luma = (0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2]).clamp(0.0, 1.0);
        [
            luma.powf(self.mix) as f32,
            self.light as f32,
            self.dark as f32,
        ]
    }

    pub fn is_exact(&self) -> bool {
        *self == Weight::EXACT
    }
}

/// The paint pass's identity curve.
pub const IDENTITY: [f32; 3] = [0.0, 1.0, 1.0];

/// Applies a paint curve to coverage `c` (the paint shader's arithmetic).
pub fn apply(curve: [f32; 3], c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if curve[1] == 1.0 && curve[2] == 1.0 {
        return c;
    }
    let light = 1.0 - (1.0 - c).powf(f64::from(curve[1]));
    let dark = 1.0 - (1.0 - c).powf(f64::from(curve[2]));
    dark + (light - dark) * f64::from(curve[0])
}

/// `(device size, expansion x, expansion y = −dy, exponent after the curve)`.
type Row = (f64, f64, f64, f64);

/// The Canvas2D fit, smoothed (`text-fit`).
const CANVAS2D: &[Row] = &[
    (8.0, 0.05, 0.05, 1.15),
    (12.0, 0.075, 0.09, 1.12),
    (16.0, 0.088, 0.11, 1.14),
    (20.0, 0.125, 0.13, 1.1),
    (24.0, 0.135, 0.145, 1.11),
    (28.0, 0.158, 0.168, 1.125),
    (32.0, 0.18, 0.192, 1.145),
    (40.0, 0.228, 0.235, 1.19),
    (48.0, 0.272, 0.282, 1.24),
    (64.0, 0.305, 0.31, 1.18),
    (160.0, 0.305, 0.32, 1.16),
];
/// Canvas2D's white- and black-paint coverage curves as exponents.
const CANVAS2D_LIGHT: f64 = 1.77;
const CANVAS2D_DARK: f64 = 0.90;
/// Above this device size Chromium draws glyphs from their plain outlines
/// at exact positions (measured on Menlo's H, 150 to 300 px), with this
/// expansion and a linear curve; from 256 px they are paths, unsmoothed.
const CANVAS2D_LARGE_ABOVE: f64 = 162.0;
const CANVAS2D_LARGE: Row = (200.0, 0.425, 0.325, 0.962);
const CANVAS2D_PATH_ABOVE: f64 = 256.0;

/// S4's CoreGraphics fit: expansion `min(0.0175 × size, 0.34)`, the ink up
/// by `min(expansion, 0.30)`; black paint `1 − (1 − c)^0.55`, mixed by
/// `luma^1.4`.
const CORETEXT: &[Row] = &[
    (0.0, 0.0, 0.0, 1.0),
    (17.142857, 0.30, 0.30, 1.0),
    (19.428571, 0.34, 0.30, 1.0),
];

fn interpolate(rows: &[Row], size: f64) -> Row {
    let first = rows[0];
    if size <= first.0 {
        return first;
    }
    for w in rows.windows(2) {
        let (a, b) = (w[0], w[1]);
        if size <= b.0 {
            let t = (size - a.0) / (b.0 - a.0);
            let lerp = |x: f64, y: f64| x + (y - x) * t;
            return (size, lerp(a.1, b.1), lerp(a.2, b.2), lerp(a.3, b.3));
        }
    }
    *rows.last().unwrap()
}

impl Smoothing {
    /// The weighting of a glyph fill at `size` device px.
    pub fn at(self, size: f64) -> Weight {
        if !size.is_finite() || size <= 0.0 {
            return Weight::EXACT;
        }
        match self {
            Smoothing::Exact => Weight::EXACT,
            Smoothing::Canvas2d => {
                if size >= CANVAS2D_PATH_ABOVE {
                    return Weight::EXACT;
                }
                if size > CANVAS2D_LARGE_ABOVE {
                    let (_, x, y, g) = CANVAS2D_LARGE;
                    return Weight {
                        expand_x: x,
                        expand_y: y,
                        dy: -y,
                        light: g,
                        dark: g,
                        mix: 1.0,
                        snap: false,
                    };
                }
                let (_, x, y, g) = interpolate(CANVAS2D, size);
                Weight {
                    expand_x: x,
                    expand_y: y,
                    dy: -y,
                    light: CANVAS2D_LIGHT * g,
                    dark: CANVAS2D_DARK * g,
                    mix: 1.0,
                    snap: true,
                }
            }
            Smoothing::CoreText => {
                let (_, x, y, _) = interpolate(CORETEXT, size);
                Weight {
                    expand_x: x,
                    expand_y: x,
                    dy: -y,
                    light: 1.0,
                    dark: 0.55,
                    mix: 1.4,
                    snap: false,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_is_the_identity_at_every_size() {
        for size in [4.0, 30.0, 300.0] {
            assert!(Smoothing::Exact.at(size).is_exact());
        }
        assert_eq!(Weight::EXACT.curve([1.0, 1.0, 1.0, 1.0]), IDENTITY);
        assert_eq!(apply(IDENTITY, 0.37), 0.37);
    }

    #[test]
    fn canvas2d_reproduces_the_fit() {
        let w = Smoothing::Canvas2d.at(30.0);
        assert!((w.expand_x - 0.169).abs() < 1e-9, "{w:?}");
        assert!((w.dy + 0.18).abs() < 1e-9, "{w:?}");
        // White paint at half coverage: the measured curve's 0.71, then the
        // size's exponent.
        let white = w.curve([1.0, 1.0, 1.0, 1.0]);
        let measured = 1.0 - (1.0 - 0.71f64).powf(1.135);
        assert!(
            (apply(white, 0.5) - measured).abs() < 0.02,
            "{}",
            apply(white, 0.5)
        );
        // Large text, then paths.
        assert_eq!(Smoothing::Canvas2d.at(200.0).expand_x, 0.425);
        assert!(Smoothing::Canvas2d.at(300.0).is_exact());
    }

    #[test]
    fn coretext_grows_with_size_to_its_limit() {
        assert!(Smoothing::CoreText.at(10.0).expand_x < Smoothing::CoreText.at(17.0).expand_x);
        assert_eq!(Smoothing::CoreText.at(40.0).expand_x, 0.34);
        assert_eq!(Smoothing::CoreText.at(40.0).dy, -0.30);
    }
}
