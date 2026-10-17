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
use pm_protocol::color::BlendMode;

use crate::assets::AssetResolver;
use crate::capability::{self, Feature, Unsupported};
use crate::composite::{self, EffectHost};
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
    /// A layer through its own target or an adjustment of the accumulator
    /// (masks, effects, matte, blend mode; `crate::composite`). Its draw's
    /// matrix covers the target.
    Composite(Box<crate::composite::Node>),
    /// A source sampled through a UV window (`u_uv`: offset, scale), hard
    /// edges outside 0..1, multiplied by `alpha`.
    Texture {
        source: Source,
        uv: [f64; 4],
        alpha: f64,
    },
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
    },
    /// A nested composition (precomp) or a group's compositing boundary.
    Comp(Box<CompPlan>),
    /// The 3D scene pass output for one object layer.
    Object3d(Box<Object3dPlan>),
    /// Shape content (`crate::vector`).
    Vector(Box<crate::vector::VectorSource>),
    /// A shader layer or an extension fragment layer
    /// (`crate::shader_layers`).
    Shader(Box<crate::shader_layers::ShaderSource>),
}

/// Inputs and outputs of a build.
pub struct Build<'r> {
    pub assets: &'r dyn AssetResolver,
    pub failures: Vec<LayerFailure>,
    pub unsupported: Vec<Unsupported>,
    retime: RetimeCache,
    precomp_depth: u32,
    /// The 3D state of the compositing passes being built (innermost last).
    pass3d: Vec<crate::scene3d::Pass3d>,
    options3d: crate::scene3d::Options3d,
    /// Prepares effects (`crate::composite::EffectHost`); without one,
    /// layers with renderable effects are unsupported.
    effects: Option<&'r dyn EffectHost>,
    /// Today's `opt.matteDepth`: matte passes entered so far (carried into
    /// precomps, as `PM._renderOptions3D` carries it).
    matte_depth: u32,
    /// The matte source a matte pass renders, which draws even when it is
    /// switched off (`{ ...source, on: true }`).
    forced_on: Option<*const Node>,
    /// A decision 19 fallback runner draws programs the native pipeline
    /// cannot (`crate::shader_layers::fallback`); without one they route.
    shader_fallback: bool,
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
    build_with_effects(
        ev, root_comp, project, time, width, height, scale, assets, None,
    )
}

/// [`build`] with an effect host for layers with effects and adjustment
/// layers.
#[allow(clippy::too_many_arguments)]
pub fn build_with_effects<'r>(
    ev: &mut Evaluator,
    root_comp: &Comp<'_>,
    project: &Comp<'_>,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    assets: &'r dyn AssetResolver,
    effects: Option<&'r dyn EffectHost>,
) -> (CompPlan, Build<'r>) {
    build_with(
        ev,
        root_comp,
        project,
        time,
        width,
        height,
        scale,
        assets,
        effects,
        crate::scene3d::Options3d::default(),
    )
}

/// [`build_with_effects`] with the 3D pass's options (view override, mesh
/// layer definitions).
#[allow(clippy::too_many_arguments)]
pub fn build_with<'r>(
    ev: &mut Evaluator,
    root_comp: &Comp<'_>,
    project: &Comp<'_>,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    assets: &'r dyn AssetResolver,
    effects: Option<&'r dyn EffectHost>,
    options3d: crate::scene3d::Options3d,
) -> (CompPlan, Build<'r>) {
    build_full(
        ev, root_comp, project, time, width, height, scale, assets, effects, options3d, false,
    )
}

