use pm_reference::compare::{Bounds, Class};

/// Each interior pixel keeps the original legacy bound or proves corrected
/// authored paint directly. A correction elsewhere in the frame must not
/// invalidate a pixel that already satisfies the unchanged legacy rule.
/// Edges, unreferenced pixels and geometry retain their bounds.
pub fn reference_problems(
    report: &pm_reference::compare::Report,
    bounds: &Bounds,
    engine: &[f64],
    today: &[f64],
    reference: &[f64],
    classes: &[Class],
) -> Vec<String> {
    let mut problems = report.failures(bounds);
    if !problems.iter().any(|p| p.starts_with("interior:")) {
        return problems;
    }
    assert_eq!(engine.len(), classes.len() * 4);
    assert_eq!(engine.len(), today.len());
    assert_eq!(engine.len(), reference.len());
    problems.retain(|p| !p.starts_with("interior:"));
    let error = |a: &[f64], b: &[f64]| {
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs() * 255.0)
            .fold(0.0_f64, f64::max)
    };
    let (mut corrected_error, mut corrected_excess) = (0.0_f64, 0.0_f64);
    for (i, class) in classes.iter().enumerate() {
        let (e, t, r) = (
            &engine[i * 4..][..4],
            &today[i * 4..][..4],
            &reference[i * 4..][..4],
        );
        if *class != Class::Interior
            || r.iter().any(|v| v.is_nan())
            || error(e, t) <= bounds.interior_cv + 1e-9
        {
            continue;
        }
        let er = error(e, r);
        corrected_error = corrected_error.max(er);
        corrected_excess = corrected_excess.max(er - error(t, r));
    }
    if corrected_error > 1.0 + 1e-9 {
        problems.push(format!(
            "interior reference: {corrected_error} CV (authored bound 1)"
        ));
    }
    if corrected_excess > bounds.precision_cv + 1e-9 {
        problems.push(format!(
            "interior reference: per-pixel excess {corrected_excess} CV (precision {})",
            bounds.precision_cv
        ));
    }
    problems
}

/// A full-frame text difference needs independent evidence that its glyph
/// raster passed, or that only the separately judged mask edge differs.
pub fn text_context_is_owned_gap(
    in_isolation: bool,
    paired_isolation: bool,
    bad_pixels: usize,
    outside_mask_band: usize,
) -> bool {
    !in_isolation && (paired_isolation || (bad_pixels > 0 && outside_mask_band == 0))
}
