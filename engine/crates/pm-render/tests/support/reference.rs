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
//! gradients, none); 2D solids and affine level 1 stills at mip level zero;
//! and their authored group, precomp and track-matte compositing graph
//! (transforms, size anchor, fit, opacity, parametric rectangle/ellipse
//! masks and blend) in projects that composite in E. Each child source is
//! painted from its own layers into f64 pixels before the parent samples it.
//! Other image mappings leave their box unreferenced (NaN); unsupported
//! content leaves the frame unreferenced. The comparator judges those pixels
//! against today's capture until their owner models them here
//! (gate-policy.md, "Who converts what").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pm_eval::node;
use pm_eval::{Comp, Evaluator, Scope};
use pm_model::{Document, Node};
use pm_protocol::color::{BlendMode, Precision, WorkingEncoding};
use pm_reference::compare::{Class, classify};
use pm_reference::composite::{Blend, Rgba, blend, scale};
use pm_reference::coverage::{FillRule, fill};
use pm_render::AssetResolver;
use pm_render::composite::build::MaskRoute;

use super::sampling_reference::Bitmap;

/// Coverage cells per pixel side for paths and solids.
pub const PATH_SAMPLES: usize = 16;

#[derive(Clone, Copy)]
struct Depth {
    precomp: usize,
    matte: usize,
}

/// A reference frame.
pub struct Reference {
    pub width: usize,
    pub height: usize,
    /// Premultiplied RGBA in the working encoding; NaN where not modelled.
    pub rgba: Vec<f64>,
    /// Pixels where modelled content is partially covered.
    pub edge: Vec<bool>,
    /// Pixels read from a modelled decoded source through a declared filter.
    pub resampled: Vec<bool>,
    /// Why the frame is not modelled (empty when it is).
    pub unmodelled: Vec<String>,
}

impl Reference {
    /// Pixel classes: edges grown by 2 pixels; interior elsewhere.
    pub fn classes(&self) -> Vec<Class> {
        classify(&self.edge, &self.resampled, self.width, self.height, 2)
    }

