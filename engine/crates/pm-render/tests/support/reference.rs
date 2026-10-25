//! Reference frames of documents for the quality gates
//! (docs/engine/gate-policy.md).
//!
//! Authored values (keyframes, world matrices, anchors, opacity, blend
//! names, colors, fills) are evaluated by `pm-eval`, whose own gates keep
//! them exact. Coverage, compositing and color math come from
//! `pm-reference`, never from the engine's renderer. The reading of the
//! document is shared with the engine (`plan::derived_scale`,
//! `normalize_fill`, `project_color`, `blend_mode`, `has_masks`,
//! `vector::level2`): a bug there would be in both, which the interior
//! check against today's capture still catches.
//!
//! Modelled so far: the composition background (solid, linear and radial
//! gradients, none), 2D solid layers (world matrix, size anchor, opacity,
//! blend mode, radius) and 2D shape layers (shapes and editable paths:
//! fills and strokes) in projects that composite in E.
//! Image layers leave their box unreferenced (NaN); any other content
//! leaves the whole frame unreferenced. The comparator judges unreferenced
//! pixels against today's capture until the lane that owns that content
//! models it here (gate-policy.md, "Who converts what").

use pm_eval::node;
use pm_eval::{Comp, Evaluator, Scope};
use pm_model::{Document, Node};
use pm_protocol::color::{BlendMode, Precision, WorkingEncoding};
use pm_reference::compare::{Class, classify};
use pm_reference::composite::{Blend, Rgba, blend, scale};
use pm_reference::coverage::{FillRule, fill};

/// Coverage cells per pixel side for paths and solids.
pub const PATH_SAMPLES: usize = 16;

/// A reference frame.
#[derive(Clone)]
pub struct Reference {
    pub width: usize,
    pub height: usize,
    /// Premultiplied RGBA in the working encoding; NaN where not modelled.
    pub rgba: Vec<f64>,
    /// Pixels where modelled content is partially covered.
    pub edge: Vec<bool>,
    /// What the frame holds that is not modelled (empty when nothing).
    pub unmodelled: Vec<String>,
    /// An unmodelled layer's footprint could not be bounded: the whole
    /// frame is unreferenced.
    pub whole_frame: bool,
}

impl Reference {
    /// Pixel classes: edges grown by 2 pixels; interior elsewhere.
    pub fn classes(&self) -> Vec<Class> {
        let none = vec![false; self.edge.len()];
        classify(&self.edge, &none, self.width, self.height, 2)
    }

    /// The capture's opaque view: color over black clamped to 0…1, alpha 1.
    pub fn opaque(&self) -> Vec<f64> {
        self.rgba
            .chunks_exact(4)
            .flat_map(|p| {
                let a = if p[3].is_nan() { f64::NAN } else { 1.0 };
                [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0), p[2].clamp(0.0, 1.0), a]
            })
            .collect()
    }

    /// The share of pixels the reference models.
    pub fn modelled(&self) -> f64 {
        let n = self.rgba.chunks_exact(4).filter(|p| !p[3].is_nan()).count();
        n as f64 / (self.width * self.height).max(1) as f64
    }

    pub fn alpha(&self) -> Vec<f64> {
        self.rgba.chunks_exact(4).map(|p| p[3]).collect()
    }
}

/// `#RRGGBB` or `#RRGGBBAA` as straight E values; `None` for anything the
/// reference does not read.
fn hex(s: &str) -> Option<[f64; 4]> {
    let d = s.strip_prefix('#')?;
    if !(d.len() == 6 || d.len() == 8) || !d.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let c = |i: usize| f64::from(u8::from_str_radix(&d[i..i + 2], 16).unwrap()) / 255.0;
    Some([c(0), c(2), c(4), if d.len() == 8 { c(6) } else { 1.0 }])
}

