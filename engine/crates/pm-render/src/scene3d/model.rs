//! The 3D scene a composition (or a legacy `scene3d` layer) renders at one
//! time and view, as `SceneRuntime.sync` and `compositionRuntime` build the
//! three.js scene graph (`core/scene3d/runtime.ts`, `service.ts`):
//! objects with their meshes, materials (`materialPreview` applied, slots,
//! maps with placement), lights with three.js's shadow settings, the
//! camera, the environment, and the editor look.
//!
//! [`SceneBuilder`] reads the scene description (`Scene3D`: `objects`,
//! `lights`, `camera`, `environment`) with a channel evaluator, so the same
//! code serves `compositionScene` output (channels already evaluated),
//! legacy scene layers (channels evaluated per `o.<id>.` path), and tests
//! (scenes the three.js reference renders).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pm_geom3d::{Geometry, Group, MeshSource, Parameters, Source};
use pm_model::json::{Map, Value};

use crate::js;
use crate::three::materials::Side;
use crate::three::math::{self, Mat4, Vec3};

/// `THREE.MathUtils.degToRad`.
pub fn deg_to_rad(d: f64) -> f64 {
    d * (std::f64::consts::PI / 180.0)
}

/// A camera projection (`PerspectiveCamera` or `OrthographicCamera` after
/// `resizeCamera`: half-height 3, half-width 3 × aspect, divided by zoom).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    Perspective {
        fov: f64,
        zoom: f64,
        near: f64,
        far: f64,
    },
    Orthographic {
        zoom: f64,
        near: f64,
        far: f64,
    },
}

/// The render camera (or the editor camera of a view override).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera3d {
    /// `matrixWorld`.
    pub world: Mat4,
    pub projection: Projection,
}

impl Camera3d {
    /// `projectionMatrix` for a canvas of `width` × `height` pixels.
    pub fn projection_matrix(&self, width: u32, height: u32) -> Mat4 {
        let aspect = f64::from(width) / js::max(1.0, f64::from(height));
        match self.projection {
            Projection::Perspective {
                fov,
                zoom,
                near,
                far,
            } => math::perspective(fov, aspect, zoom, near, far),
            Projection::Orthographic { zoom, near, far } => {
                math::orthographic(aspect, zoom, near, far)
            }
        }
    }

    pub fn orthographic(&self) -> bool {
        matches!(self.projection, Projection::Orthographic { .. })
    }

    /// `matrixWorldInverse`.
    pub fn view(&self) -> Mat4 {
        math::invert(&self.world)
    }
}

/// A texture a material samples (`new THREE.Texture(bitmap)` with
/// `RepeatWrapping`, mipmapped, trilinear), with its placement transform.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture3d {
    pub asset: String,
    pub path: PathBuf,
    /// Chromium SVG map upload (distinct from a 2D SVG bucket).
    pub svg: Option<pm_protocol::svg_raster::SvgRasterRequest>,
    /// Color and emissive maps decode sRGB (`SRGBColorSpace`); data maps
    /// are linear.
    pub srgb: bool,
    /// `texture.matrix` (`setUvTransform` of offset, repeat, rotation),
    /// column-major 3 × 3.
    pub transform: [f64; 9],
}

/// The six `MeshStandardMaterial` maps the scene schema has.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialMaps {
    pub color: Option<Texture3d>,
    pub normal: Option<Texture3d>,
    pub roughness: Option<Texture3d>,
    pub metalness: Option<Texture3d>,
    pub emissive: Option<Texture3d>,
    pub ao: Option<Texture3d>,
}

impl MaterialMaps {
    pub fn flags(&self) -> crate::three::materials::Maps {
        crate::three::materials::Maps {
            color: self.color.is_some(),
            normal: self.normal.is_some(),
            roughness: self.roughness.is_some(),
            metalness: self.metalness.is_some(),
            emissive: self.emissive.is_some(),
            ao: self.ao.is_some(),
        }
    }

    /// Every map slot, in [`MaterialMaps::all`]'s order.
    pub fn all_mut(&mut self) -> [&mut Option<Texture3d>; 6] {
        [
            &mut self.color,
            &mut self.normal,
            &mut self.roughness,
            &mut self.metalness,
            &mut self.emissive,
            &mut self.ao,
        ]
    }

    pub fn all(&self) -> [(&'static str, Option<&Texture3d>); 6] {
        [
            ("map", self.color.as_ref()),
            ("normalMap", self.normal.as_ref()),
            ("roughnessMap", self.roughness.as_ref()),
            ("metalnessMap", self.metalness.as_ref()),
            ("emissiveMap", self.emissive.as_ref()),
            ("aoMap", self.ao.as_ref()),
        ]
    }
}

/// Which three.js material class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialKind {
    Standard,
    /// `MeshBasicMaterial` (the missing-model box, unlit glTF materials).
    Basic,
    /// `MeshPhysicalMaterial` (glTF materials with physical extensions).
    Physical,
    /// `PointsMaterial` (glTF points).
    Points,
    /// `LineBasicMaterial` (glTF lines).
    Line,
}

/// A material as three.js holds it when the pass draws.
#[derive(Debug, Clone, PartialEq)]
pub struct Material3d {
    pub kind: MaterialKind,
    /// `material.name` (`SHADER_NAME`).
    pub name: String,
    /// Linear (`Color.set`).
    pub color: Vec3,
    pub roughness: f64,
    pub metalness: f64,
    pub emissive: Vec3,
    pub emissive_intensity: f64,
    pub opacity: f64,
    pub transparent: bool,
    pub depth_write: bool,
    pub side: Side,
    pub wireframe: bool,
    pub maps: MaterialMaps,
    /// A glTF material's state beyond these fields (its class parameters,
    /// file maps, vertex colors, alpha test); `None` for scene materials.
    pub ext: Option<Arc<super::gltf::MaterialExt>>,
}