    /// The capture's opaque sRGB byte boundary: over black, independently
    /// gamut-mapped in f64. Unknown source pixels stay explicitly unknown.
    pub fn opaque(&self) -> Vec<f64> {
        self.rgba
            .chunks_exact(4)
            .flat_map(|p| {
                if p.iter().any(|v| v.is_nan()) {
                    return [f64::NAN; 4];
                }
                let rgb = pm_reference::color::css_gamut_map_srgb([p[0], p[1], p[2]]);
                [rgb[0], rgb[1], rgb[2], 1.0]
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
fn gradient(
    fill: &pm_render::plan::Fill,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) -> Option<[f64; 3]> {
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

/// Track-matte strength from the independently painted, premultiplied
/// source. The authored luma mode uses encoded Rec.709 coefficients.
fn matte_strength(matte: &Option<(Reference, bool, bool)>, index: usize) -> (f64, bool) {
    let Some((surface, luma, inverted)) = matte else {
        return (1.0, false);
    };
    let px = &surface.rgba[index * 4..][..4];
    let value = if *luma {
        pm_reference::composite::luma709([px[0], px[1], px[2]])
    } else {
        px[3]
    };
    (
        if *inverted { 1.0 - value } else { value },
        surface.edge[index] || surface.resampled[index],
    )
}

#[allow(clippy::too_many_arguments)]
fn render_matte<'d>(
    scope: &mut Scope<'_, 'd>,
    root: &'d Node,
    assets: &dyn AssetResolver,
    layer: &'d Node,
    seconds: f64,
    scale_xy: (f64, f64),
    size: (usize, usize),
    bitmaps: &mut HashMap<PathBuf, Option<Bitmap>>,
    depth: Depth,
) -> Result<Option<(Reference, bool, bool)>, String> {
    if depth.matte >= pm_render::composite::MATTE_MAX_DEPTH as usize {
        return Ok(None);
    }
    let Some(source) = pm_render::composite::build::matte_source(scope.layers(), layer) else {
        return Ok(None);
    };
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let (luma, inverted) =
        pm_render::composite::build::matte_mode(scope, layer, seconds).map_err(fail)?;
    let (w, h) = size;
    let mut surface = Reference {
        width: w,
        height: h,
        rgba: vec![0.0; w * h * 4],
        edge: vec![false; w * h],
        resampled: vec![false; w * h],
        unmodelled: Vec::new(),
    };
    draw_pass(
        scope,
        root,
        assets,
        seconds,
        scale_xy,
        None,
        Some(source),
        &mut surface,
        bitmaps,
        Depth {
            matte: depth.matte + 1,
            ..depth
        },
    )?;
    Ok(Some((surface, luma, inverted)))
}

/// Independent f64 evaluation of the parametric mask's signed-distance
/// coverage. The mask values are authored/evaluated facts; only this pixel
/// formula is the reference. The second result marks a possible mask edge.
fn mask_coverage(route: &MaskRoute, at: (f64, f64)) -> (f64, bool) {
    let MaskRoute::Parametric(masks) = route else {
        return (1.0, false);
    };
    let inv = masks.inverse;
    let local = (
        inv[0] * at.0 + inv[3] * at.1 + inv[6],
        inv[1] * at.0 + inv[4] * at.1 + inv[7],
    );
    let pixel_radius = (inv[0].hypot(inv[1]) + inv[3].hypot(inv[4])).max(1e-9);
    let mut cov: f64 = if masks.masks.has_add { 0.0 } else { 1.0 };
    let mut edge = false;
    for m in &masks.masks.masks {
        if m.size.0 < 0.01 || m.size.1 < 0.01 {
            continue;
        }
        let (mut x, mut y) = (local.0 - m.center.x, local.1 - m.center.y);
        if m.rotation.abs() > 1e-4 {
            let (s, c) = m.rotation.sin_cos();
            (x, y) = (c * x + s * y, -s * x + c * y);
        }
        let half = (m.size.0.max(0.002) / 2.0, m.size.1.max(0.002) / 2.0);
        let distance = match m.shape {
            pm_vector::MaskShape::Rect => {
                let d = (x.abs() - half.0, y.abs() - half.1);
                d.0.max(0.0).hypot(d.1.max(0.0)) + d.0.max(d.1).min(0.0)
            }
            pm_vector::MaskShape::Ellipse => {
                ((x / half.0).hypot(y / half.1) - 1.0) * half.0.min(half.1)
            }
        };
        edge |= distance.abs() <= m.feather.max(pixel_radius);
        let a = if m.feather > 0.01 {
            let t = ((distance + m.feather / 2.0) / m.feather).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        } else if distance <= 0.0 {
            1.0
        } else {
            0.0
        };
        cov = match m.mode {
            pm_vector::MaskMode::Add => cov.max(a),
            pm_vector::MaskMode::Subtract => cov.min(1.0 - a),
        };
    }
    (cov, edge)
}

/// The source reader is a boundary, not a pixel oracle for the renderer:
/// it supplies decoded texels; all placement, filtering and compositing
/// below are independent f64 calculations.
fn decoded_bitmap(path: &Path) -> Result<Option<Bitmap>, String> {
    let pool = pm_media::pool::FramePool::new(0);
    let (still, failed) =
        pm_render::media::level1_bitmap(path, &pool).map_err(|e| format!("bitmap source {e:?}"))?;
    if failed || still.bit_depth != 8 || !still.frame.format.rgba_premultiplied {
        return Ok(None);
    }
    let (width, height) = (still.frame.format.width, still.frame.format.height);
    let bytes = still
        .frame
        .with_planes(|planes| {
            let plane = planes.first()?;
            if planes.len() != 1 || plane.desc.format != pm_media::frame::PlaneFormat::Rgba8 {
                return None;
            }
            let mut bytes = Vec::with_capacity(width * height * 4);
            for y in 0..height {
                // The plane view owns `height` rows of `stride` bytes.
                bytes.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(plane.ptr.add(y * plane.stride), width * 4)
                });
            }
            Some(bytes)
        })
        .map_err(|e| format!("bitmap planes {e}"))?;
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    Ok(Some(Bitmap {
        width,
        height,
        rgba: bytes.into_iter().map(|v| f64::from(v) / 255.0).collect(),
    }))
}

/// The authored fit window in decoded-source coordinates.
fn image_uv(fit: &str, source: (f64, f64), box_size: (f64, f64)) -> Option<[f64; 4]> {
    if ![source.0, source.1, box_size.0, box_size.1]
        .iter()
        .all(|v| v.is_finite() && *v > 0.0)
    {
        return None;
    }
    let source_ratio = source.0 / source.1;
    let box_ratio = box_size.0 / box_size.1;
    Some(match fit {
        "cover" if source_ratio > box_ratio => {
            let span = box_ratio / source_ratio;
            [(1.0 - span) / 2.0, 0.0, span, 1.0]
        }
        "cover" => {
            let span = source_ratio / box_ratio;
            [0.0, (1.0 - span) / 2.0, 1.0, span]
        }
        "contain" if source_ratio > box_ratio => {
            let span = source_ratio / box_ratio;
            [0.0, (1.0 - span) / 2.0, 1.0, span]
        }
        "contain" => {
            let span = box_ratio / source_ratio;
            [(1.0 - span) / 2.0, 0.0, span, 1.0]
        }
        _ => [0.0, 0.0, 1.0, 1.0],
    })
}

/// The reference frame of `composition` at `time`, `width × height`
/// output pixels at the legacy derived scale, with or without the
/// background (transparent captures draw none).
pub fn render(
    document: &Document,
    assets: &dyn AssetResolver,
    composition: &str,
    time: pm_protocol::common::Ticks,
    width: u32,
    height: u32,
    background: bool,
) -> Reference {
    let (w, h) = (width as usize, height as usize);
    let mut out = Reference {
        width: w,
        height: h,
        rgba: vec![0.0; w * h * 4],
        edge: vec![false; w * h],
        resampled: vec![false; w * h],
        unmodelled: Vec::new(),
    };
    if let Err(why) = draw(document, assets, composition, time, &mut out, background) {
        out.unmodelled.push(why);
        out.rgba.fill(f64::NAN);
    }
    out
}

fn draw(
    document: &Document,
    assets: &dyn AssetResolver,
    composition: &str,
    time: pm_protocol::common::Ticks,
    out: &mut Reference,
    background: bool,
) -> Result<(), String> {
    let root = document.root().ok_or("no document")?;
    let comp = Comp::resolve(document, composition).map_err(|e| format!("{e:?}"))?;
    let seconds = comp.seconds_at(time.get());
    draw_comp(
        &comp,
        &comp,
        root,
        assets,
        seconds,
        out,
        background,
        Depth {
            precomp: 0,
            matte: 0,
        },
    )
}

fn draw_comp<'d>(
    comp: &Comp<'d>,
    project: &Comp<'d>,
    root: &'d Node,
    assets: &dyn AssetResolver,
    seconds: f64,
    out: &mut Reference,
    background: bool,
    depth: Depth,
) -> Result<(), String> {
    let (w, h) = (out.width, out.height);
    let color = pm_render::composite::color::project_color(root)?;
    if color.working() != WorkingEncoding::Standard {
        return Err("linear-light compositing".into());
    }
    if background {
        let fill =
            pm_render::plan::normalize_fill(comp.member("backgroundFill"), comp.member("bg"));
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
                out.rgba[(y * w + x) * 4..][..4].copy_from_slice(&c);
            }
        }
    }
    let scale = pm_render::plan::derived_scale(comp, w as u32, h as u32);
    let mut ev = Evaluator::new();
    let mut scope = Scope::new(&mut ev, comp, project);
    let mut bitmaps: HashMap<PathBuf, Option<Bitmap>> = HashMap::new();
    draw_pass(
        &mut scope,
        root,
        assets,
        seconds,
        scale,
        None,
        None,
        out,
        &mut bitmaps,
        depth,
    )
}

