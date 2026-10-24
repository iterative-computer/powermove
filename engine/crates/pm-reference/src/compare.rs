//! The gate comparison (docs/engine/gate-policy.md): the engine's frame and
//! today's capture, each against the reference, per pixel class.
//!
//! Frames are RGBA in 0…1 (8-bit captures divided by 255); errors are
//! reported in 8-bit code values (CV), per pixel as the largest channel
//! difference. A reference value that is NaN marks a pixel the reference
//! does not model.

/// What a pixel is judged by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// At least `radius` pixels from any edge, no resampling: authored
    /// colors, blends and mattes. Within the interior bound of today.
    Interior,
    /// Near an edge (partial coverage): at least as close to the
    /// reference as today.
    Edge,
    /// Drawn from a resampled source: at least as close to the reference
    /// as today.
    Resampled,
    /// No reference here (content no lane models yet): the gate's previous
    /// bound against today's capture.
    Unreferenced,
}

const CLASSES: [Class; 4] = [Class::Interior, Class::Edge, Class::Resampled, Class::Unreferenced];

/// Error statistics over one class, in CV.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub pixels: usize,
    pub max: f64,
    /// The smallest error at least 99.9% of the pixels are within.
    pub p999: f64,
    pub mean: f64,
}

impl Stats {
    pub fn of(mut errors: Vec<f64>) -> Stats {
        if errors.is_empty() {
            return Stats::default();
        }
        errors.sort_by(f64::total_cmp);
        let n = errors.len();
        Stats {
            pixels: n,
            max: errors[n - 1],
            p999: errors[(0.999 * n as f64).ceil() as usize - 1],
            mean: errors.iter().sum::<f64>() / n as f64,
        }
    }
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "max {:.2} p99.9 {:.2} mean {:.3}",
            self.max, self.p999, self.mean
        )
    }
}

/// One class of a [`Report`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClassReport {
    pub engine_vs_reference: Stats,
    pub today_vs_reference: Stats,
    pub engine_vs_today: Stats,
}

/// The bounds a gate states (gate-policy.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// Interior pixels: largest difference from today (1 CV).
    pub interior_cv: f64,
    /// Edge and resampled pixels: how much each statistic of the engine's
    /// error may exceed today's, for the rounding of the compared frames.
    pub precision_cv: f64,
    /// Unreferenced pixels: the gate's previous bound against today, as
    /// (p99.9, max).
    pub unreferenced_cv: (f64, f64),
}

impl Bounds {
    /// 8-bit frames: half a code of rounding in each; unreferenced pixels
    /// keep section 25's Standard class (≤ 1 CV for 99.9%, ≤ 4 for any).
    pub const EIGHT_BIT: Bounds = Bounds {
        interior_cv: 1.0,
        precision_cv: 0.5,
        unreferenced_cv: (1.0, 4.0),
    };
    /// Half-float frames (transparent captures): one f16 rounding step
    /// below 1.0 (2⁻¹¹ = 0.125 CV); unreferenced pixels keep the alpha
    /// class (≤ 1 CV for any).
    pub const HALF_FLOAT: Bounds = Bounds {
        interior_cv: 1.0,
        precision_cv: 255.0 / 2048.0,
        unreferenced_cv: (1.0, 1.0),
    };
}

/// Per-class statistics of a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub classes: [ClassReport; 4],
}

impl Report {
    pub fn class(&self, class: Class) -> &ClassReport {
        &self.classes[CLASSES.iter().position(|c| *c == class).unwrap()]
    }