impl Material3d {
    /// `new THREE.MeshStandardMaterial()` (a source material kept as created).
    pub fn standard_default() -> Material3d {
        Material3d {
            kind: MaterialKind::Standard,
            name: String::new(),
            color: [1.0, 1.0, 1.0],
            roughness: 1.0,
            metalness: 0.0,
            emissive: [0.0; 3],
            emissive_intensity: 1.0,
            opacity: 1.0,
            transparent: false,
            depth_write: true,
            side: Side::Front,
            wireframe: false,
            maps: MaterialMaps::default(),
            ext: None,
        }
    }

    /// `new THREE.MeshBasicMaterial({ color: 0xff6600, wireframe: true })`.
    pub fn missing_box() -> Material3d {
        Material3d {
            kind: MaterialKind::Basic,
            name: String::new(),
            color: [
                math::srgb_to_linear(1.0),
                math::srgb_to_linear(f64::from(0x66u8) / 255.0),
                math::srgb_to_linear(0.0),
            ],
            roughness: 1.0,
            metalness: 0.0,
            emissive: [0.0; 3],
            emissive_intensity: 1.0,
            opacity: 1.0,
            transparent: false,
            depth_write: true,
            side: Side::Front,
            wireframe: true,
            maps: MaterialMaps::default(),
            ext: None,
        }
    }

    /// `material.visible` is never turned off by the runtime; a material
    /// draws unless the pass hides it.
    pub fn is_transparent(&self) -> bool {
        self.transparent
    }
}

/// The materials of a mesh: one (groups ignored) or an array indexed by the
/// geometry's groups (`slots`).
#[derive(Debug, Clone, PartialEq)]
pub enum Materials {
    Single(Box<Material3d>),
    Slots(Vec<Material3d>),
}

impl Materials {
    /// The materials in slot order.
    pub fn iter(&self) -> std::slice::Iter<'_, Material3d> {
        match self {
            Materials::Single(m) => std::slice::from_ref(m.as_ref()).iter(),
            Materials::Slots(list) => list.iter(),
        }
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Material3d> {
        match self {
            Materials::Single(m) => std::slice::from_mut(m.as_mut()).iter_mut(),
            Materials::Slots(list) => list.iter_mut(),
        }
    }
}

/// Geometry shared by meshes with the same source (`signature`).
#[derive(Clone)]
pub struct Geometry3d {
    /// Hash of the source (the GPU mesh cache key).
    pub key: u64,
    pub geometry: Arc<Geometry>,
    /// `geometry.computeBoundingSphere()`: center and radius.
    pub sphere: ([f64; 3], f64),
    /// A glTF geometry's other attributes (tangents, colors, secondary UV
    /// sets, skin attributes, morph targets, primitive mode).
    pub extra: Option<Arc<super::gltf::VertexExtra>>,
}

/// `BufferGeometry.computeBoundingSphere()`: the bounding box center and
/// the largest distance from it.
pub fn bounding_sphere(g: &Geometry) -> ([f64; 3], f64) {
    let Some((lo, hi)) = g.bounds() else {
        return ([0.0; 3], 0.0);
    };
    let c = [
        (f64::from(lo[0]) + f64::from(hi[0])) * 0.5,
        (f64::from(lo[1]) + f64::from(hi[1])) * 0.5,
        (f64::from(lo[2]) + f64::from(hi[2])) * 0.5,
    ];
    let mut max = 0f64;
    for p in g.positions.chunks_exact(3) {
        let d = [
            c[0] - f64::from(p[0]),
            c[1] - f64::from(p[1]),
            c[2] - f64::from(p[2]),
        ];
        max = max.max(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
    }
    (c, max.sqrt())
}

/// The key names the geometry (a source hash or an OBJ file version), so
/// debug output and content keys carry it instead of every vertex.
impl std::fmt::Debug for Geometry3d {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Geometry3d")
            .field("key", &format_args!("{:016x}", self.key))
            .field("vertices", &self.geometry.vertex_count())
            .field("sphere", &self.sphere)
            .finish()
    }
}

impl PartialEq for Geometry3d {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

/// A drawable (`THREE.Mesh`).
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh3d {
    pub geometry: Geometry3d,
    /// `matrixWorld`.
    pub world: Mat4,
    pub materials: Materials,
    pub cast_shadow: bool,
    pub receive_shadow: bool,
    /// A glTF drawable's state (mode, skinning, instances, morph
    /// influences, culling sphere, sort id); `None` for scene meshes.
    pub gltf: Option<Arc<super::gltf::MeshExt>>,
}

/// A scene object (one layer, or one object of a legacy scene).
#[derive(Debug, Clone, PartialEq)]
pub struct Object3d {
    pub id: String,
    /// `object.visible` and every ancestor's (legacy hierarchies).
    pub visible: bool,
    /// Its object origin in world space (`getWorldPosition`).
    pub origin: Vec3,
    pub meshes: Vec<Mesh3d>,
    /// `userData.layerOpacity` (`PM.worldOpacity`).
    pub layer_opacity: f64,
    /// `userData.layerOccludes`: no enabled mask.
    pub occludes: bool,
    /// A missing model (the orange box).
    pub missing: bool,
}

/// Light classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
    RectArea,
}

