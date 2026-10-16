//! The render graph of one frame, built from the document through
//! `pm-eval` (plan section 8, "Render graph").
//!
//! [`build`] walks a composition the way the legacy compositor's
//! `renderProject` does (`legacy/gl/compositor.ts`): the compositing pass of
//! the composition or of one group (non-boundary groups flattened, boundary
//! groups as one entry), bottom to top, with today's skip rules (matte
//! sources, solo, non-visual types, inactive layers, opacity ≤ 0.001), and
//! for each drawn layer the content quad today's `contentQuad` computes:
//! size, anchor, UV window, and the layer's world matrix scaled to output
//! pixels. Every number keeps the TypeScript expression order, so the
//! matrices the GPU receives equal today's after the same f32 conversion.
//!
//! The plan is pure data: the executor (`gpu`) prepares sources and draws
//! it; the same document, composition, time, and request always give the
//! same plan.

use std::path::PathBuf;

use pm_eval::groups::ActiveTransition;
use pm_eval::node;
use pm_eval::retime::RetimeCache;
use pm_eval::transform::{Mat2d, mul};
use pm_eval::{Comp, Evaluator, JsResult, JsValue, Scope};
use pm_model::{JsString, Node};

use crate::assets::AssetResolver;
use crate::capability::{self, Feature, Unsupported};
use crate::error::LayerFailure;
use crate::js;
use crate::scene3d::Object3dPlan;

/// One composition (or one group's members) rendered into a target.
#[derive(Debug, Clone, PartialEq)]
pub struct CompPlan {
    pub composition: String,
    /// A group's compositing boundary: drawn 1:1 in its parent's pixel
    /// space (a precomp is sampled through its layer's transform).
    pub group: bool,
    /// Target size in pixels (the full output of this level).
    pub width: u32,
    pub height: u32,
    pub background: Background,
    /// Bottom to top.
    pub draws: Vec<Draw>,
}

/// How a target starts.
#[derive(Debug, Clone, PartialEq)]
pub enum Background {
    Transparent,
    /// Premultiplied clear color.
    Solid([f64; 4]),
    /// Opaque black, then the gradient (`FRAG_BACKGROUND_FILL`).
    Gradient(Gradient),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub radial: bool,
    pub angle_degrees: f64,
    /// RGB (E) and position 0..1, at most 8, sorted.
    pub stops: Vec<([f64; 3], f64)>,
}

/// One quad drawn into a target with premultiplied source-over.
#[derive(Debug, Clone, PartialEq)]
pub struct Draw {
    /// The layer this draw belongs to at the top of the frame (the depth
    /// and id surface reports it).
    pub layer: String,
    /// Unit quad to target pixels, `[a, b, c, d, e, f]` (x' = a·x + c·y + e).
    pub matrix: Mat2d,
    pub content: Content,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// A premultiplied color.
    Solid([f64; 4]),
    /// Solid or texture content drawn through a 2.5D plane or with level 2
    /// sampling (`crate::space25d`, `crate::sampling`).
    Placed(Box<Placed>),
    /// A source sampled through a UV window (`u_uv`: offset, scale), hard
    /// edges outside 0..1, multiplied by `alpha`.
    Texture {
        source: Source,
        uv: [f64; 4],
        alpha: f64,
    },
}

/// Content with a projective placement or a level 2 resampler.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// [`Content::Solid`] or [`Content::Texture`].
    pub content: Content,
    /// A 2.5D layer's homography: unit quad to target pixels, column-major
    /// (replaces the draw's affine matrix for drawing).
    pub plane: Option<crate::space25d::Plane>,
    /// Level 2 raster content: the resampler and analytic edges.
    pub level2: Option<crate::sampling::Sampling>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Image {
        asset: String,
        path: PathBuf,
    },
    Video {
        asset: String,
        path: PathBuf,
        /// Source time in seconds after retiming and today's end clamp.
        time: f64,
        /// Interpret Footage frame rate override (`interpret.frameRate`):
        /// frame n shows at n / rate.
        rate: Option<pm_time::Rate>,
    },
    /// A nested composition (precomp) or a group's compositing boundary.
    Comp(Box<CompPlan>),
    /// The 3D scene pass output for one object layer.
    Object3d(Box<Object3dPlan>),
    /// Motion-blur samples accumulated with additive blending into a
    /// transparent target in the parent's pixel space (`crate::motion`).
    Accumulate(Box<CompPlan>),
}