    /// Why the frame fails the gate; empty when it passes.
    ///
    /// - Interior: every pixel within `interior_cv` of today.
    /// - Edge and resampled: the engine's max, p99.9 and mean error against
    ///   the reference each at most today's plus `precision_cv`.
    /// - Unreferenced: the gate's previous bound against today's capture.
    pub fn failures(&self, bounds: &Bounds) -> Vec<String> {
        let mut out = Vec::new();
        let interior = &self.class(Class::Interior).engine_vs_today;
        if interior.max > bounds.interior_cv + 1e-9 {
            out.push(format!(
                "interior: {:.2} CV from today (bound {})",
                interior.max, bounds.interior_cv
            ));
        }
        for class in [Class::Edge, Class::Resampled] {
            let r = self.class(class);
            let (e, t) = (&r.engine_vs_reference, &r.today_vs_reference);
            for (name, ev, tv) in [
                ("max", e.max, t.max),
                ("p99.9", e.p999, t.p999),
                ("mean", e.mean, t.mean),
            ] {
                if ev > tv + bounds.precision_cv + 1e-9 {
                    out.push(format!(
                        "{class:?}: engine {name} {ev:.3} CV from the reference, today {tv:.3}"
                    ));
                }
            }
        }
        let u = &self.class(Class::Unreferenced).engine_vs_today;
        let (p999, max) = bounds.unreferenced_cv;
        if u.p999 > p999 + 1e-9 || u.max > max + 1e-9 {
            out.push(format!("unreferenced: {u} from today (bound p99.9 {p999}, max {max})"));
        }
        out
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (class, r) in CLASSES.iter().zip(&self.classes) {
            if r.engine_vs_today.pixels == 0 {
                continue;
            }
            write!(f, "\n    {:<12} {:>8} px  vs today {}", format!("{class:?}"), r.engine_vs_today.pixels, r.engine_vs_today)?;
            if *class != Class::Unreferenced {
                write!(
                    f,
                    " | vs reference: engine {} / today {}",
                    r.engine_vs_reference, r.today_vs_reference
                )?;
            }
        }
        Ok(())
    }
}

fn pixel_error(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs() * 255.0)
        .fold(0.0, f64::max)
}

/// Compares RGBA frames of equal size: `engine` and `today` against
/// `reference` (NaN: not modelled) per `classes`. A pixel whose reference
/// is NaN counts as [`Class::Unreferenced`] whatever its class says.
pub fn compare(engine: &[f64], today: &[f64], reference: &[f64], classes: &[Class]) -> Report {
    assert!(engine.len() == today.len() && today.len() == reference.len());
    assert_eq!(engine.len(), classes.len() * 4);
    let mut errors: [[Vec<f64>; 3]; 4] = Default::default();
    for (i, class) in classes.iter().enumerate() {
        let px = |f: &[f64]| -> [f64; 4] { std::array::from_fn(|c| f[i * 4 + c]) };
        let (e, t, r) = (px(engine), px(today), px(reference));
        let class = if r.iter().any(|v| v.is_nan()) {
            Class::Unreferenced
        } else {
            *class
        };
        let k = CLASSES.iter().position(|c| *c == class).unwrap();
        errors[k][2].push(pixel_error(&e, &t));
        if class != Class::Unreferenced {
            errors[k][0].push(pixel_error(&e, &r));
            errors[k][1].push(pixel_error(&t, &r));
        }
    }
    Report {
        classes: errors.map(|[e, t, et]| ClassReport {
            engine_vs_reference: Stats::of(e),
            today_vs_reference: Stats::of(t),
            engine_vs_today: Stats::of(et),
        }),
    }
}

/// Pixel classes from per-pixel facts: `edge` (some modelled content is
/// partially covered there) and `resampled`, each grown by `radius` pixels
/// (Chebyshev). Resampled wins over edge, edge over interior.
pub fn classify(edge: &[bool], resampled: &[bool], width: usize, height: usize, radius: usize) -> Vec<Class> {
    let (edge, resampled) = (dilate(edge, width, height, radius), dilate(resampled, width, height, radius));
    edge.iter()
        .zip(&resampled)
        .map(|(e, r)| match (e, r) {
            (_, true) => Class::Resampled,
            (true, false) => Class::Edge,
            _ => Class::Interior,
        })
        .collect()
}

/// `mask` grown by `radius` pixels in every direction.
pub fn dilate(mask: &[bool], width: usize, height: usize, radius: usize) -> Vec<bool> {
    let grow = |src: &[bool], horizontal: bool| -> Vec<bool> {
        let mut out = vec![false; src.len()];
        for y in 0..height {
            for x in 0..width {
                if !src[y * width + x] {
                    continue;
                }
                let (lo, hi, at): (usize, usize, &dyn Fn(usize) -> usize) = if horizontal {
                    (x.saturating_sub(radius), (x + radius).min(width - 1), &|i| y * width + i)
                } else {
                    (y.saturating_sub(radius), (y + radius).min(height - 1), &|i| i * width + x)
                };
                for i in lo..=hi {
                    out[at(i)] = true;
                }
            }
        }
        out
    };
    grow(&grow(mask, true), false)
}

