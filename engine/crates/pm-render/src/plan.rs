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
use std::sync::Arc;

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

/// The [`OutputView`] of a level with output origin `origin`. A level at
/// `(0, 0)` has none unless `windowed`: then it shows only part of its
/// composition, so the output is not the composition's whole resolution.
fn level_view(
    scope: &Scope<'_, '_>,
    origin: (f64, f64),
    windowed: bool,
    width: u32,
    height: u32,
    scale: (f64, f64),
) -> Option<OutputView> {
    if origin == (0.0, 0.0) && !windowed {
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
    /// A solid layer's rectangle.
    Solid(Solid),
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

/// A solid layer: its premultiplied color over its box (the draw's unit
/// quad), with `radius` circular corners, covered analytically
/// (`fs_solid`).
#[derive(Debug, Clone, PartialEq)]
pub struct Solid {
    pub color: [f64; 4],
    /// The box in layer units (`d.w || W`, `d.h || H`).
    pub size: [f64; 2],
    /// `min(radius, |w|/2, |h|/2)`; 0 for none, a negative or a missing one.
    pub radius: f64,
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

/// A compatibility raster drawn as a source (identity: the raster itself).
#[derive(Debug, Clone)]
pub struct CompatDraw(pub std::sync::Arc<crate::compat::CompatRaster>);

impl PartialEq for CompatDraw {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Image {
        asset: String,
        path: PathBuf,
        /// The asset's Interpret Footage overrides (section 9).
        interpret: Option<pm_media::interpret::Interpretation>,
        /// The files the next frames of a playing image sequence show, read
        /// ahead on workers as a video's decoder runs ahead (empty for a
        /// still, and for a sequence while paused or exporting).
        ahead: Arc<[PathBuf]>,
        /// A sequence's file in a frame that draws Media Offline cards: a
        /// file found unreadable as it is drawn asks for the frame again,
        /// which draws its card ([`crate::media::still_unreadable`]).
        card_when_unreadable: bool,
        /// A sequence's file: the pixel size of its sequence (its first
        /// frame's). A file of another size is not drawn stretched: it is
        /// that frame's card, or a failure naming it.
        sequence_size: Option<(u32, u32)>,
    },
    Video {
        level2: bool,
        /// Persisted image-sequence/animation WebM (converter BT.601 default).
        sequence: bool,
        asset: String,
        path: PathBuf,
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
    /// Shape content (`crate::vector`).
    Vector(Box<crate::vector::VectorSource>),
    /// A shader layer or an extension fragment layer
    /// (`crate::shader_layers`).
    Shader(Box<crate::shader_layers::ShaderSource>),
    /// A text or captions raster (`crate::text`).
    Text(Box<crate::text::TextPlan>),
    /// A raster the document's renderer drew for the layer (a `raster`
    /// override's result; `crate::compat`).
    Compat(CompatDraw),
    /// Motion-blur samples accumulated with additive blending into a
    /// transparent target in the parent's pixel space (`crate::motion`).
    Accumulate(Box<CompPlan>),
}

/// An SVG image layer of a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgLayer {
    pub path: std::path::PathBuf,
    pub composition: String,
    pub layer: String,
}

/// A composition-space lease also depends on the raster frame it was
/// produced for; the same child composition can be displayed at two densities.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct InputPlanKey {
    composition: String,
    request: pm_fx_defs::inputs::Request,
    size: [u32; 2],
    scale: [u64; 2],
    origin: [u64; 2],
}

/// Inputs and outputs of a build.
pub struct Build<'r> {
    source_bytes: u64,
    source_raster: Option<(String, String, composite::effects::SourceRaster)>,
    input_active: std::collections::BTreeSet<(String, pm_fx_defs::inputs::Request)>,
    input_cache: std::collections::BTreeMap<InputPlanKey, crate::effects::resources::Producer>,
    pub assets: &'r dyn AssetResolver,
    pub failures: Vec<LayerFailure>,
    /// SVG image layers whose document has a feature the float painter
    /// does not draw (their file, composition and layer): the frame's SVG
    /// diagnostics name them.
    pub svg_layers: Vec<SvgLayer>,
    pub unsupported: Vec<Unsupported>,
    pub(crate) retime: RetimeCache,
    pub(crate) precomp_depth: u32,
    /// The 3D state of the compositing passes being built (innermost last).
    pass3d: Vec<crate::scene3d::Pass3d>,
    options3d: crate::scene3d::Options3d,
    material: Option<MaterialPlan>,
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
    /// Motion blur for layers with their switch on; `None` when the request
    /// has it off or inside a group's samples (today's `mblur: false`).
    pub(crate) blur: Option<crate::motion::Blur>,
    /// The composition time at which video sources select their frame
    /// inside motion-blur samples (the center time of their level).
    pub(crate) video_center: Option<f64>,
    pub(crate) studio_shutters: u32,
    /// The resampler of level 2 raster content.
    pub(crate) sampling: crate::sampling::Sampling,
    /// A playing viewer frame ([`BuildOptions::read_ahead`]).
    read_ahead: bool,
    /// The output origin of the level being built (the request's at the
    /// top level, in its groups, mattes, and motion-blur samples; `(0, 0)`
    /// inside a precomp, which renders its own target).
    pub(crate) origin: (f64, f64),
    /// The level is a window onto its composition: the request gave an
    /// origin, which can be `(0, 0)`. Follows `origin` into nested levels.
    windowed: bool,
    /// See [`BuildOptions::final_output`].
    pub(crate) final_output: bool,
    /// What every pass over one composition's layers reads of all of them
    /// (each boundary group is a pass), by the layer list's address and
    /// length (the document is borrowed for the whole build).
    passes: Vec<((usize, usize), std::sync::Arc<PassFacts>)>,
    /// The session's compatibility rasters (`crate::compat`): layers a
    /// producer draws, and what the frame still needs from it.
    compat: Option<&'r dyn crate::compat::CompatRasters>,
    pub compat_missing: Vec<crate::compat::CompatNeed>,
    /// See [`BuildOptions::offline_cards`].
    offline_cards: bool,
    /// [`BuildOptions::preview`].
    preview: Option<crate::compat_crop::PreviewWindow>,
    /// Layer draws kept from the last frame (`crate::reuse`): the
    /// interactive viewer's renders only.
    pub(crate) reuse: Option<&'r mut crate::reuse::DrawReuse>,
}

use crate::pass_facts::PassFacts;

impl Build<'_> {
    pub(crate) fn level_view(
        &self,
        scope: &Scope<'_, '_>,
        width: u32,
        height: u32,
        scale: (f64, f64),
    ) -> Option<OutputView> {
        level_view(scope, self.origin, self.windowed, width, height, scale)
    }
    fn charge_source(&mut self, bytes: u64) -> Result<(), Feature> {
        let total = self.source_bytes.checked_add(bytes);
        if total.is_none_or(|n| n > 384 * 1024 * 1024) {
            return Err(Feature::Definition(
                "Source rasters exceed framebuffer memory budget".into(),
            ));
        }
        self.source_bytes = total.unwrap();
        Ok(())
    }
    fn world<'d>(
        &self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        time: f64,
        scale: (f64, f64),
    ) -> JsResult<Mat2d> {
        if let Some((comp, id, raster)) = &self.source_raster
            && *comp == scope.comp.id().to_string_lossy()
            && *id == js::layer_id(layer)
        {
            return Ok(raster.world());
        }
        if let Some(material) = self.material.filter(|m| m.in_comp(scope.comp)) {
            let local = if material.owns(layer) {
                [1., 0., 0., 1., 0., 0.]
            } else {
                mul(&material.inverse, &scope.world_matrix(layer, time)?)
            };
            return Ok(mul(
                &[
                    scale.0,
                    0.,
                    0.,
                    scale.1,
                    -material.bounds.x * scale.0,
                    -material.bounds.y * scale.1,
                ],
                &local,
            ));
        }
        scaled_world(scope, layer, time, scale, self.origin)
    }
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
    /// Local material capture; addresses are compared only while the borrowed
    /// document is alive. No pointer is dereferenced or retained by a frame.
    pub material: Option<MaterialPlan>,
    /// Resolved motion blur (`crate::motion::resolve`).
    pub blur: Option<crate::motion::Blur>,
    pub sampling: crate::sampling::Sampling,
    /// A decision 19 fallback runner draws programs the native pipeline
    /// cannot translate (`crate::shader_layers::fallback`); without one
    /// they route.
    pub shader_fallback: bool,
    /// The viewer's visible window when this is a viewer frame of the open
    /// root composition (today's preview source clipping, which a `raster`
    /// override sees as its `crop` argument: `crate::compat_crop`).
    pub preview: Option<crate::compat_crop::PreviewWindow>,
    /// Draw the Media Offline card for offline media (frames that isolate
    /// failing layers); otherwise offline media is a layer failure.
    pub offline_cards: bool,
    /// The top level's output origin
    /// ([`RenderRequest::origin`](crate::RenderRequest::origin)).
    pub origin: Option<(f64, f64)>,
    /// Final output (exports, captures, render fills: `OutputKind::Final`):
    /// every level whose effects need the composition's pixel grid draws
    /// at the composition's density ([`comp_pass`]).
    pub final_output: bool,
    /// A playing viewer frame ([`RenderRequest::read_ahead`](crate::RenderRequest::read_ahead)):
    /// image sequences name the files their next frames show.
    pub read_ahead: bool,
}

