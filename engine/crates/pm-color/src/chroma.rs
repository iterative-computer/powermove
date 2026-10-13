//! Chroma siting, upsampling, decimation, and pass-through (section 9,
//! "Chroma").
//!
//! Upsampling is siting-aware and keeps co-sited samples exact: horizontal
//! linear for 4:2:2 (and 4:1:1); for 4:2:0, Catmull-Rom horizontally and
//! linear between rows vertically. A luma pixel that lies exactly on a
//! chroma sample position gets that sample unchanged (every filter's weight
//! vector is a unit vector there). The encoder-input side decimates by point
//! sampling at co-sited positions and averages two samples where the output
//! siting is centered.
//!
//! The functions here are the CPU reference; `pm-render`'s source pass
//! evaluates the same weights in WGSL and its tests compare the two.

use pm_protocol::color::{AlphaMode, ChromaSiting, ColorDescription, Subsampling};

/// Chroma subsampling factors (luma pixels per chroma sample) from the
/// log2 factors a decoded frame reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Factors {
    pub x: u32,
    pub y: u32,
}

impl Factors {
    pub fn from_log2(log2_w: u8, log2_h: u8) -> Factors {
        Factors {
            x: 1 << log2_w,
            y: 1 << log2_h,
        }
    }

    pub fn of(subsampling: Subsampling) -> Factors {
        match subsampling {
            Subsampling::S444 => Factors { x: 1, y: 1 },
            Subsampling::S422 => Factors { x: 2, y: 1 },
            Subsampling::S420 => Factors { x: 2, y: 2 },
        }
    }

    /// Chroma plane size for a luma size (rounded up, as decoders allocate).
    pub fn chroma_size(&self, width: u32, height: u32) -> (u32, u32) {
        (width.div_ceil(self.x), height.div_ceil(self.y))
    }
}

/// Where chroma sample `(0, 0)` sits on the luma grid, in luma pixels from
/// luma sample `(0, 0)`.
pub fn sample_origin(siting: ChromaSiting, factors: Factors) -> (f64, f64) {
    let centered_x = f64::from(factors.x - 1) / 2.0;
    let x = if siting.horizontally_cosited() {
        0.0
    } else {
        centered_x
    };
    let y = if factors.y == 1 {
        0.0
    } else {
        match siting {
            ChromaSiting::TopLeft | ChromaSiting::Top => 0.0,
            ChromaSiting::BottomLeft | ChromaSiting::Bottom => f64::from(factors.y - 1),
            ChromaSiting::Left | ChromaSiting::Center => f64::from(factors.y - 1) / 2.0,
        }
    };
    (x, y)
}

/// The filter used along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// No subsampling along this axis.
    None,
    /// Two-tap linear.
    Linear,
    /// Four-tap Catmull-Rom (cubic, interpolating).
    CatmullRom,
}

/// The upsampling filters of section 9 for a subsampling: (horizontal,
/// vertical).
pub fn filters(factors: Factors) -> (Filter, Filter) {
    let vertical = if factors.y > 1 {
        Filter::Linear
    } else {
        Filter::None
    };
    let horizontal = match (factors.x > 1, factors.y > 1) {
        (false, _) => Filter::None,
        (true, true) => Filter::CatmullRom,
        (true, false) => Filter::Linear,
    };
    (horizontal, vertical)
}

/// Catmull-Rom weights for taps at −1, 0, +1, +2 around a fraction `t` in
/// 0..1. At `t = 0` the weights are exactly `[0, 1, 0, 0]`.
pub fn catmull_rom_weights(t: f64) -> [f64; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        (-t3 + 2.0 * t2 - t) * 0.5,
        (3.0 * t3 - 5.0 * t2 + 2.0) * 0.5,
        (-3.0 * t3 + 4.0 * t2 + t) * 0.5,
        (t3 - t2) * 0.5,
    ]
}

/// The chroma-plane coordinate (in chroma samples) of luma pixel `p` along
/// one axis.
pub fn chroma_coordinate(p: u32, origin: f64, factor: u32) -> f64 {
    (f64::from(p) - origin) / f64::from(factor)
}