/// [`build_with`], with programs the native pipeline cannot translate
/// planned for the decision 19 fallback runner when `shader_fallback`.
#[allow(clippy::too_many_arguments)]
pub fn build_full<'r>(
    ev: &mut Evaluator,
    root_comp: &Comp<'_>,
    project: &Comp<'_>,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    assets: &'r dyn AssetResolver,
    effects: Option<&'r dyn EffectHost>,
    options3d: crate::scene3d::Options3d,
    shader_fallback: bool,
) -> (CompPlan, Build<'r>) {
    let mut b = Build {
        assets,
        failures: Vec::new(),
        unsupported: Vec::new(),
        retime: RetimeCache::new(),
        precomp_depth: 0,
        pass3d: Vec::new(),
        options3d,
        effects,
        matte_depth: 0,
        forced_on: None,
        shader_fallback,
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
fn comp_pass<'d>(
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
        // `PM.transitionDef`: a host knows its registered transitions.
        let host = b.effects;
        let known = |ty: &JsString| host.is_none_or(|h| h.transition_known(ty));
        let mut boundary = |s: &mut Scope<'_, 'd>, l: &'d Node| {
            s.group_creates_compositing_boundary(l, time, true, &known)
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
    // `compositionOrderedLayers` and the pass's 3D state.
    let pass = match crate::scene3d::order_pass(
        scope,
        &b.pass3d,
        pass,
        time,
        width,
        height,
        b.assets,
        b.options3d.view.as_ref(),
        b.precomp_depth == 0,
    ) {
        Ok((ordered, ctx)) => {
            b.pass3d.push(ctx);
            ordered
        }
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
    b.pass3d.pop();
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
    let active = if b.forced_on.is_some_and(|p| std::ptr::eq(p, layer)) {
        scope.active_forced_on(layer, time)?
    } else {
        scope.active(layer, time)?
    };
    if !active {
        return Ok(None);
    }
    let alpha = js::clamp(scope.num(layer, "opacity", time)? / 100.0, 0.0, 1.0);
    if alpha <= 0.001 {
        return Ok(None);
    }
    let comp_id = scope.comp.id().to_string_lossy();
    let refuse = |b: &mut Build<'_>, f: Feature| {
        b.unsupported.push(Unsupported::layer(&comp_id, layer, f));
    };
    let has_fx = scope.has_renderable_effects(layer, time)?;
    let has_masks = composite::build::has_masks(scope, layer, time)?;
    let blend = composite::build::blend_mode(scope, layer, time)?;
    // Effects prepared by the host (the effects lane); without one, a layer
    // with renderable effects is routed.
    let effects = if has_fx {
        let frame = composite::effects::EffectFrame {
            width,
            height,
            scale,
            time,
        };
        match b.effects {
            None => {
                refuse(
                    b,
                    if ty == "adjustment" {
                        Feature::Adjustment
                    } else {
                        Feature::Effects
                    },
                );
                return Ok(None);
            }
            Some(host) => match host.prepare(scope, layer, frame)? {
                Ok(e) => e,
                Err(f) => {
                    refuse(b, f);
                    return Ok(None);
                }
            },
        }
    } else {
        None
    };
    // The transition node (`activeTransition`, `runTransition`): prepared by
    // the host; without one, a layer in a transition is routed.
    let transition = match b.effects {
        None => {
            let active: Option<ActiveTransition<'d>> =
                scope.active_transition(layer, time, &|_| true)?;
            if active.is_some() && ty != "adjustment" {
                refuse(b, Feature::Transition);
                return Ok(None);
            }
            None
        }
        Some(_) if ty == "adjustment" => None,
        Some(host) => {
            let frame = composite::effects::EffectFrame {
                width,
                height,
                scale,
                time,
            };
            match host.prepare_transition(scope, layer, frame)? {
                Ok(t) => t,
                Err(f) => {
                    refuse(b, f);
                    return Ok(None);
                }
            }
        }
    };
    let id = js::layer_id(layer);
    let full = [f64::from(width), 0.0, 0.0, f64::from(height), 0.0, 0.0];
    let node = |n: composite::Node| Draw {
        layer: id.clone(),
        matrix: full,
        content: Content::Composite(Box::new(n)),
    };
    if ty == "adjustment" {
        // An adjustment layer without an effect that renders is a no-op.
        let Some(effects) = effects else {
            return Ok(None);
        };
        let Some(masks) = layer_masks(scope, b, layer, time, scale, has_masks)? else {
            return Ok(None);
        };
        return Ok(Some(vec![node(composite::Node::Adjustment(
            composite::AdjustmentNode {
                layer: id.clone(),
                effects,
                masks,
                blend,
                alpha,
            },
        ))]));
    }
    let mblur = scope.evaluated(layer, node::member(Some(layer), "mblur"), time, "l.mblur")?;
    if mblur.truthy() {
        refuse(b, Feature::MotionBlur);
        return Ok(None);
    }
    let matted = js::flag(layer, "matteSource");
    // Today's fast path draws straight into the accumulator; anything else
    // goes through the layer's own target.
    let slow = has_masks || has_fx || blend != BlendMode::Normal || matted || transition.is_some();
    // The layer's own target: masks, effects, matte, then the composite.
    let wrap = |scope: &mut Scope<'_, 'd>,
                b: &mut Build<'_>,
                content: Vec<Draw>,
                composite_alpha: f64|
     -> JsResult<Option<Vec<Draw>>> {
        let Some(masks) = layer_masks(scope, b, layer, time, scale, has_masks)? else {
            return Ok(None);
        };
        let matte = if matted {
            layer_matte(scope, b, layer, time, width, height, scale)?
        } else {
            None
        };
        Ok(Some(vec![node(composite::Node::Layer(
            composite::LayerNode {
                layer: id.clone(),
                content,
                masks,
                effects: effects.clone(),
                matte,
                blend,
                alpha: composite_alpha,
                transition: transition.clone(),
            },
        ))]))
    };
    if ty == "group" {
        if b.forced_on.is_some_and(|p| std::ptr::eq(p, layer)) {
            // Today's matte pass of a group renders the group inside itself
            // until the stack overflows; the matte is empty and the layer
            // reported. The editor never makes a group a matte source.
            return Err(pm_eval::JsThrow {
                kind: "RangeError",
                message: "Maximum call stack size exceeded (a group used as a track matte)"
                    .to_owned(),
            });
        }
        // A boundary group: its members composite into their own target,
        // drawn with the group's opacity.
        let inner = comp_pass(scope, b, time, width, height, scale, true, Some(layer));
        let draw = |alpha: f64| Draw {
            layer: id.clone(),
            matrix: full,
            content: Content::Texture {
                source: Source::Comp(Box::new(inner.clone())),
                uv: [0.0, 0.0, 1.0, 1.0],
                alpha,
            },
        };
        if !slow {
            return Ok(Some(vec![draw(alpha)]));
        }
        return wrap(scope, b, vec![draw(1.0)], alpha);
    }
    // 3D object layers draw in screen space (`c.screenSpace`), never as planes.
    if pm_eval::parenting::Role3d::of(layer).is_none() && scope.is_3d_layer(layer)? {
        refuse(b, Feature::TwoAndAHalfD);
        return Ok(None);
    }
    let Some(quad) = content_quad(scope, b, layer, time, width, height, scale)? else {
        return Ok(None);
    };
    let world = scaled_world(scope, layer, time, scale)?;
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
            Draw {
                layer: id.clone(),
                matrix: mul(&world, &[w, 0.0, 0.0, h, -ax, -ay]),
                content: Content::Solid([rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha]),
            }
        }
        Quad::Texture {
            source,
            w,
            h,
            ax,
            ay,
            uv,
        } => Draw {
            layer: id.clone(),
            matrix: mul(&world, &[w, 0.0, 0.0, h, -ax, -ay]),
            content: Content::Texture { source, uv, alpha },
        },
        Quad::Screen { source } => Draw {
            layer: id.clone(),
            matrix: full,
            content: Content::Texture {
                source,
                uv: [0.0, 0.0, 1.0, 1.0],
                alpha,
            },
        },
        Quad::Device { source, rect } => Draw {
            layer: id.clone(),
            matrix: [rect[2], 0.0, 0.0, rect[3], rect[0], rect[1]],
            content: Content::Texture {
                source,
                uv: [0.0, 0.0, 1.0, 1.0],
                alpha,
            },
        },
    };
    if slow {
        return wrap(scope, b, vec![draw], 1.0);
    }
    Ok(Some(vec![draw]))
}