/// The background fill (`FRAG_BACKGROUND_FILL`, the authored formula): `t`
/// along the angle (or the radius × 1.4142) from the pixel center with y up,
/// colors mixed stop by stop with `smoothstep` (a hard step where two stops
/// coincide), in E.
fn gradient(fill: &pm_render::plan::Fill, x: usize, y: usize, w: usize, h: usize) -> Option<[f64; 3]> {
    let stops: Vec<([f64; 4], f64)> = fill
        .stops
        .iter()
        .map(|(c, pos)| hex(c).map(|c| (c, pos / 100.0)))
        .collect::<Option<_>>()?;
    let p = [
        (x as f64 + 0.5) / w as f64 - 0.5,
        (h as f64 - (y as f64 + 0.5)) / h as f64 - 0.5,
    ];
    // 1.4142 is the formula's literal, not √2.
    #[allow(clippy::approx_constant)]
    let t = if fill.kind == "radial" {
        (p[0].hypot(p[1]) * 1.4142).clamp(0.0, 1.0)
    } else {
        let a = fill.angle.to_radians();
        (p[0] * a.cos() + p[1] * a.sin() + 0.5).clamp(0.0, 1.0)
    };
    let mut color = [stops[0].0[0], stops[0].0[1], stops[0].0[2]];
    for pair in stops.windows(2) {
        let ((_, e0), (right, e1)) = (pair[0], pair[1]);
        let amount = if e1 <= e0 {
            if t >= e0 { 1.0 } else { 0.0 }
        } else {
            let k = ((t - e0) / (e1 - e0)).clamp(0.0, 1.0);
            k * k * (3.0 - 2.0 * k)
        };
        for i in 0..3 {
            color[i] += (right[i] - color[i]) * amount;
        }
    }
    Some(color)
}

fn reference_blend(mode: BlendMode) -> Blend {
    match mode {
        BlendMode::Normal => Blend::Normal,
        BlendMode::Add => Blend::LinearDodge,
        BlendMode::Screen => Blend::Screen,
        BlendMode::Multiply => Blend::Multiply,
        BlendMode::Overlay => Blend::Overlay,
        BlendMode::SoftLight => Blend::SoftLightPhotoshop,
        BlendMode::Difference => Blend::Difference,
        BlendMode::Lighten => Blend::Lighten,
        BlendMode::Darken => Blend::Darken,
    }
}

/// The reference frame of `composition` at `time`, `width × height`
/// output pixels at the legacy derived scale, with or without the
/// background (transparent captures draw none), with `samples` motion-blur
/// samples per blurred layer (1: none). `effects`: the definitions the
/// engine draws with (for effect padding).
#[allow(clippy::too_many_arguments)]
pub fn render(
    document: &Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    width: u32,
    height: u32,
    background: bool,
    samples: u32,
    effects: &pm_render::effects::Effects,
) -> Reference {
    let (w, h) = (width as usize, height as usize);
    let mut out = Reference {
        width: w,
        height: h,
        rgba: vec![0.0; w * h * 4],
        edge: vec![false; w * h],
        unmodelled: Vec::new(),
        whole_frame: false,
    };
    if let Err(why) = draw(document, composition, time, &mut out, background, samples, effects) {
        out.unmodelled.push(why);
        out.rgba.fill(f64::NAN);
    }
    out
}

