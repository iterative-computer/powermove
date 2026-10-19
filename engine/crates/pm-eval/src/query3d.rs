//! 3D picks and bounds without three.js (plan section 8; appendix C rows
//! C9 and C10): the scenes `compositionRuntime` and `sceneRuntime` build
//! (`core/scene3d/service.ts`, `runtime.ts`), placed from the document and
//! asked through [`pm_geom3d::query`].
//!
//! - **Composition scenes.** Every active object layer of
//!   [`Scope::composition_scene`] at its `world3D` matrix (a generated
//!   recipe part moved by `animatedPart`'s offset when its group's recipe
//!   is animated), with `sync`'s geometry (sources, OBJ meshes, imported
//!   glTF models posed at `time − layer.from`, the missing-model box) and
//!   materials (side, opacity, slots). The render camera is the topmost
//!   active camera layer's (or the scene camera's) at the composition's
//!   aspect; the editor view supplies its own camera
//!   ([`Geometry3d::view_camera`]).
//! - **Legacy scenes** (`powermove.3d.scene` layers and mesh-definition
//!   layers, `sceneForLayer`): the layer's own scene with its object
//!   hierarchy and channels evaluated on the layer.
//!
//! Model assets come from the host ([`Geometry3d::model`]): OBJ meshes and
//! glTF models parsed by `pm-geom3d` from the asset's bytes, as the
//! renderer's asset store holds them.

use std::collections::HashMap;
use std::sync::Arc;

use pm_geom3d::gltf::math::{self as m3, Mat4, Vec3};
use pm_geom3d::gltf::pick::{Picker, Posed, Side};
use pm_geom3d::query::{
    self, Box3, Camera, Facing, Lens, Materials, ModelMaterials, Object, QueryError, Scene, Shape,
    TriangleMesh,
};
use pm_geom3d::{Group, MeshSource, Parameters, Source};
use pm_model::json::{Map, Value};
use pm_model::{JsString, Node};

use crate::bounds::{Bounds, LayerHost};
use crate::js_sem::{JsResult, JsThrow, JsValue};
use crate::node;
use crate::parenting::Role3d;
use crate::scope::{Scope, flag, is_type};

/// A model asset as the runtime's asset store holds it
/// (`PM.assets.get(id)`).
#[derive(Clone)]
pub enum ModelAsset {
    /// No asset, or one without geometry (offline, a `.blend` source): the
    /// orange missing-model box.
    Missing,
    /// An OBJ model (`asset.mesh`): triangles and `asset.vertices`.
    Mesh(Arc<TriangleMesh>, usize),
    /// An imported glTF model (`asset.object3d`).
    Gltf(Arc<Picker>),
    /// A model today's runtime holds geometry for (`asset.object3d`) that
    /// this engine does not load: every question about a scene with it
    /// fails with [`MODEL_UNLOADABLE`] and the reason, never answering with
    /// the missing-model box (amendment 21).
    Unloadable(Arc<str>),
}

/// The [`JsThrow`] kind of a question that met an [`ModelAsset::Unloadable`]
/// model: not today's answer, so hosts report it as a named failure.
pub const MODEL_UNLOADABLE: &str = "ModelUnloadable";

fn unloadable(why: &str) -> JsThrow {
    JsThrow {
        kind: MODEL_UNLOADABLE,
        message: why.to_owned(),
    }
}

impl ModelAsset {
    /// `asset.vertices` (the importer's count).
    fn vertices(&self) -> usize {
        match self {
            ModelAsset::Missing => 0,
            ModelAsset::Mesh(_, v) => *v,
            ModelAsset::Gltf(p) => p.model.vertices,
            ModelAsset::Unloadable(_) => 0,
        }
    }
}

/// What the host supplies to answer 3D questions.
pub trait Geometry3d {
    /// The model asset `id`.
    fn model(&mut self, id: &str) -> ModelAsset;
    /// The editor's view camera (`viewportCamera` in the editor view, with
    /// its current projection), or `None` for the render camera.
    fn view_camera(&self) -> Option<Camera>;
    /// Built source geometry by content key (shared across questions).
    fn source_mesh(
        &mut self,
        key: u64,
        build: &mut dyn FnMut() -> TriangleMesh,
    ) -> Arc<TriangleMesh> {
        let _ = key;
        Arc::new(build())
    }
    /// A posed model (hosts may keep recent poses).
    fn posed(&mut self, picker: &Arc<Picker>, world: &Mat4, time: f64) -> Arc<Posed> {
        Arc::new(picker.pose(world, time))
    }
    /// Scenes built during one question.
    fn scenes(&mut self) -> &mut SceneCache;
}

/// Composition scenes by (composition, time) for one question.
#[derive(Default)]
pub struct SceneCache {
    compositions: HashMap<(usize, u64), Arc<CompositionScene3d>>,
}

/// `compositionRuntime(PM, time, comp)`'s objects and camera.
pub struct CompositionScene3d {
    pub scene: Scene,
    /// The render camera at the composition's aspect.
    pub camera: Camera,
}

/// The kind `HostOnly` questions throw without a geometry host.
fn host_only(what: &str) -> JsThrow {
    JsThrow {
        kind: crate::bounds::HOST_ONLY,
        message: format!("{what} need the host's 3D geometry"),
    }
}

fn query_error(e: QueryError) -> JsThrow {
    match e {
        QueryError::MissingGroupMaterial => JsThrow::type_error(query::MISSING_GROUP_MATERIAL),
    }
}