/// The layer's masks for its node: `Some(None)` when nothing is masked,
/// `None` when the masks are unsupported (recorded).
fn layer_masks<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    has_masks: bool,
) -> JsResult<Option<Option<composite::Masks>>> {
    if !has_masks {
        return Ok(Some(None));
    }
    if pm_eval::parenting::Role3d::of(layer).is_some() {
        // Today a masked 3D object layer draws its scene pass turned by
        // 180° (measured on the composite/3d-mask corpus case); routed
        // until that is matched (docs/engine/composite.md).
        let comp_id = scope.comp.id().to_string_lossy();
        b.unsupported
            .push(Unsupported::layer(&comp_id, layer, Feature::Masks));
        return Ok(None);
    }
    let world = scaled_world(scope, layer, time, scale)?;
    let plane = if scope.is_3d_layer(layer)? {
        let h = scope.plane_matrix(layer, time, false)?;
        let (sx, sy) = scale;
        Some([
            h[0] * sx,
            h[1] * sy,
            h[2],
            h[3] * sx,
            h[4] * sy,
            h[5],
            h[6] * sx,
            h[7] * sy,
            h[8],
        ])
    } else {
        None
    };
    match composite::build::masks(scope, layer, time, world, plane)? {
        composite::build::MaskRoute::Unmasked => Ok(Some(None)),
        composite::build::MaskRoute::Parametric(m) => Ok(Some(Some(m))),
        composite::build::MaskRoute::Path => {
            let comp_id = scope.comp.id().to_string_lossy();
            b.unsupported
                .push(Unsupported::layer(&comp_id, layer, Feature::PathMasks));
            Ok(None)
        }
    }
}