fn draw(
    document: &Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    out: &mut Reference,
    background: bool,
    samples: u32,
    effects: &pm_render::effects::Effects,
) -> Result<(), String> {
    let (w, h) = (out.width, out.height);
    let root = document.root().ok_or("no document")?;
    let color = pm_render::composite::color::project_color(root)?;
    if color.working() != WorkingEncoding::Standard {
        return Err("linear-light compositing".into());
    }
    // 8-bit storage holds 0…1 only.
    let clamp = |c: Rgba| -> Rgba {
        if color.precision == Precision::Unorm8 {
            c.map(|v| v.clamp(0.0, 1.0))
        } else {
            c
        }
    };
    let comp = Comp::resolve(document, composition).map_err(|e| format!("{e:?}"))?;
    let seconds = comp.seconds_at(time.get());
    if background {
        let fill = pm_render::plan::normalize_fill(comp.member("backgroundFill"), comp.member("bg"));
        for y in 0..h {
            for x in 0..w {
                let c: Rgba = match fill.kind.as_str() {
                    "none" => [0.0; 4],
                    "solid" => {
                        let c = hex(&fill.stops[0].0).ok_or("background color")?;
                        [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
                    }
                    _ => {
                        let c = gradient(&fill, x, y, w, h).ok_or("background stops")?;
                        [c[0], c[1], c[2], 1.0]
                    }
                };
                out.rgba[(y * w + x) * 4..][..4].copy_from_slice(&clamp(c));
            }
        }
    }
    let (sx, sy) = pm_render::plan::derived_scale(&comp, w as u32, h as u32);
    // Expressions evaluate as the engine evaluates them (pm-eval with pm-expr).
    let mut ev = Evaluator::new().with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = Scope::new(&mut ev, &comp, &comp);
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let layers: Vec<&Node> = comp.layers().iter().collect();
    let solo = layers.iter().any(|l| node::truthy(node::member(Some(l), "solo")));
    // Motion blur (C-K4): a blurred layer is the average of its content at
    // `t + ((s + ½)/n − ½) · shutter / fps`, masked at the center time.
    let blur = (samples > 1).then(|| {
        let fps = pm_render::motion::level_fps(&comp);
        let shutter = pm_render::motion::shutter_frames(comp.node()) / fps;
        (0..samples)
            .map(|s| ((f64::from(s) + 0.5) / f64::from(samples) - 0.5) * shutter)
            .collect::<Vec<f64>>()
    });
    // Layers used as track mattes draw only through their targets.
    let matte_sources: Vec<pm_model::JsString> = layers
        .iter()
        .filter_map(|l| node::string(node::member(Some(l), "matteSource")).cloned())
        .collect();
    // Bottom (last) to top (first), with today's skip rules.
    for &layer in layers.iter().rev() {
        let ty = pm_render::js::member_string(layer, "type").unwrap_or_default();
        let flag = |k: &str| node::truthy(node::member(Some(layer), k));
        let group = flag("group") || ty == "group";
        if (solo && !flag("solo")) || matches!(ty.as_str(), "null" | "audio") {
            continue;
        }
        if node::string(node::member(Some(layer), "id")).is_some_and(|id| matte_sources.contains(id)) {
            continue;
        }
        if !group && !scope.active(layer, seconds).map_err(fail)? {
            continue;
        }
        let opacity = scope.num(layer, "opacity", seconds).map_err(fail)? / 100.0;
        let opacity = if opacity.is_nan() { opacity } else { opacity.clamp(0.0, 1.0) };
        if !group && opacity <= 0.001 {
            continue;
        }
        // What this reference does not model: the layer's footprint, grown
        // by its effects' padding, is unreferenced (and anything drawn over
        // it); the rest of the frame keeps its reference.
        let unmodelled = if group || ty == "precomp" {
            Some("groups and precomps")
        } else if !matches!(ty.as_str(), "solid" | "image" | "shape") {
            Some("other layer types")
        } else if flag("matteSource") {
            Some("mattes")
        } else if scope.is_3d_layer(layer).map_err(fail)? {
            Some("2.5D planes")
        } else if scope.active_transition(layer, seconds, &|_| true).map_err(fail)?.is_some() {
            Some("transitions")
        } else if scope.has_renderable_effects(layer, seconds).map_err(fail)? {
            Some("effects")
        } else {
            None
        };
        let world = scope.world_matrix(layer, seconds).map_err(fail)?;
        let offset = scope.size_anchor_offset(layer, seconds).map_err(fail)?;
        let scaled = kurbo::Affine::new([
            world[0] * sx,
            world[1] * sy,
            world[2] * sx,
            world[3] * sy,
            world[4] * sx,
            world[5] * sy,
        ]);
        let device = scaled * kurbo::Affine::translate((offset.x, offset.y));
        if let Some(why) = unmodelled {
            if !out.unmodelled.iter().any(|u| u == why) {
                out.unmodelled.push(why.into());
            }
            let times: Vec<f64> = match &blur {
                Some(offsets) if flag("mblur") => offsets.iter().map(|o| seconds + o).collect(),
                _ => vec![seconds],
            };
            footprint(&mut scope, effects, layer, &ty, &times, seconds, (sx, sy), out)?;
            continue;
        }
        let d = scope.ev.resolve_content(scope.comp, layer, seconds).map_err(fail)?;
        if ty == "image" {
            // Resampled content no lane models yet: its box (centered on
            // the anchor), grown by a pixel, is unreferenced.
            let sw = pm_render::js::or_number(pm_render::js::get(&d, "w"), w as f64);
            let sh = pm_render::js::or_number(pm_render::js::get(&d, "h"), h as f64);
            let rect = kurbo::Rect::new(-sw / 2.0, -sh / 2.0, sw / 2.0, sh / 2.0);
            unreference(out, &(device * kurbo::Shape::to_path(&rect, 1e-9)), 1);
            continue;
        }
        let mode = reference_blend(
            pm_render::composite::build::blend_mode(&mut scope, layer, seconds).map_err(fail)?,
        );
        let blurred = blur.as_ref().filter(|_| {
            scope
                .evaluated(layer, node::member(Some(layer), "mblur"), seconds, "l.mblur")
                .is_ok_and(|v| v.truthy())
        });
        // The layer's own pixels: one draw, or the samples' average.
        let times: Vec<f64> = match blurred {
            Some(offsets) => offsets.iter().map(|o| seconds + o).collect(),
            None => vec![seconds],
        };
        let mut layer_px = vec![([0.0f64; 4], false); w * h];
        for &t in &times {
            let sample = content_pixels(&mut scope, layer, &ty, t, (sx, sy), w, h)?;
            for (acc, (px, edge)) in layer_px.iter_mut().zip(sample) {
                for c in 0..4 {
                    acc.0[c] += px[c] / times.len() as f64;
                }
                acc.1 |= edge;
            }
        }
        if blurred.is_some() {
            // Averaged edges are partial coverage wherever samples differ.
            for p in &mut layer_px {
                p.1 |= p.0[3] > 0.0 && p.0[3] < 1.0 - 1e-12;
            }
        }
        // Masks (at the center time), from the mask reference.
        if let Some((mask, band)) = super::mask_reference::layer_mask(&mut scope, layer, seconds, scaled, w, h)? {
            for (i, p) in layer_px.iter_mut().enumerate() {
                p.0 = scale(p.0, mask[i]);
                p.1 |= band[i];
            }
        }
        for (i, (px, edge)) in layer_px.iter().enumerate() {
            if px[3] <= 0.0 && !edge {
                continue;
            }
            out.edge[i] |= *edge;
            let at = &mut out.rgba[i * 4..][..4];
            let dst = [at[0], at[1], at[2], at[3]];
            at.copy_from_slice(&clamp(blend(mode, scale(*px, opacity), dst)));
        }
    }
    Ok(())
}

/// A solid or shape layer's own pixels at `time` (premultiplied, before
/// opacity and masks), and where they are partially covered.
fn content_pixels<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    ty: &str,
    time: f64,
    (sx, sy): (f64, f64),
    w: usize,
    h: usize,
) -> Result<Vec<(Rgba, bool)>, String> {
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let world = scope.world_matrix(layer, time).map_err(fail)?;
    let offset = scope.size_anchor_offset(layer, time).map_err(fail)?;
    let device = kurbo::Affine::new([
        world[0] * sx,
        world[1] * sy,
        world[2] * sx,
        world[3] * sy,
        world[4] * sx,
        world[5] * sy,
    ]) * kurbo::Affine::translate((offset.x, offset.y));
    let d = scope.ev.resolve_content(scope.comp, layer, time).map_err(fail)?;
    if ty == "shape" {
        // The authored geometry and paint (pm-vector's, gated exactly by its
        // own tests), covered and composited here.
        let has_paths = node::array(node::member(node::member(Some(layer), "d"), "paths"))
            .is_some_and(|p| !p.is_empty());
        let scene = if has_paths {
            let (_, colors, paths) = pm_render::vector::paths(scope, layer, time).map_err(fail)?;
            pm_render::vector::paths_scene(&paths, &colors, &device.as_coeffs())
        } else {
            let (content, colors) = pm_render::vector::shape_content(&d);
            match pm_render::vector::shape_scene(&content, &colors, &device.as_coeffs()) {
                Some(scene) => scene,
                None => return Ok(vec![([0.0; 4], false); w * h]),
            }
        };
        return scene_pixels(&scene, w, h);
    }
    let sw = pm_render::js::or_number(pm_render::js::get(&d, "w"), w as f64);
    let sh = pm_render::js::or_number(pm_render::js::get(&d, "h"), h as f64);
    let paint = hex(&pm_render::js::string(pm_render::js::get(&d, "color"))).ok_or("solid color")?;
    // `radius` rounds the corners, at most half of either side
    // (SOLID-RADIUS-1: the inspector edits it).
    let radius = pm_render::js::get(&d, "radius").to_number();
    let radius = if radius > 0.0 { radius.min(sw.abs() / 2.0).min(sh.abs() / 2.0) } else { 0.0 };
    let outline = kurbo::Shape::to_path(&kurbo::RoundedRect::new(0.0, 0.0, sw, sh, radius), 1e-9);
    let coverage = fill(&(device * outline), FillRule::NonZero, PATH_SAMPLES, w, h);
    let src: Rgba = [paint[0] * paint[3], paint[1] * paint[3], paint[2] * paint[3], paint[3]];
    let mut out = vec![([0.0; 4], false); w * h];
    for y in coverage.y0..coverage.y0 + coverage.height {
        for x in coverage.x0..coverage.x0 + coverage.width {
            let k = coverage.at(x, y);
            if k > 0.0 {
                out[y * w + x] = (scale(src, k), k < 1.0 - 1e-12);
            }
        }
    }
    Ok(out)
}

