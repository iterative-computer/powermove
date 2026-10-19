//! Blender surfaces in the render graph (plan sections 8, 9, 17; contract
//! 11; amendment 11). Design notes: `docs/engine/blender.md`.
//!
//! With `render3d.enabled`, each 3D object layer's pixels come from
//! Blender: one surface per layer, frame, shutter sample, and nested
//! composition, drawn as a source node with its crop rectangle
//! (`Source::Blender`, a target-space quad over the crop), so masks,
//! effects, opacity, and blending stay per layer exactly as for the native
//! 3D pass. A surface holds premultiplied E values (decoded `code / 65535`,
//! premultiplied at f32, `pm_blender::surface`); with linear blending on the
//! compositor decodes it like the native 3D output (section 9, "3D layers").
//!
//! How a render treats Blender comes with the request
//! ([`RenderRequest::blender`](crate::RenderRequest::blender)):
//!
//! - [`BlenderRender::Native`]: draft shading — the native 3D pass draws
//!   (viewer playback, Solid and Wireframe shading, compositions without
//!   Blender).
//! - [`BlenderRender::Final`]: an export or agent capture. Compositions with
//!   `render3d.enabled` draw the surfaces the Blender stage prepared for
//!   this frame; a missing one fails the layer with today's sentence, and
//!   exports fail on it (contract 11 invariant 7: never native shading in
//!   place of a Blender surface).
//! - [`BlenderRender::Preview`]: the paused viewer in Material Preview or
//!   Rendered shading. Every composition with 3D objects builds its
//!   preview snapshot (preview samples and scale; EEVEE for Material
//!   Preview), draws the surfaces the host has for it, and draws natively
//!   while it asks for the rest — today's paused rendered preview.
//!
//! [`crate::Renderer::blender_snapshots`] plans a frame without drawing and
//! returns the snapshot of every Blender frame it needs (per nested
//! composition, at the plan's own times and sizes, so lookups always
//! match). [`frames`] is the port of today's `blenderFrameRequests` (shutter
//! samples included), checked against the corpus.

pub mod frames;
pub(crate) mod gpu;
pub mod snapshot;
mod snapshot_gltf;

use std::collections::HashSet;
use std::sync::Arc;

use pm_blender::snapshot::{BlenderFrame, FrameKey, Render3dSettings, Snapshot};
use pm_blender::{DecodedSurface, job::Surfaces};
use pm_eval::{JsResult, Scope};
use pm_model::Node;

use crate::assets::AssetResolver;
use crate::js;

pub use snapshot::SnapshotOptions;

/// Today's sentence for a final render whose Blender frame is missing
/// (`compositor.ts`).
pub const NOT_PREPARED: &str =
    "The Blender frame is not prepared. Render this frame before capturing it.";

/// The surfaces a paused viewer has, and where it reports the ones it
/// lacks (the interactive engine's preview cache).
pub trait PreviewSurfaces: Send + Sync {
    /// Material Preview (EEVEE) and the editor camera.
    fn options(&self) -> SnapshotOptions;
    /// The surface of `layer` for `snapshot`, when rendered.
    fn lookup(&self, snapshot: &Snapshot, layer: &str) -> Option<Arc<DecodedSurface>>;
    /// A preview snapshot with at least one missing surface.
    fn wanted(&self, snapshot: Snapshot);
}

/// How a render treats Blender compositions.
#[derive(Clone, Default)]
pub enum BlenderRender {
    #[default]
    Native,
    Final(Arc<Surfaces>),
    Preview(Arc<dyn PreviewSurfaces>),
}

impl std::fmt::Debug for BlenderRender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlenderRender::Native => write!(f, "Native"),
            BlenderRender::Final(s) => write!(f, "Final({} frames)", s.len()),
            BlenderRender::Preview(_) => write!(f, "Preview"),
        }
    }
}