/// A light as `sync` configures it.
#[derive(Debug, Clone, PartialEq)]
pub struct Light3d {
    pub id: String,
    pub kind: LightKind,
    /// `matrixWorld` (rotation only for rectangular area lights).
    pub world: Mat4,
    /// `target.matrixWorld` position (directional and spot lights).
    pub target: Vec3,
    /// Linear color.
    pub color: Vec3,
    pub intensity: f64,
    pub distance: f64,
    pub decay: f64,
    /// Radians.
    pub angle: f64,
    pub penumbra: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
    /// `node.castShadow && environment.shadows && shadowLights++ < 4`.
    pub cast_shadow: bool,
}

impl Light3d {
    pub fn position(&self) -> Vec3 {
        [self.world[12], self.world[13], self.world[14]]
    }
}

/// Editor looks (view override only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Look {
    pub wireframe: bool,
    pub xray: bool,
}

/// The evaluated scene of one composition pass (shared by its 3D layers)
/// or of one legacy scene layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene3d {
    /// Canvas size (`renderPass` clamps it to 1..8192).
    pub width: u32,
    pub height: u32,
    pub camera: Camera3d,
    /// `renderer.toneMappingExposure`.
    pub exposure: f64,
    /// Hemisphere light: linear sky color, ground color, intensity.
    pub hemisphere: (Vec3, Vec3, f64),
    /// Lights in scene order.
    pub lights: Vec<Light3d>,
    /// `renderer.shadowMap.enabled` (`environment.shadows !== false`).
    pub shadows: bool,
    /// `scene.background` (linear), drawn by the set's bottom layer.
    pub background: Option<Vec3>,
    pub objects: Vec<Object3d>,
    pub look: Look,
    /// `runtime.warnings` (missing textures and models).
    pub warnings: Vec<String>,
}

impl Scene3d {
    /// A scene with nothing in it (renders that do not use one).
    pub fn empty(width: u32, height: u32) -> Scene3d {
        Scene3d {
            width,
            height,
            camera: Camera3d {
                world: math::IDENTITY,
                projection: Projection::Perspective {
                    fov: 45.0,
                    zoom: 1.0,
                    near: 0.01,
                    far: 1000.0,
                },
            },
            exposure: 1.0,
            hemisphere: ([1.0; 3], [0.35; 3], 0.5),
            lights: Vec::new(),
            shadows: false,
            background: None,
            objects: Vec::new(),
            look: Look::default(),
            warnings: Vec::new(),
        }
    }

    /// The canvas size `renderPass` uses.
    pub fn canvas(&self) -> (u32, u32) {
        (self.width.clamp(1, 8192), self.height.clamp(1, 8192))
    }

    /// Applies an editor look: Wireframe sets every material's wireframe
    /// flag; X-ray makes every material transparent at 0.45 × its opacity
    /// with no depth writes (`SceneRuntime.render`'s `look`).
    pub fn apply_look(&mut self, look: Look) {
        self.look = look;
        if !look.wireframe && !look.xray {
            return;
        }
        let edit = |m: &mut Material3d| {
            // `render(look)` edits mesh materials only (not points or lines).
            if matches!(m.kind, MaterialKind::Points | MaterialKind::Line) {
                return;
            }
            if look.wireframe {
                m.wireframe = true;
            }
            if look.xray {
                m.transparent = true;
                m.opacity *= 0.45;
                m.depth_write = false;
            }
        };
        for o in &mut self.objects {
            for mesh in &mut o.meshes {
                match &mut mesh.materials {
                    Materials::Single(m) => edit(m),
                    Materials::Slots(list) => list.iter_mut().for_each(edit),
                }
            }
        }
    }
}

/// Why a scene cannot be drawn by this pass (routed to the legacy engine).
pub type Unsupported = String;

/// Channel values (`values(p, prefix)` in `sync`).
pub trait Channels {
    /// The evaluated value of `channel` at `path` (`evaluate(v, path)`).
    fn evaluate(&mut self, channel: &Value, path: &str) -> Value;
}

/// `compositionScene` output: every channel's `v` is already evaluated.
pub struct Evaluated;

impl Channels for Evaluated {
    fn evaluate(&mut self, channel: &Value, _path: &str) -> Value {
        channel_v(channel).cloned().unwrap_or(Value::Null)
    }
}

fn channel_v(c: &Value) -> Option<&Value> {
    match c {
        Value::Object(m) => m.get("v"),
        _ => None,
    }
}

/// How a scene finds textures.
pub trait TextureSource {
    /// The image file of an image asset, or `None` when it is not an image
    /// asset (`asset.kind !== 'image'`) or offline.
    fn image(&self, asset: &str) -> Option<PathBuf>;
    fn svg(&self, _asset: &str) -> Option<pm_protocol::svg_raster::SvgRasterRequest> {
        None
    }
}

/// What an object's `assetId` resolves to (`makeObject`).
#[derive(Debug, Clone)]
pub enum ModelLookup {
    /// No usable asset (absent, offline, or without geometry): the orange
    /// wireframe box.
    Missing,
    /// An OBJ model asset (`asset.mesh`): its triangles with a
    /// `MeshStandardMaterial` the object's material then configures.
    Obj {
        key: u64,
        mesh: Arc<pm_geom3d::obj::ObjMesh>,
    },
    /// A glTF or GLB scene (`asset.object3d`).
    Gltf(Arc<super::gltf::GltfAsset>),
    /// A glTF or GLB file this engine does not load although the importer
    /// accepted it (routed to the legacy engine).
    Unloadable(String),
}

/// How a scene finds model assets.
pub trait ModelSource {
    fn model(&self, asset: &str) -> ModelLookup;
}

/// No model assets: every `assetId` is missing.
pub struct NoModels;

impl ModelSource for NoModels {
    fn model(&self, _asset: &str) -> ModelLookup {
        ModelLookup::Missing
    }
}

/// No textures (`() => null`).
pub struct NoTextures;

