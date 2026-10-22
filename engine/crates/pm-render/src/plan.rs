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
use crate::request::VideoEnd;
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
    /// The level's output origin, when it is not `(0, 0)`
    /// ([`RenderRequest::origin`](crate::RenderRequest::origin)): the
    /// background gradient and effect coordinates follow it.
    pub view: Option<OutputView>,
}

/// Where a level's output sits in its composition at today's scale
/// (`effectViewportCoordinates`, `backgroundView` with a preview viewport).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputView {
    /// Output pixel (0, 0) in scaled composition pixels (`viewport.x × sx`).
    pub origin: [f64; 2],
    /// The whole composition in output pixels
    /// (`max(1, comp.w || W) × sx`).
    pub resolution: [f64; 2],
}

/// The [`OutputView`] of a level with output origin `origin` (`None` at
/// `(0, 0)`).
pub(crate) fn level_view(
    scope: &Scope<'_, '_>,
    origin: (f64, f64),
    width: u32,
    height: u32,
    scale: (f64, f64),
) -> Option<OutputView> {
    if origin == (0.0, 0.0) {
        return None;
    }
    let (cw, ch) = scope.comp_size();
    let side = |c: f64, w: u32| {
        js::max(
            1.0,
            if c.is_nan() || c == 0.0 {
                f64::from(w)
            } else {
                c
            },
        )
    };
    Some(OutputView {
        origin: [origin.0, origin.1],
        resolution: [side(cw, width) * scale.0, side(ch, height) * scale.1],
    })
}

/// `m` followed by the output origin: composition pixel `(x, y)` at the
/// level's scale lands at `(x − ox, y − oy)` (today's `scaledWorld` with a
/// preview viewport). Exact (`m` unchanged) at the origin.
pub(crate) fn at_origin(m: Mat2d, origin: (f64, f64)) -> Mat2d {
    if origin == (0.0, 0.0) {
        return m;
    }
    [m[0], m[1], m[2], m[3], m[4] - origin.0, m[5] - origin.1]
}

/// A plane (column-major homography to output pixels) followed by the
/// output origin.
pub(crate) fn plane_at_origin(
    p: crate::space25d::Plane,
    origin: (f64, f64),
) -> crate::space25d::Plane {
    if origin == (0.0, 0.0) {
        return p;
    }
    let mut q = p;
    for c in 0..3 {
        q[c * 3] = p[c * 3] - origin.0 * p[c * 3 + 2];
        q[c * 3 + 1] = p[c * 3 + 1] - origin.1 * p[c * 3 + 2];
    }
    q
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
        level2: bool,
        asset: String,
        path: PathBuf,
        /// The asset's Interpret Footage overrides (section 9).
        interpret: Option<pm_media::interpret::Interpretation>,
    },
    Video {
        /// Validated generated media; display geometry is preserved by its recipe.
        proxy: bool,
        level2: bool,
        /// Persisted image-sequence/animation WebM (converter BT.601 default).
        sequence: bool,
        asset: String,
        path: PathBuf,
        /// Asset-scoped viewer intent: use complete original texels even when
        /// other clips can use a reduced playback representation.
        force_original: bool,
        /// A measured playback decode scale in source coordinates.
        decode_scale: Option<f64>,
        fallback_decode_scale: Option<f64>,
        source_size: (f64, f64),
        /// Source time in seconds after retiming and today's end clamp.
        time: f64,
        /// The asset's Interpret Footage overrides (section 9). With a
        /// frame-rate override frame n shows at n / rate
        /// (`crate::retime::override_frame`).
        interpret: Option<pm_media::interpret::Interpretation>,
        /// The rate of the composition the layer is in: field-rate or
        /// frame-rate deinterlacing depends on it (section 13).
        composition_rate: pm_time::Rate,
    },
    /// A nested composition (precomp) or a group's compositing boundary.
    Comp(Box<CompPlan>),
    /// The 3D scene pass output for one object layer.
    Object3d(Box<Object3dPlan>),
    /// A Blender surface of one 3D object layer (`crate::blender`).
    Blender(Box<crate::blender::BlenderSource>),
    /// Shape content (`crate::vector`).
    Vector(Box<crate::vector::VectorSource>),
    /// A shader layer or an extension fragment layer
    /// (`crate::shader_layers`).
    Shader(Box<crate::shader_layers::ShaderSource>),
    /// A text or captions raster (`crate::text`).
    Text(Box<crate::text::TextPlan>),
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
    pub(crate) precomp_depth: u32,
    /// The 3D state of the compositing passes being built (innermost last).
    pass3d: Vec<crate::scene3d::Pass3d>,
    options3d: crate::scene3d::Options3d,
    /// Prepares effects (`crate::composite::EffectHost`); without one,
    /// layers with renderable effects are unsupported.
    effects: Option<&'r dyn EffectHost>,
    /// Today's `opt.matteDepth`: matte passes entered so far (carried into
    /// precomps, as `PM._renderOptions3D` carries it).
    pub(crate) matte_depth: u32,
    /// The matte source a matte pass renders, which draws even when it is
    /// switched off (`{ ...source, on: true }`).
    pub(crate) forced_on: Option<*const Node>,
    /// A decision 19 fallback runner draws programs the native pipeline
    /// cannot (`crate::shader_layers::fallback`); without one they route.
    shader_fallback: bool,
    /// Text layout and glyph caches (`None`: text layers are unsupported).
    text: Option<&'r mut crate::text::TextContext>,
    /// Blender surfaces of 3D object layers (`crate::blender`).
    pub(crate) blender: crate::blender::PlanState,
    /// Motion blur for layers with their switch on; `None` when the request
    /// has it off or inside a group's samples (today's `mblur: false`).
    pub(crate) blur: Option<crate::motion::Blur>,
    /// The composition time at which video sources select their frame
    /// inside motion-blur samples (the center time of their level).
    pub(crate) video_center: Option<f64>,
    /// The resampler of level 2 raster content.
    pub(crate) sampling: crate::sampling::Sampling,
    /// `videoPlaybackTime`'s `endPadding` for this output ([`VideoEnd`]).
    video_padding: f64,
    /// The output origin of the level being built (the request's at the
    /// top level, in its groups, mattes, and motion-blur samples; `(0, 0)`
    /// inside a precomp, which renders its own target).
    pub(crate) origin: (f64, f64),
    /// What every pass over one composition's layers reads of all of them
    /// (each boundary group is a pass), by the layer list's address and
    /// length (the document is borrowed for the whole build).
    passes: Vec<((usize, usize), std::sync::Arc<PassFacts>)>,
    /// Layer draws kept from the last frame (`crate::reuse`): the
    /// interactive viewer's renders only.
    pub(crate) reuse: Option<&'r mut crate::reuse::DrawReuse>,
}