// ── scene description ───────────────────────────────────────────────────

fn obj<'a>(map: &'a Map, key: &str) -> Option<&'a Map> {
    map.get(key).and_then(Value::as_object)
}

fn text(map: &Map, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(|v| JsValue::from(v).truthy())
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    }
}

fn numbers(v: Option<&Value>) -> Option<Vec<f64>> {
    v?.as_array()?.iter().map(|x| number(Some(x))).collect()
}

fn points(v: Option<&Value>) -> Option<Vec<[f64; 2]>> {
    v?.as_array()?
        .iter()
        .map(|p| {
            let xy = p.as_array()?;
            (xy.len() == 2).then_some(())?;
            Some([number(xy.first())?, number(xy.get(1))?])
        })
        .collect()
}

/// A scene source's geometry (`primitiveGeometry`); `None` for sources
/// three.js builds nothing from (an empty `BufferGeometry`).
fn source_of(source: &Map) -> Option<Source> {
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
        );
    }
    if source.get("lathe").is_some() {
        return Some(Source::Lathe {
            points: points(source.get("lathe"))?,
            segments: number(source.get("segments")).unwrap_or(0.0),
        });
    }
    if source.get("extrude").is_some() {
        return Some(Source::Extrude {
            points: points(source.get("extrude"))?,
            depth: number(source.get("depth"))?,
            bevel: number(source.get("bevel")).unwrap_or(0.0),
        });
    }
    if let Some(m) = obj(source, "mesh") {
        let groups = m
            .get("groups")
            .and_then(Value::as_array)
            .map(|g| g.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|g| {
                let g = g.as_object()?;
                Some(Group {
                    start: number(g.get("start"))? as u32,
                    count: number(g.get("count"))? as u32,
                    material_index: number(g.get("materialIndex"))? as u32,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        return Some(Source::Mesh(MeshSource {
            positions: numbers(m.get("positions"))?,
            indices: numbers(m.get("indices")).map(|v| v.iter().map(|i| *i as u32).collect()),
            normals: numbers(m.get("normals")),
            uvs: numbers(m.get("uvs")),
            groups,
        }));
    }
    None
}

fn source_key(source: &Source) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut bytes = Vec::new();
    source.write_key(&mut bytes);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// `materialPreview(definition).p.opacity`: the material's own channel,
/// or the shader input on the Alpha socket (with its key).
fn preview_opacity_channel(definition: &Map) -> Option<(&Value, Option<String>)> {
    let mut opacity = obj(definition, "p")
        .and_then(|p| p.get("opacity"))
        .map(|c| (c, None));
    if let Some(shader) = obj(definition, "shader") {
        let inputs = shader.get("inputs").and_then(Value::as_array);
        if let Some(sp) = obj(shader, "p") {
            for (key, value) in sp.iter() {
                let key = key.to_string_lossy();
                let socket = inputs.and_then(|inputs| {
                    inputs.iter().find_map(|f| {
                        let f = f.as_object()?;
                        (text(f, "id").as_deref() == Some(key.as_str()))
                            .then(|| text(f, "socket"))
                            .flatten()
                    })
                });
                let s = socket.as_deref();
                // Earlier targets of `materialPreview`'s chain take the input.
                let earlier = s == Some("Base Color")
                    || key == "color"
                    || s == Some("Roughness")
                    || key == "roughness"
                    || s == Some("Metallic")
                    || key == "metalness"
                    || s == Some("Emission Color")
                    || key == "emission"
                    || s == Some("Emission Strength")
                    || key == "intensity";
                if !earlier && s == Some("Alpha") {
                    opacity = Some((value, Some(key)));
                }
            }
        }
    }
    opacity
}

/// `values()`'s rule for one channel: `stored` is the channel's `v`,
/// `value` its evaluation.
fn kept(stored: &JsValue, value: JsValue) -> JsValue {
    match stored {
        JsValue::Number(_) => match value {
            JsValue::Number(n) if n.is_finite() => value,
            _ => stored.clone(),
        },
        _ if value.type_of() == stored.type_of() => value,
        _ => stored.clone(),
    }
}

/// Channel evaluation for one scene: composition scenes carry evaluated
/// values; legacy scenes evaluate their channels on the layer.
trait Channels<'d> {
    /// The value of channel `c` (a `{v, kf, expr}` object) at `path`.
    fn value(&mut self, c: &Value, node: Option<&'d Node>, path: &str) -> JsResult<JsValue>;
}

/// Composition scenes: `prop => prop.v`.
struct Stored;

impl<'d> Channels<'d> for Stored {
    fn value(&mut self, c: &Value, _: Option<&'d Node>, _: &str) -> JsResult<JsValue> {
        Ok(c.as_object()
            .and_then(|m| m.get("v"))
            .map_or(JsValue::Undefined, JsValue::from))
    }
}

/// `PM.evP(layer, prop, time, path)` on the layer's own channels.
struct OnLayer<'s, 'a, 'd> {
    scope: &'s mut Scope<'a, 'd>,
    layer: &'d Node,
    time: f64,
}

impl<'d> Channels<'d> for OnLayer<'_, '_, 'd> {
    fn value(&mut self, c: &Value, node: Option<&'d Node>, path: &str) -> JsResult<JsValue> {
        let stored = c
            .as_object()
            .and_then(|m| m.get("v"))
            .map_or(JsValue::Undefined, JsValue::from);
        match node {
            Some(n) => {
                let v = self.scope.channel_p(self.layer, Some(n), self.time, path)?;
                Ok(kept(&stored, v))
            }
            None => Ok(stored),
        }
    }
}

/// A channel of `p` (a `Map` of channels) with its document node.
fn channel<'d>(
    ch: &mut dyn Channels<'d>,
    p: Option<&Map>,
    pnode: Option<&'d Node>,
    key: &str,
    prefix: &str,
) -> JsResult<JsValue> {
    let Some(c) = p.and_then(|p| p.get(key)) else {
        return Ok(JsValue::Undefined);
    };
    let n = node::member(pnode, key);
    ch.value(c, n, &format!("{prefix}.{key}"))
}

