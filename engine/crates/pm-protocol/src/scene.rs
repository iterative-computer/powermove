//! `EvaluatedScene` (contract 1, sections 7, 8, 10).
//!
//! The output of `pm-eval` for one (document revision, composition, time,
//! optional view): everything the render graph needs and nothing it must
//! re-derive. 2D layers, 3D worlds, depth sets, and ordering are separate
//! computations and separate fields; depth sets decide shadows and depth
//! sharing, ordering decides draw order, and neither implies the other.
//!
//! The scene crosses processes for parity runs, agent `get_3d_scene`, and
//! pm-cli; inside an engine it is passed by reference.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::color::{BlendMode, ToneMapping3d};
use crate::common::{AssetId, CompositionId, DefinitionId, EffectInstanceId, FrameRate, LayerId, Mat3, Mat4, Revision, Rgba, Size, Ticks};
use crate::error::FailureReport;
use crate::frame::{Projection, ViewOverride};
use crate::validate::{ensure, ensure_unique, Validate, Violation};
use crate::version::ContractId;

/// `compat` level (section 6): 1 keeps today's behavior, 2 gets fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(try_from = "u8", into = "u8")]
#[cfg_attr(feature = "ts", ts(type = "1 | 2"))]
pub enum Compat {
    Level1,
    Level2,
}

impl TryFrom<u8> for Compat {
    type Error = String;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Compat::Level1),
            2 => Ok(Compat::Level2),
            other => Err(format!("compat level must be 1 or 2, got {other}")),
        }
    }
}

impl From<Compat> for u8 {
    fn from(value: Compat) -> u8 {
        match value {
            Compat::Level1 => 1,
            Compat::Level2 => 2,
        }
    }
}

/// An evaluated parameter value (effects, transitions, extension layers, shaders).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind", content = "value")]
pub enum ParamValue {
    Number(f64),
    /// Toggles bind as `float` for effects and `bool` for extension layers (section 11).
    Toggle(bool),
    /// RGB in the working encoding.
    Color([f64; 3]),
    /// `vec2` to `vec4` uniforms. Shader-layer numeric controls send one float,
    /// so these stay 0 there, as today (section 11).
    Vector(Vec<f64>),
}

/// Composition background (appendix B).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum Background {
    None,
    Solid { color: Rgba },
    /// Up to 8 stops; `angle_degrees` for linear, radial uses angle only.
    Linear { stops: Vec<GradientStop>, angle_degrees: f64 },
    Radial { stops: Vec<GradientStop>, angle_degrees: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GradientStop {
    pub position: f64,
    pub color: Rgba,
}

/// The composition shutter (section 5 rule 8): motion-blur sample `i` of
/// `n` is evaluated at t + (phase + (i + ½)/n × angle) / 360 frames. The
/// render graph evaluates one scene per sample; this scene is the center
/// time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Shutter {
    pub angle_degrees: f64,
    pub phase_degrees: f64,
}

impl Shutter {
    /// Today's `composition.shutter` (0 to 2 frames, centered): 0.5 frame is
    /// 180° at −90°.
    pub fn from_frames(frames: f64) -> Self {
        let angle_degrees = frames * 360.0;
        Self { angle_degrees, phase_degrees: -angle_degrees / 2.0 }
    }

    /// Sample offsets in frames for `n` samples.
    pub fn sample_offsets(&self, n: u32) -> Vec<f64> {
        (0..n).map(|i| (self.phase_degrees + (i as f64 + 0.5) / n as f64 * self.angle_degrees) / 360.0).collect()
    }
}

/// Most background gradient stops.
pub const MAX_GRADIENT_STOPS: usize = 8;

/// Layer kinds (section 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LayerKind {
    Solid,
    Shape,
    Text,
    Captions,
    Image,
    Svg,
    Video,
    Shader,
    ExtensionFragment,
    ExtensionMesh,
    Object3d,
    Light3d,
    Camera3d,
    LegacyScene3d,
    Precomp,
    Adjustment,
    Group,
    ModelGroup,
    Null,
    Audio,
}