use crate::pass_facts::PassFacts;

impl Build<'_> {
    fn pass_facts(&mut self, layers: &[Node], all: &[&Node]) -> std::sync::Arc<PassFacts> {
        let key = (layers.as_ptr() as usize, layers.len());
        if let Some((_, facts)) = self.passes.iter().find(|(k, _)| *k == key) {
            return facts.clone();
        }
        let facts = match self.reuse.as_deref_mut() {
            Some(reuse) => reuse.pass_facts(layers, all),
            None => std::sync::Arc::new(PassFacts::of(all)),
        };
        self.passes.push((key, facts.clone()));
        facts
    }
}

/// Options of a build beyond the frame's geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BuildOptions {
    /// Resolved motion blur (`crate::motion::resolve`).
    pub blur: Option<crate::motion::Blur>,
    pub sampling: crate::sampling::Sampling,
    /// A decision 19 fallback runner draws programs the native pipeline
    /// cannot translate (`crate::shader_layers::fallback`); without one
    /// they route.
    pub shader_fallback: bool,
    /// The end-of-clip padding of video sources ([`VideoEnd`]).
    pub video_end: VideoEnd,
    /// The top level's output origin
    /// ([`RenderRequest::origin`](crate::RenderRequest::origin)).
    pub origin: Option<(f64, f64)>,
}

/// Today's `PC_MAX_DEPTH`.
pub const PRECOMP_MAX_DEPTH: u32 = 6;

/// Builds the plan of `composition` at `time` (seconds) into a target of
/// `width` × `height` at `scale` output pixels per composition pixel, with
/// the video end padding of the output kind (`video_end`).
#[allow(clippy::too_many_arguments)]
pub fn build<'r>(
    ev: &mut Evaluator,
    root_comp: &Comp<'_>,
    project: &Comp<'_>,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    video_end: VideoEnd,
    assets: &'r dyn AssetResolver,
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
        None,
        crate::scene3d::Options3d::default(),
        None,
        BuildOptions {
            video_end,
            ..BuildOptions::default()
        },
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
        None,
        BuildOptions::default(),
    )
}

/// [`build_with_effects`] with the 3D pass's options (view override, mesh
/// layer definitions), text layout, motion blur, and the level 2
/// resampler: text and captions layers draw through `text` (its fonts and
/// glyph caches; `None`: they are unsupported).
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
    text: Option<&'r mut crate::text::TextContext>,
    options: BuildOptions,
) -> (CompPlan, Build<'r>) {
    let blender = crate::blender::PlanState::new(options3d.blender.clone());
    build_blender(
        ev, root_comp, project, time, width, height, scale, assets, effects, options3d, text,
        options, blender, None,
    )
}