/// Inputs and outputs of a build.
pub struct Build<'r> {
    pub assets: &'r dyn AssetResolver,
    pub failures: Vec<LayerFailure>,
    pub unsupported: Vec<Unsupported>,
    pub(crate) retime: RetimeCache,
    precomp_depth: u32,
    /// Motion blur for layers with their switch on; `None` when the request
    /// has it off or inside a group's samples (today's `mblur: false`).
    pub(crate) blur: Option<crate::motion::Blur>,
    /// The composition time at which video sources select their frame
    /// inside motion-blur samples (the center time of their level).
    pub(crate) video_center: Option<f64>,
    /// The resampler of level 2 raster content.
    pub(crate) sampling: crate::sampling::Sampling,
}

/// Options of a build beyond the frame's geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BuildOptions {
    /// Resolved motion blur (`crate::motion::resolve`).
    pub blur: Option<crate::motion::Blur>,
    pub sampling: crate::sampling::Sampling,
}

/// Today's `PC_MAX_DEPTH`.
pub const PRECOMP_MAX_DEPTH: u32 = 6;

/// Builds the plan of `composition` at `time` (seconds) into a target of
/// `width` × `height` at `scale` output pixels per composition pixel.
#[allow(clippy::too_many_arguments)]
pub fn build<'r>(
    ev: &mut Evaluator,
    root_comp: &Comp<'_>,
    project: &Comp<'_>,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    assets: &'r dyn AssetResolver,
) -> (CompPlan, Build<'r>) {
    build_with_options(
        ev,
        root_comp,
        project,
        time,
        width,
        height,
        scale,
        assets,
        BuildOptions::default(),
    )
}

/// [`build`] with motion blur and the level 2 resampler.
#[allow(clippy::too_many_arguments)]
pub fn build_with_options<'r>(
    ev: &mut Evaluator,
    root_comp: &Comp<'_>,
    project: &Comp<'_>,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    assets: &'r dyn AssetResolver,
    options: BuildOptions,
) -> (CompPlan, Build<'r>) {
    let mut b = Build {
        assets,
        failures: Vec::new(),
        unsupported: Vec::new(),
        retime: RetimeCache::new(),
        precomp_depth: 0,
        blur: options.blur,
        video_center: None,
        sampling: options.sampling,
    };
    let mut scope = Scope::new(ev, root_comp, project);
    let plan = comp_pass(&mut scope, &mut b, time, width, height, scale, false, None);
    (plan, b)
}

/// The legacy scale of a composition into `W` × `H`
/// (`W / max(1, Number(comp.w) || W)`).
pub fn derived_scale(comp: &Comp<'_>, width: u32, height: u32) -> (f64, f64) {
    let (w, h) = (f64::from(width), f64::from(height));
    let cw = node::to_number(comp.member("w"));
    let ch = node::to_number(comp.member("h"));
    let or = |v: f64, f: f64| if v.is_nan() || v == 0.0 { f } else { v };
    (w / js::max(1.0, or(cw, w)), h / js::max(1.0, or(ch, h)))
}

fn layer_failure(b: &mut Build<'_>, comp: &Comp<'_>, layer: &Node, message: String) {
    b.failures.push(LayerFailure {
        composition: comp.id().to_string_lossy(),
        layer: js::layer_id(layer),
        message,
    });
}