fn num_or(v: &JsValue, d: f64) -> f64 {
    match v {
        JsValue::Undefined => d,
        v => v.to_number(),
    }
}

/// A material definition's facing (`materialPreview`, then `values`).
fn facing<'d>(
    ch: &mut dyn Channels<'d>,
    definition: Option<&Map>,
    dnode: Option<&'d Node>,
    prefix: &str,
) -> JsResult<Facing> {
    let Some(definition) = definition else {
        return Ok(Facing::DEFAULT);
    };
    let side = if truthy(definition.get("doubleSided")) {
        Side::Double
    } else {
        Side::Front
    };
    let opacity = match preview_opacity_channel(definition) {
        Some((c, shader_key)) => {
            let node = match &shader_key {
                None => node::member(node::member(dnode, "p"), "opacity"),
                Some(k) => node::member(node::member(node::member(dnode, "shader"), "p"), k),
            };
            ch.value(c, node, &format!("{prefix}.opacity"))?.to_number()
        }
        None => f64::NAN,
    };
    Ok(Facing { side, opacity })
}

/// One object as `sync` makes it.
struct Spec {
    id: String,
    visible: bool,
    parent: Option<String>,
    local: Mat4,
    shape: ShapeSpec,
}

enum ShapeSpec {
    Source(Option<Source>, Materials),
    Model(ModelAsset, ModelMaterials, Materials),
}

/// `sync`'s objects of a scene description.
fn objects<'d>(
    scene: &Map,
    scene_node: Option<&'d Node>,
    ch: &mut dyn Channels<'d>,
    g: &mut dyn Geometry3d,
) -> JsResult<Vec<Spec>> {
    let mut out = Vec::new();
    let nodes = node::array(node::member(scene_node, "objects")).unwrap_or(&[]);
    let mut imported = 0usize;
    for (i, o) in scene
        .get("objects")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        let Some(o) = o.as_object() else {
            continue;
        };
        let onode = nodes.get(i);
        let id = text(o, "id").unwrap_or_default();
        let p = obj(o, "p");
        let pnode = node::member(onode, "p");
        let prefix = format!("o.{id}");
        let mut get = |k: &str| channel(ch, p, pnode, k, &prefix);
        let (x, y, z) = (get("x")?, get("y")?, get("z")?);
        let (rx, ry, rz) = (get("rx")?, get("ry")?, get("rz")?);
        let (sx, sy, sz) = (get("sx")?, get("sy")?, get("sz")?);
        let visible = get("visible")? != JsValue::Bool(false);
        let rad = |d: &JsValue| d.to_number() * (std::f64::consts::PI / 180.0);
        let q = euler_xyz(rad(&rx), rad(&ry), rad(&rz));
        let local = m3::compose(
            [x.to_number(), y.to_number(), z.to_number()],
            q,
            [sx.to_number(), sy.to_number(), sz.to_number()],
        );
        let source = obj(o, "source").cloned().unwrap_or_default();
        let slots: Vec<&Map> = o
            .get("slots")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_object).collect())
            .unwrap_or_default();
        let slot_nodes = node::array(node::member(onode, "slots")).unwrap_or(&[]);
        let use_source = truthy(o.get("useSourceMaterials")) && slots.is_empty();
        let mprefix = format!("o.{id}.m");
        // `materialPreview(undefined)` reads `undefined.p` for a missing
        // material (a missing model keeps its own material).
        let missing_model =
            text(&source, "assetId").is_some_and(|a| matches!(g.model(&a), ModelAsset::Missing));
        let undefined_material = if slots.is_empty() {
            obj(o, "material").is_none()
        } else {
            slots.iter().any(|s| obj(s, "material").is_none())
        };
        if !use_source && !missing_model && undefined_material {
            return Err(JsThrow::type_error(
                "Cannot read properties of undefined (reading 'p')",
            ));
        }
        let materials = if use_source {
            None
        } else if slots.is_empty() {
            Some(Materials::Single(facing(
                ch,
                obj(o, "material"),
                node::member(onode, "material"),
                &mprefix,
            )?))
        } else {
            let mut list = Vec::new();
            for (k, s) in slots.iter().enumerate() {
                list.push(facing(
                    ch,
                    obj(s, "material"),
                    node::member(slot_nodes.get(k), "material"),
                    &mprefix,
                )?);
            }
            Some(Materials::Slots(list))
        };
        let shape = match text(&source, "assetId") {
            Some(asset) => {
                let a = g.model(&asset);
                if let ModelAsset::Unloadable(why) = &a {
                    return Err(unloadable(why));
                }
                imported += a.vertices();
                let rule = match &materials {
                    None => ModelMaterials::Source,
                    Some(Materials::Single(f)) => ModelMaterials::Single(*f),
                    Some(Materials::Slots(l)) => ModelMaterials::Slots(l.clone()),
                };
                ShapeSpec::Model(
                    a,
                    rule,
                    materials.unwrap_or(Materials::Single(Facing::DEFAULT)),
                )
            }
            None => ShapeSpec::Source(
                source_of(&source),
                materials.unwrap_or(Materials::Single(Facing::DEFAULT)),
            ),
        };
        out.push(Spec {
            id,
            visible,
            parent: text(o, "parent"),
            local,
            shape,
        });
    }
    if imported > 2_000_000 {
        return Err(JsThrow {
            kind: "Error",
            message: "Imported scene exceeds two million vertices; simplify or split the models"
                .into(),
        });
    }
    Ok(out)
}