/// Marks the pixels `outline` (output pixels) touches, grown by `grow`
/// pixels, unreferenced.
fn unreference(out: &mut Reference, outline: &kurbo::BezPath, grow: usize) {
    let (w, h) = (out.width, out.height);
    let c = fill(outline, FillRule::NonZero, 1, w, h);
    let mut inside = vec![false; w * h];
    for y in c.y0..c.y0 + c.height {
        for x in c.x0..c.x0 + c.width {
            inside[y * w + x] = c.at(x, y) > 0.0;
        }
    }
    for (i, _) in pm_reference::compare::dilate(&inside, w, h, grow).iter().enumerate().filter(|(_, g)| **g) {
        out.rgba[i * 4..][..4].fill(f64::NAN);
    }
}

/// An unmodelled layer's footprint: what its content covers under its 2D
/// world matrix (solids and shapes: their own pixels; other content: its
/// box) at every time it is drawn, grown by its effects' padding (C-K6: the
/// sum of their `viewportPadding` parameters) and 2 pixels, is
/// unreferenced. A footprint this cannot bound (groups, precomps, 2.5D
/// planes, transitions, which move content) is the whole frame.
#[allow(clippy::too_many_arguments)]
fn footprint<'d>(
    scope: &mut Scope<'_, 'd>,
    effects: &pm_render::effects::Effects,
    layer: &'d Node,
    ty: &str,
    times: &[f64],
    seconds: f64,
    (sx, sy): (f64, f64),
    out: &mut Reference,
) -> Result<(), String> {
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let (w, h) = (out.width, out.height);
    let content = matches!(ty, "solid" | "shape");
    let boxed = matches!(ty, "image" | "shader" | "extension");
    let bounded = (content || boxed)
        && !node::truthy(node::member(Some(layer), "group"))
        && !scope.is_3d_layer(layer).map_err(fail)?
        && scope.active_transition(layer, seconds, &|_| true).map_err(fail)?.is_none();
    let padding: f64 = match pm_render::effects::plan::effect_steps(scope, effects, layer, seconds).map_err(fail)? {
        Ok(steps) => steps.iter().map(|s| s.padding).sum(),
        Err(_) => f64::INFINITY,
    };
    if !bounded || !padding.is_finite() {
        out.rgba.fill(f64::NAN);
        out.whole_frame = true;
        return Ok(());
    }
    let mut covered = vec![false; w * h];
    let mut grow = 2;
    for &t in times {
        let world = scope.world_matrix(layer, t).map_err(fail)?;
        let offset = scope.size_anchor_offset(layer, t).map_err(fail)?;
        let device = kurbo::Affine::new([
            world[0] * sx,
            world[1] * sy,
            world[2] * sx,
            world[3] * sy,
            world[4] * sx,
            world[5] * sy,
        ]) * kurbo::Affine::translate((offset.x, offset.y));
        let c = device.as_coeffs();
        grow = grow.max((padding * c[0].hypot(c[1]).max(c[2].hypot(c[3]))).ceil() as usize + 2);
        if content {
            for (i, (px, edge)) in content_pixels(scope, layer, ty, t, (sx, sy), w, h)?.into_iter().enumerate() {
                covered[i] |= px[3] > 0.0 || edge;
            }
        } else {
            let d = scope.ev.resolve_content(scope.comp, layer, t).map_err(fail)?;
            let bw = pm_render::js::or_number(pm_render::js::get(&d, "w"), w as f64);
            let bh = pm_render::js::or_number(pm_render::js::get(&d, "h"), h as f64);
            let rect = kurbo::Rect::new(-bw / 2.0, -bh / 2.0, bw / 2.0, bh / 2.0);
            let f = fill(&(device * kurbo::Shape::to_path(&rect, 1e-9)), FillRule::NonZero, 1, w, h);
            for y in f.y0..f.y0 + f.height {
                for x in f.x0..f.x0 + f.width {
                    covered[y * w + x] |= f.at(x, y) > 0.0;
                }
            }
        }
    }
    for (i, g) in pm_reference::compare::dilate(&covered, w, h, grow).into_iter().enumerate() {
        if g {
            out.rgba[i * 4..][..4].fill(f64::NAN);
        }
    }
    Ok(())
}