/// `renderProject(proj, T, W, H, opt)` for one pass.
#[allow(clippy::too_many_arguments)]
pub(crate) fn comp_pass<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    transparent: bool,
    group_parent: Option<&'d Node>,
) -> CompPlan {
    let comp_id = scope.comp.id().to_string_lossy();
    let all: Vec<&'d Node> = scope.layers().iter().collect();
    let background = if transparent {
        Background::Transparent
    } else {
        background(scope.comp)
    };
    let mut plan = CompPlan {
        composition: comp_id.clone(),
        group: group_parent.is_some(),
        width,
        height,
        background,
        draws: Vec::new(),
    };
    let parent_id = group_parent.map(|g| node::get(g, b"id").to_js());
    // `compositingPass`: boundary groups stay one entry.
    let pass = {
        let known = |_: &JsString| true;
        // A group with motion blur is a boundary only while motion blur
        // applies (`opt.mblur !== false`).
        let blur_on = b.blur.is_some();
        let mut boundary = |s: &mut Scope<'_, 'd>, l: &'d Node| {
            s.group_creates_compositing_boundary(l, time, blur_on, &known)
        };
        match scope.compositing_layers(&all, parent_id.as_ref(), &mut boundary) {
            Ok(p) => p,
            Err(e) => {
                b.failures.push(LayerFailure {
                    composition: comp_id,
                    layer: group_parent.map(js::layer_id).unwrap_or_default(),
                    message: e.to_string(),
                });
                return plan;
            }
        }
    };
    // `compositionOrderedLayers`: 2.5D runs back to front.
    let pass = match crate::space25d::order(scope, pass, time) {
        Ok(p) => p,
        Err(e) => {
            b.failures.push(LayerFailure {
                composition: comp_id,
                layer: group_parent.map(js::layer_id).unwrap_or_default(),
                message: e.to_string(),
            });
            return plan;
        }
    };
    let solo = all.iter().any(|l| js::flag(l, "solo"));
    let matte_sources: Vec<JsValue> = all
        .iter()
        .map(|l| node::get(l, b"matteSource").to_js())
        .collect();
    // Bottom (last) to top (first).
    for &layer in pass.iter().rev() {
        let id = node::get(layer, b"id").to_js();
        if matte_sources.iter().any(|m| same_value_zero(m, &id)) {
            continue;
        }
        match draw_layer(scope, b, layer, time, width, height, scale, solo, &all) {
            Ok(Some(draws)) => plan.draws.extend(draws),
            Ok(None) => {}
            Err(e) => {
                let comp = scope.comp.clone();
                layer_failure(b, &comp, layer, e.to_string());
            }
        }
    }
    plan
}

fn same_value_zero(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Number(x), JsValue::Number(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => pm_eval::js_sem::strict_eq(a, b),
    }
}