impl PartialEq for BlenderRender {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (BlenderRender::Native, BlenderRender::Native) => true,
            (BlenderRender::Final(a), BlenderRender::Final(b)) => Arc::ptr_eq(a, b),
            (BlenderRender::Preview(a), BlenderRender::Preview(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// One Blender surface drawn as a source.
#[derive(Clone)]
pub struct BlenderSource {
    pub layer: String,
    pub compatibility: bool,
    pub surface: Arc<DecodedSurface>,
}

impl std::fmt::Debug for BlenderSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BlenderSource({} {}×{} crop {:?})",
            self.layer, self.surface.size.width, self.surface.size.height, self.surface.crop
        )
    }
}

impl PartialEq for BlenderSource {
    fn eq(&self, other: &Self) -> bool {
        self.layer == other.layer
            && self.compatibility == other.compatibility
            && Arc::ptr_eq(&self.surface, &other.surface)
    }
}

/// Collecting the snapshots a frame needs.
pub(crate) struct Collect {
    pub preview: bool,
    pub options: SnapshotOptions,
    pub snapshots: Vec<Snapshot>,
    /// Frames whose snapshot cannot be built, with why.
    pub errors: Vec<(BlenderFrame, String)>,
}

/// The Blender state of one plan build.
#[derive(Default)]
pub(crate) struct PlanState {
    pub render: BlenderRender,
    pub collect: Option<Collect>,
    /// Composition ids of the precomps being planned, outermost first.
    pub path: Vec<String>,
    /// Frames already handled in this build.
    seen: HashSet<FrameKey>,
    /// Preview snapshots built in this build, by frame.
    previews: Vec<(FrameKey, Option<Arc<Snapshot>>)>,
}

impl PlanState {
    pub fn new(render: BlenderRender) -> PlanState {
        PlanState {
            render,
            ..Default::default()
        }
    }

    pub fn collecting(preview: bool, options: SnapshotOptions) -> PlanState {
        PlanState {
            collect: Some(Collect {
                preview,
                options,
                snapshots: Vec::new(),
                errors: Vec::new(),
            }),
            ..Default::default()
        }
    }
}

/// What the plan does with a 3D object layer.
pub(crate) enum Planned {
    /// Draw this surface over `rect` (target pixels: x, y, width, height).
    Draw(BlenderSource, [f64; 4]),
    /// The layer fails with this message.
    Fail(String),
}

/// Whether the composition renders with Blender (`render3d.enabled`).
pub fn blender_enabled(comp: &pm_eval::Comp<'_>) -> bool {
    pm_eval::node::truthy(pm_eval::node::member(comp.member("render3d"), "enabled"))
}

/// The size of a preview frame: today's preview renders each composition at
/// its own size (times the preview scale, in the snapshot), whatever the
/// viewer's zoom, so zooming and panning reuse its surfaces.
fn preview_size(comp: &pm_eval::Comp<'_>) -> (u32, u32) {
    let px = |key: &str| {
        let v = pm_eval::node::to_number(comp.member(key));
        if v.is_finite() && v >= 1.0 {
            v.round().min(f64::from(u32::MAX)) as u32
        } else {
            1
        }
    };
    (px("w"), px("h"))
}

fn frame_of(
    state: &PlanState,
    comp_id: &str,
    time: f64,
    width: u32,
    height: u32,
    preview: bool,
) -> BlenderFrame {
    let mut path = state.path.clone();
    if path.last().map(String::as_str) != Some(comp_id) {
        path.push(comp_id.to_owned());
    }
    BlenderFrame {
        composition_path: path,
        time,
        width,
        height,
        shutter: None,
        preview,
    }
}

fn draw(
    layer: &str,
    surface: Arc<DecodedSurface>,
    width: u32,
    height: u32,
    compatibility: bool,
) -> Planned {
    let crop = gpu::crop(&surface, compatibility);
    let r = [
        f64::from(crop.x) / f64::from(surface.size.width),
        f64::from(crop.y) / f64::from(surface.size.height),
        f64::from(crop.width) / f64::from(surface.size.width),
        f64::from(crop.height) / f64::from(surface.size.height),
    ];
    let (w, h) = (f64::from(width), f64::from(height));
    Planned::Draw(
        BlenderSource {
            layer: layer.to_owned(),
            compatibility,
            surface,
        },
        [r[0] * w, r[1] * h, r[2] * w, r[3] * h],
    )
}

