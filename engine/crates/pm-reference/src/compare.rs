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
    /// Per pixel, the engine's error against the reference minus today's
    /// (negative where the engine is closer).
    pub excess: Stats,
}

/// The bounds a gate states (gate-policy.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// Interior pixels: largest difference from today (1 CV).
    pub interior_cv: f64,
    /// Edge and resampled pixels: how much each statistic of the engine's
    /// error may exceed today's, for the rounding of the compared frames.
    pub precision_cv: f64,
    /// The gate's previous per-pixel bound, as (p99.9, max): unreferenced
    /// pixels against today, and each edge or resampled pixel's error
    /// beyond today's (which `|engine − today|` within it implies).
    pub per_pixel_cv: (f64, f64),
}

impl Bounds {
    /// 8-bit frames: half a code of rounding in each; per pixel, section
    /// 25's Standard class (≤ 1 CV for 99.9%, ≤ 4 for any).
    pub const EIGHT_BIT: Bounds = Bounds {
        interior_cv: 1.0,
        precision_cv: 0.5,
        per_pixel_cv: (1.0, 4.0),
    };
    /// Half-float frames (transparent captures): one f16 rounding step
    /// below 1.0 (2⁻¹¹ = 0.125 CV); per pixel, the alpha class (≤ 1 CV for
    /// any).
    pub const HALF_FLOAT: Bounds = Bounds {
        interior_cv: 1.0,
        precision_cv: 255.0 / 2048.0,
        per_pixel_cv: (1.0, 1.0),
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
    ///   the reference each at most today's plus `precision_cv`; and per
    ///   pixel, the engine's error beyond today's within `per_pixel_cv`, so
    ///   no pixel can regress under today's worst error elsewhere.
    /// - Unreferenced: `per_pixel_cv` against today's capture.
    pub fn failures(&self, bounds: &Bounds) -> Vec<String> {
        let (p999, max) = bounds.per_pixel_cv;
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
            let x = &r.excess;
            if x.pixels > 0 && (x.p999 > p999 + 1e-9 || x.max > max + 1e-9) {
                out.push(format!(
                    "{class:?}: per pixel, the engine is further from the reference than today by {x} (bound p99.9 {p999}, max {max})"
                ));
            }
        }
        let u = &self.class(Class::Unreferenced).engine_vs_today;
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
                    " | vs reference: engine {} / today {} | per-pixel excess max {:.2} p99.9 {:.2}",
                    r.engine_vs_reference, r.today_vs_reference, r.excess.max, r.excess.p999
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
    let mut errors: [[Vec<f64>; 4]; 4] = Default::default();
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
            let (er, tr) = (pixel_error(&e, &r), pixel_error(&t, &r));
            errors[k][0].push(er);
            errors[k][1].push(tr);
            errors[k][3].push(er - tr);
        }
    }
    Report {
        classes: errors.map(|[e, t, et, x]| ClassReport {
            engine_vs_reference: Stats::of(e),
            today_vs_reference: Stats::of(t),
            engine_vs_today: Stats::of(et),
            excess: Stats::of(x),
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
        // Swapped: today is closer, so the engine fails the edge rule (all
        // three statistics, and per pixel: 25.5 CV beyond today's).
        let r = compare(&today, &engine, &reference, &classes);
        assert_eq!(r.failures(&Bounds::EIGHT_BIT).len(), 4);
        // NaN references are judged against today.
        let r = compare(&engine, &today, &[f64::NAN; 8], &classes);
        assert_eq!(r.class(Class::Unreferenced).engine_vs_today.pixels, 2);
    }

    /// The review's case (B1): an edge band shaped like a level 1 solid's,
    /// where today's aliased edges are ~85 CV off the reference, and the
    /// engine regresses 12 exact pixels to 80 CV. The class statistics stay
    /// under today's; the per-pixel rule catches it.
    #[test]
    fn sparse_edge_regressions_fail() {
        let n = 2175;
        let bg = [0.1, 0.1, 0.1, 1.0];
        let fg = [0.1 + 170.0 / 255.0, 0.1, 0.1, 1.0];
        let mut reference = Vec::new();
        let mut today = Vec::new();
        for i in 0..n {
            // 400 partial pixels: exact coverage k, today all or nothing.
            let k = if i < 400 { (i % 20) as f64 / 20.0 + 0.025 } else { (i % 2) as f64 };
            let mix = |a: f64| -> [f64; 4] { std::array::from_fn(|c| bg[c] + (fg[c] - bg[c]) * a) };
            reference.extend(mix(k));
            today.extend(mix(k.round()));
        }
        let mut engine = today.clone();
        for i in (400..n).step_by(100).take(12) {
            engine[i * 4] += 80.0 / 255.0;
        }
        let classes = vec![Class::Edge; n];
        let r = *compare(&engine, &today, &reference, &classes).class(Class::Edge);
        let (e, t) = (r.engine_vs_reference, r.today_vs_reference);
        assert!(e.max <= t.max && e.p999 <= t.p999 + 0.5 && e.mean <= t.mean + 0.5);
        let report = compare(&engine, &today, &reference, &classes);
        let failures = report.failures(&Bounds::EIGHT_BIT);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].contains("per pixel"));
        // Improvements pass: the engine at the reference everywhere.
        assert!(compare(&reference, &today, &reference, &classes).failures(&Bounds::EIGHT_BIT).is_empty());
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