impl LayerKind {
    pub const fn is_visual(self) -> bool {
        !matches!(self, LayerKind::Null | LayerKind::Audio)
    }

    /// Layers that take part in 3D worlds (depth sets and the scene pass).
    pub const fn is_3d(self) -> bool {
        matches!(self, LayerKind::Object3d | LayerKind::Light3d | LayerKind::Camera3d)
    }
}

/// The layer's transform space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum LayerTransform {
    /// Layer-to-composition, including parenting and group transforms.
    Flat { matrix: Mat3 },
    /// 2.5D: layer-to-composition through the 3D group's perspective
    /// (homography planes, section 8 "2.5D").
    Perspective { matrix: Mat4, group: Option<LayerId> },
    /// A 3D world member; the matrix is layer-to-world.
    World { matrix: Mat4 },
}

/// Where a layer's pixels come from. Geometry-heavy payloads (paths, text
/// runs, meshes) are referenced by id and fetched from pm-eval caches inside
/// the engine; the scene carries what the graph keys on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum LayerSource {
    Solid { color: Rgba, size: Size, radius: f64 },
    /// Shapes, text, and captions: vector content evaluated by pm-eval and
    /// drawn by pm-vector or pm-text; `content_key` hashes the evaluated geometry.
    Vector { content_key: String, bounds: crate::common::Rect },
    /// A media frame from an asset at a source time (after retiming).
    Media { asset: AssetId, source_time: Ticks, always_original: bool },
    /// A user shader or extension fragment layer.
    Fragment { definition: Option<DefinitionId>, params: Vec<NamedParam>, size: Size },
    /// A legacy OBJ or third-party mesh definition.
    Mesh { definition: DefinitionId, asset: Option<AssetId> },
    /// A precomp sampled at a child time (keep frame rate quantizes it).
    Precomp { composition: CompositionId, time: Ticks },
    /// The output of a 3D world for this layer (scene pass or Blender surface).
    World { world: u32 },
    /// Effects run on the accumulator below.
    Adjustment,
    /// A group: a transform space, or a compositing boundary when styled.
    Group { isolated: bool },
    /// Layers with no pixels (null, audio, lights and cameras).
    None,
}

/// A named parameter value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NamedParam {
    pub key: String,
    pub value: ParamValue,
}

/// An evaluated effect instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EvaluatedEffect {
    pub instance: EffectInstanceId,
    pub definition: DefinitionId,
    /// Animatable `<fx>.$enabled` at this time.
    pub enabled: bool,
    pub compat: Compat,
    pub params: Vec<NamedParam>,
}

/// Mask modes for parametric masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    Add,
    Subtract,
}

/// An evaluated mask.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum EvaluatedMask {
    Rect { center: [f64; 2], size: [f64; 2], rotation_degrees: f64, feather: f64, mode: MaskMode, enabled: bool },
    Ellipse { center: [f64; 2], size: [f64; 2], rotation_degrees: f64, feather: f64, mode: MaskMode, enabled: bool },
    /// Bezier path masks; geometry keyed like vector content.
    Path { content_key: String, feather: f64, mode: MaskMode, enabled: bool },
}

/// Track matte modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MatteMode {
    Alpha,
    AlphaInverted,
    /// Rec.709 coefficients on E-encoded values.
    Luma,
    LumaInverted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MatteRef {
    pub source: LayerId,
    pub mode: MatteMode,
}

/// A transition active on the layer at this time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EvaluatedTransition {
    pub definition: DefinitionId,
    /// `u_prog` in 0..1.
    pub progress: f64,
    pub params: Vec<NamedParam>,
}