/// A compositing pass, with pure transform groups flattened by pm-eval's
/// authored group index. Boundary groups have their own transparent f64
/// surface; their opacity and blend are applied once to the finished image.
#[allow(clippy::too_many_arguments)]
fn draw_pass<'d>(
    scope: &mut Scope<'_, 'd>,
    root: &'d Node,
    assets: &dyn AssetResolver,
    seconds: f64,
    scale_xy: (f64, f64),
    parent: Option<&'d Node>,
    explicit: Option<&'d Node>,
    out: &mut Reference,
    bitmaps: &mut HashMap<PathBuf, Option<Bitmap>>,
    depth: Depth,
) -> Result<(), String> {
    let (w, h) = (out.width, out.height);
    let (sx, sy) = scale_xy;
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let all: Vec<&Node> = scope.layers().iter().collect();
    let solo = all
        .iter()
        .any(|l| node::truthy(node::member(Some(l), "solo")));
    let parent_id = parent.map(|layer| node::get(layer, b"id").to_js());
    let mut boundary = |s: &mut Scope<'_, 'd>, layer: &'d Node| {
        s.group_creates_compositing_boundary(layer, seconds, true, &|_| true)
    };
    let layers = if let Some(layer) = explicit {
        vec![layer]
    } else {
        scope
            .compositing_layers(&all, parent_id.as_ref(), &mut boundary)
            .map_err(fail)?
    };
    let matte_sources: Vec<_> = all
        .iter()
        .filter_map(|layer| node::member(Some(layer), "matteSource"))
        .map(|id| node::to_js_string(Some(id)))
        .collect();
    // Bottom (last) to top (first), with today's skip rules.
    for &layer in layers.iter().rev() {
        let ty = pm_render::js::member_string(layer, "type").unwrap_or_default();
        let flag = |k: &str| node::truthy(node::member(Some(layer), k));
        if explicit.is_none()
            && matte_sources
                .iter()
                .any(|id| *id == node::to_js_string(node::member(Some(layer), "id")))
        {
            continue;
        }
        if matches!(ty.as_str(), "null" | "audio") {
            continue;
        }
        if explicit.is_none() && solo && !flag("solo") {
            let within_solo_group = scope
                .group_ancestors(layer)
                .map_err(fail)?
                .iter()
                .any(|group| node::truthy(node::member(Some(group), "solo")));
            let contains_solo = if ty == "group" {
                let mut found = false;
                for candidate in all
                    .iter()
                    .filter(|candidate| node::truthy(node::member(Some(candidate), "solo")))
                {
                    found |= scope
                        .group_ancestors(candidate)
                        .map_err(fail)?
                        .iter()
                        .any(|group| std::ptr::eq(*group, layer));
                }
                found
            } else {
                false
            };
            if !within_solo_group && !contains_solo {
                continue;
            }
        }
        let active = if explicit.is_some() {
            scope.active_forced_on(layer, seconds)
        } else {
            scope.active(layer, seconds)
        };
        if !active.map_err(fail)? {
            continue;
        }
        let opacity = scope.num(layer, "opacity", seconds).map_err(fail)? / 100.0;
        let opacity = if opacity.is_nan() {
            opacity
        } else {
            opacity.clamp(0.0, 1.0)
        };
        if opacity <= 0.001 {
            continue;
        }
        if ty == "group" {
            if flag("mblur")
                || scope.has_renderable_effects(layer, seconds).map_err(fail)?
                || scope
                    .active_transition(layer, seconds, &|_| true)
                    .map_err(fail)?
                    .is_some()
            {
                return Err("group effects, motion blur or transition".into());
            }
            let mut child = Reference {
                width: w,
                height: h,
                rgba: vec![0.0; w * h * 4],
                edge: vec![false; w * h],
                resampled: vec![false; w * h],
                unmodelled: Vec::new(),
            };
            draw_pass(
                scope,
                root,
                assets,
                seconds,
                scale_xy,
                Some(layer),
                None,
                &mut child,
                bitmaps,
                depth,
            )?;
            let mode = reference_blend(
                pm_render::composite::build::blend_mode(scope, layer, seconds).map_err(fail)?,
            );
            let world = scope.world_matrix(layer, seconds).map_err(fail)?;
            let scaled_world = [
                world[0] * sx,
                world[1] * sy,
                world[2] * sx,
                world[3] * sy,
                world[4] * sx,
                world[5] * sy,
            ];
            let masks =
                if pm_render::composite::build::has_masks(scope, layer, seconds).map_err(fail)? {
                    let route = pm_render::composite::build::masks(
                        scope,
                        layer,
                        seconds,
                        scaled_world,
                        None,
                        w as u32,
                        h as u32,
                    )
                    .map_err(fail)?;
                    if matches!(route, MaskRoute::Path(_)) {
                        return Err("Bezier path masks".into());
                    }
                    Some(route)
                } else {
                    None
                };
            let matte = if flag("matteSource") {
                render_matte(
                    scope,
                    root,
                    assets,
                    layer,
                    seconds,
                    scale_xy,
                    (w, h),
                    bitmaps,
                    depth,
                )?
            } else {
                None
            };
            for i in 0..w * h {
                let x = i % w;
                let y = i / w;
                let (mask, mask_edge) = masks.as_ref().map_or((1.0, false), |route| {
                    mask_coverage(route, (x as f64 + 0.5, y as f64 + 0.5))
                });
                let (matte_value, matte_edge) = matte_strength(&matte, i);
                let source = scale(
                    [
                        child.rgba[4 * i],
                        child.rgba[4 * i + 1],
                        child.rgba[4 * i + 2],
                        child.rgba[4 * i + 3],
                    ],
                    opacity * mask * matte_value,
                );
                let destination = [
                    out.rgba[4 * i],
                    out.rgba[4 * i + 1],
                    out.rgba[4 * i + 2],
                    out.rgba[4 * i + 3],
                ];
                out.rgba[4 * i..4 * i + 4].copy_from_slice(&blend(mode, source, destination));
                if mode == Blend::Normal && source[3] >= 1.0 - 1e-12 {
                    out.edge[i] = false;
                    out.resampled[i] = false;
                }
                out.edge[i] |= child.edge[i] || mask_edge || matte_edge;
                out.resampled[i] |= child.resampled[i];
            }
            out.unmodelled.extend(child.unmodelled);
            continue;
        }
        if ty == "adjustment" {
            if scope.has_renderable_effects(layer, seconds).map_err(fail)? {
                return Err("adjustment effects".into());
            }
            continue;
        }
        if !matches!(ty.as_str(), "solid" | "image" | "precomp") {
            return Err(format!("{ty} layers"));
        }
        if ty == "precomp" && depth.precomp >= pm_render::plan::PRECOMP_MAX_DEPTH as usize {
            continue;
        }
        if flag("mblur")
            || flag("threeD")
            || scope.has_renderable_effects(layer, seconds).map_err(fail)?
            || scope
                .active_transition(layer, seconds, &|_| true)
                .map_err(fail)?
                .is_some()
            || scope.is_3d_layer(layer).map_err(fail)?
        {
            return Err("motion blur, 3D, effects or transitions".into());
        }
        let matte = if flag("matteSource") {
            render_matte(
                scope,
                root,
                assets,
                layer,
                seconds,
                scale_xy,
                (w, h),
                bitmaps,
                depth,
            )?
        } else {
            None
        };
        let d = scope
            .ev
            .resolve_content(scope.comp, layer, seconds)
            .map_err(fail)?;
        let default_width = if ty == "precomp" {
            node::to_number(scope.comp.member("w"))
        } else {
            w as f64
        };
        let default_height = if ty == "precomp" {
            node::to_number(scope.comp.member("h"))
        } else {
            h as f64
        };
        let sw = pm_render::js::or_number(pm_render::js::get(&d, "w"), default_width);
        let sh = pm_render::js::or_number(pm_render::js::get(&d, "h"), default_height);
        let (sw, sh) = if ty == "precomp" {
            (sw.max(1.0), sh.max(1.0))
        } else {
            (sw, sh)
        };
        // Render the child from authored layers into f64 source texels. The
        // parent then filters those texels; no native framebuffer enters the
        // expected image.
        let precomp = if ty == "precomp" {
            let id = pm_render::js::string(pm_render::js::get(&d, "comp"));
            let sub = Comp::in_root(root, &id, scope.comp.revision())
                .map_err(|e| format!("precomp {id}: {e:?}"))?;
            let target_w = (sw * sx).round().max(2.0);
            let target_h = (sh * sy).round().max(2.0);
            if !target_w.is_finite()
                || !target_h.is_finite()
                || target_w > 16384.0
                || target_h > 16384.0
            {
                return Err(format!("precomp {id}: target outside reference bounds"));
            }
            let child_time = scope.source_time(layer, seconds, None).map_err(fail)?;
            let child_time = pm_render::retime::keep_frame_rate(layer, &sub, child_time);
            let (tw, th) = (target_w as usize, target_h as usize);
            let mut child = Reference {
                width: tw,
                height: th,
                rgba: vec![0.0; tw * th * 4],
                edge: vec![false; tw * th],
                resampled: vec![false; tw * th],
                unmodelled: Vec::new(),
            };
            draw_comp(
                &sub,
                scope.project,
                root,
                assets,
                child_time,
                &mut child,
                false,
                Depth {
                    precomp: depth.precomp + 1,
                    ..depth
                },
            )?;
            if !child.unmodelled.is_empty() {
                return Err(format!("precomp {id}: {}", child.unmodelled.join(", ")));
            }
            Some(Bitmap {
                width: tw,
                height: th,
                rgba: child.rgba,
            })
        } else {
            None
        };
        let world = scope.world_matrix(layer, seconds).map_err(fail)?;
        let offset = scope.size_anchor_offset(layer, seconds).map_err(fail)?;
        let scaled_world = [
            world[0] * sx,
            world[1] * sy,
            world[2] * sx,
            world[3] * sy,
            world[4] * sx,
            world[5] * sy,
        ];
        let masks =
            if pm_render::composite::build::has_masks(scope, layer, seconds).map_err(fail)? {
                let route = pm_render::composite::build::masks(
                    scope,
                    layer,
                    seconds,
                    scaled_world,
                    None,
                    w as u32,
                    h as u32,
                )
                .map_err(fail)?;
                if matches!(route, MaskRoute::Path(_)) {
                    return Err("Bezier path masks".into());
                }
                Some(route)
            } else {
                None
            };
        let device =
            kurbo::Affine::new(scaled_world) * kurbo::Affine::translate((offset.x, offset.y));
        if ty == "image" || ty == "precomp" {
            // Image quads are centered on the layer origin (ax=w/2,
            // ay=h/2); solids below use the top-left local origin.
            let rect = if ty == "precomp" {
                kurbo::Rect::new(0.0, 0.0, sw, sh)
            } else {
                kurbo::Rect::new(-sw / 2.0, -sh / 2.0, sw / 2.0, sh / 2.0)
            };
            let coverage = fill(
                &(device * kurbo::Shape::to_path(&rect, 1e-9)),
                FillRule::NonZero,
                PATH_SAMPLES,
                w,
                h,
            );
            let asset_id = pm_render::js::string(pm_render::js::get(&d, "asset"));
            let record = root.get("assets").and_then(|v| v.get(&asset_id));
            let path = assets.path(&asset_id, record);
            let bitmap = precomp.as_ref().or_else(|| {
                path.as_ref().and_then(|path| {
                    bitmaps
                        .entry(path.clone())
                        .or_insert_with(|| decoded_bitmap(path).ok().flatten())
                        .as_ref()
                })
            });
            let [a, b, c, dd, e, f] = device.as_coeffs();
            let det = a * dd - b * c;
            let natural = record.map(|r| {
                (
                    node::to_number(node::member(Some(r), "w")),
                    node::to_number(node::member(Some(r), "h")),
                )
            });
            let fit = pm_render::js::string(pm_render::js::get(&d, "fit"));
            let model = bitmap.and_then(|bitmap| {
                let source = natural.unwrap_or((bitmap.width as f64, bitmap.height as f64));
                let uv = if ty == "precomp" {
                    Some([0.0, 0.0, 1.0, 1.0])
                } else {
                    image_uv(&fit, source, (sw, sh))
                }?;
                let du = uv[2] * bitmap.width as f64 / sw;
                let dv = uv[3] * bitmap.height as f64 / sh;
                let dx = (du * dd / det, -dv * b / det);
                let dy = (-du * c / det, dv * a / det);
                (det.is_finite()
                    && det.abs() > 1e-12
                    && Bitmap::mip_zero(dx, dy)
                    && (ty == "precomp"
                        || record.is_some_and(|r| pm_render::footage::interpretation(r).is_none()))
                    && !pm_render::sampling::is_level2(layer))
                .then_some((bitmap, uv, dx, dy))
            });
            if let Some((bitmap, uv, dx, dy)) = model {
                let mode = reference_blend(
                    pm_render::composite::build::blend_mode(scope, layer, seconds).map_err(fail)?,
                );
                for y in coverage.y0..coverage.y0 + coverage.height {
                    for x in coverage.x0..coverage.x0 + coverage.width {
                        let k = coverage.at(x, y);
                        if k <= 0.0 {
                            continue;
                        }
                        let p = (x as f64 + 0.5 - e, y as f64 + 0.5 - f);
                        let local_x = (dd * p.0 - c * p.1) / det;
                        let local_y = (-b * p.0 + a * p.1) / det;
                        let u = (uv[0]
                            + uv[2] * (local_x / sw + if ty == "precomp" { 0.0 } else { 0.5 }))
                        .clamp(0.0, 1.0)
                            * bitmap.width as f64;
                        let v = (uv[1]
                            + uv[3] * (local_y / sh + if ty == "precomp" { 0.0 } else { 0.5 }))
                        .clamp(0.0, 1.0)
                            * bitmap.height as f64;
                        let (mask, mask_edge) = masks.as_ref().map_or((1.0, false), |m| {
                            mask_coverage(m, (x as f64 + 0.5, y as f64 + 0.5))
                        });
                        if mask <= 0.0 {
                            continue;
                        }
                        let index = y * w + x;
                        let (matte_value, matte_edge) = matte_strength(&matte, index);
                        let src = scale(
                            bitmap.sample((u, v), dx, dy),
                            opacity * k * mask * matte_value,
                        );
                        if mode == Blend::Normal && src[3] >= 1.0 - 1e-12 {
                            out.edge[index] = false;
                        }
                        if k < 1.0 - 1e-12 || mask_edge || matte_edge {
                            out.edge[index] = true;
                        }
                        out.resampled[index] = true;
                        let px = &mut out.rgba[index * 4..][..4];
                        let dst = [px[0], px[1], px[2], px[3]];
                        px.copy_from_slice(&blend(mode, src, dst));
                    }
                }
            } else {
                out.unmodelled.push(format!(
                    "image {asset_id}: mapping or source outside f64 mip-zero model"
                ));
                let mut inside = vec![false; w * h];
                for y in coverage.y0..coverage.y0 + coverage.height {
                    for x in coverage.x0..coverage.x0 + coverage.width {
                        inside[y * w + x] = coverage.at(x, y) > 0.0;
                    }
                }
                let grown = pm_reference::compare::dilate(&inside, w, h, 1);
                for (i, present) in grown.into_iter().enumerate() {
                    if present {
                        out.rgba[i * 4..][..4].fill(f64::NAN);
                    }
                }
            }
            continue;
        }
        let paint =
            hex(&pm_render::js::string(pm_render::js::get(&d, "color"))).ok_or("solid color")?;
        // Level 2 honors `radius` (at most half of either side); level 1
        // ignores it, as shipped (decision SOLID-RADIUS-1 is open).
        let radius = pm_render::js::get(&d, "radius").to_number();
        let radius = if pm_render::vector::level2(layer) && radius > 0.0 {
            radius.min(sw.abs() / 2.0).min(sh.abs() / 2.0)
        } else {
            0.0
        };
        let outline =
            kurbo::Shape::to_path(&kurbo::RoundedRect::new(0.0, 0.0, sw, sh, radius), 1e-9);
        let coverage = fill(&(device * outline), FillRule::NonZero, PATH_SAMPLES, w, h);
        let mode = reference_blend(
            pm_render::composite::build::blend_mode(scope, layer, seconds).map_err(fail)?,
        );
        let a = paint[3] * opacity;
        let src: Rgba = [paint[0] * a, paint[1] * a, paint[2] * a, a];
        for y in coverage.y0..coverage.y0 + coverage.height {
            for x in coverage.x0..coverage.x0 + coverage.width {
                let k = coverage.at(x, y);
                if k <= 0.0 {
                    continue;
                }
                let (mask, mask_edge) = masks.as_ref().map_or((1.0, false), |m| {
                    mask_coverage(m, (x as f64 + 0.5, y as f64 + 0.5))
                });
                if mask <= 0.0 {
                    continue;
                }
                let index = y * w + x;
                let (matte_value, matte_edge) = matte_strength(&matte, index);
                let source = scale(src, k * mask * matte_value);
                if mode == Blend::Normal && source[3] >= 1.0 - 1e-12 {
                    out.edge[index] = false;
                    out.resampled[index] = false;
                }
                if k < 1.0 - 1e-12 || mask_edge || matte_edge {
                    out.edge[index] = true;
                }
                let px = &mut out.rgba[index * 4..][..4];
                let dst = [px[0], px[1], px[2], px[3]];
                px.copy_from_slice(&blend(mode, source, dst));
            }
        }
    }
    Ok(())
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
        let f = renderer
            .render(document, composition, time, &r, resolver)
            .unwrap();
        renderer.read_rgba(&f).unwrap()
    };
    (frame(&preview), frame(&export))
}