/// Device-pixel tolerance of the reference's stroke outlines.
const STROKE_TOLERANCE: f64 = 1e-4;

/// A vector layer's own pixels (premultiplied, source-over item by item in
/// order) over a `w × h` frame, and where any item is partially covered.
/// Fills are covered exactly; strokes are the exact region their joins and
/// caps define (`pm_reference::stroke`), covered the same way.
pub fn scene_pixels(
    scene: &pm_vector::render::scene::CoverageScene,
    w: usize,
    h: usize,
) -> Result<Vec<(Rgba, bool)>, String> {
    use pm_vector::render::scene::{FillRule as Rule, ItemStyle};
    let mut out = vec![([0.0; 4], false); w * h];
    for item in &scene.items {
        let (device, rule, cells) = match &item.style {
            ItemStyle::Fill(rule) => (
                item.transform * item.path.clone(),
                match rule {
                    Rule::NonZero => FillRule::NonZero,
                    Rule::EvenOdd => FillRule::EvenOdd,
                },
                PATH_SAMPLES,
            ),
            ItemStyle::Stroke(stroke) => {
                let c = item.transform.as_coeffs();
                let scale = c[0].hypot(c[1]).max(c[2].hypot(c[3])).max(1e-12);
                let region = pm_reference::stroke::outline(&item.path, stroke, STROKE_TOLERANCE / scale)
                    .ok_or("invalid stroke parameters")?;
                // Overlapping pieces at the edge count within a cell: finer
                // cells bound that error (1/n² of a pixel per cell).
                let cells = if region.folds { 4 * PATH_SAMPLES } else { PATH_SAMPLES };
                (item.transform * region.path, FillRule::NonZero, cells)
            }
        };
        let coverage = fill(&device, rule, cells, w, h);
        let paint = item.premultiplied().map(f64::from);
        for y in coverage.y0..coverage.y0 + coverage.height {
            for x in coverage.x0..coverage.x0 + coverage.width {
                let k = coverage.at(x, y);
                if k <= 0.0 {
                    continue;
                }
                let (px, edge) = &mut out[y * w + x];
                *edge |= k < 1.0 - 1e-12;
                *px = blend(Blend::Normal, scale(paint, k), *px);
            }
        }
    }
    Ok(out)
}