/// One layer at one time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EvaluatedLayer {
    pub id: LayerId,
    pub kind: LayerKind,
    pub compat: Compat,
    /// Active at this time, not hidden, and passing solo.
    pub visible: bool,
    pub opacity: f64,
    pub blend: BlendMode,
    pub transform: LayerTransform,
    /// The group this layer is inside, if any.
    pub group: Option<LayerId>,
    pub source: LayerSource,
    pub effects: Vec<EvaluatedEffect>,
    pub masks: Vec<EvaluatedMask>,
    pub matte: Option<MatteRef>,
    pub transition: Option<EvaluatedTransition>,
    pub motion_blur: bool,
}

/// Most parametric GPU masks per layer (more are a level 2 fix, appendix A 14).
pub const MAX_PARAMETRIC_MASKS: usize = 8;

/// A light in a 3D world.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum LightKind {
    /// Hemisphere ambient; ground color is 0.35× the sky color.
    Ambient,
    Sun,
    Point { distance: f64, decay: f64 },
    Spot { distance: f64, angle_degrees: f64, penumbra: f64, decay: f64 },
    /// Rectangular area light (LTC tables, no native shadows).
    Area { width: f64, height: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Light3d {
    /// The light layer, or `None` for the studio default sun and glTF-embedded lights.
    pub layer: Option<LayerId>,
    pub kind: LightKind,
    pub color: [f64; 3],
    pub intensity: f64,
    /// Light-to-world (rotation and scale of the layer are ignored for lights).
    pub world: Mat4,
    pub casts_shadow: bool,
    /// Embedded glTF punctual lights light the scene but count toward neither limit.
    pub embedded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Camera3d {
    /// The topmost active camera layer, or `None` for the studio default
    /// (camera at (3, 2, 6), 45° field of view).
    pub layer: Option<LayerId>,
    /// World-to-camera; roll comes from the layer rotation.
    pub view: Mat4,
    pub projection: Projection,
}

/// What a 3D object draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum ModelSource {
    /// A primitive, lathe, extrude, or recipe mesh from pm-geom3d; the key
    /// hashes the generator inputs.
    Generated { geometry_key: String },
    /// An imported glTF or GLB asset, with its animation time
    /// `max(0, t − layer.from)` (section 5 rule 10).
    Imported { asset: AssetId, animation_time: Ticks },
    /// A missing model: an orange wireframe box in output, as today.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Object3d {
    pub layer: LayerId,
    pub model: ModelSource,
    /// Object-to-world.
    pub world: Mat4,
    pub opacity: f64,
    /// Writes depth only with no transparent material, opacity ≥ 0.999, and no enabled mask.
    pub writes_depth: bool,
    /// Camera depth of the object origin, for ordering.
    pub origin_depth: f64,
}

/// One 3D world: the models, lights, and camera of a composition's 3D layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct World3d {
    pub camera: Camera3d,
    pub lights: Vec<Light3d>,
    pub objects: Vec<Object3d>,
    pub tone_mapping: ToneMapping3d,
    /// Blender renders this world's layers (`render3d.enabled`).
    pub blender: bool,
}

/// Limits that fail a 3D render when exceeded (section 3; may be raised, never lowered).
pub const MAX_ACTIVE_MODELS: usize = 512;
pub const MAX_ACTIVE_LIGHTS: usize = 16;
pub const MAX_SHADOW_LIGHTS: usize = 4;

/// The layers sharing a depth prepass and shadow maps with one another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DepthSet {
    pub world: u32,
    /// Object layers in the set.
    pub layers: Vec<LayerId>,
    /// Models in the set that cast shadows into it (only same-set models do).
    pub shadow_casters: Vec<LayerId>,
    /// The bottom layer of the set draws the solid background, opaque and not tone-mapped.
    pub background_layer: LayerId,
}

/// One run of consecutive 3D layers in stack order, re-sorted for drawing.
/// Blocks are never reordered across runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OrderedRun {
    /// Index of the run's first layer in [`EvaluatedScene::layers`].
    pub start: u32,
    /// The run's layers back to front: models by camera depth of their
    /// origin, lights and cameras at depth 0.
    pub order: Vec<LayerId>,
}