/// `Quaternion.setFromEuler` for order `XYZ`: `[x, y, z, w]`.
fn euler_xyz(x: f64, y: f64, z: f64) -> [f64; 4] {
    let (c1, c2, c3) = (libm::cos(x / 2.0), libm::cos(y / 2.0), libm::cos(z / 2.0));
    let (s1, s2, s3) = (libm::sin(x / 2.0), libm::sin(y / 2.0), libm::sin(z / 2.0));
    [
        s1 * c2 * c3 + c1 * s2 * s3,
        c1 * s2 * c3 - s1 * c2 * s3,
        c1 * c2 * s3 + s1 * s2 * c3,
        c1 * c2 * c3 - s1 * s2 * s3,
    ]
}

/// Placed objects: world matrices given (composition) or from the
/// hierarchy (legacy scenes, `parent.matrixWorld × matrix` top down).
fn place(
    specs: Vec<Spec>,
    given: Option<&HashMap<String, Mat4>>,
    mixer: &dyn Fn(&str) -> f64,
    g: &mut dyn Geometry3d,
) -> JsResult<Scene> {
    let index: HashMap<&str, usize> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();
    let mut worlds: Vec<Option<(Mat4, bool)>> = vec![None; specs.len()];
    fn resolve(
        i: usize,
        specs: &[Spec],
        index: &HashMap<&str, usize>,
        worlds: &mut Vec<Option<(Mat4, bool)>>,
        depth: usize,
    ) -> JsResult<(Mat4, bool)> {
        if let Some(w) = worlds[i] {
            return Ok(w);
        }
        if depth > specs.len() {
            return Err(JsThrow {
                kind: "RangeError",
                message: "3D object parenting contains a cycle".into(),
            });
        }
        let s = &specs[i];
        let (pw, pv) = match s.parent.as_deref().and_then(|p| index.get(p)) {
            Some(&p) if p != i => resolve(p, specs, index, worlds, depth + 1)?,
            _ => (m3::IDENTITY, true),
        };
        let w = (m3::multiply(&pw, &s.local), pv && s.visible);
        worlds[i] = Some(w);
        Ok(w)
    }
    let mut objects = Vec::with_capacity(specs.len());
    for i in 0..specs.len() {
        let (world, visible) = match given {
            Some(map) => (
                map.get(&specs[i].id).copied().unwrap_or(m3::IDENTITY),
                specs[i].visible,
            ),
            None => resolve(i, &specs, &index, &mut worlds, 0)?,
        };
        let s = &specs[i];
        let shape = match &s.shape {
            ShapeSpec::Source(source, materials) => {
                let mesh = match source {
                    Some(src) => {
                        let key = source_key(src);
                        g.source_mesh(key, &mut || TriangleMesh::new(&src.geometry()))
                    }
                    None => Arc::new(TriangleMesh::new(&pm_geom3d::Geometry {
                        positions: Vec::new(),
                        normals: Vec::new(),
                        uvs: Vec::new(),
                        indices: Vec::new(),
                        indexed: false,
                        groups: Vec::new(),
                    })),
                };
                Shape::Mesh {
                    mesh,
                    materials: materials.clone(),
                }
            }
            ShapeSpec::Model(asset, rule, materials) => match asset {
                // Refused when the spec was built.
                ModelAsset::Missing | ModelAsset::Unloadable(_) => Shape::Mesh {
                    mesh: query::missing_box(),
                    materials: Materials::Single(Facing::DEFAULT),
                },
                ModelAsset::Mesh(mesh, _) => Shape::Mesh {
                    mesh: mesh.clone(),
                    materials: materials.clone(),
                },
                ModelAsset::Gltf(picker) => {
                    let time = mixer(&s.id);
                    let time = if time.is_nan() || time < 0.0 {
                        0.0
                    } else {
                        time
                    };
                    Shape::Model {
                        posed: g.posed(picker, &world, time),
                        picker: picker.clone(),
                        materials: rule.clone(),
                    }
                }
            },
        };
        objects.push(Object {
            id: s.id.clone(),
            visible,
            world,
            shape,
        });
    }
    Ok(Scene { objects })
}