/// Preview == export (gate-policy.md): the frame the paused viewer asks for
/// and the frame an export asks for, from contract 2 requests, with the
/// same motion-blur samples (sample counts per output kind are a setting,
/// not a pipeline difference).
pub fn preview_and_export(
    renderer: &mut pm_render::Renderer,
    document: &Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    size: pm_protocol::common::Size,
    motion_blur: pm_render::motion::MotionBlur,
    resolver: &pm_render::MapResolver,
) -> (Vec<f32>, Vec<f32>) {
    use pm_protocol::color::{OutputColor, ViewerColorSpace};
    use pm_protocol::frame::{
        FramePurpose, FrameRequest, MediaPreference, MotionBlurKind, PreviewResolution,
    };
    use pm_protocol::priority::PriorityClass;
    let export = FrameRequest {
        composition: pm_protocol::common::CompositionId::new(composition),
        time,
        purpose: FramePurpose::Export,
        priority: PriorityClass::UserJob,
        output_size: size,
        region: None,
        scale: 1.0,
        resolution: PreviewResolution::Full,
        media: MediaPreference::Originals,
        color: OutputColor::SrgbImage,
        precision: Precision::Float16,
        motion_blur: MotionBlurKind::Export,
        view: None,
        draft_3d: false,
        draft_render: false,
        depth_id: false,
        overlay: false,
    };
    let preview = FrameRequest {
        purpose: FramePurpose::Viewer,
        priority: PriorityClass::Interactive,
        color: OutputColor::Viewer {
            space: ViewerColorSpace::Srgb,
        },
        motion_blur: MotionBlurKind::PreviewPaused,
        ..export.clone()
    };
    let mut frame = |request: &FrameRequest| {
        let mut r = pm_render::RenderRequest::from_frame_request(request).unwrap();
        // The legacy scale of the composition, as the corpus renders it.
        r.scale = None;
        r.motion_blur = motion_blur;
        let f = renderer.render(document, composition, time, &r, resolver).unwrap();
        renderer.read_rgba(&f).unwrap()
    };
    (frame(&preview), frame(&export))
}