/// `groupContainsSolo(group, layers, ancestors)`.
fn group_contains_solo<'d>(
    scope: &mut Scope<'_, 'd>,
    group: &'d Node,
    all: &[&'d Node],
) -> JsResult<bool> {
    if js::layer_type(group) != "group" {
        return Ok(false);
    }
    let gid = node::get(group, b"id").to_js();
    for &l in all {
        if !js::flag(l, "solo") {
            continue;
        }
        for a in scope.group_ancestors(l)? {
            if pm_eval::js_sem::strict_eq(&node::get(a, b"id").to_js(), &gid) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The draws of one pass entry, or `None` when it draws nothing.
#[allow(clippy::too_many_arguments)]
fn draw_layer<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    solo: bool,
    all: &[&'d Node],
) -> JsResult<Option<Vec<Draw>>> {
    let ty = js::layer_type(layer);
    if solo && !js::flag(layer, "solo") {
        let ancestors = scope.group_ancestors(layer)?;
        if !ancestors.iter().any(|g| js::flag(g, "solo"))
            && !group_contains_solo(scope, layer, all)?
        {
            return Ok(None);
        }
    }
    if ty != "group" && matches!(ty.as_str(), "null" | "audio") {
        return Ok(None);
    }
    if !scope.active(layer, time)? {
        return Ok(None);
    }
    let alpha = js::clamp(scope.num(layer, "opacity", time)? / 100.0, 0.0, 1.0);
    if alpha <= 0.001 {
        return Ok(None);
    }
    let comp_id = scope.comp.id().to_string_lossy();
    // Everything that would leave the fast path is unsupported here; the
    // capability check lists the same features statically.
    let mut refuse = |f: Feature| {
        b.unsupported.push(Unsupported::layer(&comp_id, layer, f));
    };
    if scope.has_renderable_effects(layer, time)? {
        refuse(if ty == "adjustment" {
            Feature::Adjustment
        } else {
            Feature::Effects
        });
        return Ok(None);
    }
    if ty == "adjustment" {
        // An adjustment layer without an enabled effect is a no-op.
        return Ok(None);
    }
    let mut masked = false;
    if let Some(masks) = node::array(node::member(Some(layer), "masks")) {
        for m in masks {
            let mid = node::to_js_string(node::member(Some(m), "id")).to_string_lossy();
            let on = scope.evaluated(
                layer,
                node::member(Some(m), "on"),
                time,
                &format!("m.{mid}.on"),
            )?;
            if on != JsValue::Bool(false) {
                masked = true;
            }
        }
    }
    if masked {
        refuse(Feature::Masks);
        return Ok(None);
    }
    let blend = scope.evaluated(layer, node::member(Some(layer), "blend"), time, "l.blend")?;
    if blend.truthy() && blend != JsValue::from("normal") {
        let name = blend.to_js_string().to_string_lossy();
        // Unknown names map to normal (`BLEND_ID[...] || 0`).
        if matches!(
            name.as_str(),
            "add"
                | "screen"
                | "multiply"
                | "overlay"
                | "softlight"
                | "difference"
                | "lighten"
                | "darken"
        ) {
            refuse(Feature::BlendMode(name));
            return Ok(None);
        }
    }
    let mblur = scope.evaluated(layer, node::member(Some(layer), "mblur"), time, "l.mblur")?;
    let blur = if mblur.truthy() { b.blur } else { None };
    let transition: Option<ActiveTransition<'d>> =
        scope.active_transition(layer, time, &|_| true)?;
    if transition.is_some() {
        refuse(Feature::Transition);
        return Ok(None);
    }
    if js::flag(layer, "matteSource") {
        refuse(Feature::TrackMatte);
        return Ok(None);
    }
    if ty == "group" {
        if let Some(blur) = blur {
            // The group's members at every sample time, accumulated.
            let draw = crate::motion::plan::blur_group(
                scope, b, layer, time, width, height, scale, alpha, blur,
            );
            return Ok(Some(vec![draw]));
        }
        // A boundary group (only opacity can make it one here): its members
        // composite into their own target, drawn with the group's opacity.
        let inner = comp_pass(scope, b, time, width, height, scale, true, Some(layer));
        let id = js::layer_id(layer);
        return Ok(Some(vec![Draw {
            layer: id,
            matrix: [f64::from(width), 0.0, 0.0, f64::from(height), 0.0, 0.0],
            content: Content::Texture {
                source: Source::Comp(Box::new(inner)),
                uv: [0.0, 0.0, 1.0, 1.0],
                alpha,
            },
        }]));
    }
    if let Some(blur) = blur {
        let draw = crate::motion::plan::blur_layer(
            scope, b, layer, time, width, height, scale, alpha, blur,
        )?;
        return Ok(draw.map(|d| vec![d]));
    }
    Ok(layer_draw(scope, b, layer, time, width, height, scale, alpha)?.map(|d| vec![d]))
}

/// `drawContent(L, T, W, H, alpha)`: the layer's content quad at `time`
/// through its world matrix, or its 2.5D plane, with `alpha` folded in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layer_draw<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    alpha: f64,
) -> JsResult<Option<Draw>> {
    let Some(quad) = content_quad(scope, b, layer, time, width, height, scale)? else {
        return Ok(None);
    };
    let world = scaled_world(scope, layer, time, scale)?;
    let id = js::layer_id(layer);
    let ty = js::layer_type(layer);
    let level2 = (crate::sampling::is_level2(layer) && crate::sampling::samples_layer_type(&ty))
        .then_some(b.sampling);
    let draw = match quad {
        Quad::Solid {
            rgb,
            a,
            w,
            h,
            ax,
            ay,
        } => {
            let alpha = alpha * a;
            let plane = crate::space25d::projected(scope, layer, time, scale, w, h, ax, ay)?;
            Draw {
                layer: id,
                matrix: mul(&world, &[w, 0.0, 0.0, h, -ax, -ay]),
                content: placed(
                    Content::Solid([rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha]),
                    plane,
                    None,
                ),
            }
        }
        Quad::Texture {
            source,
            w,
            h,
            ax,
            ay,
            uv,
        } => {
            let plane = crate::space25d::projected(scope, layer, time, scale, w, h, ax, ay)?;
            Draw {
                layer: id,
                matrix: mul(&world, &[w, 0.0, 0.0, h, -ax, -ay]),
                content: placed(Content::Texture { source, uv, alpha }, plane, level2),
            }
        }
        Quad::Screen { source } => Draw {
            layer: id,
            matrix: [f64::from(width), 0.0, 0.0, f64::from(height), 0.0, 0.0],
            content: Content::Texture {
                source,
                uv: [0.0, 0.0, 1.0, 1.0],
                alpha,
            },
        },
    };
    Ok(Some(draw))
}

/// `content` as it is, or wrapped with its plane and resampler.
fn placed(
    content: Content,
    plane: Option<crate::space25d::Plane>,
    level2: Option<crate::sampling::Sampling>,
) -> Content {
    if plane.is_none() && level2.is_none() {
        return content;
    }
    Content::Placed(Box::new(Placed {
        content,
        plane,
        level2,
    }))
}

/// `scaledWorld(L, T, W, H)` without a preview viewport.
fn scaled_world<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
) -> JsResult<Mat2d> {
    let w = scope.world_matrix(layer, time)?;
    let (sx, sy) = scale;
    Ok([
        w[0] * sx,
        w[1] * sy,
        w[2] * sx,
        w[3] * sy,
        w[4] * sx,
        w[5] * sy,
    ])
}

enum Quad {
    Solid {
        rgb: [f64; 3],
        a: f64,
        w: f64,
        h: f64,
        ax: f64,
        ay: f64,
    },
    Texture {
        source: Source,
        w: f64,
        h: f64,
        ax: f64,
        ay: f64,
        uv: [f64; 4],
    },
    /// Screen-space content covering the whole target (3D layers).
    Screen { source: Source },
}

/// `contentQuad(L, T, W, H)`: `nativeContentQuad` plus the size anchor.
fn content_quad<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
) -> JsResult<Option<Quad>> {
    let quad = native_content_quad(scope, b, layer, time, width, height, scale)?;
    let Some(mut quad) = quad else {
        return Ok(None);
    };
    if matches!(quad, Quad::Screen { .. }) {
        return Ok(Some(quad));
    }
    let offset = scope.size_anchor_offset(layer, time)?;
    if offset.x != 0.0 || offset.y != 0.0 {
        match &mut quad {
            Quad::Solid { ax, ay, .. } | Quad::Texture { ax, ay, .. } => {
                *ax -= offset.x;
                *ay -= offset.y;
            }
            Quad::Screen { .. } => {}
        }
    }
    Ok(Some(quad))
}