/// The area (px²) and centroid (px, pixel centers at `x + ½`) of an alpha
/// image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub area: f64,
    pub centroid: [f64; 2],
}

pub fn geometry(alpha: &[f64], width: usize) -> Geometry {
    let (mut area, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for (i, a) in alpha.iter().enumerate() {
        area += a;
        sx += a * ((i % width) as f64 + 0.5);
        sy += a * ((i / width) as f64 + 0.5);
    }
    Geometry {
        area,
        centroid: if area > 0.0 { [sx / area, sy / area] } else { [0.0; 2] },
    }
}

/// Authored geometry kept exactly: area within `tolerance` relative and
/// centroid within `tolerance` px (the gates use 1e-3).
pub fn same_geometry(engine: &Geometry, reference: &Geometry, tolerance: f64) -> Result<(), String> {
    let da = (engine.area - reference.area).abs() / reference.area.max(1.0);
    let dc = (engine.centroid[0] - reference.centroid[0])
        .abs()
        .max((engine.centroid[1] - reference.centroid[1]).abs());
    if da > tolerance || dc > tolerance {
        return Err(format!(
            "geometry: area {:.4} vs {:.4} ({da:.2e} relative), centroid off by {dc:.2e} px",
            engine.area, reference.area
        ));
    }
    Ok(())
}

/// Preview == export: the two frames are identical, value for value.
pub fn identical(preview: &[f32], export: &[f32]) -> Result<(), String> {
    if preview.len() != export.len() {
        return Err(format!("sizes differ: {} vs {}", preview.len(), export.len()));
    }
    let (n, max) = preview
        .iter()
        .zip(export)
        .filter(|(a, b)| a.to_bits() != b.to_bits())
        .fold((0usize, 0.0f32), |(n, m), (a, b)| (n + 1, m.max((a - b).abs())));
    if n > 0 {
        return Err(format!("preview differs from export in {n} values (max {max:e})"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_verdicts() {
        let s = Stats::of((0..1000).map(f64::from).collect());
        assert_eq!((s.max, s.p999, s.pixels), (999.0, 998.0, 1000));
        // Engine closer than today on the edge, equal on the interior.
        let today = [0.5, 0.5, 0.5, 1.0, 0.2, 0.2, 0.2, 1.0];
        let engine = [0.5, 0.5, 0.5, 1.0, 0.3, 0.3, 0.3, 1.0];
        let reference = [0.5, 0.5, 0.5, 1.0, 0.31, 0.31, 0.31, 1.0];
        let classes = [Class::Interior, Class::Edge];
        let r = compare(&engine, &today, &reference, &classes);
        assert!(r.failures(&Bounds::EIGHT_BIT).is_empty(), "{:?}", r.failures(&Bounds::EIGHT_BIT));
        // Swapped: today is closer, so the engine fails the edge rule.
        let r = compare(&today, &engine, &reference, &classes);
        assert_eq!(r.failures(&Bounds::EIGHT_BIT).len(), 3);
        // NaN references are judged against today.
        let r = compare(&engine, &today, &[f64::NAN; 8], &classes);
        assert_eq!(r.class(Class::Unreferenced).engine_vs_today.pixels, 2);
    }

    #[test]
    fn classes_grow_from_edges() {
        let mut edge = vec![false; 25];
        edge[12] = true;
        let c = classify(&edge, &[false; 25], 5, 5, 1);
        assert_eq!(c.iter().filter(|c| **c == Class::Edge).count(), 9);
        assert_eq!(c[0], Class::Interior);
    }

    #[test]
    fn geometry_and_identity() {
        let g = geometry(&[0.0, 1.0, 1.0, 0.0], 2);
        assert_eq!(g, Geometry { area: 2.0, centroid: [1.0, 1.0] });
        assert!(same_geometry(&g, &g, 1e-3).is_ok());
        assert!(identical(&[1.0, 2.0], &[1.0, 2.0]).is_ok());
        assert!(identical(&[1.0, 2.0], &[1.0, 2.5]).is_err());
    }
}