/// `applyTrackMatte`: the matte source rendered alone (`renderProject`
/// with `mattePass`, `explicitLayers`, and `{ ...source, on: true }`), or
/// `None` when the source is missing or the matte depth limit is reached.
#[allow(clippy::too_many_arguments)]
fn layer_matte<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
) -> JsResult<Option<composite::Matte>> {
    let source = composite::build::matte_source(scope.layers(), layer);
    let (luma, inverted) = composite::build::matte_mode(scope, layer, time)?;
    let Some(source) = source else {
        return Ok(None);
    };
    if b.matte_depth >= composite::MATTE_MAX_DEPTH {
        return Ok(None);
    }
    b.matte_depth += 1;
    let outer = b.forced_on.replace(source as *const Node);
    let pass = matte_pass(scope, b, source, time, width, height, scale);
    b.forced_on = outer;
    b.matte_depth -= 1;
    Ok(Some(composite::Matte {
        luma,
        inverted,
        source: Box::new(pass),
    }))
}

/// One layer rendered alone into a transparent target in its parent's
/// pixel space (the matte pass). Solo and matte-source skipping do not
/// apply.
fn matte_pass<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    source: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
) -> CompPlan {
    let mut plan = CompPlan {
        composition: scope.comp.id().to_string_lossy(),
        group: true,
        width,
        height,
        background: Background::Transparent,
        draws: Vec::new(),
    };
    let all = [source];
    let solo = js::flag(source, "solo");
    match draw_layer(scope, b, source, time, width, height, scale, solo, &all) {
        Ok(Some(draws)) => plan.draws.extend(draws),
        Ok(None) => {}
        Err(e) => {
            let comp = scope.comp.clone();
            layer_failure(b, &comp, source, e.to_string());
        }
    }
    plan
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
    /// Target-space content covering exactly `rect` (level 2 vectors).
    Device { source: Source, rect: [f64; 4] },
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
    if matches!(quad, Quad::Screen { .. } | Quad::Device { .. }) {
        return Ok(Some(quad));
    }
    let offset = scope.size_anchor_offset(layer, time)?;
    if offset.x != 0.0 || offset.y != 0.0 {
        match &mut quad {
            Quad::Solid { ax, ay, .. } | Quad::Texture { ax, ay, .. } => {
                *ax -= offset.x;
                *ay -= offset.y;
            }
            Quad::Screen { .. } | Quad::Device { .. } => {}
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
        "solid" if crate::vector::level2(layer) => {
            // Level 2 solids draw at output resolution with analytic edges
            // (section 8), honoring `d.radius` (section 10); radius 0 is a
            // plain rectangle.
            let color = js::string(js::get(&d, "color"));
            let c = pm_color::standard::parse_hex(&color);
            let rgba = [
                c.rgb[0] as f32,
                c.rgb[1] as f32,
                c.rgb[2] as f32,
                c.alpha as f32,
            ];
            let world = vector_world(scope, layer, time, scale)?;
            let scene = crate::vector::level2::solid_scene(
                js::or_number(js::get(&d, "w"), wf),
                js::or_number(js::get(&d, "h"), hf),
                js::get(&d, "radius").to_number(),
                rgba,
                &world,
            );
            let target = pm_vector::render::scene::PixelRect::new(0, 0, width, height);
            Ok(scene.pixel_bounds(target).map(|rect| {
                device_quad(crate::vector::VectorQuad::Device {
                    source: crate::vector::VectorSource::Scene { scene, rect },
                    rect,
                })
            }))
        }
        "shape" => {
            let world = vector_world(scope, layer, time, scale)?;
            let target = pm_vector::render::scene::PixelRect::new(0, 0, width, height);
            let quad = crate::vector::plan_shape(scope, layer, &d, time, &world, target)?;
            Ok(quad.map(device_quad))
        }
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
                // Level 2 SVG images draw at output resolution
                // (crate::vector::svg); level 1 and SVG text stay routed.
                if !crate::vector::level2(layer) {
                    b.unsupported
                        .push(Unsupported::layer(&comp_id, layer, Feature::SvgImage));
                    return Ok(None);
                }
                let sw = js::or_number(&rec("w"), 1.0);
                let sh = js::or_number(&rec("h"), 1.0);
                let bw = js::or_number(js::get(&d, "w"), wf);
                let bh = js::or_number(js::get(&d, "h"), hf);
                let uv = fit_uv(&js::string(js::get(&d, "fit")), sw, sh, bw, bh);
                let world = vector_world(scope, layer, time, scale)?;
                let target = pm_vector::render::scene::PixelRect::new(0, 0, width, height);
                return match crate::vector::svg::plan_svg(&path, bw, bh, uv, &world, target) {
                    Ok(crate::vector::svg::SvgPlan::Drawn(quad)) => {
                        Ok(quad.map(|q| device_quad(*q)))
                    }
                    Ok(crate::vector::svg::SvgPlan::Text) => {
                        b.unsupported
                            .push(Unsupported::layer(&comp_id, layer, Feature::SvgImage));
                        Ok(None)
                    }
                    Err(message) => {
                        b.failures.push(LayerFailure {
                            composition: comp_id,
                            layer: js::layer_id(layer),
                            message,
                        });
                        Ok(None)
                    }
                };
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
                let st = scope.source_time(layer, time, Some(&mut b.retime))?;
                let dur = rec("dur").to_number();
                Source::Video {
                    asset: asset_id,
                    path,
                    time: playback_time(st, dur),
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
            let tw = js::max(2.0, js::round(w * scale.0));
            let th = js::max(2.0, js::round(hh * scale.1));
            if !(tw.is_finite() && th.is_finite()) || tw > 16384.0 || th > 16384.0 {
                b.failures.push(LayerFailure {
                    composition: comp_id,
                    layer: js::layer_id(layer),
                    message: format!("precomp target {tw}×{th} is outside 2..16384"),
                });
                return Ok(None);
            }
            let (tw, th) = (tw as u32, th as u32);
            let child_time = scope.source_time(layer, time, Some(&mut b.retime))?;
            let sub = match Comp::in_root(root, &sub_id, scope.comp.revision()) {
                Ok(c) => c,
                Err(_) => return Ok(None),
            };
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
            Ok(Some(Quad::Texture {
                source: Source::Comp(Box::new(inner)),
                w,
                h: hh,
                ax: 0.0,
                ay: 0.0,
                uv: [0.0, 0.0, 1.0, 1.0],
            }))
        }
        "shader" => {
            let planned = crate::shader_layers::plan::shader_layer(
                scope,
                layer,
                &d,
                time,
                (width, height),
                scale,
                b.shader_fallback,
            )?;
            Ok(shader_quad(b, &comp_id, layer, planned))
        }
        "extension" => {
            let def = js::string(js::get(&d, "definition"));
            // Legacy scene layers and mesh layers the 3D pass knows are its
            // own (`crate::scene3d::legacy`), by id as today.
            if pm_eval::parenting::Role3d::of(layer).is_none()
                && def != crate::scene3d::legacy::SCENE3D_DEFINITION
                && !b.options3d.mesh_definitions.contains_key(&def)
                && let Some(planned) = crate::shader_layers::plan::extension_layer(
                    scope,
                    layer,
                    &d,
                    &def,
                    time,
                    (width, height),
                    scale,
                    b.effects.and_then(|e| e.layer_definitions()),
                    b.shader_fallback,
                )?
            {
                return Ok(shader_quad(b, &comp_id, layer, planned));
            }
            match pm_eval::parenting::Role3d::of(layer) {
                Some(pm_eval::parenting::Role3d::Object) => {
                    match crate::scene3d::plan_layer(b.pass3d.last(), layer) {
                        Ok(plan) => Ok(Some(Quad::Screen {
                            source: Source::Object3d(Box::new(plan)),
                        })),
                        Err(reason) => {
                            b.unsupported.push(Unsupported::layer(
                                &comp_id,
                                layer,
                                Feature::Scene3d(reason),
                            ));
                            Ok(None)
                        }
                    }
                }
                Some(_) => Ok(None),
                None => match crate::scene3d::legacy::plan_extension(
                    scope,
                    layer,
                    &d,
                    &def,
                    time,
                    (width, height),
                    scale,
                    b.assets,
                    &b.options3d,
                )? {
                    Some(Ok((source, w, h))) => Ok(Some(Quad::Texture {
                        source,
                        w,
                        h,
                        ax: 0.0,
                        ay: 0.0,
                        uv: [0.0, 0.0, 1.0, 1.0],
                    })),
                    Some(Err(reason)) => {
                        b.unsupported.push(Unsupported::layer(
                            &comp_id,
                            layer,
                            Feature::Scene3d(reason),
                        ));
                        Ok(None)
                    }
                    None => {
                        b.unsupported.push(Unsupported::layer(
                            &comp_id,
                            layer,
                            Feature::ExtensionDefinition(def),
                        ));
                        Ok(None)
                    }
                },
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

/// The quad of a planned shader or extension fragment layer: `{tex, w, h,
/// ax: 0, ay: 0, uv: [0, 0, 1, 1], fromFbo: true}`.
fn shader_quad(
    b: &mut Build<'_>,
    comp_id: &str,
    layer: &Node,
    planned: crate::shader_layers::plan::Planned,
) -> Option<Quad> {
    use crate::shader_layers::plan::Planned;
    match planned {
        Planned::Draw { source, w, h } => Some(Quad::Texture {
            source: Source::Shader(source),
            w,
            h,
            ax: 0.0,
            ay: 0.0,
            uv: [0.0, 0.0, 1.0, 1.0],
        }),
        Planned::Refused(message) => {
            b.failures.push(LayerFailure {
                composition: comp_id.to_owned(),
                layer: js::layer_id(layer),
                message,
            });
            None
        }
        Planned::Unsupported(feature) => {
            b.unsupported
                .push(Unsupported::layer(comp_id, layer, feature));
            None
        }
    }
}

/// The world matrix of a vector layer in target pixels, including the size
/// anchor offset `contentQuad` applies to its quad.
fn vector_world<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
) -> JsResult<Mat2d> {
    let world = scaled_world(scope, layer, time, scale)?;
    let offset = scope.size_anchor_offset(layer, time)?;
    Ok(mul(&world, &[1.0, 0.0, 0.0, 1.0, offset.x, offset.y]))
}

/// A planned vector quad as the render graph's quad.
fn device_quad(q: crate::vector::VectorQuad) -> Quad {
    match q {
        crate::vector::VectorQuad::Layer {
            source,
            w,
            h,
            ax,
            ay,
        } => Quad::Texture {
            source: Source::Vector(Box::new(source)),
            w,
            h,
            ax,
            ay,
            uv: [0.0, 0.0, 1.0, 1.0],
        },
        crate::vector::VectorQuad::Device { source, rect } => Quad::Device {
            source: Source::Vector(Box::new(source)),
            rect: [
                f64::from(rect.x),
                f64::from(rect.y),
                f64::from(rect.width),
                f64::from(rect.height),
            ],
        },
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
pub fn playback_time(time: f64, dur: f64) -> f64 {
    let dur = if dur.is_nan() { 0.0 } else { dur };
    js::max(0.0, js::min(js::max(0.0, dur - 0.04), time))
}

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

    #[test]
    fn playback_time_clamps_like_video_timing() {
        assert_eq!(playback_time(-1.0, 2.0), 0.0);
        assert_eq!(playback_time(1.0, 2.0), 1.0);
        assert!((playback_time(5.0, 2.0) - 1.96).abs() < 1e-12);
        assert_eq!(playback_time(0.5, f64::NAN), 0.0);
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