#[allow(clippy::too_many_arguments)]
fn native_content_quad<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
) -> JsResult<Option<Quad>> {
    let ty = js::layer_type(layer);
    let comp_id = scope.comp.id().to_string_lossy();
    let (wf, hf) = (f64::from(width), f64::from(height));
    let d = scope.ev.resolve_content(scope.comp, layer, time)?;
    match ty.as_str() {
        "solid" => {
            let color = js::string(js::get(&d, "color"));
            let c = pm_color::standard::parse_hex(&color);
            Ok(Some(Quad::Solid {
                rgb: c.rgb,
                a: c.alpha,
                w: js::or_number(js::get(&d, "w"), wf),
                h: js::or_number(js::get(&d, "h"), hf),
                ax: 0.0,
                ay: 0.0,
            }))
        }
        "image" | "video" => {
            let asset_id = js::string(js::get(&d, "asset"));
            let root = scope.comp.root();
            let Some(record) = node::member(node::member(Some(root), "assets"), &asset_id) else {
                return Ok(None);
            };
            let Some(path) = b.assets.path(&asset_id, Some(record)) else {
                b.failures.push(LayerFailure {
                    composition: comp_id,
                    layer: js::layer_id(layer),
                    message: format!("media for asset {asset_id} is offline"),
                });
                return Ok(None);
            };
            let rec = |k: &str| node::js_value_opt(node::member(Some(record), k));
            let format = js::string(&rec("format")).to_ascii_lowercase();
            if ty == "image" && format == "svg" {
                b.unsupported
                    .push(Unsupported::layer(&comp_id, layer, Feature::SvgImage));
                return Ok(None);
            }
            if ty == "video"
                && (rec("imageSequence").truthy()
                    || matches!(format.as_str(), "gif" | "webp" | "png" | "apng" | "avif"))
            {
                b.unsupported
                    .push(Unsupported::layer(&comp_id, layer, Feature::ImageSequence));
                return Ok(None);
            }
            let sw = js::or_number(&rec("w"), 1.0);
            let sh = js::or_number(&rec("h"), 1.0);
            let bw = js::or_number(js::get(&d, "w"), wf);
            let bh = js::or_number(js::get(&d, "h"), hf);
            let fit = js::string(js::get(&d, "fit"));
            let uv = fit_uv(&fit, sw, sh, bw, bh);
            let source = if ty == "video" {
                // Inside motion-blur samples video keeps the center time's
                // frame (`crate::motion`).
                let at = b.video_center.unwrap_or(time);
                let st = scope.source_time(layer, at, Some(&mut b.retime))?;
                let rate = crate::retime::rate_override(record);
                let time = match rate {
                    // The file's frames on the overridden grid; the last
                    // frame clamps when the media is opened.
                    Some(_) => js::max(0.0, st),
                    None => playback_time(st, rec("dur").to_number()),
                };
                Source::Video {
                    asset: asset_id,
                    path,
                    time,
                    rate,
                }
            } else {
                Source::Image {
                    asset: asset_id,
                    path,
                }
            };
            Ok(Some(Quad::Texture {
                source,
                w: bw,
                h: bh,
                ax: bw / 2.0,
                ay: bh / 2.0,
                uv,
            }))
        }
        "precomp" => {
            if b.precomp_depth >= PRECOMP_MAX_DEPTH {
                return Ok(None);
            }
            let root = scope.comp.root();
            let Some((sub_id, _)) = capability::precomp_target(root, layer) else {
                return Ok(None);
            };
            let cw = node::to_number(scope.comp.member("w"));
            let ch = node::to_number(scope.comp.member("h"));
            let w = js::max(1.0, or_chain(js::get(&d, "w"), cw, wf));
            let hh = js::max(1.0, or_chain(js::get(&d, "h"), ch, hf));
            let (tw, th) = if crate::sampling::is_level2(layer) {
                // Continuous rasterization (section 8): the child renders
                // at the density it is drawn at, never below its own size.
                let density = crate::sampling::precomp_density(scope, layer, time, scale, w, hh)?;
                (
                    js::max(2.0, js::round(w * js::max(scale.0, density.0))),
                    js::max(2.0, js::round(hh * js::max(scale.1, density.1))),
                )
            } else {
                (
                    js::max(2.0, js::round(w * scale.0)),
                    js::max(2.0, js::round(hh * scale.1)),
                )
            };
            if !(tw.is_finite() && th.is_finite()) || tw > 16384.0 || th > 16384.0 {
                b.failures.push(LayerFailure {
                    composition: comp_id,
                    layer: js::layer_id(layer),
                    message: format!("precomp target {tw}×{th} is outside 2..16384"),
                });
                return Ok(None);
            }
            let (tw, th) = (tw as u32, th as u32);
            let sub = match Comp::in_root(root, &sub_id, scope.comp.revision()) {
                Ok(c) => c,
                Err(_) => return Ok(None),
            };
            let child_time = scope.source_time(layer, time, Some(&mut b.retime))?;
            let child_time = crate::retime::keep_frame_rate(layer, &sub, child_time);
            // The child's video frames follow the center time of this level
            // inside motion-blur samples.
            let outer_center = b.video_center;
            if let Some(center) = outer_center {
                let c = scope.source_time(layer, center, Some(&mut b.retime))?;
                b.video_center = Some(crate::retime::keep_frame_rate(layer, &sub, c));
            }
            let inner_scale = derived_scale(&sub, tw, th);
            b.precomp_depth += 1;
            let project = scope.project;
            let inner = {
                let mut inner_scope = Scope::new(&mut *scope.ev, &sub, project);
                comp_pass(
                    &mut inner_scope,
                    b,
                    child_time,
                    tw,
                    th,
                    inner_scale,
                    true,
                    None,
                )
            };
            b.precomp_depth -= 1;
            b.video_center = outer_center;
            Ok(Some(Quad::Texture {
                source: Source::Comp(Box::new(inner)),
                w,
                h: hh,
                ax: 0.0,
                ay: 0.0,
                uv: [0.0, 0.0, 1.0, 1.0],
            }))
        }
        "extension" => {
            let def = js::string(js::get(&d, "definition"));
            match pm_eval::parenting::Role3d::of(layer) {
                Some(pm_eval::parenting::Role3d::Object) => {
                    match crate::scene3d::plan_object(scope, layer, time, width, height) {
                        Ok(Ok(plan)) => Ok(Some(Quad::Screen {
                            source: Source::Object3d(Box::new(plan)),
                        })),
                        Ok(Err(reason)) => {
                            b.unsupported.push(Unsupported::layer(
                                &comp_id,
                                layer,
                                Feature::Scene3d(reason),
                            ));
                            Ok(None)
                        }
                        Err(e) => Err(e),
                    }
                }
                Some(_) => Ok(None),
                None => {
                    b.unsupported.push(Unsupported::layer(
                        &comp_id,
                        layer,
                        Feature::ExtensionDefinition(def),
                    ));
                    Ok(None)
                }
            }
        }
        "null" | "audio" | "group" => Ok(None),
        other => {
            b.unsupported.push(Unsupported::layer(
                &comp_id,
                layer,
                Feature::LayerType(other.to_owned()),
            ));
            Ok(None)
        }
    }
}