/// Taps (chroma index, weight) for one axis at luma pixel `p`; indices are
/// clamped to the plane (edge samples repeat).
pub fn taps(p: u32, origin: f64, factor: u32, size: u32, filter: Filter) -> Vec<(u32, f64)> {
    let clamp = |i: i64| i.clamp(0, i64::from(size) - 1) as u32;
    match filter {
        Filter::None => vec![(p.min(size - 1), 1.0)],
        Filter::Linear => {
            let u = chroma_coordinate(p, origin, factor);
            let i = u.floor();
            let t = u - i;
            let i = i as i64;
            vec![(clamp(i), 1.0 - t), (clamp(i + 1), t)]
        }
        Filter::CatmullRom => {
            let u = chroma_coordinate(p, origin, factor);
            let i = u.floor();
            let w = catmull_rom_weights(u - i);
            let i = i as i64;
            (0..4).map(|k| (clamp(i - 1 + k as i64), w[k])).collect()
        }
    }
}

/// A chroma plane (one channel) at chroma resolution.
pub struct Plane<'a> {
    pub width: u32,
    pub height: u32,
    pub samples: &'a [f32],
}

/// Upsamples one chroma channel to luma resolution (CPU reference, f64
/// accumulation rounded to f32).
pub fn upsample(
    plane: &Plane<'_>,
    luma_width: u32,
    luma_height: u32,
    factors: Factors,
    siting: ChromaSiting,
) -> Vec<f32> {
    let (ox, oy) = sample_origin(siting, factors);
    let (fh, fv) = filters(factors);
    let mut out = Vec::with_capacity((luma_width * luma_height) as usize);
    for y in 0..luma_height {
        let ty = taps(y, oy, factors.y, plane.height, fv);
        for x in 0..luma_width {
            let tx = taps(x, ox, factors.x, plane.width, fh);
            let mut acc = 0.0f64;
            for &(j, wy) in &ty {
                for &(i, wx) in &tx {
                    if wx == 0.0 || wy == 0.0 {
                        continue;
                    }
                    acc += wx * wy * f64::from(plane.samples[(j * plane.width + i) as usize]);
                }
            }
            out.push(acc as f32);
        }
    }
    out
}

/// The declared 4:2:0 vertical decimation filter (section 9: "with a
/// declared filter for 4:2:0"): taps on luma rows `2y − 1, 2y, 2y + 1,
/// 2y + 2` for a chroma row centered between rows `2y` and `2y + 1`.
///
/// It is the exact inverse of the linear vertical upsampler: upsampled rows
/// are `r(2y) = ¼ c(y−1) + ¾ c(y)` and `r(2y+1) = ¾ c(y) + ¼ c(y+1)`, and
/// `(3 r(2y) − r(2y−1) + 3 r(2y+1) − r(2y+2)) / 4 = c(y)`, also at the
/// frame edges with clamped rows. A 4:2:0 source exported to 4:2:0
/// therefore keeps its chroma (section 2, invariant (1)); a two-row average
/// would blur it by `[1, 6, 1] / 8` on every round trip.
pub const VERTICAL_420: [(i64, f64); 4] = [(-1, -0.25), (0, 0.75), (1, 0.75), (2, -0.25)];

/// Decimates a luma-resolution channel to chroma resolution for an output
/// with `siting` (encoder input): point sampling at co-sited positions;
/// where a 2× output sample is centered between two rows or columns, the
/// declared inverse-linear filter ([`VERTICAL_420`]); other factors average
/// the two neighbours.
pub fn downsample(
    full: &[f32],
    width: u32,
    height: u32,
    factors: Factors,
    siting: ChromaSiting,
) -> Vec<f32> {
    let (cw, ch) = factors.chroma_size(width, height);
    let (ox, oy) = sample_origin(siting, factors);
    let axis = |c: u32, origin: f64, factor: u32, size: u32| -> Vec<(u32, f64)> {
        if factor == 1 {
            return vec![(c, 1.0)];
        }
        let p = f64::from(c * factor) + origin;
        let lo = p.floor();
        let clamp = |v: f64| (v as i64).clamp(0, i64::from(size) - 1) as u32;
        if p == lo {
            vec![(clamp(lo), 1.0)]
        } else if factor == 2 && p - lo == 0.5 {
            VERTICAL_420
                .iter()
                .map(|&(k, w)| (clamp(lo + k as f64), w))
                .collect()
        } else {
            let t = p - lo;
            vec![(clamp(lo), 1.0 - t), (clamp(lo + 1.0), t)]
        }
    };
    let mut out = Vec::with_capacity((cw * ch) as usize);
    for cy in 0..ch {
        let ty = axis(cy, oy, factors.y, height);
        for cx in 0..cw {
            let tx = axis(cx, ox, factors.x, width);
            let mut acc = 0.0f64;
            for &(y, wy) in &ty {
                for &(x, wx) in &tx {
                    acc += wx * wy * f64::from(full[(y * width + x) as usize]);
                }
            }
            out.push(acc as f32);
        }
    }
    out
}