impl TextureSource for NoTextures {
    fn image(&self, _asset: &str) -> Option<PathBuf> {
        None
    }
}

pub(crate) fn obj<'a>(map: &'a Map, key: &str) -> Option<&'a Map> {
    match map.get(key) {
        Some(Value::Object(m)) => Some(m),
        _ => None,
    }
}

pub(crate) fn arr<'a>(map: &'a Map, key: &str) -> &'a [Value] {
    match map.get(key) {
        Some(Value::Array(a)) => a,
        _ => &[],
    }
}

pub(crate) fn text(map: &Map, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

pub(crate) fn boolean(map: &Map, key: &str) -> Option<bool> {
    match map.get(key) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    }
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    }
}

/// The evaluated channel values of `p` (`values(p, prefix)`): a number
/// channel keeps its stored value when the evaluation is not finite;
/// other channels when the evaluation changes type.
pub fn values(
    p: Option<&Map>,
    prefix: &str,
    channels: &mut dyn Channels,
) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let Some(p) = p else {
        return out;
    };
    for (k, v) in p.iter() {
        let key = k.to_string_lossy();
        let stored = channel_v(v).cloned().unwrap_or(Value::Null);
        let value = channels.evaluate(v, &format!("{prefix}.{key}"));
        let kept = match (&stored, &value) {
            (Value::Number(_), Value::Number(n)) if n.is_finite() => value,
            (Value::Number(_), _) => stored,
            (Value::String(_), Value::String(_))
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Null, Value::Null) => value,
            (Value::Object(_) | Value::Array(_), Value::Object(_) | Value::Array(_)) => value,
            _ => stored,
        };
        out.insert(key, kept);
    }
    out
}

fn num(v: &HashMap<String, Value>, key: &str) -> Option<f64> {
    number(v.get(key))
}

fn hex(v: &HashMap<String, Value>, key: &str) -> Result<Vec3, Unsupported> {
    match v.get(key) {
        Some(Value::String(s)) => math::color_hex(&s.to_string_lossy())
            .ok_or_else(|| format!("3D color {key} is not #RRGGBB")),
        _ => Err(format!("3D color {key} is missing")),
    }
}

/// `Matrix3.setUvTransform(tx, ty, sx, sy, rotation, 0, 0)`, column-major.
pub fn uv_transform(tx: f64, ty: f64, sx: f64, sy: f64, rotation: f64) -> [f64; 9] {
    let (cx, cy) = (0.0, 0.0);
    let c = libm::cos(rotation);
    let s = libm::sin(rotation);
    // `set(n11, n12, n13, n21, n22, n23, n31, n32, n33)` is row-major.
    let n11 = sx * c;
    let n12 = sx * s;
    let n13 = -sx * (c * cx + s * cy) + cx + tx;
    let n21 = -sy * s;
    let n22 = sy * c;
    let n23 = -sy * (-s * cx + c * cy) + cy + ty;
    [n11, n21, 0.0, n12, n22, 0.0, n13, n23, 1.0]
}

/// The identity `texture.matrix` (no placement).
pub const UV_IDENTITY: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

fn sha_key(parts: &[&[u8]]) -> u64 {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    u64::from_le_bytes(h.finalize()[..8].try_into().unwrap())
}

fn numbers(v: Option<&Value>) -> Option<Vec<f64>> {
    match v {
        Some(Value::Array(a)) => a.iter().map(|x| number(Some(x))).collect(),
        _ => None,
    }
}