/// `Math.max(1, Number(d.w || comp.w || W))` before the `max`.
fn or_chain(v: &JsValue, comp: f64, fallback: f64) -> f64 {
    if v.truthy() {
        v.to_number()
    } else if comp != 0.0 && !comp.is_nan() {
        comp
    } else {
        fallback
    }
}

/// The UV window for `fit` (`cover`, `contain`, anything else stretches).
pub fn fit_uv(fit: &str, sw: f64, sh: f64, bw: f64, bh: f64) -> [f64; 4] {
    let ar = sw / sh;
    let br = bw / bh;
    match fit {
        "cover" => {
            if ar > br {
                let s = br / ar;
                [(1.0 - s) / 2.0, 0.0, s, 1.0]
            } else {
                let s = ar / br;
                [0.0, (1.0 - s) / 2.0, 1.0, s]
            }
        }
        "contain" => {
            if ar > br {
                let s = ar / br;
                [0.0, (1.0 - s) / 2.0, 1.0, s]
            } else {
                let s = br / ar;
                [(1.0 - s) / 2.0, 0.0, s, 1.0]
            }
        }
        _ => [0.0, 0.0, 1.0, 1.0],
    }
}

/// `videoPlaybackTime(asset, time)` for container sources: clamped to
/// `[0, dur − 0.04]`. The engine then selects the frame by section 5 rule 7
/// (today's microsecond rounding only steers Chromium's seek to the same
/// frame).
pub use crate::retime::playback_time;