/// A bounded source-local material canvas, without its receiving model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialPlan {
    pub composition: usize,
    pub layer: usize,
    pub inverse: Mat2d,
    pub bounds: pm_fx_defs::spatial::Extent,
    pub group: bool,
}
impl MaterialPlan {
    fn in_comp(self, comp: &Comp<'_>) -> bool {
        self.composition == std::ptr::from_ref(comp.node()) as usize
    }
    fn owns(self, layer: &Node) -> bool {
        self.layer == std::ptr::from_ref(layer) as usize
    }
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
        BuildOptions::default(),
        None,
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
        None,
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
    compat: Option<&'r dyn crate::compat::CompatRasters>,
) -> (CompPlan, Build<'r>) {
    build_reusing(
        ev, root_comp, project, time, width, height, scale, assets, effects, options3d, text,
        options, compat, None,
    )
}

/// [`build_with`] with the renderer's draw reuse.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_reusing<'r>(
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
    compat: Option<&'r dyn crate::compat::CompatRasters>,
    reuse: Option<&'r mut crate::reuse::DrawReuse>,
) -> (CompPlan, Build<'r>) {
    let mut reuse = reuse;
    if let Some(r) = reuse.as_deref_mut() {
        r.begin(
            root_comp.root(),
            root_comp.revision(),
            time,
            crate::reuse::BuildKey {
                sampling: options.sampling,
                blur: options.blur,
                view_override: options3d.view.is_some(),
                origin: options.origin.map(|(x, y)| (x.to_bits(), y.to_bits())),
                final_output: options.final_output,
                read_ahead: options.read_ahead,
            },
        );
    }
    let mut b = Build {
        source_raster: None,
        source_bytes: 0,
        assets,
        failures: Vec::new(),
        svg_layers: Vec::new(),
        unsupported: Vec::new(),
        retime: RetimeCache::new(),
        precomp_depth: 0,
        pass3d: Vec::new(),
        options3d,
        material: options.material,
        effects,
        matte_depth: 0,
        forced_on: None,
        input_active: Default::default(),
        input_cache: Default::default(),
        shader_fallback: options.shader_fallback,
        text,
        blur: options.blur,
        video_center: None,
        studio_shutters: 1,
        sampling: options.sampling,
        origin: options.origin.unwrap_or((0.0, 0.0)),
        windowed: options.origin.is_some(),
        final_output: options.final_output,
        read_ahead: options.read_ahead,
        passes: Vec::new(),
        compat,
        compat_missing: Vec::new(),
        preview: options.preview,
        offline_cards: options.offline_cards,
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
    pm_perf::bump(pm_perf::Counter::CompPasses);
    let comp_id = scope.comp.id().to_string_lossy();
    let all: Vec<&'d Node> = scope.layers().iter().collect();
    let background = if transparent {
        Background::Transparent
    } else {
        background(scope.comp)
    };
    let view = b.level_view(scope, width, height, scale);
    // The built-in pixel contract: an effect or transition without an owner
    // binding reads its pixel sizes (`u_texel`, `u_res`, `v_px`,
    // `texelFetch`) on the grid it is drawn on. At another density than the
    // composition's (a resized export, a viewer zoom or playback scale) a
    // level whose chains need the composition's grid (every such chain in
    // final output; in a viewer frame, one whose shader addresses physical
    // texels) is drawn whole at the composition's size and density, then
    // resampled to its output, so an export's look never depends on its size.
    // Electron extension effects use the output raster directly and do not
    // request this replay. A window onto the composition keeps its density.
    if view.is_none()
        && ((scale.0 - 1.0).abs() > 1e-9 || (scale.1 - 1.0).abs() > 1e-9)
        && let Some(host) = b.effects
    {
        let mut needs = false;
        for &layer in &all {
            match host.needs_composition_density(scope, layer, time, b.final_output) {
                Ok(true) => {
                    needs = true;
                    break;
                }
                Ok(false) => {}
                Err(_) => {}
            }
        }
        if needs {
            let (cw, ch) = scope.comp_size();
            let side = |c: f64, w: u32, s: f64| {
                if c.is_finite() && c > 0.0 {
                    c.round().max(1.0) as u32
                } else {
                    ((f64::from(w) / s).round().max(1.0)) as u32
                }
            };
            let (w1, h1) = (
                side(cw, width, scale.0.abs()),
                side(ch, height, scale.1.abs()),
            );
            let inner = comp_pass(
                scope,
                b,
                time,
                w1,
                h1,
                (1.0, 1.0),
                transparent,
                group_parent,
            );
            return CompPlan {
                composition: comp_id.clone(),
                group: group_parent.is_some(),
                width,
                height,
                background: Background::Transparent,
                // Resampled as level 2 raster content: the request's
                // resampler (Lanczos-3 in final output) and exact edges.
                draws: vec![Draw {
                    layer: String::new(),
                    matrix: [f64::from(width), 0.0, 0.0, f64::from(height), 0.0, 0.0],
                    content: Content::Placed(Box::new(Placed {
                        content: Content::Texture {
                            source: Source::Comp(Box::new(inner)),
                            uv: [0.0, 0.0, 1.0, 1.0],
                            alpha: 1.0,
                        },
                        plane: None,
                        level2: Some(b.sampling),
                    })),
                }],
                view,
            };
        }
    }
    let mut plan = CompPlan {
        composition: comp_id.clone(),
        group: group_parent.is_some(),
        width,
        height,
        background,
        draws: Vec::new(),
        view,
    };
    if group_parent.is_none()
        && b.precomp_depth == 0
        && let Some(material) = b.material.filter(|m| m.in_comp(scope.comp))
    {
        if let Some(layer) = all.iter().copied().find(|l| material.owns(l)) {
            match draw_layer(scope, b, layer, time, width, height, scale, false, &all) {
                Ok(Some(draws)) => plan.draws = draws,
                Ok(None) => {}
                Err(error) => {
                    let comp = scope.comp.clone();
                    layer_failure(b, &comp, layer, error.to_string());
                }
            }
        }
        plan.background = Background::Transparent;
        return plan;
    }
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
                pm_perf::bump(pm_perf::Counter::LayersPlanned);
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
    let material_root = b
        .material
        .is_some_and(|m| m.in_comp(scope.comp) && m.owns(layer));
    let alpha = if material_root {
        1.
    } else {
        js::clamp(scope.num(layer, "opacity", time)? / 100.0, 0.0, 1.0)
    };
    if alpha <= 0.001 {
        return Ok(None);
    }
    let comp_id = scope.comp.id().to_string_lossy();
    let refuse = |b: &mut Build<'_>, f: Feature| {
        b.unsupported.push(Unsupported::layer(&comp_id, layer, f));
    };
    let has_fx = scope.has_renderable_effects(layer, time)?;
    let source_owner = has_fx && node::is_str(node::member(Some(layer), "effectSpace"), "source");
    let has_masks = composite::build::has_masks(scope, layer, time)?;
    let blend = composite::build::blend_mode(scope, layer, time)?;
    // Effects prepared by the host (the effects lane); without one, a layer
    // with renderable effects is routed.
    let effects = if has_fx && !source_owner {
        let capture_owner = if let Some(material) = b.material.filter(|m| m.in_comp(scope.comp)) {
            Some(composite::effects::OwnerSpace {
                world: b.world(scope, layer, time, scale)?,
                bounds: material.owns(layer).then_some(material.bounds),
            })
        } else {
            None
        };
        let frame = composite::effects::EffectFrame {
            owner: capture_owner,
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
            Some(host) => match prepare_effects(scope, b, layer, frame, None)? {
                Ok(e) => e,
                Err(f) => {
                    if !host.bypasses_refusals() {
                        refuse(b, f);
                        return Ok(None);
                    }
                    layer_failure(b, scope.comp, layer, format!("effects bypassed: {f}"));
                    None
                }
            },
        }
    } else {
        None
    };
    let effects = match effects {
        Some(effects) => match effect_inputs(scope, b, layer, effects, width, height, scale, time)?
        {
            Ok(effects) => Some(effects),
            Err(f) => {
                if !b.effects.is_some_and(|h| h.bypasses_refusals()) {
                    refuse(b, f);
                    return Ok(None);
                }
                layer_failure(b, scope.comp, layer, format!("effects bypassed: {f}"));
                None
            }
        },
        None => None,
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
                owner: None,
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
        let masks = layer_masks(scope, b, layer, time, scale, width, height, has_masks)?;
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
            scope,
            b,
            layer,
            time,
            scale,
            width,
            height,
            has_masks && !source_owner,
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
    if source_owner {
        let n = blur.map_or(1, |b| b.samples.max(1));
        let mut draws = Vec::new();
        for sample in 0..n {
            let at = blur.map_or(time, |blur| {
                time + crate::motion::sample_offset(
                    sample,
                    n,
                    blur.shutter_frames,
                    crate::motion::level_fps(scope.comp),
                )
            });
            let (plan, raster) = match source_plan(
                scope,
                b,
                layer,
                at,
                width,
                height,
                scale,
                pm_fx_defs::inputs::Stage::Effects,
                None,
                true,
            )? {
                Ok(p) => p,
                Err(f) => {
                    refuse(b, f);
                    return Ok(None);
                }
            };
            let e = raster.extent;
            let world = b.world(scope, layer, at, scale)?;
            let opacity = if material_root {
                1.
            } else {
                js::clamp(scope.num(layer, "opacity", at)? / 100., 0., 1.)
            } / n as f64;
            draws.push(Draw {
                layer: id.clone(),
                matrix: mul(&world, &[e.width, 0., 0., e.height, e.x, e.y]),
                content: Content::Texture {
                    source: Source::Comp(Box::new(plan)),
                    uv: [0., 0., 1., 1.],
                    alpha: opacity,
                },
            });
        }
        if n > 1 {
            draws = vec![Draw {
                layer: id.clone(),
                matrix: full,
                content: Content::Texture {
                    source: Source::Accumulate(Box::new(CompPlan {
                        composition: comp_id.clone(),
                        group: true,
                        width,
                        height,
                        background: Background::Transparent,
                        draws,
                        view: b.level_view(scope, width, height, scale),
                    })),
                    uv: [0., 0., 1., 1.],
                    alpha: 1.,
                },
            }];
        }
        return wrap(scope, b, draws, 1.);
    }
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
    if matches!(quad, Quad::Screen { .. })
        && b.source_raster.as_ref().is_some_and(|(comp, id, _)| {
            *comp == scope.comp.id().to_string_lossy() && *id == js::layer_id(layer)
        })
    {
        return Err(pm_eval::JsThrow {
            kind: "EffectInputError",
            message: "Screen-space content cannot supply an unclipped source raster".into(),
        });
    }
    let world = b.world(scope, layer, time, scale)?;
    let id = js::layer_id(layer);
    let full = [f64::from(width), 0.0, 0.0, f64::from(height), 0.0, 0.0];
    let ty = js::layer_type(layer);
    let draw = match quad {
        Quad::Solid {
            rgb,
            a,
            w,
            h,
            radius,
            ax,
            ay,
        } => {
            let alpha = alpha * a;
            let plane = crate::space25d::projected(scope, layer, time, scale, w, h, ax, ay)?
                .map(|p| plane_at_origin(p, b.origin));
            // `NaN > 0` is false: a missing radius is none.
            let radius = if radius > 0.0 { radius } else { 0.0 };
            Draw {
                layer: id.clone(),
                matrix: mul(&world, &[w, 0.0, 0.0, h, -ax, -ay]),
                content: placed(
                    Content::Solid(Solid {
                        color: [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha],
                        size: [w, h],
                        radius: radius.min(w.abs() / 2.0).min(h.abs() / 2.0),
                    }),
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
            let plane = crate::space25d::projected(scope, layer, time, scale, w, h, ax, ay)?
                .map(|p| plane_at_origin(p, b.origin));
            // Electron projects legacy canvas sources through the same
            // bilinear sampler in 2D and 3D. Native level-2 planes retain
            // their output-density resampler.
            let sampling = (matches!(
                ty.as_str(),
                "image" | "video" | "precomp" | "shader" | "extension"
            ) || (plane.is_some() && crate::sampling::is_level2(layer)))
            .then_some(b.sampling);
            Draw {
                layer: id.clone(),
                matrix: mul(&world, &[w, 0.0, 0.0, h, -ax, -ay]),
                content: placed(Content::Texture { source, uv, alpha }, plane, sampling),
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
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    width: u32,
    height: u32,
    has_masks: bool,
) -> JsResult<Option<composite::Masks>> {
    if !has_masks {
        return Ok(None);
    }
    let world = b.world(scope, layer, time, scale)?;
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
        .map(|p| plane_at_origin(p, b.origin))
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
        view: b.level_view(scope, width, height, scale),
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
        radius: f64,
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
    if let Some(settings) = b.options3d.studio {
        let extrusion = node::member(node::member(Some(layer), "d"), "extrusion");
        if matches!(ty.as_str(), "text" | "shape")
            && node::truthy(node::member(Some(layer), "threeD"))
            && node::truthy(node::member(extrusion, "enabled"))
        {
            return Ok(Some(Quad::Screen {
                source: Source::Object3d(Box::new(Object3dPlan {
                    layer: js::layer_id(layer),
                    scene: std::sync::Arc::new(crate::scene3d::Scene3d::empty(width, height)),
                    kind: crate::scene3d::Render3d::Studio {
                        request: crate::scene3d::provider::SceneRequest {
                            composition: comp_id.clone(),
                            revision: settings.revision,
                            ticks: crate::compat::ticks_of(time).get(),
                            width,
                            height,
                        },
                        settings,
                        final_output: b.options3d.studio_final,
                        shutter_samples: b.studio_shutters,
                    },
                })),
            }));
        }
    }
    if let Some(quad) = compat_quad(scope, b, layer, &ty, time, scale)? {
        return Ok(quad);
    }
    let (wf, hf) = b
        .source_raster
        .as_ref()
        .filter(|(comp, id, _)| {
            *comp == scope.comp.id().to_string_lossy() && *id == js::layer_id(layer)
        })
        .map_or((f64::from(width), f64::from(height)), |(_, _, raster)| {
            (raster.bounds.width, raster.bounds.height)
        });
    let d = scope.ev.resolve_drawn_content(scope.comp, layer, time)?;
    match ty.as_str() {
        "shape" => {
            if !crate::vector::level2(layer) {
                let world = vector_world(scope, b, layer, time, scale)?;
                let density = crate::text::continuous_raster_scale(&world);
                return Ok(
                    crate::vector::legacy_raster(scope, layer, &d, time, density)?.map(|q| {
                        Quad::Texture {
                            source: Source::Vector(Box::new(q.source)),
                            w: q.w,
                            h: q.h,
                            ax: q.ax,
                            ay: q.ay,
                            uv: [0.0, 0.0, 1.0, 1.0],
                        }
                    }),
                );
            }
            if scope.is_3d_layer(layer)? {
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
            let world = vector_world(scope, b, layer, time, scale)?;
            let target = pm_vector::render::scene::PixelRect::new(0, 0, width, height);
            let quad = crate::vector::plan_shape(scope, layer, &d, time, &world, target)?;
            Ok(quad.map(device_quad))
        }
        "solid" => {
            // The inspector edits and animates `radius` on solids
            // (SOLID-RADIUS-1): the corners are rounded as authored.
            let color = js::string(js::get(&d, "color"));
            let c = pm_color::standard::parse_hex(&color);
            Ok(Some(Quad::Solid {
                rgb: c.rgb,
                a: c.alpha,
                w: js::or_number(js::get(&d, "w"), wf),
                h: js::or_number(js::get(&d, "h"), hf),
                radius: js::get(&d, "radius").to_number(),
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
                // Not offline yet: its file is still being looked up.
                if b.assets.resolving(&asset_id) {
                    return Ok(None);
                }
                // Frames that isolate failing layers (the viewer, agent
                // stills) draw the Media Offline card in the layer's box;
                // exports never do (the job refuses offline originals).
                if b.offline_cards {
                    let file =
                        js::member_string(record, "name").unwrap_or_else(|| asset_id.clone());
                    let bw = js::or_number(js::get(&d, "w"), wf);
                    let bh = js::or_number(js::get(&d, "h"), hf);
                    if let Some(card) = offline_card(
                        scope,
                        b,
                        layer,
                        time,
                        scale,
                        (width, height),
                        (bw, bh),
                        &file,
                    )? {
                        return Ok(card);
                    }
                }
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
                // Every SVG image is drawn at output resolution (`vector::svg`).
                // A document Chromium shows as a broken `<img>` paints nothing.
                let drawable = pm_media::image::svg::broken_image_file(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))
                    .and_then(|broken| {
                        if broken {
                            return Ok(None);
                        }
                        crate::vector::svg::document(&path)
                            .map(|(doc, _)| Some(doc.unsupported_feature().is_some()))
                    });
                match drawable {
                    Ok(Some(unsupported)) => {
                        if unsupported {
                            b.svg_layers.push(SvgLayer {
                                path: path.clone(),
                                composition: comp_id.clone(),
                                layer: js::layer_id(layer),
                            });
                        }
                    }
                    Ok(None) => return Ok(None),
                    Err(message) => {
                        b.failures.push(LayerFailure {
                            composition: comp_id,
                            layer: js::layer_id(layer),
                            message,
                        });
                        return Ok(None);
                    }
                }
                // `const bw = d.w || W, bh = d.h || H` with the asset's
                // `w`/`h` (its natural size, persisted).
                let sw = js::or_number(&rec("w"), 1.0);
                let sh = js::or_number(&rec("h"), 1.0);
                let bw = js::or_number(js::get(&d, "w"), wf);
                let bh = js::or_number(js::get(&d, "h"), hf);
                let uv = fit_uv(&js::string(js::get(&d, "fit")), sw, sh, bw, bh);
                let target = pm_vector::render::scene::PixelRect::new(0, 0, width, height);
                // Frames while playing reproject a cached source raster.
                let playback = b.sampling == crate::sampling::Sampling::Playback;
                // The document parsed above; a planning error is one there.
                let mut plan = |_: &mut Scope<'_, 'd>, world: &Mat2d, target| {
                    Ok(
                        crate::vector::svg::plan_svg(&path, bw, bh, uv, world, target, playback)
                            .ok()
                            .flatten()
                            .map(|q| *q),
                    )
                };
                if scope.is_3d_layer(layer)? {
                    return local_level2(scope, layer, time, scale, b.origin, target, &mut plan);
                }
                let world = vector_world(scope, b, layer, time, scale)?;
                return match crate::vector::svg::plan_svg(
                    &path, bw, bh, uv, &world, target, playback,
                ) {
                    Ok(quad) => Ok(quad.map(|q| device_quad(*q))),
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
                && let Some(manifest) = crate::media::native_sequence(&path)
            {
                return sequence_frame(
                    scope,
                    b,
                    layer,
                    time,
                    scale,
                    (width, height),
                    (wf, hf),
                    &d,
                    record,
                    asset_id,
                    &manifest,
                );
            }
            let sequence = crate::footage::is_sequence(record);
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
            let mut interpret = crate::footage::interpretation(record);
            let mut path = path;
            // Drawn from a proxy or optimized copy: the interpretation as
            // the copy carries it, or the original when it can't.
            if let Some(original) = b.assets.original(&asset_id).filter(|o| *o != path) {
                let i = interpret.take().unwrap_or_default();
                interpret = match crate::footage::for_copy(i, &original, sequence) {
                    crate::footage::CopyReading::Copy(i) => i,
                    crate::footage::CopyReading::Original(i) => {
                        path = original;
                        Some(i)
                    }
                };
            }
            let source = if ty == "video" {
                // Inside motion-blur samples video keeps the center time's
                // frame (`crate::motion`).
                let at = b.video_center.unwrap_or(time);
                let st = scope.source_time(layer, at, Some(&mut b.retime))?;
                let rate = interpret.as_ref().and_then(|i| i.frame_rate);
                // Native selection uses the source's PTS (or conformed
                // grid) and holds its last frame. Browser seek padding
                // hid valid frames and made preview differ from export.
                // A sequence's rendition shows the slot its files show
                // (`pm_media::image::sequence::sequence_slot`), at the
                // conformed rate when there is one.
                let time = sequence
                    .then(|| crate::media::sequence_playback_time(record, st, rate))
                    .flatten()
                    .unwrap_or_else(|| js::max(0.0, st));
                Source::Video {
                    level2: crate::sampling::is_level2(layer),
                    sequence,
                    asset: asset_id,
                    path,
                    time,
                    interpret,
                    composition_rate: scope.comp.rate().map_or(pm_time::Rate::FPS_30, |r| r.rate),
                }
            } else {
                Source::Image {
                    asset: asset_id,
                    path,
                    interpret,
                    ahead: Arc::from([]),
                    card_when_unreadable: false,
                    sequence_size: None,
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
            let (tw, th) = if crate::sampling::is_level2(layer)
                || node::truthy(node::member(Some(layer), "continuousRaster"))
            {
                // Continuous rasterization (section 8): the child renders
                // at the density it is drawn at, never below its own size.
                let density = if b.source_raster.as_ref().is_some_and(|(comp, id, _)| {
                    *comp == scope.comp.id().to_string_lossy() && *id == js::layer_id(layer)
                }) {
                    scale
                } else {
                    crate::sampling::precomp_density(scope, layer, time, scale, w, hh)?
                };
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
            let outer_windowed = std::mem::take(&mut b.windowed);
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
            b.windowed = outer_windowed;
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
                (wf.ceil() as u32, hf.ceil() as u32),
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
                    match if let Some(settings) = b.options3d.studio {
                        Ok(crate::scene3d::Object3dPlan {
                            layer: js::layer_id(layer),
                            scene: std::sync::Arc::new(crate::scene3d::Scene3d::empty(
                                width, height,
                            )),
                            kind: crate::scene3d::Render3d::Studio {
                                request: crate::scene3d::provider::SceneRequest {
                                    composition: comp_id.clone(),
                                    revision: settings.revision,
                                    ticks: crate::compat::ticks_of(time).get(),
                                    width,
                                    height,
                                },
                                settings,
                                final_output: b.options3d.studio_final,
                                shutter_samples: b.studio_shutters,
                            },
                        })
                    } else {
                        crate::scene3d::plan_layer(b.pass3d.last(), layer)
                    } {
                        Ok(plan) => Ok(Some(Quad::Screen {
                            source: Source::Object3d(Box::new(plan)),
                        })),
                        // The composition's scene did not build (today's
                        // `compositionScene` throws): every object layer
                        // draws nothing and fails, as the compositor's
                        // catch does (an export fails with it).
                        Err(message) => {
                            b.failures.push(LayerFailure {
                                composition: comp_id,
                                layer: js::layer_id(layer),
                                message,
                            });
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
                    // A legacy scene that does not sync, or a mesh layer
                    // that does not build: the compositor's catch falls
                    // through to the missing-definition checkerboard.
                    Some(Err(error)) => {
                        let mut planned = crate::shader_layers::plan::missing_placeholder(
                            scope,
                            layer,
                            &d,
                            time,
                            (width, height),
                            scale,
                            b.shader_fallback,
                        )?;
                        if let crate::shader_layers::plan::Planned::Draw { source, .. } =
                            &mut planned
                        {
                            source.scene_error = Some(error);
                        }
                        Ok(shader_quad(b, &comp_id, layer, planned))
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

/// A layer a compatibility producer draws (`crate::compat`): the layers
/// today's compositor rasterizes through `PM.raster` (text, captions, and
/// shapes, at `continuousRasterScale(scaledWorld(L, T, W, H))`), when the
/// session has a `raster` override. `None`: drawn natively. `Some(None)`:
/// nothing is drawn (between caption cues, or still missing: the frame
/// then needs it).
fn compat_quad<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    ty: &str,
    time: f64,
    scale: (f64, f64),
) -> JsResult<Option<Option<Quad>>> {
    use crate::compat::{CompatAnswer, CompatSource};
    let Some(compat) = b.compat else {
        return Ok(None);
    };
    if !matches!(ty, "text" | "captions" | "shape")
        || !compat.producing(CompatSource::RasterOverride)
    {
        return Ok(None);
    }
    // `compat` level 2 layers are the engine's own: no `PM.raster` today.
    let level2 = node::to_number(node::member(Some(layer), "compat")) >= 2.0
        || (ty == "shape" && crate::vector::level2(layer));
    if level2 {
        return Ok(None);
    }
    if ty == "captions" && b.text.as_deref().is_some_and(|t| t.captions_hidden()) {
        return Ok(Some(None));
    }
    let world = b.world(scope, layer, time, scale)?;
    let raster_scale = crate::text::continuous_raster_scale(&world);
    // Today's preview source clipping: the window the compositor passes to
    // `PM.raster` in a viewer frame, or no raster call at all.
    let crop = match preview_crop(scope, b, layer, ty, time, scale, raster_scale)? {
        crate::compat_crop::CropPlan::Outside => return Ok(Some(None)),
        crate::compat_crop::CropPlan::Solid => return Ok(None),
        crate::compat_crop::CropPlan::Full => None,
        crate::compat_crop::CropPlan::Crop(window) => Some(window),
    };
    let comp_id = scope.comp.id().to_string_lossy();
    let id = js::layer_id(layer);
    match compat.answer(
        &comp_id,
        &id,
        time,
        raster_scale,
        CompatSource::RasterOverride,
        crop,
    ) {
        CompatAnswer::Native => Ok(None),
        CompatAnswer::Raster(raster) => {
            let [w, h] = raster.size;
            // Today's compositor centres a plain shape's raster on the layer
            // whatever anchor it reports (`nativeContentQuad`): a wrapper that
            // passes a shape through without the crop argument hands back the
            // whole raster anchored at its padding, drawn centred all the same.
            let paths = node::member(node::member(Some(layer), "d"), "paths")
                .is_some_and(|v| matches!(v.kind(), pm_model::NodeKind::Array(a) if !a.is_empty()));
            let [ax, ay] = if ty == "shape" && !paths {
                [w / 2.0, h / 2.0]
            } else {
                raster.anchor
            };
            let uv = raster.uv;
            Ok(Some(Some(Quad::Texture {
                source: Source::Compat(CompatDraw(raster)),
                w,
                h,
                ax,
                ay,
                uv,
            })))
        }
        CompatAnswer::Missing => {
            let need = crate::compat::CompatNeed {
                composition: comp_id.as_str().into(),
                layer: id.as_str().into(),
                time: crate::compat::ticks_of(time),
                source: CompatSource::RasterOverride,
                scale: raster_scale,
                crop,
            };
            if !b.compat_missing.contains(&need) {
                b.compat_missing.push(need);
            }
            Ok(Some(None))
        }
        CompatAnswer::Failed(message) => {
            b.failures.push(LayerFailure {
                composition: comp_id,
                layer: id,
                message: format!("raster override failed: {message}"),
            });
            Ok(Some(None))
        }
    }
}

/// Today's preview source clipping of a layer `PM.raster` draws
/// (`nativeContentQuad`, `canClipPreviewSources`): only in a viewer frame
/// of the open root composition, for a plain 2D text or shape layer (no
/// paths, renderable effects, masks, track matte, or transitions; text
/// without animators, styles, or typography anchoring). Anything else is
/// drawn from the whole raster, as today.
#[allow(clippy::too_many_arguments)]
fn preview_crop<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    ty: &str,
    time: f64,
    scale: (f64, f64),
    raster_scale: f64,
) -> JsResult<crate::compat_crop::CropPlan> {
    use crate::compat_crop::{CropPlan, ShapeFacts, TextGeometry};
    let Some(window) = b.preview else {
        return Ok(CropPlan::Full);
    };
    if !matches!(ty, "text" | "shape")
        || !(scope.comp.at_root() || scope.comp.is_open_canonical())
        || b.precomp_depth > 0
    {
        return Ok(CropPlan::Full);
    }
    let d = node::member(Some(layer), "d");
    let listed = |n: Option<&Node>, key: &str| {
        node::member(n, key).is_some_and(|v| match v.kind() {
            pm_model::NodeKind::Array(items) => !items.is_empty(),
            _ => false,
        })
    };
    if listed(d, "paths")
        || listed(Some(layer), "masks")
        || node::truthy(node::member(Some(layer), "matteSource"))
        || node::truthy(node::member(Some(layer), "transitionIn"))
        || node::truthy(node::member(Some(layer), "transitionOut"))
        || scope.has_renderable_effects(layer, time)?
    {
        return Ok(CropPlan::Full);
    }
    // A 3D layer's plane: drawn from the whole raster here (see
    // docs/engine/extensions-compat.md, "Crop").
    if scope.is_3d_layer(layer)? {
        return Ok(CropPlan::Full);
    }
    let mut visible = vector_world(scope, b, layer, time, scale)?;
    visible[4] -= window.x;
    visible[5] -= window.y;
    if ty == "shape" {
        let content = scope.ev.resolve_content(scope.comp, layer, time)?;
        let (shape, _) = crate::vector::shape_content(&content);
        let color = js::string(js::get(&content, "color"));
        let hex6 = color.len() == 7
            && color.starts_with('#')
            && color[1..].bytes().all(|c| c.is_ascii_hexdigit());
        let plain_rect = matches!(js::get(&content, "shape"), pm_eval::JsValue::String(s) if s.to_string_lossy() == "rect")
            && hex6;
        let facts = ShapeFacts {
            w: shape.w,
            h: shape.h,
            stroke: shape.stroke,
            plain_rect,
            corner_inset: pm_vector::corner::corner_inset(&shape.corners()),
        };
        return Ok(crate::compat_crop::shape_plan(
            &facts,
            raster_scale,
            &visible,
            window.width,
            window.height,
        ));
    }
    if listed(d, "animators")
        || listed(d, "styles")
        || node::truthy(node::member(d, "fontAnchorBounds"))
    {
        return Ok(CropPlan::Full);
    }
    let Some(fonts) = b.text.as_deref_mut().and_then(|t| t.fonts.as_deref_mut()) else {
        return Ok(CropPlan::Full);
    };
    let placed = scope.text_raster(layer, time, fonts.measure())?;
    let g = TextGeometry::at(
        placed.w,
        placed.h,
        placed.anchor_x,
        placed.anchor_y,
        raster_scale,
    );
    Ok(crate::compat_crop::text_plan(
        &g,
        &visible,
        window.width,
        window.height,
    ))
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

/// Composition frames a playing image sequence reads ahead of the playhead.
const SEQUENCE_AHEAD: usize = 6;

/// An image sequence read in place: the frame its source time selects,
/// drawn as a still (at the file's depth, with its alpha, tags and
/// profile). A number with no file is that frame's Media Offline card
/// where frames isolate failing layers, and a named failure elsewhere.
#[allow(clippy::too_many_arguments)]
fn sequence_frame<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    size: (u32, u32),
    (wf, hf): (f64, f64),
    d: &pm_eval::Content,
    record: &Node,
    asset: String,
    manifest: &pm_media::image::sequence::Manifest,
) -> JsResult<Option<Quad>> {
    let interpret = crate::footage::interpretation(record);
    // Interpret Footage's rate, else the rate it was imported at (exact:
    // 24000/1001 for 23.976).
    let fps = interpret
        .as_ref()
        .and_then(|i| i.frame_rate)
        .or_else(|| manifest.rate())
        .map_or(30.0, |r| r.to_f64());
    // Inside motion-blur samples the center time's frame, as for video.
    let at = b.video_center.unwrap_or(time);
    let seconds = scope.source_time(layer, at, Some(&mut b.retime))?;
    let number = manifest.number_at(seconds, fps);
    let path = manifest.file(number);
    // While playing, the files of the next frames are read ahead: each is a
    // whole still to decode, and one frame's time is too short for that.
    let mut ahead = Vec::new();
    if b.read_ahead {
        let step = 1.0 / scope.comp.rate().map_or(30.0, |r| r.rate.to_f64());
        let mut last = number;
        for i in 1..=SEQUENCE_AHEAD {
            let s = scope.source_time(layer, at + i as f64 * step, Some(&mut b.retime))?;
            let next = manifest.number_at(s, fps);
            if next != last && next != number {
                ahead.push(manifest.file(next));
                last = next;
            }
        }
    }
    let bw = js::or_number(js::get(d, "w"), wf);
    let bh = js::or_number(js::get(d, "h"), hf);
    let rec = |k: &str| node::js_value_opt(node::member(Some(record), k));
    let sw = js::or_number(&rec("w"), 1.0);
    let sh = js::or_number(&rec("h"), 1.0);
    // A missing file, one that exists but can't be read (truncated,
    // corrupt, no permission) and one of another size than the sequence's
    // is that frame's Media Offline card.
    let missing = crate::media::stamp(&path).is_none();
    let other_size =
        crate::media::still_size(&path).filter(|&(w, h)| (w as f64, h as f64) != (sw, sh));
    if missing
        || other_size.is_some()
        || crate::media::still_unreadable(&path)
        || crate::media::decode_failed(&path)
    {
        let file = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if b.offline_cards
            && let Some(card) = offline_card(scope, b, layer, time, scale, size, (bw, bh), &file)?
        {
            return Ok(card);
        }
        let message = match other_size {
            _ if missing => format!("frame {number} of the image sequence is missing: {file}"),
            Some((w, h)) => other_size_message(Some(number), (w, h), (sw, sh), &file),
            // There, but it does not read: a damaged file.
            None => crate::DamagedFrame {
                file: path.clone(),
                seconds: None,
                reason: format!("frame {number} of the image sequence can't be read"),
            }
            .message(),
        };
        b.failures.push(LayerFailure {
            composition: scope.comp.id().to_string_lossy(),
            layer: js::layer_id(layer),
            message,
        });
        return Ok(None);
    }
    let uv = fit_uv(&js::string(js::get(d, "fit")), sw, sh, bw, bh);
    Ok(Some(Quad::Texture {
        source: Source::Image {
            asset,
            path,
            interpret,
            ahead: ahead.into(),
            card_when_unreadable: b.offline_cards,
            sequence_size: Some((sw as u32, sh as u32)),
        },
        w: bw,
        h: bh,
        ax: bw / 2.0,
        ay: bh / 2.0,
        uv,
    }))
}

/// Why a sequence's file of another size is not drawn.
pub(crate) fn other_size_message(
    number: Option<i64>,
    (w, h): (usize, usize),
    (sw, sh): (f64, f64),
    file: &str,
) -> String {
    let frame = number.map_or_else(|| "a frame".to_owned(), |n| format!("frame {n}"));
    format!("{frame} of the image sequence is {w}×{h}, not {sw}×{sh} like the sequence: {file}")
}

/// The Media Offline card in a layer's `bw`×`bh` box naming `file`, as
/// frames that isolate failing layers draw it; None when it cannot be drawn.
#[allow(clippy::too_many_arguments)]
fn offline_card<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &Build<'_>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    (width, height): (u32, u32),
    (bw, bh): (f64, f64),
    file: &str,
) -> JsResult<Option<Option<Quad>>> {
    let target = pm_vector::render::scene::PixelRect::new(0, 0, width, height);
    let playback = b.sampling == crate::sampling::Sampling::Playback;
    let world = vector_world(scope, b, layer, time, scale)?;
    // The box's height on screen picks how much the card says.
    let device_h = bh * world[2].hypot(world[3]);
    let tier = crate::offline_card::Tier::for_height(device_h);
    let Ok(card) = crate::offline_card::path(file, bw, bh, tier) else {
        return Ok(None);
    };
    let svg = |world: &Mat2d, target| {
        crate::vector::svg::plan_svg(&card, bw, bh, [0.0, 0.0, 1.0, 1.0], world, target, playback)
    };
    if scope.is_3d_layer(layer)? {
        let mut plan = |_: &mut Scope<'_, 'd>, world: &Mat2d, target| {
            Ok(svg(world, target).ok().flatten().map(|q| *q))
        };
        return local_level2(scope, layer, time, scale, b.origin, target, &mut plan).map(Some);
    }
    Ok(svg(&world, target)
        .ok()
        .map(|quad| quad.map(|q| device_quad(*q))))
}

/// The world matrix of a vector layer in target pixels, including the size
/// anchor offset `contentQuad` applies to its quad.
fn vector_world<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &Build<'_>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
) -> JsResult<Mat2d> {
    let world = b.world(scope, layer, time, scale)?;
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
    let Some(first) = plan(scope, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], clip)? else {
        return Ok(None);
    };
    let crate::vector::VectorQuad::Device { rect: r, .. } = first;
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
    let crate::vector::VectorQuad::Device { source, rect } = second;
    Ok(Some(Quad::Texture {
        source: Source::Vector(Box::new(source)),
        w: f64::from(rect.width) / dx,
        h: f64::from(rect.height) / dy,
        ax: -f64::from(rect.x) / dx,
        ay: -f64::from(rect.y) / dy,
        uv: [0.0, 0.0, 1.0, 1.0],
    }))
}

/// A planned vector quad as the render graph's quad.
fn device_quad(q: crate::vector::VectorQuad) -> Quad {
    match q {
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
    let world = b.world(scope, layer, time, scale)?;
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
    let level2 = node::to_number(node::member(Some(layer), "compat")) >= 2.0;
    if level2 && scope.is_3d_layer(layer)? {
        return local_text(scope, b, layer, ty, time, scale);
    }
    if level2 {
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
    // In 2D and 3D, Canvas2D first paints a layer-local bitmap at the compositor's
    // quarter-step density, then WebGL samples it through the world matrix.
    // Rasterizing after that matrix changes glyph phase, smoothing size and
    // edge filtering, even for a fractional translation with no rotation.
    let density = crate::text::continuous_raster_scale(&world);
    let quad = if ty == "text" {
        text.text_layer(scope, layer, time, density)?
    } else {
        text.captions_layer(scope, layer, time, density)?
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

/// Text or captions on a 2.5D plane: layout stays at its compatibility
/// level, then the ink is painted at the plane's local density and projected
/// through its full 3D transform.
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
    let level2 = node::to_number(node::member(Some(layer), "compat")) >= 2.0;
    let mut lay =
        |scope: &mut Scope<'_, 'd>, m: kurbo::Affine, target: (u32, u32)| match (ty, level2) {
            ("text", true) => text.text_layer_level2(scope, layer, time, m, target),
            ("captions", true) => text.captions_layer_level2(scope, layer, time, m, target),
            ("text", false) => text.text_layer_level1_device(scope, layer, time, m, target),
            ("captions", false) => text.captions_layer_level1_device(scope, layer, time, m, target),
            _ => unreachable!("text and captions only"),
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
    let (dx, dy) = (
        crate::space25d::local::stepped(dx, bounds[2]),
        crate::space25d::local::stepped(dy, bounds[3]),
    );
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

/// Build an unclipped local owner at the exact requested time. Only final
/// placement sees the parent transform and opacity; input leases retain this plan.
#[allow(clippy::too_many_arguments)]
fn source_plan<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    stage: pm_fx_defs::inputs::Stage,
    prefix: Option<usize>,
    grow: bool,
) -> JsResult<Result<(CompPlan, composite::effects::SourceRaster), Feature>> {
    use pm_fx_defs::spatial::Extent;
    let refuse = |s: &str| Feature::Definition(s.into());
    if matches!(
        js::layer_type(layer).as_str(),
        "group" | "adjustment" | "null" | "audio"
    ) || scope.is_3d_layer(layer)?
        || pm_eval::parenting::Role3d::of(layer).is_some()
    {
        return Ok(Err(refuse("Source raster requires a 2D content owner")));
    }
    // renderInputSource takes the complete content raster box, including
    // shape/path raster margins, rather than the selection's size bounds.
    let bounds = if js::layer_type(layer) == "shape" {
        if node::array(node::member(node::member(Some(layer), "d"), "paths"))
            .is_some_and(|a| !a.is_empty())
        {
            let (_, _, paths) = scope.path_geometry(layer, time)?;
            let r = paths.bounds;
            Some(Extent {
                x: r.x0,
                y: r.y0,
                width: r.w,
                height: r.h,
            })
        } else {
            let content = scope.ev.resolve_content(scope.comp, layer, time)?;
            let (shape, _) = crate::vector::shape_content(&content);
            let r = pm_vector::ShapeRaster::new(&shape, 1.);
            Some(Extent {
                x: -r.w / 2.,
                y: -r.h / 2.,
                width: r.w,
                height: r.h,
            })
        }
    } else {
        scope.size_bounds(layer, time)?.map(|r| Extent {
            x: r.x0,
            y: r.y0,
            width: r.x1 - r.x0,
            height: r.y1 - r.y0,
        })
    };
    let Some(mut bounds) = bounds else {
        return Ok(Err(refuse("Source raster bounds unavailable")));
    };
    let offset = scope.size_anchor_offset(layer, time)?;
    bounds.x += offset.x;
    bounds.y += offset.y;
    let mut raster = composite::effects::SourceRaster {
        bounds,
        extent: bounds,
    };
    if let Err(f) = raster.size() {
        return Ok(Err(f));
    }
    let active = scope.active_forced_on(layer, time)?;
    let effects = if active && stage == pm_fx_defs::inputs::Stage::Effects {
        let Some(host) = b.effects else {
            return Ok(Err(Feature::Effects));
        };
        let prepared = host.prepare_source(
            scope,
            layer,
            composite::effects::EffectFrame {
                owner: None,
                width,
                height,
                scale,
                time,
            },
            prefix,
            raster,
            grow,
        );
        b.failures.extend(host.take_bypassed());
        let (effects, grown) = match prepared? {
            Ok(e) => e,
            Err(f) if host.bypasses_refusals() => {
                layer_failure(b, scope.comp, layer, format!("effects bypassed: {f}"));
                (None, raster)
            }
            Err(f) => return Ok(Err(f)),
        };
        raster = grown;
        match effects {
            Some(e) => match effect_inputs(scope, b, layer, e, width, height, scale, time)? {
                Ok(e) => Some(e),
                Err(f) => return Ok(Err(f)),
            },
            None => None,
        }
    } else {
        None
    };
    let (w, h) = match raster.size() {
        Ok(s) => {
            if active {
                s
            } else {
                (1, 1)
            }
        }
        Err(f) => return Ok(Err(f)),
    };
    // Main's default framebuffer allowance, with its six-surface reserve.
    // Charge at float32 (the largest working format) before allocating any GPU target.
    let charge = match &effects {
        Some(e) => e.passes.source_allocation_bytes((w, h)),
        None => Some(u64::from(w) * u64::from(h) * 16 * 6),
    };
    let Some(charge) = charge else {
        return Ok(Err(refuse("Effect host has no source allocation estimate")));
    };
    if let Err(f) = b.charge_source(charge) {
        return Ok(Err(f));
    }
    let old_raster = b.source_raster.replace((
        scope.comp.id().to_string_lossy(),
        js::layer_id(layer),
        raster,
    ));
    let old_origin = std::mem::replace(&mut b.origin, (0., 0.));
    let old_windowed = std::mem::take(&mut b.windowed);
    let old_preview = b.preview.take();
    // Typed source requests and source-owner shutter samples select their exact
    // time, rather than the ordinary motion path's held center video frame.
    let old_video_center = b.video_center.take();
    let result = (|| -> JsResult<Result<CompPlan, Feature>> {
        let content = if active {
            layer_draw(
                scope,
                b,
                layer,
                time,
                w,
                h,
                (raster.world()[0], raster.world()[3]),
                1.,
            )?
            .into_iter()
            .collect()
        } else {
            vec![]
        };
        let masks = if active && stage != pm_fx_defs::inputs::Stage::Source {
            match composite::build::masks(scope, layer, time, raster.world(), None, w, h)? {
                composite::build::MaskRoute::Unmasked => None,
                composite::build::MaskRoute::Parametric(m)
                | composite::build::MaskRoute::Path(m) => Some(m),
            }
        } else {
            None
        };
        Ok(Ok(CompPlan {
            composition: scope.comp.id().to_string_lossy(),
            group: false,
            width: w,
            height: h,
            background: Background::Transparent,
            view: None,
            draws: vec![Draw {
                layer: js::layer_id(layer),
                matrix: [w as f64, 0., 0., h as f64, 0., 0.],
                content: Content::Composite(Box::new(composite::Node::Layer(
                    composite::LayerNode {
                        layer: js::layer_id(layer),
                        content,
                        masks,
                        effects,
                        matte: None,
                        blend: BlendMode::Normal,
                        alpha: 1.,
                        transition: None,
                    },
                ))),
            }],
        }))
    })();
    b.source_raster = old_raster;
    b.origin = old_origin;
    b.windowed = old_windowed;
    b.preview = old_preview;
    b.video_center = old_video_center;
    Ok(result?.map(|p| (p, raster)))
}

fn prepare_effects<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    frame: composite::effects::EffectFrame,
    prefix: Option<usize>,
) -> JsResult<Result<Option<composite::PreparedEffects>, Feature>> {
    let host = b.effects.expect("effect host checked by caller");
    let result = host.prepare_measured(
        scope,
        layer,
        frame,
        prefix,
        b.text.as_deref_mut().and_then(|t| t.measure()),
    );
    b.failures.extend(host.take_bypassed());
    result
}

#[allow(clippy::too_many_arguments)]
fn effect_inputs<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    owner: &'d Node,
    mut effects: composite::PreparedEffects,
    width: u32,
    height: u32,
    scale: (f64, f64),
    time: f64,
) -> JsResult<Result<composite::PreparedEffects, Feature>> {
    loop {
        let bytes = b.source_bytes;
        let mut failed = None;
        match effect_inputs_once(
            scope,
            b,
            owner,
            effects.clone(),
            width,
            height,
            scale,
            time,
            &mut failed,
        )? {
            Ok(effects) => return Ok(Ok(effects)),
            Err(feature) => {
                if !b.effects.is_some_and(|h| h.bypasses_refusals()) {
                    return Ok(Err(feature));
                }
                let Some(instance) = failed else {
                    return Ok(Err(feature));
                };
                let Some(passes) = effects.passes.without_effect(&instance) else {
                    return Ok(Err(feature));
                };
                b.source_bytes = bytes;
                b.failures.push(crate::LayerFailure {
                    composition: scope.comp.id().to_string_lossy(),
                    layer: js::layer_id(owner),
                    message: format!("effect {instance} bypassed: {feature}"),
                });
                effects.passes = passes;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn effect_inputs_once<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    owner: &'d Node,
    mut effects: composite::PreparedEffects,
    width: u32,
    height: u32,
    scale: (f64, f64),
    time: f64,
    failed: &mut Option<String>,
) -> JsResult<Result<composite::PreparedEffects, Feature>> {
    use pm_fx_defs::inputs::{Space, Stage};
    let requests = effects.passes.input_requests();
    if requests.is_empty() {
        return Ok(Ok(effects));
    }
    let mut producers = vec![];
    for reference in requests {
        *failed = Some(reference.effect.clone());
        let request = reference.request;
        let active_key = (scope.comp.id().to_string_lossy(), request.clone());
        let cache_key = InputPlanKey {
            composition: active_key.0.clone(),
            request: request.clone(),
            size: [width, height],
            scale: [scale.0.to_bits(), scale.1.to_bits()],
            origin: [b.origin.0.to_bits(), b.origin.1.to_bits()],
        };
        if let Some(producer) = b.input_cache.get(&cache_key).cloned() {
            // The executor currently retains separate consumer targets. A
            // shared plan is not a shared GPU allocation across chains.
            if let Err(f) = b.charge_source(producer.source_bytes) {
                return Ok(Err(f));
            }
            producers.push(producer);
            continue;
        }
        if b.input_active.contains(&active_key) {
            return Ok(Err(Feature::Definition(format!(
                "Effect input cycle at {}",
                request.layer_id
            ))));
        }
        if b.input_cache.len() + b.input_active.len() >= pm_fx_defs::graph::MAX_FRAME_INPUTS {
            return Ok(Err(Feature::Definition(
                "Effect input lease limit exceeded".into(),
            )));
        }
        let source = scope
            .layers()
            .iter()
            .find(|l| js::layer_id(l) == request.layer_id);
        let Some(source) = source else {
            return Ok(Err(Feature::Definition(format!(
                "Effect input layer is missing: {}",
                request.layer_id
            ))));
        };
        if matches!(js::layer_type(source).as_str(), "group" | "adjustment")
            || js::flag(source, "threeD")
        {
            return Ok(Err(Feature::Definition(
                "Effect input group/3D adapter unavailable".into(),
            )));
        }
        let at = request.time().unwrap_or(time);
        let source_before = b.source_bytes;
        if request.space == Space::Source {
            b.input_active.insert(active_key.clone());
            let result = source_plan(
                scope,
                b,
                source,
                at,
                width,
                height,
                scale,
                request.stage,
                request.prefix,
                node::is_str(node::member(Some(source), "effectSpace"), "source"),
            );
            b.input_active.remove(&active_key);
            let (plan, raster) = match result? {
                Ok(p) => p,
                Err(f) => return Ok(Err(f)),
            };
            let producer = crate::effects::resources::Producer {
                source_bytes: b.source_bytes - source_before,
                source_extent: Some(raster.extent),
                request: request.clone(),
                plan: std::sync::Arc::new(plan),
                owner: raster.owner(),
            };
            b.input_cache.insert(cache_key, producer.clone());
            producers.push(producer);
            continue;
        }
        let owner_binding = match crate::effects::plan::owner(
            scope,
            source,
            at,
            scale,
            &mut b.text.as_deref_mut().and_then(|t| t.measure()),
        )? {
            Ok(o) => o,
            Err(f) => return Ok(Err(f)),
        };
        b.input_active.insert(active_key.clone());
        let result = (|| -> JsResult<Result<crate::effects::resources::Producer, Feature>> {
            let mut content = vec![];
            let active = scope.active_forced_on(source, at)?;
            let source_owner = request.stage == Stage::Effects
                && node::is_str(node::member(Some(source), "effectSpace"), "source");
            if active && source_owner {
                let (plan, raster) = match source_plan(
                    scope,
                    b,
                    source,
                    at,
                    width,
                    height,
                    scale,
                    Stage::Effects,
                    request.prefix,
                    true,
                )? {
                    Ok(p) => p,
                    Err(f) => return Ok(Err(f)),
                };
                let e = raster.extent;
                content.push(Draw {
                    layer: js::layer_id(source),
                    matrix: mul(
                        &scaled_world(scope, source, at, scale, b.origin)?,
                        &[e.width, 0., 0., e.height, e.x, e.y],
                    ),
                    content: Content::Texture {
                        source: Source::Comp(Box::new(plan)),
                        uv: [0., 0., 1., 1.],
                        alpha: 1.,
                    },
                });
            }
            if active
                && !source_owner
                && let Some(draw) = layer_draw(scope, b, source, at, width, height, scale, 1.)?
            {
                content.push(draw);
            }
            let masks = if request.stage != Stage::Source && !source_owner {
                let has = composite::build::has_masks(scope, source, at)?;
                layer_masks(scope, b, source, at, scale, width, height, has)?
            } else {
                None
            };
            let nested = if request.stage == Stage::Effects && active && !source_owner {
                if b.effects.is_none() {
                    return Ok(Err(Feature::Effects));
                }
                match prepare_effects(
                    scope,
                    b,
                    source,
                    composite::effects::EffectFrame {
                        owner: None,
                        width,
                        height,
                        scale,
                        time: at,
                    },
                    request.prefix,
                )? {
                    Ok(Some(e)) => {
                        if e.backdrop {
                            return Ok(Err(Feature::Definition(
                                "Effect input backdrop producer unavailable".into(),
                            )));
                        }
                        match effect_inputs(scope, b, source, e, width, height, scale, at)? {
                            Ok(e) => Some(e),
                            Err(f) => return Ok(Err(f)),
                        }
                    }
                    Ok(None) => None,
                    Err(f) => return Ok(Err(f)),
                }
            } else {
                None
            };
            // Source and mask stages precede the track matte. A complete
            // composition-space effects producer includes it; an exclusive
            // effect prefix (including Echo history) deliberately omits it.
            let matte = if active && request.stage == Stage::Effects && request.prefix.is_none() {
                layer_matte(scope, b, source, at, width, height, scale)?
            } else {
                None
            };
            let draws = vec![Draw {
                layer: js::layer_id(owner),
                matrix: [width as f64, 0., 0., height as f64, 0., 0.],
                content: Content::Composite(Box::new(composite::Node::Layer(
                    composite::LayerNode {
                        layer: js::layer_id(source),
                        content,
                        masks,
                        effects: nested,
                        matte,
                        blend: pm_protocol::color::BlendMode::Normal,
                        alpha: 1.,
                        transition: None,
                    },
                ))),
            }];
            Ok(Ok(crate::effects::resources::Producer {
                source_bytes: b.source_bytes - source_before,
                source_extent: None,
                request: request.clone(),
                owner: owner_binding,
                plan: std::sync::Arc::new(CompPlan {
                    composition: scope.comp.id().to_string_lossy(),
                    group: true,
                    width,
                    height,
                    background: Background::Transparent,
                    draws,
                    view: b.level_view(scope, width, height, scale),
                }),
            }))
        })();
        b.input_active.remove(&active_key);
        let producer = match result? {
            Ok(p) => p,
            Err(f) => return Ok(Err(f)),
        };
        b.input_cache.insert(cache_key, producer.clone());
        producers.push(producer);
    }
    *failed = None;
    if let Some(passes) = effects.passes.with_input_plans(producers) {
        effects.passes = passes;
        Ok(Ok(effects))
    } else {
        Ok(Err(Feature::Definition(
            "Effect host cannot bind producer plans".into(),
        )))
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