/// The plan of a 3D object layer when Blender is involved; `None` lets the
/// native 3D pass draw it. `in_samples`: the layer is drawn at a
/// motion-blur sample time (exports render a surface per shutter sample;
/// the paused viewer draws samples natively, as today, whose preview
/// frames exist only at the frame's own time).
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_object<'d>(
    scope: &mut Scope<'_, 'd>,
    state: &mut PlanState,
    assets: &dyn AssetResolver,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    in_samples: bool,
) -> JsResult<Option<Planned>> {
    let comp_id = scope.comp.id().to_string_lossy();
    let enabled = blender_enabled(scope.comp);
    let id = js::layer_id(layer);
    let level2 = pm_eval::node::to_number(pm_eval::node::member(Some(layer), "compat")) >= 2.0;
    let compatibility = !level2;
    if let Some(collect) = &state.collect {
        let preview = collect.preview;
        if !(enabled || preview) || (preview && in_samples) {
            return Ok(None);
        }
        let (fw, fh) = if preview {
            preview_size(scope.comp)
        } else {
            (width, height)
        };
        let frame = frame_of(state, &comp_id, time, fw, fh, preview);
        if state.seen.insert(frame.lookup()) {
            let options = collect.options.clone();
            let built = snapshot::build(scope, &frame, assets, &options)?;
            let collect = state.collect.as_mut().expect("collecting");
            match built {
                Ok(s) => collect.snapshots.push(s),
                Err(e) => collect.errors.push((frame, e)),
            }
        }
        return Ok(None);
    }
    match state.render.clone() {
        BlenderRender::Native => Ok(None),
        BlenderRender::Final(surfaces) => {
            if !enabled {
                return Ok(None);
            }
            let key = frame_of(state, &comp_id, time, width, height, false).lookup();
            Ok(Some(match surfaces.get(&key).and_then(|f| f.get(&id)) {
                Some(s) => draw(&id, s.clone(), width, height, compatibility),
                None => Planned::Fail(NOT_PREPARED.to_owned()),
            }))
        }
        BlenderRender::Preview(host) => {
            if in_samples {
                return Ok(None);
            }
            let (fw, fh) = preview_size(scope.comp);
            let frame = frame_of(state, &comp_id, time, fw, fh, true);
            let key = frame.lookup();
            let snapshot = match state.previews.iter().find(|(k, _)| *k == key) {
                Some((_, s)) => s.clone(),
                None => {
                    let built = snapshot::build(scope, &frame, assets, &host.options())?
                        .ok()
                        .map(Arc::new);
                    if let Some(s) = &built
                        && s.layers().iter().any(|l| host.lookup(s, l).is_none())
                    {
                        host.wanted((**s).clone());
                    }
                    state.previews.push((key, built.clone()));
                    built
                }
            };
            Ok(snapshot
                .and_then(|s| host.lookup(&s, &id))
                .map(|surface| draw(&id, surface, width, height, compatibility)))
        }
    }
}

/// The snapshots of a frame ([`crate::Renderer::blender_snapshots`]).
#[derive(Debug, Default)]
pub struct BlenderSnapshots {
    /// One per Blender frame, in plan order.
    pub snapshots: Vec<Snapshot>,
    /// Frames whose snapshot could not be built, with why (an imported
    /// model, a recipe part, missing media, invalid settings).
    pub errors: Vec<(BlenderFrame, String)>,
}

impl BlenderSnapshots {
    pub(crate) fn from_build(state: PlanState) -> BlenderSnapshots {
        match state.collect {
            Some(c) => BlenderSnapshots {
                snapshots: c.snapshots,
                errors: c.errors,
            },
            None => BlenderSnapshots::default(),
        }
    }
}

/// Settings of a composition (`render3DSchema.parse`).
pub fn settings_of(comp: &pm_eval::Comp<'_>) -> Result<Render3dSettings, String> {
    let stored = comp.member("render3d").map(Node::to_value);
    Render3dSettings::parse(stored.as_ref().filter(|v| !v.is_null()))
}