/// The scene camera's lens and pose from a scene description's camera
/// (`sync`: `near ≥ .0001`, `far ≥ near + .01`, `zoom ≥ .001`, field of
/// view clamped to [1, 150]).
fn lens<'d>(
    camera: Option<&Map>,
    cnode: Option<&'d Node>,
    ch: &mut dyn Channels<'d>,
) -> JsResult<(Lens, Vec3, Vec3)> {
    let p = camera.and_then(|c| obj(c, "p"));
    let pnode = node::member(cnode, "p");
    let mut get =
        |k: &str, d: f64| -> JsResult<f64> { Ok(num_or(&channel(ch, p, pnode, k, "camera")?, d)) };
    let position = [
        get("x", f64::NAN)?,
        get("y", f64::NAN)?,
        get("z", f64::NAN)?,
    ];
    let target = [
        get("targetX", f64::NAN)?,
        get("targetY", f64::NAN)?,
        get("targetZ", f64::NAN)?,
    ];
    let jmax = |a: f64, b: f64| {
        if a.is_nan() || b.is_nan() {
            f64::NAN
        } else {
            a.max(b)
        }
    };
    let near = jmax(0.0001, get("near", f64::NAN)?);
    let far = jmax(near + 0.01, get("far", f64::NAN)?);
    let zoom = jmax(0.001, get("zoom", f64::NAN)?);
    let ortho = camera.and_then(|c| text(c, "projection")).as_deref() == Some("orthographic");
    let lens = if ortho {
        Lens::Orthographic { zoom, near, far }
    } else {
        let fov = get("fov", f64::NAN)?;
        // `MathUtils.clamp`: `Math.max(min, Math.min(max, value))`.
        let fov = jmax(
            1.0,
            if fov.is_nan() {
                f64::NAN
            } else {
                fov.min(150.0)
            },
        );
        Lens::Perspective {
            fov,
            zoom,
            near,
            far,
        }
    };
    Ok((lens, position, target))
}