/// The composition-level facts pass-through needs (section 9, "Pass-through").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFacts {
    /// The output frame is exactly one source with nothing else drawn and no
    /// background showing.
    pub single_source: bool,
    /// The source is placed 1:1 at the output's origin and size: no
    /// transform, crop, or resampling.
    pub untransformed: bool,
    /// No effects, masks, mattes, transitions, or opacity below 1.
    pub effect_free: bool,
}

/// Whether the engine copies a source's Y′CbCr planes into the output
/// unchanged: a single opaque, untransformed, effect-free source whose
/// format, range, and subsampling match the output.
pub fn pass_through(
    source: &ColorDescription,
    output: &ColorDescription,
    facts: FrameFacts,
) -> bool {
    let opaque = matches!(source.alpha, AlphaMode::Opaque);
    let output_opaque = matches!(output.alpha, AlphaMode::Opaque);
    let sited = source.subsampling == Subsampling::S444 || source.siting == output.siting;
    facts.single_source
        && facts.untransformed
        && facts.effect_free
        && opaque
        && output_opaque
        && source.primaries == output.primaries
        && source.transfer == output.transfer
        && source.matrix == output.matrix
        && source.range == output.range
        && source.subsampling == output.subsampling
        && source.bit_depth == output.bit_depth
        && sited
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::color::{MatrixCoefficients, Primaries, Range, Transfer};

    fn ramp(w: u32, h: u32) -> Vec<f32> {
        (0..w * h)
            .map(|i| ((i * 37 % 101) as f32) / 100.0 - 0.3)
            .collect()
    }

    #[test]
    fn origins_follow_h273() {
        let f420 = Factors::of(Subsampling::S420);
        assert_eq!(sample_origin(ChromaSiting::Left, f420), (0.0, 0.5));
        assert_eq!(sample_origin(ChromaSiting::Center, f420), (0.5, 0.5));
        assert_eq!(sample_origin(ChromaSiting::TopLeft, f420), (0.0, 0.0));
        assert_eq!(sample_origin(ChromaSiting::Top, f420), (0.5, 0.0));
        assert_eq!(sample_origin(ChromaSiting::BottomLeft, f420), (0.0, 1.0));
        assert_eq!(sample_origin(ChromaSiting::Bottom, f420), (0.5, 1.0));
        let f422 = Factors::of(Subsampling::S422);
        assert_eq!(sample_origin(ChromaSiting::Left, f422), (0.0, 0.0));
        let f411 = Factors::from_log2(2, 0);
        assert_eq!(sample_origin(ChromaSiting::Center, f411), (1.5, 0.0));
    }

    #[test]
    fn cosited_422_samples_are_exact() {
        let (cw, ch) = (17, 9);
        let samples = ramp(cw, ch);
        let plane = Plane {
            width: cw,
            height: ch,
            samples: &samples,
        };
        let f = Factors::of(Subsampling::S422);
        let up = upsample(&plane, 2 * cw, ch, f, ChromaSiting::Left);
        for y in 0..ch {
            for i in 0..cw {
                assert_eq!(
                    up[(y * 2 * cw + 2 * i) as usize],
                    samples[(y * cw + i) as usize]
                );
            }
            // Between samples: the average (linear).
            for i in 0..cw - 1 {
                let a = f64::from(samples[(y * cw + i) as usize]);
                let b = f64::from(samples[(y * cw + i + 1) as usize]);
                let v = f64::from(up[(y * 2 * cw + 2 * i + 1) as usize]);
                assert!((v - (a + b) / 2.0).abs() < 1e-6);
            }
        }
        // Point decimation at the co-sited columns recovers the plane.
        let down = downsample(&up, 2 * cw, ch, f, ChromaSiting::Left);
        assert_eq!(down, samples);
    }

    #[test]
    fn cosited_420_columns_are_exact_on_cosited_rows() {
        let (cw, ch) = (11, 7);
        let samples = ramp(cw, ch);
        let plane = Plane {
            width: cw,
            height: ch,
            samples: &samples,
        };
        let f = Factors::of(Subsampling::S420);
        let up = upsample(&plane, 2 * cw, 2 * ch, f, ChromaSiting::TopLeft);
        for j in 0..ch {
            for i in 0..cw {
                assert_eq!(
                    up[(2 * j * 2 * cw + 2 * i) as usize],
                    samples[(j * cw + i) as usize]
                );
            }
        }
    }

    #[test]
    fn declared_420_filter_inverts_linear_upsampling() {
        // Left siting (type 0): co-sited horizontally, centered vertically.
        let (cw, ch) = (5u32, 6u32);
        let samples: Vec<f32> = (0..cw * ch)
            .map(|i| ((i * 37 % 101) as f32) / 101.0 - 0.2)
            .collect();
        let plane = Plane {
            width: cw,
            height: ch,
            samples: &samples,
        };
        let f = Factors { x: 2, y: 2 };
        let full = upsample(&plane, cw * 2, ch * 2, f, ChromaSiting::Left);
        let back = downsample(&full, cw * 2, ch * 2, f, ChromaSiting::Left);
        for (a, b) in back.iter().zip(&samples) {
            assert!((a - b).abs() < 1e-6, "{a} against {b}");
        }
        let weights: f64 = VERTICAL_420.iter().map(|t| t.1).sum();
        assert_eq!(weights, 1.0);
    }

    #[test]
    fn upsampling_constant_planes_is_exact() {
        for siting in [
            ChromaSiting::Left,
            ChromaSiting::Center,
            ChromaSiting::TopLeft,
            ChromaSiting::Bottom,
        ] {
            for s in [Subsampling::S420, Subsampling::S422, Subsampling::S444] {
                let f = Factors::of(s);
                let (w, h) = (13, 9);
                let (cw, ch) = f.chroma_size(w, h);
                let samples = vec![0.3125f32; (cw * ch) as usize];
                let plane = Plane {
                    width: cw,
                    height: ch,
                    samples: &samples,
                };
                assert!(
                    upsample(&plane, w, h, f, siting)
                        .iter()
                        .all(|&v| v == 0.3125)
                );
            }
        }
    }

    #[test]
    fn catmull_rom_weights_sum_to_one() {
        for k in 0..=64 {
            let t = f64::from(k) / 64.0;
            let w = catmull_rom_weights(t);
            assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-15);
        }
        assert_eq!(catmull_rom_weights(0.0), [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn pass_through_needs_matching_formats_and_a_plain_frame() {
        let source = ColorDescription {
            primaries: Primaries::Bt709,
            transfer: Transfer::DisplayE,
            matrix: MatrixCoefficients::Bt709,
            range: Range::Limited,
            subsampling: Subsampling::S422,
            siting: ChromaSiting::Left,
            alpha: AlphaMode::Opaque,
            bit_depth: 10,
        };
        let facts = FrameFacts {
            single_source: true,
            untransformed: true,
            effect_free: true,
        };
        assert!(pass_through(&source, &source, facts));
        assert!(!pass_through(
            &source,
            &ColorDescription {
                range: Range::Full,
                ..source
            },
            facts
        ));
        assert!(!pass_through(
            &source,
            &ColorDescription {
                subsampling: Subsampling::S420,
                ..source
            },
            facts
        ));
        assert!(!pass_through(
            &ColorDescription {
                alpha: AlphaMode::Straight,
                ..source
            },
            &source,
            facts
        ));
        assert!(!pass_through(
            &source,
            &source,
            FrameFacts {
                untransformed: false,
                ..facts
            }
        ));
    }
}