fn points(v: Option<&Value>) -> Option<Vec<[f64; 2]>> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .map(|p| match p {
                Value::Array(xy) if xy.len() == 2 => {
                    Some([number(xy.first())?, number(xy.get(1))?])
                }
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// The geometry source of a scene object, or why it is not drawn here.
pub fn source_of(source: &Map) -> Result<Source, Unsupported> {
    if let Some(name) = text(source, "primitive") {
        let p = obj(source, "parameters");
        let get = |k: &str| p.and_then(|p| number(p.get(k)));
        return Source::primitive(
            &name,
            &Parameters {
                width: get("width"),
                height: get("height"),
                depth: get("depth"),
                radius: get("radius"),
                radius_top: get("radiusTop"),
                radius_bottom: get("radiusBottom"),
                tube: get("tube"),
                segments: get("segments"),
                detail: get("detail"),
            },
        )
        .ok_or_else(|| format!("{name} primitives"));
    }
    if source.get("lathe").is_some() {
        return Ok(Source::Lathe {
            points: points(source.get("lathe")).ok_or("a lathe profile")?,
            segments: number(source.get("segments")).unwrap_or(0.0),
        });
    }
    if source.get("extrude").is_some() {
        return Ok(Source::Extrude {
            points: points(source.get("extrude")).ok_or("an extrude shape")?,
            depth: number(source.get("depth")).ok_or("an extrude depth")?,
            bevel: number(source.get("bevel")).unwrap_or(0.0),
        });
    }
    if let Some(Value::Object(m)) = source.get("mesh") {
        let groups = arr(m, "groups")
            .iter()
            .map(|g| match g {
                Value::Object(g) => Some(Group {
                    start: number(g.get("start"))? as u32,
                    count: number(g.get("count"))? as u32,
                    material_index: number(g.get("materialIndex"))? as u32,
                }),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .ok_or("mesh groups")?;
        return Ok(Source::Mesh(MeshSource {
            positions: numbers(m.get("positions")).ok_or("mesh positions")?,
            indices: numbers(m.get("indices")).map(|v| v.iter().map(|i| *i as u32).collect()),
            normals: numbers(m.get("normals")),
            uvs: numbers(m.get("uvs")),
            groups,
        }));
    }
    Err("an unknown 3D source".into())
}

/// Builds [`Scene3d`]s, keeping built geometry across frames.
#[derive(Default)]
pub struct SceneBuilder {
    geometries: GeometryCache,
    /// Mixer time of imported models per object id (`time − layer.from`),
    /// and the time of the others.
    animation: HashMap<String, f64>,
    animation_default: f64,
}

/// A bounding sphere: center and radius.
pub type Sphere = ([f64; 3], f64);

/// Built geometry by source hash.
type GeometryCache = HashMap<u64, (Arc<Geometry>, Sphere)>;

/// How a scene's objects and lights are placed.
pub enum Placement<'a> {
    /// `SceneRuntime.sync` alone: object hierarchies from their own
    /// channels, lights and the camera from theirs (legacy scene layers).
    Channels,
    /// `compositionRuntime`: objects at given world matrices (flattened),
    /// lights at world positions aiming at world targets, the camera at a
    /// pose (`None` for the studio camera from its channels).
    Composition {
        objects: &'a HashMap<String, Mat4>,
        lights: &'a HashMap<String, (Vec3, Vec3)>,
        camera: Option<(Vec3, Vec3, Vec3)>,
    },
}

/// Per-object layer facts (`userData.layerOpacity`, `layerOccludes`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerFacts {
    pub opacity: f64,
    pub occludes: bool,
}

impl SceneBuilder {
    /// The mixer time of imported models: `per_object[id]`, else `default`
    /// (`mixer.setTime(max(0, animationTime(id) ?? time))`).
    pub fn set_animation_times(&mut self, default: f64, per_object: HashMap<String, f64>) {
        self.animation_default = default;
        self.animation = per_object;
    }

    fn geometry(&mut self, source: &Source) -> Geometry3d {
        let mut identity = Vec::new();
        source.write_key(&mut identity);
        let key = sha_key(&[b"source 1", &identity]);
        self.cached(key, || source.geometry())
    }

    /// An OBJ model's geometry (`key` names the file version).
    fn obj(&mut self, key: u64, mesh: &pm_geom3d::obj::ObjMesh) -> Geometry3d {
        self.cached(key, || obj_triangles(mesh))
    }

    /// The geometry under `key`, built once and kept across scenes and
    /// frames (bounded).
    fn cached(&mut self, key: u64, build: impl FnOnce() -> Geometry) -> Geometry3d {
        if let Some((g, sphere)) = self.geometries.get(&key) {
            return Geometry3d {
                key,
                geometry: g.clone(),
                sphere: *sphere,
                extra: None,
            };
        }
        static SHARED: std::sync::OnceLock<std::sync::Mutex<GeometryCache>> =
            std::sync::OnceLock::new();
        let shared = SHARED.get_or_init(Default::default);
        let found = shared.lock().unwrap().get(&key).cloned();
        let (g, sphere) = match found {
            Some(f) => f,
            None => {
                let g = build();
                let sphere = bounding_sphere(&g);
                let entry = (Arc::new(g), sphere);
                let mut map = shared.lock().unwrap();
                if map.len() >= 512 {
                    map.clear();
                }
                map.insert(key, entry.clone());
                entry
            }
        };
        self.geometries.insert(key, (g.clone(), sphere));
        Geometry3d {
            key,
            geometry: g,
            sphere,
            extra: None,
        }
    }

    /// Drops geometry no scene used since the last call.
    pub fn retain(&mut self, keep: &[u64]) {
        self.geometries.retain(|k, _| keep.contains(k));
    }

    /// `materialPreview(definition)` then the material's values, maps, and
    /// placement, as `sync` sets a `MeshStandardMaterial`.
    #[allow(clippy::too_many_arguments)]
    fn material(
        &mut self,
        definition: &Map,
        name: String,
        prefix: &str,
        channels: &mut dyn Channels,
        textures: &dyn TextureSource,
        warnings: &mut Vec<String>,
    ) -> Result<Material3d, Unsupported> {
        // `materialPreview`: shader inputs mapped onto the material channels.
        let mut p: Map = obj(definition, "p").cloned().unwrap_or_default();
        if let Some(shader) = obj(definition, "shader") {
            let inputs = arr(shader, "inputs");
            if let Some(sp) = obj(shader, "p") {
                for (key, value) in sp.iter() {
                    let key = key.to_string_lossy();
                    let socket = inputs.iter().find_map(|f| match f {
                        Value::Object(f) if text(f, "id").as_deref() == Some(key.as_str()) => {
                            text(f, "socket")
                        }
                        _ => None,
                    });
                    let s = socket.as_deref();
                    let target = if s == Some("Base Color") || key == "color" {
                        Some("color")
                    } else if s == Some("Roughness") || key == "roughness" {
                        Some("roughness")
                    } else if s == Some("Metallic") || key == "metalness" {
                        Some("metalness")
                    } else if s == Some("Emission Color") || key == "emission" {
                        Some("emissive")
                    } else if s == Some("Emission Strength") || key == "intensity" {
                        Some("emissiveIntensity")
                    } else if s == Some("Alpha") {
                        Some("opacity")
                    } else {
                        None
                    };
                    if let Some(t) = target {
                        p.insert(t, value.clone());
                    }
                }
            }
        }
        let m = values(Some(&p), prefix, channels);
        let opacity = num(&m, "opacity").unwrap_or(f64::NAN);
        let transparent = opacity < 1.0;
        let double = boolean(definition, "doubleSided") == Some(true);
        let placement = obj(definition, "placement");
        let transform = match placement {
            Some(pl) => {
                let g = |k: &str, d: f64| number(pl.get(k)).unwrap_or(d);
                uv_transform(
                    g("offsetX", 0.0),
                    g("offsetY", 0.0),
                    g("scaleX", 1.0),
                    g("scaleY", 1.0),
                    deg_to_rad(g("rotation", 0.0)),
                )
            }
            None => UV_IDENTITY,
        };
        let maps = obj(definition, "maps");
        let mut map = |key: &str, srgb: bool| -> Option<Texture3d> {
            let id = maps.and_then(|m| text(m, key))?;
            match textures.image(&id) {
                Some(path) => Some(Texture3d {
                    svg: textures.svg(&id),
                    asset: id,
                    path,
                    srgb,
                    transform,
                }),
                None => {
                    warnings.push(format!("Missing texture: {id}"));
                    None
                }
            }
        };
        let maps = MaterialMaps {
            color: map("color", true),
            normal: map("normal", false),
            roughness: map("roughness", false),
            metalness: map("metalness", false),
            emissive: map("emissive", true),
            ao: map("ao", false),
        };
        Ok(Material3d {
            kind: super::model::MaterialKind::Standard,
            name,
            color: hex(&m, "color")?,
            roughness: num(&m, "roughness").ok_or("roughness")?,
            metalness: num(&m, "metalness").ok_or("metalness")?,
            emissive: hex(&m, "emissive")?,
            emissive_intensity: num(&m, "emissiveIntensity").ok_or("emissiveIntensity")?,
            opacity,
            transparent,
            depth_write: !transparent,
            side: if double { Side::Double } else { Side::Front },
            wireframe: false,
            maps,
            ext: None,
        })
    }

    /// Builds the scene. `facts` holds each object's layer opacity and
    /// masks (absent: opacity 1, occluding). Model assets come from
    /// `models`: a missing one draws the orange box, an OBJ model its mesh,
    /// and an imported glTF scene is the glTF lane's (unsupported here).
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        &mut self,
        scene: &Map,
        width: u32,
        height: u32,
        placement: &Placement<'_>,
        facts: &HashMap<String, LayerFacts>,
        channels: &mut dyn Channels,
        textures: &dyn TextureSource,
        models: &dyn ModelSource,
    ) -> Result<Scene3d, Unsupported> {
        let mut warnings = Vec::new();
        let environment = obj(scene, "environment").ok_or("the 3D environment")?;
        let shadows = boolean(environment, "shadows") != Some(false);
        // Objects.
        let nodes = arr(scene, "objects");
        let mut locals: HashMap<String, (Mat4, bool, Option<String>)> = HashMap::new();
        let mut objects = Vec::new();
        // Imported models to place once world matrices are known: object
        // index, asset, materials, use-source flag.
        let mut imported: Vec<(usize, Arc<super::gltf::GltfAsset>, Materials, bool)> = Vec::new();
        let mut imported_vertices = 0usize;
        for node in nodes {
            let Value::Object(node) = node else {
                continue;
            };
            let id = text(node, "id").ok_or("a 3D object id")?;
            let v = values(obj(node, "p"), &format!("o.{id}"), channels);
            let visible = !matches!(v.get("visible"), Some(Value::Bool(false)));
            let g = |k: &str, d: f64| num(&v, k).unwrap_or(d);
            let q = euler_xyz(
                deg_to_rad(g("rx", 0.0)),
                deg_to_rad(g("ry", 0.0)),
                deg_to_rad(g("rz", 0.0)),
            );
            let local = math::compose(
                [g("x", 0.0), g("y", 0.0), g("z", 0.0)],
                q,
                [g("sx", 1.0), g("sy", 1.0), g("sz", 1.0)],
            );
            locals.insert(id.clone(), (local, visible, text(node, "parent")));
            let source = obj(node, "source").ok_or("a 3D object source")?;
            let cast = boolean(node, "castShadow") != Some(false);
            let receive = boolean(node, "receiveShadow") != Some(false);
            let mut missing = false;
            let mut gltf_asset = None;
            let materials = if let Some(asset) = text(source, "assetId") {
                match models.model(&asset) {
                    ModelLookup::Gltf(a) => {
                        imported_vertices += a.model.vertices;
                        gltf_asset = Some(a);
                        Some(self.geometry(&Source::Primitive(pm_geom3d::Primitive::Box {
                            width: 1.0,
                            height: 1.0,
                            depth: 1.0,
                        })))
                    }
                    ModelLookup::Unloadable(why) => return Err(why),
                    ModelLookup::Missing => {
                        warnings.push(format!("Missing model: {asset}"));
                        missing = true;
                        None
                    }
                    ModelLookup::Obj { key, mesh } => Some(self.obj(key, &mesh)),
                }
            } else if let Some(name) = text(source, "primitive")
                && !pm_geom3d::Primitive::known(&name)
                && matches!(placement, Placement::Composition { .. })
            {
                // `compositionRuntime` syncs its scene unvalidated:
                // `primitiveGeometry` returns nothing for a primitive it does
                // not know, and `new THREE.Mesh(undefined)` has an empty
                // `BufferGeometry`, so the object draws nothing, as today. (A
                // legacy scene layer's `parseScene` refuses it and the layer
                // fails, here as today.)
                Some(
                    self.cached(sha_key(&[b"empty geometry 1"]), || pm_geom3d::Geometry {
                        positions: Vec::new(),
                        normals: Vec::new(),
                        uvs: Vec::new(),
                        indices: Vec::new(),
                        indexed: false,
                        groups: Vec::new(),
                    }),
                )
            } else {
                Some(self.geometry(&source_of(source)?))
            };
            let slots = arr(node, "slots");
            let use_source = boolean(node, "useSourceMaterials") == Some(true);
            let (geometry, materials) = match materials {
                None => (
                    self.geometry(&Source::Primitive(pm_geom3d::Primitive::Box {
                        width: 1.0,
                        height: 1.0,
                        depth: 1.0,
                    })),
                    Materials::Single(Box::new(Material3d::missing_box())),
                ),
                Some(geometry) if use_source && slots.is_empty() => (
                    geometry,
                    Materials::Single(Box::new(Material3d::standard_default())),
                ),
                Some(geometry) => {
                    let base = obj(node, "material").cloned().unwrap_or_default();
                    let shader_name = obj(&base, "shader").and_then(|s| text(s, "name"));
                    let prefix = format!("o.{id}.m");
                    if slots.is_empty() {
                        let name = shader_name.unwrap_or_else(|| "Surface".into());
                        let m =
                            self.material(&base, name, &prefix, channels, textures, &mut warnings)?;
                        (geometry, Materials::Single(Box::new(m)))
                    } else {
                        let mut list = Vec::new();
                        for slot in slots {
                            let Value::Object(slot) = slot else {
                                continue;
                            };
                            let definition = obj(slot, "material").cloned().unwrap_or_default();
                            let name = text(slot, "name")
                                .filter(|n| !n.is_empty())
                                .or_else(|| {
                                    obj(&definition, "shader").and_then(|s| text(s, "name"))
                                })
                                .unwrap_or_else(|| "Surface".into());
                            list.push(self.material(
                                &definition,
                                name,
                                &prefix,
                                channels,
                                textures,
                                &mut warnings,
                            )?);
                        }
                        (geometry, Materials::Slots(list))
                    }
                }
            };
            let fact = facts.get(&id).copied().unwrap_or(LayerFacts {
                opacity: 1.0,
                occludes: true,
            });
            if let Some(a) = gltf_asset {
                imported.push((
                    objects.len(),
                    a,
                    materials.clone(),
                    use_source && slots.is_empty(),
                ));
            }
            objects.push(Object3d {
                id,
                visible,
                origin: [0.0; 3],
                meshes: vec![Mesh3d {
                    geometry,
                    world: math::IDENTITY,
                    materials,
                    cast_shadow: cast,
                    receive_shadow: receive,
                    gltf: None,
                }],
                layer_opacity: fact.opacity,
                occludes: fact.occludes,
                missing,
            });
        }
        // World matrices: given (composition), or local chains (`parent`).
        for o in &mut objects {
            let world = match placement {
                Placement::Composition { objects: given, .. } => given
                    .get(&o.id)
                    .copied()
                    .ok_or_else(|| format!("the world matrix of 3D object {}", o.id))?,
                Placement::Channels => {
                    let mut chain = vec![o.id.clone()];
                    let mut m = locals[&o.id].0;
                    let mut visible = locals[&o.id].1;
                    let mut parent = locals[&o.id].2.clone();
                    while let Some(p) = parent {
                        if chain.contains(&p) {
                            return Err("3D object parenting contains a cycle".into());
                        }
                        let Some((pm, pv, pp)) = locals.get(&p) else {
                            return Err(format!("Missing 3D parent: {p}"));
                        };
                        m = math::multiply(pm, &m);
                        visible &= *pv;
                        chain.push(p);
                        parent = pp.clone();
                    }
                    o.visible = visible;
                    m
                }
            };
            o.origin = [world[12], world[13], world[14]];
            for mesh in &mut o.meshes {
                mesh.world = world;
            }
        }
        if imported_vertices > 2_000_000 {
            return Err("an imported scene over two million vertices".into());
        }
        // Imported models: drawables and embedded lights at their mixer time.
        let mut embedded = Vec::new();
        for (index, asset, materials, use_source) in imported {
            let o = &mut objects[index];
            let time = self
                .animation
                .get(&o.id)
                .copied()
                .unwrap_or(self.animation_default);
            let rule = match (&materials, use_source) {
                (_, true) => super::gltf::Override::Source,
                (Materials::Single(m), false) => super::gltf::Override::Single(m),
                (Materials::Slots(list), false) => super::gltf::Override::Slots(list),
            };
            let (cast, receive) = o
                .meshes
                .first()
                .map_or((true, true), |m| (m.cast_shadow, m.receive_shadow));
            let world = o.meshes.first().map_or(math::IDENTITY, |m| m.world);
            let instance = super::gltf::instantiate(
                &asset,
                &o.id,
                index,
                &world,
                js::max(0.0, time),
                o.visible,
                rule,
                cast,
                receive,
            );
            o.meshes = instance.meshes;
            embedded.extend(instance.lights);
        }
        // Lights. Embedded lights follow the scene's light layers in a
        // composition (its objects move after the lights) and precede them
        // in a legacy scene (objects are created first).
        let mut lights = Vec::new();
        let embedded_first = matches!(placement, Placement::Channels);
        if embedded_first {
            lights.append(&mut embedded);
        }
        let mut shadow_lights = 0;
        for node in arr(scene, "lights") {
            let Value::Object(node) = node else {
                continue;
            };
            let id = text(node, "id").unwrap_or_default();
            let kind = match text(node, "type").as_deref().unwrap_or("sun") {
                "sun" => LightKind::Directional,
                "point" => LightKind::Point,
                "spot" => LightKind::Spot,
                "area" => LightKind::RectArea,
                other => return Err(format!("{other} lights")),
            };
            let v = values(obj(node, "p"), &format!("light.{id}"), channels);
            let g = |k: &str, d: f64| num(&v, k).unwrap_or(d);
            let cast = boolean(node, "castShadow") != Some(false) && shadows && {
                let first = shadow_lights < 4;
                shadow_lights += 1;
                first
            };
            let (mut position, mut target) = (
                [g("x", -3.0), g("y", 5.0), g("z", 4.0)],
                [g("targetX", 0.0), g("targetY", 0.0), g("targetZ", 0.0)],
            );
            if let Placement::Composition { lights: placed, .. } = placement
                && let Some((p, t)) = placed.get(&id)
            {
                position = *p;
                target = *t;
            }
            let world = if kind == LightKind::RectArea {
                // `light.lookAt(target)` (a light looks from its position).
                math::camera_world(position, target, [0.0, 1.0, 0.0])
            } else {
                let mut m = math::IDENTITY;
                m[12] = position[0];
                m[13] = position[1];
                m[14] = position[2];
                m
            };
            lights.push(Light3d {
                id,
                kind,
                world,
                target,
                color: hex(&v, "color")?,
                intensity: g("intensity", 3.0),
                distance: g("distance", 0.0),
                decay: g("decay", 2.0),
                angle: deg_to_rad(g("angle", 45.0)),
                penumbra: g("penumbra", 0.4),
                width: g("width", 1.0),
                height: g("height", 1.0),
                visible: !matches!(v.get("visible"), Some(Value::Bool(false))),
                cast_shadow: cast && kind != LightKind::RectArea,
            });
        }
        lights.append(&mut embedded);
        // Camera.
        let camera = obj(scene, "camera").ok_or("the 3D camera")?;
        let c = values(obj(camera, "p"), "camera", channels);
        let g = |k: &str, d: f64| num(&c, k).unwrap_or(d);
        let near = js::max(0.0001, g("near", 0.01));
        let far = js::max(near + 0.01, g("far", 1000.0));
        let zoom = js::max(0.001, g("zoom", 1.0));
        let projection = if text(camera, "projection").as_deref() == Some("orthographic") {
            Projection::Orthographic { zoom, near, far }
        } else {
            Projection::Perspective {
                fov: js::clamp(g("fov", 45.0), 1.0, 150.0),
                zoom,
                near,
                far,
            }
        };
        let (position, target, up) = match placement {
            Placement::Composition {
                camera: Some(pose), ..
            } => *pose,
            _ => (
                [g("x", 3.0), g("y", 2.0), g("z", 6.0)],
                [g("targetX", 0.0), g("targetY", 0.0), g("targetZ", 0.0)],
                [0.0, 1.0, 0.0],
            ),
        };
        let camera = Camera3d {
            world: math::camera_world(position, target, up),
            projection,
        };
        // Environment.
        let e = values(obj(environment, "p"), "environment", channels);
        let sky = hex(&e, "ambientColor")?;
        let background = match environment.get("background") {
            Some(Value::String(s)) => Some(
                math::color_hex(&s.to_string_lossy())
                    .ok_or("a 3D background that is not #RRGGBB")?,
            ),
            _ => None,
        };
        Ok(Scene3d {
            width,
            height,
            camera,
            exposure: num(&e, "exposure").unwrap_or(1.0),
            hemisphere: (
                sky,
                [sky[0] * 0.35, sky[1] * 0.35, sky[2] * 0.35],
                num(&e, "ambient").unwrap_or(0.5),
            ),
            lights,
            shadows,
            background,
            objects,
            look: Look::default(),
            warnings,
        })
    }
}

/// An OBJ model asset's geometry (`new BufferGeometry()` with the parsed
/// positions, normals, and texture coordinates; not indexed).
pub fn obj_geometry(key: u64, mesh: &pm_geom3d::obj::ObjMesh) -> Geometry3d {
    let g = obj_triangles(mesh);
    let sphere = bounding_sphere(&g);
    Geometry3d {
        key,
        geometry: Arc::new(g),
        sphere,
        extra: None,
    }
}

/// An OBJ mesh as non-indexed triangles.
fn obj_triangles(mesh: &pm_geom3d::obj::ObjMesh) -> Geometry {
    Geometry {
        positions: mesh.positions.clone(),
        normals: mesh.normals.clone(),
        uvs: mesh.texcoords.clone(),
        indices: (0..mesh.vertex_count as u32).collect(),
        indexed: false,
        groups: Vec::new(),
    }
}

/// `Quaternion.setFromEuler` for order `XYZ`: `[x, y, z, w]`.
pub fn euler_xyz(x: f64, y: f64, z: f64) -> [f64; 4] {
    let (c1, c2, c3) = (libm::cos(x / 2.0), libm::cos(y / 2.0), libm::cos(z / 2.0));
    let (s1, s2, s3) = (libm::sin(x / 2.0), libm::sin(y / 2.0), libm::sin(z / 2.0));
    [
        s1 * c2 * c3 + c1 * s2 * s3,
        c1 * s2 * c3 - s1 * c2 * s3,
        c1 * c2 * s3 + s1 * s2 * c3,
        c1 * c2 * c3 - s1 * s2 * s3,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv_transform_matches_three() {
        assert_eq!(uv_transform(0.0, 0.0, 1.0, 1.0, 0.0), UV_IDENTITY);
        let m = uv_transform(0.25, -0.5, 2.0, 3.0, std::f64::consts::FRAC_PI_2);
        // u' = sx·(c·u + s·v) + tx
        assert!((m[0] - 2.0 * libm::cos(std::f64::consts::FRAC_PI_2)).abs() < 1e-15);
        assert_eq!(m[3], 2.0);
        assert_eq!(m[6], 0.25);
        assert_eq!(m[1], -3.0);
        assert_eq!(m[7], -0.5);
    }

    #[test]
    fn missing_box_is_orange_in_linear() {
        let m = Material3d::missing_box();
        assert_eq!(m.color[0], 1.0);
        assert!(m.wireframe);
        assert_eq!(m.kind, MaterialKind::Basic);
    }
}