#[cfg(test)]
mod tests {
    use super::super::doc;
    use super::*;
    use serde_json::json;

    fn authored_graph_frame(mut project: serde_json::Value, width: u32, height: u32) -> Reference {
        project["colorDepth"] = json!(32);
        let loaded = pm_model::json::parse(&project.to_string()).expect("authored graph JSON");
        let document = Document::new(Some(loaded), 1);
        render(
            &document,
            &pm_render::MapResolver::default(),
            "Ptest-root",
            super::super::ticks(0.0),
            width,
            height,
            false,
        )
    }

    fn pixel(frame: &Reference, x: usize, y: usize) -> [f64; 4] {
        let i = (y * frame.width + x) * 4;
        frame.rgba[i..i + 4].try_into().expect("RGBA pixel")
    }

    fn assert_pixel(actual: [f64; 4], expected: [f64; 4]) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() < 1e-12, "{actual:?} != {expected:?}");
        }
    }

    fn solid(id: &str, color: &str, width: u32, height: u32, x: f64, y: f64) -> serde_json::Value {
        doc::layer(
            id,
            "solid",
            json!({ "color": color, "w": width, "h": height }),
            x,
            y,
            1.0,
        )
    }

    #[test]
    fn graph_reference_composites_group_once_before_opacity_and_blend() {
        // Top green completely covers lower red within the group. A 50%
        // multiply group over gray therefore has no red from the child.
        let mut top = solid("top", "#00FF00", 4, 4, 0.0, 0.0);
        top["group"] = json!("g");
        let mut lower = solid("lower", "#FF0000", 4, 4, 0.0, 0.0);
        lower["group"] = json!("g");
        let mut group = doc::layer("g", "group", json!({}), 0.0, 0.0, 1.0);
        group["blend"] = json!("multiply");
        doc::set(&mut group, "opacity", json!(50));
        let back = solid("back", "#808080", 4, 4, 0.0, 0.0);
        let frame = authored_graph_frame(
            doc::project(4, 4, 1.0, doc::no_fill(), vec![top, lower, group, back]),
            4,
            4,
        );
        assert_eq!(frame.modelled(), 1.0, "{:?}", frame.unmodelled);
        assert_pixel(
            pixel(&frame, 2, 2),
            [64.0 / 255.0, 128.0 / 255.0, 64.0 / 255.0, 1.0],
        );
    }

    #[test]
    fn graph_reference_paints_child_before_parent_precomp_blend() {
        let top = solid("top", "#00FF00", 4, 4, 0.0, 0.0);
        let lower = solid("lower", "#FF0000", 4, 4, 0.0, 0.0);
        let mut nested = doc::layer(
            "nested",
            "precomp",
            json!({ "comp": "C001", "w": 4, "h": 4 }),
            0.0,
            0.0,
            1.0,
        );
        nested["blend"] = json!("multiply");
        doc::set(&mut nested, "opacity", json!(50));
        let back = solid("back", "#808080", 4, 4, 0.0, 0.0);
        let mut project = doc::project(4, 4, 1.0, doc::no_fill(), vec![nested, back]);
        doc::composition(&mut project, "C001", 4, 4, 1.0, vec![top, lower]);
        let frame = authored_graph_frame(project, 4, 4);
        assert_eq!(frame.modelled(), 1.0, "{:?}", frame.unmodelled);
        // Opaque green hides red in C001; the parent applies multiply at 50%.
        assert_pixel(
            pixel(&frame, 2, 2),
            [64.0 / 255.0, 128.0 / 255.0, 64.0 / 255.0, 1.0],
        );
    }

    #[test]
    fn graph_reference_matte_source_uses_fractional_coverage_and_is_hidden() {
        // The source spans x=.5..2.5: [half, full, half, empty] coverage.
        // It is green with 128/255 alpha, so encoded Rec.709 luma is
        // 0.7152 times its premultiplied alpha.
        for (mode, luma, inverted) in [
            ("alpha", false, false),
            ("alpha-inverted", false, true),
            ("luma", true, false),
            ("luma-inverted", true, true),
        ] {
            let mut target = solid("target", "#FF0000", 4, 2, 0.0, 0.0);
            target["matteSource"] = json!("matte");
            target["matteMode"] = doc::ch(json!(mode));
            let matte = solid("matte", "#00FF0080", 2, 2, 0.5, 0.0);
            let frame = authored_graph_frame(
                doc::project(4, 2, 1.0, doc::no_fill(), vec![target, matte]),
                4,
                2,
            );
            assert_eq!(frame.modelled(), 1.0, "{mode}: {:?}", frame.unmodelled);
            for (x, coverage) in [0.5, 1.0, 0.5, 0.0].into_iter().enumerate() {
                let source = coverage * (128.0 / 255.0) * if luma { 0.7152 } else { 1.0 };
                let alpha = if inverted { 1.0 - source } else { source };
                assert_pixel(pixel(&frame, x, 0), [alpha, 0.0, 0.0, alpha]);
            }
        }
    }

    #[test]
    fn fit_windows_preserve_authored_cover_and_contain_geometry() {
        assert_eq!(
            image_uv("cover", (2.0, 1.0), (1.0, 1.0)),
            Some([0.25, 0.0, 0.5, 1.0])
        );
        assert_eq!(
            image_uv("contain", (2.0, 1.0), (1.0, 1.0)),
            Some([0.0, -0.5, 1.0, 2.0])
        );
        assert_eq!(
            image_uv("fill", (2.0, 1.0), (1.0, 1.0)),
            Some([0.0, 0.0, 1.0, 1.0])
        );
    }

    #[test]
    fn mask_coverage_applies_analytic_solid_and_feather_values() {
        let one = pm_vector::mask::ParametricMask {
            center: kurbo::Point::new(10.0, 10.0),
            size: (10.0, 8.0),
            rotation: 0.0,
            feather: 4.0,
            shape: pm_vector::MaskShape::Rect,
            mode: pm_vector::MaskMode::Add,
        };
        let route = MaskRoute::Parametric(pm_render::composite::Masks {
            inverse: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            masks: pm_vector::ParametricMasks {
                masks: vec![one],
                has_add: true,
            },
            path: None,
        });
        assert_eq!(mask_coverage(&route, (10.0, 10.0)).0, 1.0);
        assert_eq!(mask_coverage(&route, (30.0, 10.0)).0, 0.0);
        assert_eq!(mask_coverage(&route, (15.0, 10.0)).0, 0.5);
    }
}