/// Everything the render graph needs for one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EvaluatedScene {
    pub composition: CompositionId,
    pub revision: Revision,
    pub time: Ticks,
    pub frame_rate: FrameRate,
    pub size: Size,
    pub background: Background,
    pub shutter: Shutter,
    /// The view override the scene was evaluated with (editor camera and Local View change worlds).
    pub view: Option<ViewOverride>,
    /// Layers bottom to top in stack order.
    pub layers: Vec<EvaluatedLayer>,
    pub worlds: Vec<World3d>,
    pub depth_sets: Vec<DepthSet>,
    pub ordering: Vec<OrderedRun>,
    /// Per-layer evaluation failures (expression errors, missing definitions).
    pub failures: Vec<FailureReport>,
}

impl Validate for EvaluatedScene {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::EvaluatedScene;
        ensure(self.composition.is_valid(), C, || "invalid composition id".into())?;
        ensure(self.frame_rate.is_valid(), C, || "frame rate must be positive".into())?;
        ensure(!self.size.is_empty(), C, || "composition size must be positive".into())?;
        ensure((0.0..=720.0).contains(&self.shutter.angle_degrees) && self.shutter.phase_degrees.is_finite(), C, || {
            "shutter angle is 0 to 720 degrees (0 to 2 frames)".into()
        })?;
        if let Background::Linear { stops, .. } | Background::Radial { stops, .. } = &self.background {
            ensure((2..=MAX_GRADIENT_STOPS).contains(&stops.len()), C, || {
                format!("gradients have 2 to {MAX_GRADIENT_STOPS} stops, got {}", stops.len())
            })?;
            ensure(stops.iter().all(|s| (0.0..=1.0).contains(&s.position)) && stops.windows(2).all(|w| w[0].position <= w[1].position), C, || {
                "gradient stop positions are in 0..1 and ascending".into()
            })?;
        }
        ensure_unique(self.layers.iter().map(|l| &l.id), C, "layer id")?;
        let index_of = |id: &LayerId| self.layers.iter().position(|l| &l.id == id);
        for layer in &self.layers {
            ensure((0.0..=1.0).contains(&layer.opacity), C, || format!("layer {:?} opacity {} outside 0..1", layer.id, layer.opacity))?;
            ensure(layer.masks.iter().filter(|m| !matches!(m, EvaluatedMask::Path { .. })).count() <= MAX_PARAMETRIC_MASKS || layer.compat == Compat::Level2, C, || {
                format!("layer {:?} has more than {MAX_PARAMETRIC_MASKS} parametric masks at compat level 1", layer.id)
            })?;
            if let Some(matte) = &layer.matte {
                ensure(matte.source != layer.id && index_of(&matte.source).is_some(), C, || {
                    format!("layer {:?} matte source {:?} is itself or missing", layer.id, matte.source)
                })?;
            }
            if let Some(transition) = &layer.transition {
                ensure((0.0..=1.0).contains(&transition.progress), C, || "transition progress outside 0..1".into())?;
            }
            if let LayerSource::World { world } = layer.source {
                ensure((world as usize) < self.worlds.len(), C, || format!("layer {:?} names world {world}", layer.id))?;
            }
            ensure(layer.kind.is_3d() == matches!(layer.transform, LayerTransform::World { .. }), C, || {
                format!("layer {:?}: 3D kinds and only 3D kinds use world transforms", layer.id)
            })?;
        }
        for world in &self.worlds {
            let lights = world.lights.iter().filter(|l| !l.embedded).count();
            ensure(world.objects.len() <= MAX_ACTIVE_MODELS, C, || format!("{} models exceed {MAX_ACTIVE_MODELS}", world.objects.len()))?;
            ensure(lights <= MAX_ACTIVE_LIGHTS, C, || format!("{lights} lights exceed {MAX_ACTIVE_LIGHTS}"))?;
            let shadows = world.lights.iter().filter(|l| l.casts_shadow && !l.embedded).count();
            ensure(shadows <= MAX_SHADOW_LIGHTS, C, || format!("{shadows} shadow-casting lights exceed {MAX_SHADOW_LIGHTS}"))?;
        }
        // Depth sets: object layers only, each object layer in exactly one set.
        let mut in_sets = std::collections::BTreeSet::new();
        for set in &self.depth_sets {
            ensure((set.world as usize) < self.worlds.len(), C, || format!("depth set names world {}", set.world))?;
            ensure(!set.layers.is_empty(), C, || "empty depth set".into())?;
            for id in &set.layers {
                let layer = index_of(id).map(|i| &self.layers[i]);
                ensure(layer.is_some_and(|l| l.kind == LayerKind::Object3d), C, || format!("depth set member {id:?} is not a 3D object layer"))?;
                ensure(in_sets.insert(id), C, || format!("layer {id:?} is in two depth sets"))?;
            }
            ensure(set.shadow_casters.iter().all(|c| set.layers.contains(c)), C, || "shadow casters must be members of their set".into())?;
            ensure(set.layers.contains(&set.background_layer), C, || "the background layer must be a member of its set".into())?;
            let bottom = set.layers.iter().filter_map(index_of).min();
            ensure(bottom == index_of(&set.background_layer), C, || "the background layer is the bottom layer of the set".into())?;
        }
        // Ordering: each run is a permutation of a contiguous block of 3D layers.
        let mut covered = std::collections::BTreeSet::new();
        for run in &self.ordering {
            let start = run.start as usize;
            let end = start + run.order.len();
            ensure(!run.order.is_empty() && end <= self.layers.len(), C, || format!("run at {start} is empty or past the end"))?;
            let block: std::collections::BTreeSet<&LayerId> = self.layers[start..end].iter().map(|l| &l.id).collect();
            let order: std::collections::BTreeSet<&LayerId> = run.order.iter().collect();
            ensure(block == order && order.len() == run.order.len(), C, || format!("run at {start} is not a permutation of its block"))?;
            ensure(self.layers[start..end].iter().all(|l| l.kind.is_3d()), C, || format!("run at {start} contains a non-3D layer"))?;
            for index in start..end {
                ensure(covered.insert(index), C, || format!("layer index {index} is in two runs"))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::common::{MAT3_IDENTITY, MAT4_IDENTITY};

    fn layer(id: &str, kind: LayerKind) -> EvaluatedLayer {
        EvaluatedLayer {
            id: id.into(),
            kind,
            compat: Compat::Level1,
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            transform: if kind.is_3d() { LayerTransform::World { matrix: MAT4_IDENTITY } } else { LayerTransform::Flat { matrix: MAT3_IDENTITY } },
            group: None,
            source: if kind == LayerKind::Object3d { LayerSource::World { world: 0 } } else { LayerSource::None },
            effects: vec![],
            masks: vec![],
            matte: None,
            transition: None,
            motion_blur: false,
        }
    }

    pub(crate) fn sample_scene() -> EvaluatedScene {
        let object = |id: &str, depth: f64| Object3d {
            layer: id.into(),
            model: ModelSource::Generated { geometry_key: format!("cube:{id}") },
            world: MAT4_IDENTITY,
            opacity: 1.0,
            writes_depth: true,
            origin_depth: depth,
        };
        EvaluatedScene {
            composition: "main".into(),
            revision: Revision::new(12).unwrap(),
            time: Ticks::from_whole_seconds(1),
            frame_rate: FrameRate::new(30000, 1001),
            size: Size::new(1920, 1080),
            background: Background::Solid { color: Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 } },
            shutter: Shutter::from_frames(0.5),
            view: None,
            layers: vec![
                layer("bg", LayerKind::Solid),
                layer("cube", LayerKind::Object3d),
                layer("sun", LayerKind::Light3d),
                layer("ball", LayerKind::Object3d),
                layer("title", LayerKind::Text),
            ],
            worlds: vec![World3d {
                camera: Camera3d {
                    layer: None,
                    view: MAT4_IDENTITY,
                    projection: Projection::Perspective { fov_y_degrees: 45.0, near: 0.1, far: 1000.0 },
                },
                lights: vec![Light3d {
                    layer: Some("sun".into()),
                    kind: LightKind::Sun,
                    color: [1.0, 1.0, 1.0],
                    intensity: 3.0,
                    world: MAT4_IDENTITY,
                    casts_shadow: true,
                    embedded: false,
                }],
                objects: vec![object("cube", 5.0), object("ball", 2.0)],
                tone_mapping: ToneMapping3d::AcesFilmicR180 { exposure: 1.0 },
                blender: false,
            }],
            depth_sets: vec![DepthSet {
                world: 0,
                layers: vec!["cube".into(), "ball".into()],
                shadow_casters: vec!["cube".into(), "ball".into()],
                background_layer: "cube".into(),
            }],
            ordering: vec![OrderedRun { start: 1, order: vec!["cube".into(), "sun".into(), "ball".into()] }],
            failures: vec![],
        }
    }

    #[test]
    fn sample_scene_is_valid() {
        sample_scene().validate().unwrap();
    }

    #[test]
    fn depth_sets_and_ordering_are_checked_separately() {
        let mut scene = sample_scene();
        scene.depth_sets[0].layers.push("sun".into());
        assert!(scene.validate().is_err(), "lights are not depth set members");

        let mut scene = sample_scene();
        scene.depth_sets.push(DepthSet { world: 0, layers: vec!["ball".into()], shadow_casters: vec![], background_layer: "ball".into() });
        assert!(scene.validate().is_err(), "a layer in two sets");

        let mut scene = sample_scene();
        scene.depth_sets[0].background_layer = "ball".into();
        assert!(scene.validate().is_err(), "background must be the bottom layer");

        let mut scene = sample_scene();
        scene.ordering[0].order = vec!["cube".into(), "ball".into(), "title".into()];
        assert!(scene.validate().is_err(), "a run must permute its own block");

        let mut scene = sample_scene();
        scene.ordering[0] = OrderedRun { start: 0, order: vec!["bg".into(), "cube".into()] };
        assert!(scene.validate().is_err(), "runs contain only 3D layers");
    }

    #[test]
    fn limits_and_references() {
        let mut scene = sample_scene();
        scene.worlds[0].lights = (0..5)
            .map(|_| Light3d { layer: None, kind: LightKind::Sun, color: [1.0; 3], intensity: 1.0, world: MAT4_IDENTITY, casts_shadow: true, embedded: false })
            .collect();
        assert!(scene.validate().is_err(), "more than 4 shadow lights");
        scene.worlds[0].lights.iter_mut().for_each(|l| l.embedded = true);
        assert!(scene.validate().is_ok(), "embedded lights count toward neither limit");

        let mut scene = sample_scene();
        scene.layers[4].matte = Some(MatteRef { source: "nope".into(), mode: MatteMode::Luma });
        assert!(scene.validate().is_err());

        let mut scene = sample_scene();
        scene.layers.push(layer("bg", LayerKind::Null));
        assert!(scene.validate().is_err(), "duplicate ids");
    }

    #[test]
    fn shutter_matches_todays_centered_samples() {
        // compositor.ts: dt = ((s + .5) / n − .5) × shutter, shutter in frames.
        let shutter = Shutter::from_frames(0.5);
        assert_eq!((shutter.angle_degrees, shutter.phase_degrees), (180.0, -90.0));
        for n in [1u32, 6, 10, 16, 20] {
            for (s, offset) in shutter.sample_offsets(n).iter().enumerate() {
                let today = ((s as f64 + 0.5) / n as f64 - 0.5) * 0.5;
                assert!((offset - today).abs() < 1e-15, "n={n} s={s}");
            }
        }
    }

    #[test]
    fn compat_levels_encode_as_numbers() {
        let bytes = rmp_serde::to_vec(&Compat::Level2).unwrap();
        assert_eq!(bytes, vec![2]);
        assert!(rmp_serde::from_slice::<Compat>(&rmp_serde::to_vec(&3u8).unwrap()).is_err());
    }
}