/// [`build_with`] with the Blender state of the build (collecting the
/// snapshots a frame needs, `Renderer::blender_snapshots`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_blender<'r>(
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
    text: Option<&'r mut crate::text::TextContext>,
    options: BuildOptions,
    blender: crate::blender::PlanState,
    reuse: Option<&'r mut crate::reuse::DrawReuse>,
) -> (CompPlan, Build<'r>) {
    let video_padding = video_padding(options.video_end, root_comp);
    let mut reuse = reuse;
    if let Some(r) = reuse.as_deref_mut() {
        r.begin(
            root_comp.root(),
            root_comp.revision(),
            time,
            crate::reuse::BuildKey {
                sampling: options.sampling,
                blur: options.blur,
                video_padding: video_padding.to_bits(),
                view_override: options3d.view.is_some(),
                origin: options.origin.map(|(x, y)| (x.to_bits(), y.to_bits())),
            },
        );
    }
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
        shader_fallback: options.shader_fallback,
        text,
        blender,
        blur: options.blur,
        video_center: None,
        sampling: options.sampling,
        video_padding,
        origin: options.origin.unwrap_or((0.0, 0.0)),
        passes: Vec::new(),
        reuse,
    };
    let mut scope = Scope::new(ev, root_comp, project);
    // One frame at one time: each layer's local matrix once per scope.
    scope.memoize_transforms();
    let plan = comp_pass(&mut scope, &mut b, time, width, height, scale, false, None);
    if let Some(reuse) = b.reuse.as_deref_mut() {
        reuse.end();
    }
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
    let view = level_view(scope, b.origin, width, height, scale);
    let mut plan = CompPlan {
        composition: comp_id.clone(),
        group: group_parent.is_some(),
        width,
        height,
        background,
        draws: Vec::new(),
        view,
    };
    let parent_id = group_parent.map(|g| node::get(g, b"id").to_js());
    let facts = b.pass_facts(scope.layers(), &all);
    // `compositingPass`: boundary groups stay one entry.
    let pass = {
        // `PM.transitionDef`: a host knows its registered transitions.
        let host = b.effects;
        let known = |ty: &JsString| host.is_none_or(|h| h.transition_known(ty));
        // A group with motion blur is a boundary only while motion blur
        // applies (`opt.mblur !== false`).
        let blur_on = b.blur.is_some();
        let mut boundary = |s: &mut Scope<'_, 'd>, l: &'d Node| {
            s.group_creates_compositing_boundary(l, time, blur_on, &known)
        };
        match scope.compositing_layers_with(&all, &facts.owners, parent_id.as_ref(), &mut boundary)
        {
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
    let solo = facts.solo;
    // Bottom (last) to top (first).
    for &layer in pass.iter().rev() {
        let id = node::get(layer, b"id").to_js();
        if facts.matte_sources.contains(&id) {
            continue;
        }
        let drawn =
            crate::reuse::draw(scope, b, layer, time, width, height, scale, solo, |s, b| {
                draw_layer(s, b, layer, time, width, height, scale, solo, &all)
            });
        match drawn {
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
        let masks = layer_masks(
            scope, layer, time, scale, b.origin, width, height, has_masks,
        )?;
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
    let blur = if mblur.truthy() { b.blur } else { None };
    let matted = js::flag(layer, "matteSource");
    // Today's fast path draws straight into the accumulator; anything else
    // goes through the layer's own target.
    let slow = has_masks || has_fx || blend != BlendMode::Normal || matted || transition.is_some();
    // Motion blur samples draw straight into their accumulation. A blurred
    // layer that also needs its own target (masks, effects, matte, blend
    // mode, transition) takes the accumulation as its layer target, as
    // today's slow path does (`lf` holds the samples; masks, effects, and
    // the matte read it at the center time).
    // The layer's own target: masks, effects, matte, then the composite.
    let wrap = |scope: &mut Scope<'_, 'd>,
                b: &mut Build<'_>,
                content: Vec<Draw>,
                composite_alpha: f64|
     -> JsResult<Option<Vec<Draw>>> {
        let masks = layer_masks(
            scope, layer, time, scale, b.origin, width, height, has_masks,
        )?;
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
        if let Some(blur) = blur {
            // The group's members at every sample time, accumulated; the
            // sum is drawn at the group's opacity, or is the layer target
            // of the slow path (composited at the group's opacity).
            if !slow {
                let draw = crate::motion::plan::blur_group(
                    scope, b, layer, time, width, height, scale, alpha, blur,
                );
                return Ok(Some(vec![draw]));
            }
            let draw = crate::motion::plan::blur_group(
                scope, b, layer, time, width, height, scale, 1.0, blur,
            );
            return wrap(scope, b, vec![draw], alpha);
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
    if let Some(blur) = blur {
        let draw = crate::motion::plan::blur_layer(
            scope, b, layer, time, width, height, scale, alpha, blur,
        )?;
        return match draw {
            Some(d) if slow => wrap(scope, b, vec![d], 1.0),
            Some(d) => Ok(Some(vec![d])),
            None => Ok(None),
        };
    }
    let Some(draw) = layer_draw(scope, b, layer, time, width, height, scale, alpha)? else {
        return Ok(None);
    };
    if slow {
        return wrap(scope, b, vec![draw], 1.0);
    }
    Ok(Some(vec![draw]))
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
    let world = scaled_world(scope, layer, time, scale, b.origin)?;
    let id = js::layer_id(layer);
    let full = [f64::from(width), 0.0, 0.0, f64::from(height), 0.0, 0.0];
    let ty = js::layer_type(layer);
    // Level 2 raster content: images, video, precomps, and shader and
    // extension layer output (`crate::shader_layers` renders it at the
    // density it is drawn at).
    let level2_type =
        crate::sampling::samples_layer_type(&ty) || matches!(ty.as_str(), "shader" | "extension");
    let level2 = crate::sampling::is_level2(layer).then_some(b.sampling);
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
            let plane = crate::space25d::projected(scope, layer, time, scale, w, h, ax, ay)?
                .map(|p| plane_at_origin(p, b.origin));
            Draw {
                layer: id.clone(),
                matrix: mul(&world, &[w, 0.0, 0.0, h, -ax, -ay]),
                content: placed(
                    Content::Solid([rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha]),
                    plane,
                    None,
                ),
            }
        }
        Quad::Texture {
            mut source,
            w,
            h,
            ax,
            ay,
            uv,
        } => {
            let plane = crate::space25d::projected(scope, layer, time, scale, w, h, ax, ay)?
                .map(|p| plane_at_origin(p, b.origin));
            // Level 2 image, video, and precomp content is resampled; other
            // level 2 content draws at output resolution (`crate::vector`,
            // `crate::text`), except on a plane, where its layer-space raster
            // is resampled too (`crate::space25d::local`).
            let level2 = level2.filter(|_| level2_type || plane.is_some());
            let matrix = mul(&world, &[w, 0.0, 0.0, h, -ax, -ay]);
            if let Source::Video {
                decode_scale,
                fallback_decode_scale,
                force_original,
                sequence,
                source_size,
                ..
            } = &mut source
            {
                let safe = |scale| {
                    plane.is_none() && reduced_video_scale_safe(&matrix, uv, *source_size, scale)
                };
                if *force_original || *sequence {
                    *decode_scale = None;
                } else if let Some(preferred) = *decode_scale {
                    *decode_scale = if safe(preferred) {
                        Some(preferred)
                    } else {
                        (*fallback_decode_scale).filter(|fallback| safe(*fallback))
                    };
                }
            }
            Draw {
                layer: id.clone(),
                matrix,
                content: placed(Content::Texture { source, uv, alpha }, plane, level2),
            }
        }
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
    Ok(Some(draw))
}

/// A reduced raster may be sampled only when its source-space resolution
/// covers each projected video axis. Canvas dimensions alone are insufficient:
/// a naturally sized source can be clipped in a much smaller composition.
fn reduced_video_scale_safe(
    matrix: &Mat2d,
    uv: [f64; 4],
    source_size: (f64, f64),
    decode_scale: f64,
) -> bool {
    let source_x = source_size.0 * uv[2].abs();
    let source_y = source_size.1 * uv[3].abs();
    let output_x = matrix[0].hypot(matrix[1]);
    let output_y = matrix[2].hypot(matrix[3]);
    source_x.is_finite()
        && source_y.is_finite()
        && source_x > 0.0
        && source_y > 0.0
        && output_x.is_finite()
        && output_y.is_finite()
        && decode_scale.is_finite()
        && decode_scale > 0.0
        && decode_scale + 1e-12 >= (output_x / source_x).max(output_y / source_y)
}

/// A motion-blur sample of a 3D object layer at `time`: pushes the
/// enclosing pass's 3D state rebuilt at that time (today's
/// `compositionRuntime(PM, T + dt)`); [`pop_sample_3d`] removes it. Returns
/// whether a state was pushed (other layer types, or no enclosing pass).
pub(crate) fn push_sample_3d<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
) -> JsResult<bool> {
    if pm_eval::parenting::Role3d::of(layer) != Some(pm_eval::parenting::Role3d::Object) {
        return Ok(false);
    }
    let Some(center) = b.pass3d.last() else {
        return Ok(false);
    };
    let ctx = center.at_time(
        scope,
        time,
        width,
        height,
        b.assets,
        b.options3d.view.as_ref(),
    )?;
    b.pass3d.push(ctx);
    Ok(true)
}

/// Removes the state [`push_sample_3d`] pushed.
pub(crate) fn pop_sample_3d(b: &mut Build<'_>) {
    b.pass3d.pop();
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

/// The layer's masks for its node (`None` when nothing is masked). The
/// path route's coverage canvas is the `width × height` output.
#[allow(clippy::too_many_arguments)]
fn layer_masks<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    origin: (f64, f64),
    width: u32,
    height: u32,
    has_masks: bool,
) -> JsResult<Option<composite::Masks>> {
    if !has_masks {
        return Ok(None);
    }
    let world = scaled_world(scope, layer, time, scale, origin)?;
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
        .map(|p| plane_at_origin(p, origin))
    } else {
        None
    };
    match composite::build::masks(scope, layer, time, world, plane, width, height)? {
        composite::build::MaskRoute::Unmasked => Ok(None),
        composite::build::MaskRoute::Parametric(m) | composite::build::MaskRoute::Path(m) => {
            Ok(Some(m))
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
        view: level_view(scope, b.origin, width, height, scale),
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

/// `scaledWorld(L, T, W, H)`: the output origin is the preview viewport's
/// (`(world[4] − viewport.x) × sx`, as `x · sx − origin`).
fn scaled_world<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    origin: (f64, f64),
) -> JsResult<Mat2d> {
    let w = scope.world_matrix(layer, time)?;
    let (sx, sy) = scale;
    Ok(at_origin(
        [
            w[0] * sx,
            w[1] * sy,
            w[2] * sx,
            w[3] * sy,
            w[4] * sx,
            w[5] * sy,
        ],
        origin,
    ))
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
    /// Target-space content covering exactly `rect` (x, y, width, height):
    /// level 2 vectors, and level 2 text drawn at output resolution.
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
            let (sw, sh) = (
                js::or_number(js::get(&d, "w"), wf),
                js::or_number(js::get(&d, "h"), hf),
            );
            let radius = js::get(&d, "radius").to_number();
            let mut plan = |_: &mut Scope<'_, 'd>, world: &Mat2d, target| {
                let scene = crate::vector::level2::solid_scene(sw, sh, radius, rgba, world);
                Ok(scene
                    .pixel_bounds(target)
                    .map(|rect| crate::vector::VectorQuad::Device {
                        source: crate::vector::VectorSource::Scene { scene, rect },
                        rect,
                    }))
            };
            if scope.is_3d_layer(layer)? {
                return local_level2(
                    scope,
                    layer,
                    time,
                    scale,
                    b.origin,
                    pm_vector::render::scene::PixelRect::new(0, 0, width, height),
                    &mut plan,
                );
            }
            let world = vector_world(scope, layer, time, scale, b.origin)?;
            let target = pm_vector::render::scene::PixelRect::new(0, 0, width, height);
            Ok(plan(scope, &world, target)?.map(device_quad))
        }
        "shape" => {
            if crate::vector::level2(layer) && scope.is_3d_layer(layer)? {
                let mut plan = |scope: &mut Scope<'_, 'd>, world: &Mat2d, target| {
                    crate::vector::plan_shape(scope, layer, &d, time, world, target)
                };
                return local_level2(
                    scope,
                    layer,
                    time,
                    scale,
                    b.origin,
                    pm_vector::render::scene::PixelRect::new(0, 0, width, height),
                    &mut plan,
                );
            }
            let world = vector_world(scope, layer, time, scale, b.origin)?;
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
                    message: b.assets.unavailable_reason(&asset_id, Some(record))
                        .unwrap_or_else(|| format!("media for asset {asset_id} is offline")),
                });
                return Ok(None);
            };
            let rec = |k: &str| node::js_value_opt(node::member(Some(record), k));
            let format = js::string(&rec("format")).to_ascii_lowercase();
            if ty == "image" && format == "svg" {
                if !crate::vector::level2(layer) {
                    // Level 1 (decision S1): today's `<img>` raster at
                    // today's bucket, drawn by Chromium in the document's
                    // renderer, sampled as today's image texture. A
                    // document Chromium is measured to show as a broken
                    // `<img>` paints nothing, as there.
                    match pm_media::image::svg::broken_image_file(&path) {
                        Err(e) => {
                            b.failures.push(LayerFailure {
                                composition: comp_id,
                                layer: js::layer_id(layer),
                                message: format!("{}: {e}", path.display()),
                            });
                            return Ok(None);
                        }
                        Ok(true) => return Ok(None),
                        Ok(false) => {}
                    }
                    let content = match crate::vector::svg::content_hash(&path) {
                        Ok(c) => c,
                        Err(message) => {
                            b.failures.push(LayerFailure {
                                composition: comp_id,
                                layer: js::layer_id(layer),
                                message,
                            });
                            return Ok(None);
                        }
                    };
                    // `const bw = d.w || W, bh = d.h || H` with the live
                    // asset's `w`/`h` (its natural size, persisted).
                    let sw = js::or_number(&rec("w"), 1.0);
                    let sh = js::or_number(&rec("h"), 1.0);
                    let bw = js::or_number(js::get(&d, "w"), wf);
                    let bh = js::or_number(js::get(&d, "h"), hf);
                    let uv = fit_uv(&js::string(js::get(&d, "fit")), sw, sh, bw, bh);
                    // `svgRasterDimensions(sw, sh, bw, bh, scaledWorld(L, T, W, H))`.
                    let world = scaled_world(scope, layer, time, scale, b.origin)?;
                    let (width, height, _) = pm_vector::render::svg::svg_raster_dimensions(
                        sw, sh, bw, bh, &world, 8192.0,
                    );
                    return Ok(Some(Quad::Texture {
                        source: Source::Vector(Box::new(crate::vector::VectorSource::SvgImage {
                            key: pm_protocol::svg_raster::SvgRasterKey {
                                content,
                                width,
                                height,
                            },
                            asset: asset_id,
                            path,
                        })),
                        w: bw,
                        h: bh,
                        ax: bw / 2.0,
                        ay: bh / 2.0,
                        uv,
                    }));
                }
                if let Err(message) = crate::vector::svg::document(&path) {
                    b.failures.push(LayerFailure {
                        composition: comp_id,
                        layer: js::layer_id(layer),
                        message,
                    });
                    return Ok(None);
                }
                let sw = js::or_number(&rec("w"), 1.0);
                let sh = js::or_number(&rec("h"), 1.0);
                let bw = js::or_number(js::get(&d, "w"), wf);
                let bh = js::or_number(js::get(&d, "h"), hf);
                let uv = fit_uv(&js::string(js::get(&d, "fit")), sw, sh, bw, bh);
                let world = vector_world(scope, layer, time, scale, b.origin)?;
                let target = pm_vector::render::scene::PixelRect::new(0, 0, width, height);
                return match crate::vector::svg::plan_svg(&path, bw, bh, uv, &world, target) {
                    Ok(crate::vector::svg::SvgPlan::Drawn(Some(_)))
                        if scope.is_3d_layer(layer)? =>
                    {
                        let mut plan = |_: &mut Scope<'_, 'd>, world: &Mat2d, target| {
                            Ok(
                                match crate::vector::svg::plan_svg(&path, bw, bh, uv, world, target)
                                {
                                    Ok(crate::vector::svg::SvgPlan::Drawn(q)) => q.map(|q| *q),
                                    _ => None,
                                },
                            )
                        };
                        local_level2(
                            scope,
                            layer,
                            time,
                            scale,
                            b.origin,
                            pm_vector::render::scene::PixelRect::new(0, 0, width, height),
                            &mut plan,
                        )
                    }
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
            let sequence = ty == "video"
                && (rec("imageSequence").truthy()
                    || matches!(format.as_str(), "gif" | "webp" | "png" | "apng" | "avif"));
            if sequence {
                match crate::media::sequence_rendition(&path) {
                    Ok(true) => {}
                    Ok(false) => {
                        b.unsupported.push(Unsupported::layer(
                            &comp_id,
                            layer,
                            Feature::ImageSequence,
                        ));
                        return Ok(None);
                    }
                    Err(message) => {
                        b.failures.push(LayerFailure {
                            composition: comp_id,
                            layer: js::layer_id(layer),
                            message,
                        });
                        return Ok(None);
                    }
                }
            }
            let sw = js::or_number(&rec("w"), 1.0);
            let sh = js::or_number(&rec("h"), 1.0);
            let bw = js::or_number(js::get(&d, "w"), wf);
            let bh = js::or_number(js::get(&d, "h"), hf);
            let fit = js::string(js::get(&d, "fit"));
            let uv = fit_uv(&fit, sw, sh, bw, bh);
            let interpret = crate::footage::interpretation(record);
            let source = if ty == "video" {
                // Inside motion-blur samples video keeps the center time's
                // frame (`crate::motion`).
                let at = b.video_center.unwrap_or(time);
                let st = scope.source_time(layer, at, Some(&mut b.retime))?;
                let rate = interpret.as_ref().and_then(|i| i.frame_rate);
                let time = match (
                    (sequence && rate.is_none())
                        .then(|| crate::media::sequence_playback_time(record, st))
                        .flatten(),
                    rate,
                ) {
                    (Some(t), _) => t,
                    // The file's frames on the overridden grid; the last
                    // frame clamps when the media is opened.
                    (None, Some(_)) => js::max(0.0, st),
                    (None, None) => playback_time(st, rec("dur").to_number(), b.video_padding),
                };
                Source::Video {
                    proxy: b.assets.is_proxy(&asset_id),
                    level2: crate::sampling::is_level2(layer),
                    sequence,
                    force_original: b.assets.force_full_original(&asset_id),
                    decode_scale: b.assets.preview_decode_scale(&asset_id),
                    fallback_decode_scale: b.assets.preview_decode_fallback_scale(&asset_id),
                    source_size: (sw, sh),
                    asset: asset_id,
                    path,
                    time,
                    interpret,
                    composition_rate: scope.comp.rate().map_or(pm_time::Rate::FPS_30, |r| r.rate),
                }
            } else {
                Source::Image {
                    level2: crate::sampling::is_level2(layer),
                    asset: asset_id,
                    path,
                    interpret,
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
            // A precomp renders its own target from its own origin (today's
            // preview viewport applies only to the open composition's
            // canvas).
            let outer_origin = std::mem::replace(&mut b.origin, (0.0, 0.0));
            b.blender.path.push(scope.comp.id().to_string_lossy());
            let project = scope.project;
            let inner = {
                let mut inner_scope = Scope::new(&mut *scope.ev, &sub, project);
                inner_scope.memoize_transforms();
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
            b.origin = outer_origin;
            b.blender.path.pop();
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
                    match crate::blender::plan_object(
                        scope,
                        &mut b.blender,
                        b.assets,
                        layer,
                        time,
                        width,
                        height,
                        b.video_center.is_some(),
                    )? {
                        Some(crate::blender::Planned::Draw(source, rect)) => {
                            return Ok(Some(Quad::Device {
                                source: Source::Blender(Box::new(source)),
                                rect,
                            }));
                        }
                        Some(crate::blender::Planned::Fail(message)) => {
                            b.failures.push(LayerFailure {
                                composition: comp_id,
                                layer: js::layer_id(layer),
                                message,
                            });
                            return Ok(None);
                        }
                        None => {}
                    }
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
        "text" | "captions" => text_quad(scope, b, layer, &ty, time, (width, height), scale),
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
    origin: (f64, f64),
) -> JsResult<Mat2d> {
    let world = scaled_world(scope, layer, time, scale, origin)?;
    let offset = scope.size_anchor_offset(layer, time)?;
    Ok(mul(&world, &[1.0, 0.0, 0.0, 1.0, offset.x, offset.y]))
}

/// Places vector content under a world matrix into a clip
/// ([`local_level2`]).
type PlanVector<'p, 'd> = dyn FnMut(
        &mut Scope<'_, 'd>,
        &Mat2d,
        pm_vector::render::scene::PixelRect,
    ) -> JsResult<Option<crate::vector::VectorQuad>>
    + 'p;

/// Level 2 device content of a 3D layer (`crate::space25d::local`):
/// `plan` places the content under a world matrix into a clip. It runs
/// once in content coordinates; a coverage scene is then projected through
/// the plane and covered at output resolution in `target`, and anything
/// else (a resvg raster) is planned again at the density its bounds need on
/// the plane and drawn as a layer-space texture by the level 2 resampler.
fn local_level2<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    origin: (f64, f64),
    target: pm_vector::render::scene::PixelRect,
    plan: &mut PlanVector<'_, 'd>,
) -> JsResult<Option<Quad>> {
    let clip = crate::space25d::local::unbounded();
    let rect_of = |q: &crate::vector::VectorQuad| match q {
        crate::vector::VectorQuad::Device { rect, .. } => Some(*rect),
        crate::vector::VectorQuad::Layer { .. } => None,
    };
    let Some(first) = plan(scope, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], clip)? else {
        return Ok(None);
    };
    let Some(r) = rect_of(&first) else {
        return Ok(Some(device_quad(first)));
    };
    let bounds = [
        f64::from(r.x),
        f64::from(r.y),
        f64::from(r.width),
        f64::from(r.height),
    ];
    let (dx, dy) = crate::space25d::local::density(scope, layer, time, scale, bounds)?;
    if let crate::vector::VectorQuad::Device {
        source: crate::vector::VectorSource::Scene { scene, .. },
        ..
    } = &first
        && let Some(plane) = crate::space25d::local::content_plane(scope, layer, time, scale)?
            .map(|p| plane_at_origin(p, origin))
    {
        let projected = crate::space25d::local::project_scene(scene, &plane, dx.max(dy));
        return Ok(projected.pixel_bounds(target).map(|rect| {
            device_quad(crate::vector::VectorQuad::Device {
                source: crate::vector::VectorSource::Scene {
                    scene: projected,
                    rect,
                },
                rect,
            })
        }));
    }
    let Some(second) = plan(scope, &[dx, 0.0, 0.0, dy, 0.0, 0.0], clip)? else {
        return Ok(None);
    };
    Ok(Some(match second {
        crate::vector::VectorQuad::Device { source, rect } => Quad::Texture {
            source: Source::Vector(Box::new(source)),
            w: f64::from(rect.width) / dx,
            h: f64::from(rect.height) / dy,
            ax: -f64::from(rect.x) / dx,
            ay: -f64::from(rect.y) / dy,
            uv: [0.0, 0.0, 1.0, 1.0],
        },
        layer_quad => device_quad(layer_quad),
    }))
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

/// A text or captions layer's raster quad (`crate::text`), at the density
/// `continuousRasterScale(scaledWorld(L, T, W, H))` asks for.
#[allow(clippy::too_many_arguments)]
fn text_quad<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    ty: &str,
    time: f64,
    target: (u32, u32),
    scale: (f64, f64),
) -> JsResult<Option<Quad>> {
    let comp_id = scope.comp.id().to_string_lossy();
    let Some(text) = b.text.as_deref_mut().filter(|t| t.has_fonts()) else {
        b.unsupported.push(Unsupported::layer(
            &comp_id,
            layer,
            Feature::Text(format!("{ty} layers without configured fonts")),
        ));
        return Ok(None);
    };
    if ty == "captions" && text.captions_hidden() {
        return Ok(None);
    }
    let world = scaled_world(scope, layer, time, scale, b.origin)?;
    // `compat` level 2: drawn at output resolution, no raster.
    if node::to_number(node::member(Some(layer), "compat")) >= 2.0 && scope.is_3d_layer(layer)? {
        return local_text(scope, b, layer, ty, time, scale);
    }
    if node::to_number(node::member(Some(layer), "compat")) >= 2.0 {
        let offset = scope.size_anchor_offset(layer, time)?;
        let m = mul(&world, &[1.0, 0.0, 0.0, 1.0, offset.x, offset.y]);
        let m = kurbo::Affine::new(m);
        let plan = if ty == "text" {
            text.text_layer_level2(scope, layer, time, m, target)?
        } else {
            text.captions_layer_level2(scope, layer, time, m, target)?
        };
        for what in text.take_unsupported() {
            b.unsupported
                .push(Unsupported::layer(&comp_id, layer, Feature::Text(what)));
        }
        return Ok(plan.map(|p| {
            let [x0, y0, x1, y1] = p.crop.map(f64::from);
            let rect = [x0, y0, x1 - x0, y1 - y0];
            Quad::Device {
                source: Source::Text(Box::new(p)),
                rect,
            }
        }));
    }
    let raster_scale = crate::text::continuous_raster_scale(&world);
    let quad = if ty == "text" {
        text.text_layer(scope, layer, time, raster_scale)?
    } else {
        text.captions_layer(scope, layer, time, raster_scale)?
    };
    for what in text.take_unsupported() {
        b.unsupported
            .push(Unsupported::layer(&comp_id, layer, Feature::Text(what)));
    }
    Ok(quad.map(|q| Quad::Texture {
        source: Source::Text(Box::new(q.plan)),
        w: q.w,
        h: q.h,
        ax: q.ax,
        ay: q.ay,
        uv: [0.0, 0.0, 1.0, 1.0],
    }))
}

/// Level 2 text or captions on a 2.5D plane: laid out in layer space at the
/// plane's density (`crate::space25d::local`) and drawn through the plane by
/// the level 2 resampler. The layout runs once at unit density, offset into
/// a large canvas, for its bounds.
fn local_text<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    ty: &str,
    time: f64,
    scale: (f64, f64),
) -> JsResult<Option<Quad>> {
    const MARGIN: f64 = 65536.0;
    let comp_id = scope.comp.id().to_string_lossy();
    let Some(text) = b.text.as_deref_mut().filter(|t| t.has_fonts()) else {
        return Ok(None);
    };
    let mut lay = |scope: &mut Scope<'_, 'd>, m: kurbo::Affine, target: (u32, u32)| {
        if ty == "text" {
            text.text_layer_level2(scope, layer, time, m, target)
        } else {
            text.captions_layer_level2(scope, layer, time, m, target)
        }
    };
    let side = (2.0 * MARGIN) as u32;
    let first = lay(
        scope,
        kurbo::Affine::translate((MARGIN, MARGIN)),
        (side, side),
    )?;
    let Some(first) = first else {
        for what in text.take_unsupported() {
            b.unsupported
                .push(Unsupported::layer(&comp_id, layer, Feature::Text(what)));
        }
        return Ok(None);
    };
    let [x0, y0, x1, y1] = first.crop.map(f64::from);
    let bounds = [x0 - MARGIN, y0 - MARGIN, x1 - x0, y1 - y0];
    let (dx, dy) = crate::space25d::local::density(scope, layer, time, scale, bounds)?;
    // Layer space at density, shifted so the content starts near the
    // canvas origin.
    let (ox, oy) = (
        (bounds[0] * dx).floor() - 1.0,
        (bounds[1] * dy).floor() - 1.0,
    );
    let size = (
        ((bounds[0] + bounds[2]) * dx - ox).ceil() as u32 + 2,
        ((bounds[1] + bounds[3]) * dy - oy).ceil() as u32 + 2,
    );
    let m = kurbo::Affine::new([dx, 0.0, 0.0, dy, -ox, -oy]);
    let second = lay(scope, m, size)?;
    for what in text.take_unsupported() {
        b.unsupported
            .push(Unsupported::layer(&comp_id, layer, Feature::Text(what)));
    }
    Ok(second.map(|p| {
        let [x0, y0, x1, y1] = p.crop.map(f64::from);
        Quad::Texture {
            source: Source::Text(Box::new(p)),
            w: (x1 - x0) / dx,
            h: (y1 - y0) / dy,
            ax: -(x0 + ox) / dx,
            ay: -(y0 + oy) / dy,
            uv: [0.0, 0.0, 1.0, 1.0],
        }
    }))
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

/// `videoPlaybackTime(asset, time, endPadding)` for container sources:
/// clamped to `[0, dur − endPadding]`. The engine then selects the frame by section 5 rule 7
/// (today's microsecond rounding only steers Chromium's seek to the same
/// frame).
pub use crate::retime::playback_time;

/// Today's `endPadding` for an output kind. The viewer's compositor seeks
/// with the default 0.04 s; `prepareFrame`, which every export and agent
/// still runs first, passes `1 / PM.proj.fps`, the exported composition's
/// rate (`fps` of the composition rendered, as expressions see it; at the
/// open composition that is `PM.proj.fps`). Precomps inside it keep that
/// padding, as `prepareFrame` does.
pub fn video_padding(video_end: VideoEnd, root_comp: &Comp<'_>) -> f64 {
    match video_end {
        VideoEnd::Preview => 0.04,
        VideoEnd::Final => 1.0 / root_comp.expression_fps().to_number(),
    }
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
    fn reduced_video_decode_respects_transformed_source_density() {
        let source = (1920.0, 1080.0);
        let full_uv = [0.0, 0.0, 1.0, 1.0];
        // A natural-size source clipped by a 640-wide composition still
        // projects to 960 pixels at Half preview resolution.
        let half_canvas = [960.0, 0.0, 0.0, 540.0, 0.0, 0.0];
        assert!(reduced_video_scale_safe(&half_canvas, full_uv, source, 0.5));
        assert!(!reduced_video_scale_safe(
            &half_canvas,
            full_uv,
            source,
            0.25
        ));
        // A cover crop samples only half the source width for the same quad.
        assert!(!reduced_video_scale_safe(
            &half_canvas,
            [0.25, 0.0, 0.5, 1.0],
            source,
            0.5,
        ));
        let small = [320.0, 0.0, 0.0, 180.0, 0.0, 0.0];
        assert!(reduced_video_scale_safe(&small, full_uv, source, 0.25));
    }

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