/// `PM.normalizeFill(comp.backgroundFill, comp.bg)` and the clear it gives.
pub fn background(comp: &Comp<'_>) -> Background {
    let fill = normalize_fill(comp.member("backgroundFill"), comp.member("bg"));
    match fill.kind.as_str() {
        "none" => Background::Transparent,
        "solid" => {
            let c = pm_color::standard::parse_hex(&fill.stops[0].0);
            let a = c.alpha;
            Background::Solid([c.rgb[0] * a, c.rgb[1] * a, c.rgb[2] * a, a])
        }
        kind => Background::Gradient(Gradient {
            radial: kind == "radial",
            angle_degrees: fill.angle,
            stops: fill
                .stops
                .iter()
                .map(|(color, pos)| (pm_color::standard::parse_hex(color).rgb, pos / 100.0))
                .collect(),
        }),
    }
}

/// A normalized fill: type, angle, stops (color string, position 0..100).
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub kind: String,
    pub angle: f64,
    pub stops: Vec<(String, f64)>,
}

fn is_fill_color(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() == 7 || b.len() == 9) && b[0] == b'#' && b[1..].iter().all(u8::is_ascii_hexdigit)
}

/// `PM.normalizeFill(value, fallback = '#000000')`.
pub fn normalize_fill(value: Option<&Node>, bg: Option<&Node>) -> Fill {
    let fallback = match bg.map(|n| node::js_value_opt(Some(n))) {
        None | Some(JsValue::Undefined) => "#000000".to_owned(),
        Some(v) => v.to_js_string().to_string_lossy(),
    };
    let color = |v: Option<&Node>| -> String {
        match v.and_then(|n| node::string(Some(n))) {
            Some(s) if is_fill_color(&s.to_string_lossy()) => s.to_string_lossy().to_uppercase(),
            _ => fallback.clone(),
        }
    };
    let raw = value.filter(|v| matches!(v.kind(), pm_model::NodeKind::Object(_)));
    let kind = node::string(node::member(raw, "type"))
        .map(|s| s.to_string_lossy())
        .filter(|t| matches!(t.as_str(), "solid" | "linear" | "radial" | "none"))
        .unwrap_or_else(|| "solid".to_owned());
    let mut stops: Vec<(String, f64)> = node::array(node::member(raw, "stops"))
        .unwrap_or(&[])
        .iter()
        .take(8)
        .map(|stop| {
            let pos = node::to_number(node::member(Some(stop), "position"));
            let pos = if pos.is_nan() || pos == 0.0 { 0.0 } else { pos };
            (
                color(node::member(Some(stop), "color")),
                js::clamp(pos, 0.0, 100.0),
            )
        })
        .collect();
    // Array.prototype.sort is stable; NaN cannot occur after the clamp of `|| 0`.
    stops.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    if stops.is_empty() {
        let c = node::member(raw, "color").filter(|c| node::truthy(Some(c)));
        let first = match c {
            Some(c) => color(Some(c)),
            None => {
                // `color(raw.color || fallback)`: the fallback string is checked too.
                if is_fill_color(&fallback) {
                    fallback.to_uppercase()
                } else {
                    fallback.clone()
                }
            }
        };
        stops.push((first, 0.0));
    }
    let flat = matches!(kind.as_str(), "solid" | "none");
    if !flat && stops.len() < 2 {
        let c = stops[0].0.clone();
        stops.push((c, 100.0));
    }
    if flat {
        stops = vec![(stops[0].0.clone(), 0.0)];
    }
    let angle = node::to_number(node::member(raw, "angle"));
    let angle = if angle.is_nan() || angle == 0.0 {
        0.0
    } else {
        angle
    };
    Fill {
        kind,
        angle: js::clamp(angle, -180.0, 180.0),
        stops,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_windows_match_the_compositor() {
        // A 2:1 source in a 1:1 box.
        assert_eq!(fit_uv("cover", 2.0, 1.0, 1.0, 1.0), [0.25, 0.0, 0.5, 1.0]);
        assert_eq!(fit_uv("contain", 2.0, 1.0, 1.0, 1.0), [0.0, -0.5, 1.0, 2.0]);
        assert_eq!(fit_uv("fill", 2.0, 1.0, 1.0, 1.0), [0.0, 0.0, 1.0, 1.0]);
        // A 1:2 source in a 1:1 box.
        assert_eq!(fit_uv("contain", 1.0, 2.0, 1.0, 1.0), [-0.5, 0.0, 2.0, 1.0]);
    }

    fn parse(s: &str) -> Node {
        Node::from_value(pm_model::json::parse(s).unwrap(), 0)
    }

    #[test]
    fn fills_normalize_like_the_model() {
        let f = normalize_fill(None, Some(&parse("\"#102030\"")));
        assert_eq!(f.kind, "solid");
        assert_eq!(f.stops, vec![("#102030".into(), 0.0)]);
        let v = parse(
            r##"{"type":"linear","angle":400,"stops":[{"color":"#ff0000","position":80},{"color":"bad","position":20}]}"##,
        );
        let f = normalize_fill(Some(&v), Some(&parse("\"#000000\"")));
        assert_eq!(f.angle, 180.0);
        assert_eq!(
            f.stops,
            vec![("#000000".into(), 20.0), ("#FF0000".into(), 80.0)]
        );
        let v = parse(r##"{"type":"radial","stops":[{"color":"#00ff00"}]}"##);
        let f = normalize_fill(Some(&v), None);
        assert_eq!(f.stops.len(), 2);
        assert_eq!(f.stops[1], ("#00FF00".into(), 100.0));
        let v = parse(r##"{"type":"weird"}"##);
        assert_eq!(normalize_fill(Some(&v), None).kind, "solid");
    }
}