impl<'d> Scope<'_, 'd> {
    /// `compositionRuntime(PM, time)` of the current composition.
    pub fn composition_scene3d(
        &mut self,
        time: f64,
        g: &mut dyn Geometry3d,
    ) -> JsResult<Arc<CompositionScene3d>> {
        let key = (self.comp as *const _ as usize, time.to_bits());
        if let Some(s) = g.scenes().compositions.get(&key) {
            return Ok(s.clone());
        }
        let evaluated = self.composition_scene(time)?;
        let Value::Object(scene) = &evaluated.scene else {
            return Err(JsThrow::type_error("the evaluated 3D scene"));
        };
        let layers: Vec<&'d Node> = self.layers().iter().collect();
        let mut worlds: HashMap<String, Mat4> = HashMap::new();
        let mut starts: HashMap<String, f64> = HashMap::new();
        let mut camera_layer = None;
        for l in &layers {
            match Role3d::of(l) {
                Some(Role3d::Object) if self.active(l, time)? => {
                    let id = node::to_js_string(node::member(Some(l), "id")).to_string_lossy();
                    let mut world = self.world3d(l, time, false)?;
                    if let Some(part) = self.animated_part(l, time)? {
                        let d = self.recipe_world_offset(l, time, &part)?;
                        world[12] += d[0];
                        world[13] += d[1];
                        world[14] += d[2];
                    }
                    worlds.insert(id.clone(), world);
                    // `time - (starts.get(id) || 0)`.
                    let from = node::to_number(node::member(Some(l), "from"));
                    starts.insert(
                        id,
                        if from == 0.0 || from.is_nan() {
                            time
                        } else {
                            time - from
                        },
                    );
                }
                Some(Role3d::Camera) if camera_layer.is_none() && self.active(l, time)? => {
                    camera_layer = Some(*l);
                }
                _ => {}
            }
        }
        let specs = objects(scene, None, &mut Stored, g)?;
        let mixer = |id: &str| starts.get(id).copied().unwrap_or(time);
        let objects = place(specs, Some(&worlds), &mixer, g)?;
        let (lens, position, target) = lens(obj(scene, "camera"), None, &mut Stored)?;
        let world = match camera_layer {
            Some(l) => {
                // `position.setFromMatrixPosition(world)`,
                // `up.set(0, 1, 0).transformDirection(world)`, `lookAt(target)`.
                let w = self.world3d(l, time, false)?;
                let target = self.world_aim3d(l, time)?.unwrap_or([0.0; 3]);
                let up = query::normalize([w[4], w[5], w[6]]);
                query::camera_world([w[12], w[13], w[14]], target, up)
            }
            None => query::camera_world(position, target, [0.0, 1.0, 0.0]),
        };
        let (w, h) = self.comp_size();
        let built = Arc::new(CompositionScene3d {
            scene: objects,
            camera: Camera::new(lens, world, query::aspect(w, h)),
        });
        g.scenes().compositions.insert(key, built.clone());
        Ok(built)
    }

    /// `compositionDepthIds(PM, layer, time)`: the object layers in the
    /// layer's depth set, over the composition's layers without group
    /// headers in stack order.
    pub fn depth_ids3d(&mut self, layer: &'d Node, time: f64) -> JsResult<Vec<String>> {
        let all = self.layers();
        let solo = all.iter().any(|l| flag(l, "solo"));
        let pass: Vec<&'d Node> = all.iter().filter(|l| !is_type(l, "group")).collect();
        let Some(index) = pass.iter().position(|l| std::ptr::eq(*l, layer)) else {
            return Ok(Vec::new());
        };
        let mut ids = Vec::new();
        let mut visit = |s: &mut Self, i: usize| -> JsResult<bool> {
            let l = pass[i];
            if !s.active(l, time)? {
                return Ok(true);
            }
            let Some(role) = Role3d::of(l) else {
                return Ok(false);
            };
            if role == Role3d::Object
                && (!solo || flag(l, "solo"))
                && s.world_opacity(l, time)? > 0.001
            {
                ids.push(node::to_js_string(node::member(Some(l), "id")).to_string_lossy());
            }
            Ok(true)
        };
        let mut i = index as isize;
        while i >= 0 && visit(self, i as usize)? {
            i -= 1;
        }
        let mut i = index + 1;
        while i < pass.len() && visit(self, i)? {
            i += 1;
        }
        Ok(ids)
    }

    /// `new Box3().expandByObject(runtime.objects.get(id))` for each id of
    /// the composition's runtime at `time` (`viewportBounds`): `None` for an
    /// id the runtime does not hold or whose box is empty.
    pub fn object_boxes3d(
        &mut self,
        ids: &[String],
        time: f64,
        host: &mut dyn LayerHost,
    ) -> JsResult<Vec<Option<Box3>>> {
        let Some(g) = host.geometry() else {
            return Err(host_only("3D object boxes"));
        };
        let scene = self.composition_scene3d(time, g)?;
        Ok(ids
            .iter()
            .map(|id| scene.scene.object_box(id).filter(|b| !b.is_empty()))
            .collect())
    }

    /// `runtime.objects.get(id).matrixWorld` for 3D object layers `ids` (a
    /// model's asset root sits under it): what transform targets and the
    /// gizmo place themselves with. `None` for an id without an object.
    pub fn object_worlds3d(
        &mut self,
        ids: &[String],
        time: f64,
        host: &mut dyn LayerHost,
    ) -> JsResult<Vec<Option<pm_geom3d::gltf::math::Mat4>>> {
        let Some(g) = host.geometry() else {
            return Err(host_only("3D object placements"));
        };
        let scene = self.composition_scene3d(time, g)?;
        Ok(ids
            .iter()
            .map(|id| {
                scene
                    .scene
                    .objects
                    .iter()
                    .find(|o| &o.id == id)
                    .map(|o| o.world)
            })
            .collect())
    }

    /// `compositionRuntime(PM, time).camera`: the composition's render
    /// camera after `sync` (lens and `matrixWorld`; the viewport's camera
    /// view, framing, and transform gizmos read it).
    pub fn composition_camera3d(
        &mut self,
        time: f64,
        host: &mut dyn LayerHost,
    ) -> JsResult<Camera> {
        let Some(g) = host.geometry() else {
            return Err(host_only("The 3D render camera"));
        };
        Ok(self.composition_scene3d(time, g)?.camera)
    }

    /// The camera a pick or bound uses: the editor's, or the render camera.
    fn view3d(&self, scene: &CompositionScene3d, g: &dyn Geometry3d) -> Camera {
        g.view_camera().unwrap_or(scene.camera)
    }

    /// `compositionRuntime(PM, T).pick(ndcX, ndcY, depthIds, camera) === layer.id`.
    pub fn pick_object3d(
        &mut self,
        layer: &'d Node,
        x: f64,
        y: f64,
        time: f64,
        host: &mut dyn LayerHost,
    ) -> JsResult<bool> {
        let Some(g) = host.geometry() else {
            return Err(host_only("3D object picks"));
        };
        let scene = self.composition_scene3d(time, g)?;
        let ids = self.depth_ids3d(layer, time)?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let camera = self.view3d(&scene, g);
        let hit = scene
            .scene
            .pick(&camera, x, y, Some(&ids))
            .map_err(query_error)?;
        let id = node::to_js_string(node::member(Some(layer), "id"));
        Ok(hit.is_some_and(|h| id.as_str() == Some(h)))
    }

    /// `compositionRuntime(PM, T).bounds(PM.proj.w, PM.proj.h, [L.id], camera)`.
    pub fn object_bounds3d(
        &mut self,
        layer: &'d Node,
        time: f64,
        host: &mut dyn LayerHost,
    ) -> JsResult<Option<Bounds>> {
        let Some(g) = host.geometry() else {
            return Err(host_only("3D object bounds"));
        };
        let scene = self.composition_scene3d(time, g)?;
        let camera = self.view3d(&scene, g);
        let (w, h) = self.project_size();
        let id = node::to_js_string(node::member(Some(layer), "id")).to_string_lossy();
        Ok(scene
            .scene
            .bounds(w, h, Some(&[id.as_str()]), &camera)
            .map(|r| rect_bounds(r.x0, r.y0, r.x1, r.y1)))
    }

    /// `modelGroupBounds(PM, group, time)` (the caller checked
    /// `isModelGroup`), with its depth extent (`z0`, `z1`).
    pub fn model_group_bounds3d(
        &mut self,
        group: &'d Node,
        time: f64,
        host: &mut dyn LayerHost,
    ) -> JsResult<Option<Bounds>> {
        let Some(g) = host.geometry() else {
            return Err(host_only("model group bounds"));
        };
        let scene = self.composition_scene3d(time, g)?;
        let inverse = m3::invert(&self.world3d(group, time, false)?);
        let gid = node::get(group, b"id").to_js();
        let mut b = Box3::EMPTY;
        for child in self.layers() {
            let inside = self
                .group_ancestors(child)?
                .iter()
                .any(|a| crate::js_sem::strict_eq(&node::get(a, b"id").to_js(), &gid));
            if !inside || !self.active(child, time)? {
                continue;
            }
            match Role3d::of(child) {
                Some(Role3d::Object) => {
                    let id = node::to_js_string(node::member(Some(child), "id")).to_string_lossy();
                    if let Some(o) = scene.scene.object_box(&id) {
                        b.union(&o.transformed(&inverse));
                    }
                }
                Some(_) => {
                    let w = self.world3d(child, time, false)?;
                    b.expand(m3::apply(&inverse, [w[12], w[13], w[14]]));
                }
                None => {}
            }
        }
        if b.is_empty() {
            return Ok(None);
        }
        Ok(Some(Bounds {
            z: Some((b.min[2], b.max[2])),
            ..rect_bounds(b.min[0], b.min[1], b.max[0], b.max[1])
        }))
    }

    /// `sceneRuntime(PM, L, T)` for an extension layer (a legacy scene or a
    /// mesh definition layer), with its render camera.
    fn layer_scene3d(
        &mut self,
        layer: &'d Node,
        time: f64,
        g: &mut dyn Geometry3d,
        field: Option<String>,
    ) -> JsResult<Option<(Scene, Lens, Mat4)>> {
        let d = node::member(Some(layer), "d");
        let from = node::to_number(node::member(Some(layer), "from"));
        let mixer_time = time - from;
        if node::is_str(node::member(d, "definition"), "powermove.3d.scene") {
            let scene_node = node::member(node::member(d, "data"), "scene");
            let Some(scene_node) = scene_node.filter(|s| node::truthy(Some(s))) else {
                return Ok(None);
            };
            let Value::Object(scene) = scene_node.to_value() else {
                return Ok(None);
            };
            let mut ch = OnLayer {
                scope: self,
                layer,
                time,
            };
            let specs = objects(&scene, Some(scene_node), &mut ch, g)?;
            let placed = place(specs, None, &|_| mixer_time, g)?;
            let (lens, position, target) = lens(
                obj(&scene, "camera"),
                node::member(Some(scene_node), "camera"),
                &mut ch,
            )?;
            let world = query::camera_world(position, target, [0.0, 1.0, 0.0]);
            return Ok(Some((placed, lens, world)));
        }
        let Some(field) = field else {
            return Ok(None);
        };
        // `sceneForLayer` for a mesh renderer definition.
        let params = node::member(d, "params");
        let get = |s: &mut Self, key: &str, fallback: JsValue| -> JsResult<JsValue> {
            match node::member(params, key) {
                Some(prop) if node::truthy(Some(prop)) => {
                    s.channel_p(layer, Some(prop), time, &format!("x.{key}"))
                }
                _ => Ok(fallback),
            }
        };
        let n = |v: JsValue| v.to_number();
        let yaw = n(get(self, "yaw", JsValue::Number(28.0))?);
        let pitch = n(get(self, "pitch", JsValue::Number(18.0))?);
        let distance = n(get(self, "distance", JsValue::Number(4.2))?);
        let rx = get(self, "rotationX", JsValue::Number(0.0))?;
        let mut ry = get(self, "rotationY", JsValue::Number(25.0))?;
        let rz = get(self, "rotationZ", JsValue::Number(0.0))?;
        let size = get(self, "size", JsValue::Number(1.35))?;
        let auto_rotate = get(self, "autoRotate", JsValue::Bool(false))?;
        let speed = get(self, "speed", JsValue::Number(25.0))?;
        let asset = node::js_value_opt(node::member(node::member(d, "data"), &field));
        if auto_rotate.truthy() {
            // `values.ry += (time - layer.from) * values.speed`.
            ry = JsValue::Number(ry.to_number() + (time - from) * speed.to_number());
        }
        if !asset.truthy() {
            return Ok(None);
        }
        validate_mesh_scene(
            &node::js_value_opt(node::member(Some(layer), "name")),
            &asset,
            &[&rx, &ry, &rz],
            &size,
            &[
                get(self, "objectColor", JsValue::from("#C7C4FF"))?,
                get(self, "lightColor", JsValue::from("#FFB36B"))?,
                get(self, "background", JsValue::from("#0C0D12"))?,
            ],
            &[
                get(self, "roughness", JsValue::Number(0.25))?,
                get(self, "metalness", JsValue::Number(0.7))?,
            ],
            [yaw, pitch, distance],
        )?;
        let asset_id = asset.to_js_string().to_string_lossy();
        let rad = |d: f64| d * (std::f64::consts::PI / 180.0);
        let local = m3::compose(
            [0.0; 3],
            euler_xyz(rad(n(rx)), rad(n(ry)), rad(n(rz))),
            [n(size.clone()), n(size.clone()), n(size)],
        );
        let a = g.model(&asset_id);
        if let ModelAsset::Unloadable(why) = &a {
            return Err(unloadable(why));
        }
        let shape = ShapeSpec::Model(
            a,
            ModelMaterials::Single(Facing::DEFAULT),
            Materials::Single(Facing::DEFAULT),
        );
        let specs = vec![Spec {
            id: "model".into(),
            visible: true,
            parent: None,
            local,
            shape,
        }];
        let placed = place(specs, None, &|_| mixer_time, g)?;
        // `values.yaw * Math.PI / 180`.
        let (yaw, pitch) = (
            yaw * std::f64::consts::PI / 180.0,
            pitch * std::f64::consts::PI / 180.0,
        );
        let position = [
            distance * libm::cos(pitch) * libm::sin(yaw),
            distance * libm::sin(pitch),
            distance * libm::cos(pitch) * libm::cos(yaw),
        ];
        let lens = Lens::Perspective {
            fov: 58.109208,
            zoom: 1.0,
            near: 0.01,
            far: 1000.0,
        };
        Ok(Some((
            placed,
            lens,
            query::camera_world(position, [0.0; 3], [0.0, 1.0, 0.0]),
        )))
    }

    /// `L.d.w || PM.proj.w` and `L.d.h || PM.proj.h`.
    fn layer_canvas(&self, layer: &'d Node) -> (f64, f64) {
        let d = node::member(Some(layer), "d");
        let (pw, ph) = self.project_size();
        let pick = |k: &str, p: f64| {
            let v = node::member(d, k);
            if node::truthy(v) {
                node::to_number(v)
            } else {
                p
            }
        };
        (pick("w", pw), pick("h", ph))
    }

    /// `sceneRuntime(PM, L, T).bounds(L.d.w || PM.proj.w, L.d.h || PM.proj.h)`.
    pub fn scene_bounds3d(
        &mut self,
        layer: &'d Node,
        time: f64,
        host: &mut dyn LayerHost,
    ) -> JsResult<Option<Bounds>> {
        let field = mesh_field(layer, host);
        let Some(g) = host.geometry() else {
            return Err(host_only("extension scene bounds"));
        };
        let Some((scene, lens, world)) = self.layer_scene3d(layer, time, g, field)? else {
            return Ok(None);
        };
        let (w, h) = self.layer_canvas(layer);
        let camera = Camera::new(lens, world, query::aspect(w, h));
        Ok(scene
            .bounds(w, h, None, &camera)
            .map(|r| rect_bounds(r.x0, r.y0, r.x1, r.y1)))
    }

    /// `runtime.resizeCamera(w, h); runtime.pick(px / w * 2 − 1, 1 − py / h * 2)`
    /// for an extension scene at a layer-space point.
    pub fn pick_scene3d(
        &mut self,
        layer: &'d Node,
        px: f64,
        py: f64,
        time: f64,
        host: &mut dyn LayerHost,
    ) -> JsResult<bool> {
        let field = mesh_field(layer, host);
        let Some(g) = host.geometry() else {
            return Err(host_only("extension scene picks"));
        };
        let Some((scene, lens, world)) = self.layer_scene3d(layer, time, g, field)? else {
            return Ok(false);
        };
        let (w, h) = self.layer_canvas(layer);
        let camera = Camera::new(lens, world, query::aspect(w, h));
        let hit = scene
            .pick(&camera, px / w * 2.0 - 1.0, 1.0 - py / h * 2.0, None)
            .map_err(query_error)?;
        Ok(hit.is_some())
    }
}

