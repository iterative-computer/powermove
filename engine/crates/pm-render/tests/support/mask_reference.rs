//! Reference mask coverage of a layer (docs/engine/gate-policy.md; G-V2 in
//! `tests/path_masks.rs` uses the same math).
//!
//! Authored values come from pm-eval through the engine's mask reading
//! (`composite::build::masks`: which masks are enabled, their values, the
//! route); the outlines are pm-vector's mask geometry (authored, V-K1).
//! Coverage and the feather are pm-reference's: `coverage::fill` with 16 × 16
//! cells, `gaussian::blur` (transparent edges) at the authored
//! σ = max(0, feather) · |world x| / 2, composited `source-over` (add) or
//! `destination-out` (subtract) from 1 when no mask adds. Parametric masks
//! (no path) are their authored formula (`FRAG_MASK`: a smoothstep across
//! the signed distance, max for add, min of the complement for subtract) at
//! each pixel center, in f64.

use kurbo::{Affine, Point};
use pm_reference::compare::dilate;
use pm_reference::coverage::{FillRule, fill};
use pm_reference::gaussian::{Edge, blur};
use pm_render::composite::build::{MaskRoute, masks};
use pm_vector::{MaskMode, MaskShape, ParametricMasks, PathMasks};

/// Per-pixel coverage and the edge band used to classify it.
pub type MaskCoverage = (Vec<f64>, Vec<bool>);

/// A layer's mask coverage per output pixel (`w × h`, top-down) and its band
/// (reference coverage strictly between 0 and 1, grown by 1 px), or `None`
/// when the layer draws unmasked. `device` is the layer's `scaledWorld`:
/// layer pixels (before the size anchor's offset) to output pixels, 2D.
/// `Err` for what this does not model: a 3D layer's inverse plane.
pub fn layer_mask<'d>(
    scope: &mut pm_eval::Scope<'_, 'd>,
    layer: &'d pm_model::Node,
    seconds: f64,
    device: Affine,
    w: usize,
    h: usize,
) -> Result<Option<MaskCoverage>, String> {
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    if !pm_render::composite::build::has_masks(scope, layer, seconds).map_err(fail)? {
        return Ok(None);
    }
    if scope.is_3d_layer(layer).map_err(fail)? {
        return Err("masks of a 3D layer".into());
    }
    let world = device.as_coeffs();
    let coverage = match masks(scope, layer, seconds, world, None, w as u32, h as u32).map_err(fail)? {
        MaskRoute::Unmasked => return Ok(None),
        MaskRoute::Path(m) => {
            let path = m.path.ok_or("the path route without its masks")?;
            path_coverage(&path.masks, world, w, h)
        }
        MaskRoute::Parametric(m) => parametric(&m.masks, device, w, h),
    };
    let partial: Vec<bool> = coverage.iter().map(|v| *v > 1e-9 && *v < 1.0 - 1e-9).collect();
    let band = dilate(&partial, w, h, 1);
    Ok(Some((coverage, band)))
}

/// The bands of the composition's top-level 2D layers with masks (see
/// [`layer_mask`]), active at `time`, at the legacy derived scale.
pub fn mask_bands(
    document: &pm_model::Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    w: usize,
    h: usize,
) -> Result<Vec<bool>, String> {
    let comp = pm_eval::Comp::resolve(document, composition).map_err(|e| format!("{e:?}"))?;
    let seconds = comp.seconds_at(time.get());
    let (sx, sy) = pm_render::plan::derived_scale(&comp, w as u32, h as u32);
    let mut ev = pm_eval::Evaluator::new();
    let mut scope = pm_eval::Scope::new(&mut ev, &comp, &comp);
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let mut bands = vec![false; w * h];
    for layer in comp.layers() {
        if !scope.active(layer, seconds).map_err(fail)? || scope.is_3d_layer(layer).map_err(fail)? {
            continue;
        }
        let m = scope.world_matrix(layer, seconds).map_err(fail)?;
        let device = Affine::new([m[0] * sx, m[1] * sy, m[2] * sx, m[3] * sy, m[4] * sx, m[5] * sy]);
        if let Some((_, band)) = layer_mask(&mut scope, layer, seconds, device, w, h)? {
            bands.iter_mut().zip(band).for_each(|(b, l)| *b |= l);
        }
    }
    Ok(bands)
}

/// The path route's coverage canvas: every mask in order.
pub fn path_coverage(masks: &PathMasks, world: [f64; 6], width: usize, height: usize) -> Vec<f64> {
    let mut canvas = vec![if masks.start_covered { 1.0 } else { 0.0 }; width * height];
    for m in &masks.masks {
        let outline = m.transform_from(world).affine() * &m.path;
        let cov = fill(&outline, FillRule::NonZero, 16, width, height);
        let sigma = m.feather * world[0].hypot(world[1]) / 2.0;
        let sigma = if sigma.is_finite() { sigma } else { 0.0 };
        // The blur of the coverage's window grown by the kernel's 8σ reach
        // (exact: everything outside the window is 0).
        let reach = (8.0 * sigma).ceil() as usize;
        let x0 = cov.x0.saturating_sub(reach);
        let y0 = cov.y0.saturating_sub(reach);
        let x1 = (cov.x0 + cov.width + reach).min(width);
        let y1 = (cov.y0 + cov.height + reach).min(height);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut window = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                window[y * w + x] = cov.at(x0 + x, y0 + y);
            }
        }
        let window = if sigma > 0.0 {
            blur(&window, w, h, 1, (sigma, sigma), Edge::Transparent)
        } else {
            window
        };
        for y in 0..h {
            for x in 0..w {
                let (v, a) = (window[y * w + x], &mut canvas[(y0 + y) * width + x0 + x]);
                *a = match m.mode {
                    MaskMode::Add => v + *a * (1.0 - v),
                    MaskMode::Subtract => *a * (1.0 - v),
                };
            }
        }
    }
    canvas
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `FRAG_MASK` at each pixel center, in layer pixels through the inverse
/// of `device`.
fn parametric(masks: &ParametricMasks, device: Affine, w: usize, h: usize) -> Vec<f64> {
    let inverse = device.inverse();
    let mut out = vec![0.0; w * h];
    for (i, o) in out.iter_mut().enumerate() {
        let p = inverse * Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        let mut cov: f64 = if masks.has_add { 0.0 } else { 1.0 };
        for m in &masks.masks {
            let (gw, gh) = m.size;
            if gw < 0.01 || gh < 0.01 {
                continue;
            }
            let (mut px, mut py) = (p.x - m.center.x, p.y - m.center.y);
            if m.rotation.abs() > 1e-4 {
                let (s, c) = m.rotation.sin_cos();
                (px, py) = (c * px + s * py, -s * px + c * py);
            }
            let (hx, hy) = ((gw * 0.5).max(0.001), (gh * 0.5).max(0.001));
            let d = match m.shape {
                MaskShape::Rect => {
                    let (dx, dy) = (px.abs() - hx, py.abs() - hy);
                    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)
                }
                MaskShape::Ellipse => ((px / hx).hypot(py / hy) - 1.0) * hx.min(hy),
            };
            let f = m.feather.max(0.0);
            let a = if f > 0.01 {
                1.0 - smoothstep(-f * 0.5, f * 0.5, d)
            } else if d <= 0.0 {
                1.0
            } else {
                0.0
            };
            cov = match m.mode {
                MaskMode::Add => cov.max(a),
                MaskMode::Subtract => cov.min(1.0 - a),
            };
        }
        *o = cov;
    }
    out
}