/// `parseScene`'s checks on `sceneForLayer`'s scene for a mesh definition
/// layer (a `ZodError` today): the object name, asset id, rotation and
/// size, the colors (the background may be `null`), roughness and
/// metalness, and the camera position from yaw, pitch, and distance.
fn validate_mesh_scene(
    name: &JsValue,
    asset: &JsValue,
    rotation: &[&JsValue; 3],
    size: &JsValue,
    colors: &[JsValue; 3],
    unit: &[JsValue; 2],
    camera: [f64; 3],
) -> JsResult<()> {
    let zod = |what: &str| {
        Err(JsThrow {
            kind: "ZodError",
            message: format!("Invalid 3D scene: {what}"),
        })
    };
    let finite_in = |v: &JsValue, lo: f64, hi: f64| matches!(v, JsValue::Number(n) if n.is_finite() && *n >= lo && *n <= hi);
    let len = |v: &JsValue| v.to_js_string().utf16_units().count();
    if !matches!(name, JsValue::String(_)) || len(name) < 1 || len(name) > 160 {
        return zod("object name");
    }
    if !matches!(asset, JsValue::String(_)) || len(asset) < 1 || len(asset) > 160 {
        return zod("asset id");
    }
    if rotation
        .iter()
        .any(|r| !finite_in(r, -360_000.0, 360_000.0))
    {
        return zod("rotation");
    }
    if !finite_in(size, -10_000.0, 10_000.0) {
        return zod("size");
    }
    let is_color = |v: &JsValue| match v {
        JsValue::String(s) => s.as_str().is_some_and(|s| {
            s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
        }),
        _ => false,
    };
    if !is_color(&colors[0]) || !is_color(&colors[1]) {
        return zod("color");
    }
    if !(matches!(colors[2], JsValue::Null) || is_color(&colors[2])) {
        return zod("background");
    }
    if unit.iter().any(|v| !finite_in(v, 0.0, 1.0)) {
        return zod("roughness or metalness");
    }
    let [yaw, pitch, distance] = camera;
    let (yaw, pitch) = (
        yaw * std::f64::consts::PI / 180.0,
        pitch * std::f64::consts::PI / 180.0,
    );
    let position = [
        distance * libm::cos(pitch) * libm::sin(yaw),
        distance * libm::sin(pitch),
        distance * libm::cos(pitch) * libm::cos(yaw),
    ];
    if position
        .iter()
        .any(|v| !(v.is_finite() && (-100_000.0..=100_000.0).contains(v)))
    {
        return zod("camera position");
    }
    Ok(())
}

fn mesh_field(layer: &Node, host: &dyn LayerHost) -> Option<String> {
    let definition = node::member(node::member(Some(layer), "d"), "definition");
    node::string(definition).and_then(|id: &JsString| host.mesh_asset_field(id))
}

fn rect_bounds(x0: f64, y0: f64, x1: f64, y1: f64) -> Bounds {
    Bounds {
        x0,
        y0,
        x1,
        y1,
        w: x1 - x0,
        h: y1 - y0,
        ax: 0.0,
        ay: 0.0,
        z: None,
    }
}
