//! The 3D geometry the mirror answers picks and bounds with (appendix C
//! rows C9 and C10): the renderer's model assets parsed by `pm-geom3d`,
//! source geometry and recent poses kept across questions, and the editor
//! camera a request carries.
//!
//! Model assets reach the mirror on demand, as fonts do: a question that
//! meets an asset the mirror lacks answers `{"models": [ids]}`, the
//! renderer adds each one ([`Models::set`]: an OBJ model's text, a glTF or
//! GLB model's bytes, or "missing" for an asset without geometry, which
//! draws the orange box as today) and asks again. A model today's runtime
//! holds geometry for that this engine cannot load (a glTF `pm-geom3d`
//! rejects, an `object3d` without its source bytes) is kept as
//! [`ModelAsset::Unloadable`]: questions about it fail by name.

use std::collections::HashMap;
use std::sync::Arc;

use pm_eval::bounds::LayerHost;
use pm_eval::query3d::{Geometry3d, ModelAsset, SceneCache};
use pm_geom3d::gltf::math::Mat4;
use pm_geom3d::gltf::pick::{Picker, Posed};
use pm_geom3d::query::{Camera, Lens, Scene, TriangleMesh};
use pm_model::Value;

/// How a model asset arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// No geometry (offline, not loaded, a `.blend` source).
    Missing = 0,
    /// OBJ text (`asset.sourceText`).
    Obj = 1,
    /// glTF or GLB bytes.
    Gltf = 2,
    /// Geometry today's runtime holds that the host cannot give the mirror
    /// (the bytes are the reason, UTF-8).
    Unloadable = 3,
}

impl ModelKind {
    pub fn from_u8(k: u8) -> Option<ModelKind> {
        Some(match k {
            0 => ModelKind::Missing,
            1 => ModelKind::Obj,
            2 => ModelKind::Gltf,
            3 => ModelKind::Unloadable,
            _ => return None,
        })
    }
}

/// Bounds on what the mirror keeps.
const MAX_SOURCE_MESHES: usize = 512;
const MAX_POSES: usize = 16;

/// A pose's key: the picker, the world matrix's bits, the time's bits.
type PoseKey = (usize, [u64; 16], u64);

/// The model assets and caches the mirror keeps across questions.
#[derive(Default)]
pub struct Models {
    assets: HashMap<String, ModelAsset>,
    meshes: HashMap<u64, Arc<TriangleMesh>>,
    /// Recent poses: (picker, world matrix bits, time bits).
    poses: Vec<(PoseKey, Arc<Posed>)>,
    /// `SceneRuntime.objects` keeps insertion order across syncs, including
    /// removal and reinsertion after a source edit or asset relink.
    histories: HashMap<String, Vec<(String, u64)>>,
    asset_versions: HashMap<String, u64>,
    next_asset_version: u64,
}

impl Models {
    /// Adds or replaces model `id`. An OBJ model that does not parse is kept
    /// as missing (today's importer keeps no mesh for it either); a glTF
    /// model this engine does not load is kept as unloadable. Either way the
    /// reason is returned.
    pub fn set(&mut self, id: &str, kind: ModelKind, bytes: &[u8]) -> Result<(), String> {
        let (asset, error) = match kind {
            ModelKind::Missing => (ModelAsset::Missing, None),
            ModelKind::Obj => match std::str::from_utf8(bytes)
                .map_err(|e| e.to_string())
                .and_then(pm_geom3d::obj::parse_obj)
            {
                Ok(mesh) => {
                    let g = pm_geom3d::Geometry {
                        indices: (0..mesh.vertex_count as u32).collect(),
                        positions: mesh.positions,
                        normals: mesh.normals,
                        uvs: mesh.texcoords,
                        indexed: false,
                        groups: Vec::new(),
                    };
                    (
                        ModelAsset::Mesh(
                            Arc::new(TriangleMesh::new(&g)),
                            mesh.source_vertex_count,
                            Arc::new(g),
                        ),
                        None,
                    )
                }
                Err(e) => (ModelAsset::Missing, Some(format!("OBJ model {id}: {e}"))),
            },
            ModelKind::Gltf => match pm_geom3d::gltf::load(bytes) {
                Ok(model) => (
                    ModelAsset::Gltf(Arc::new(Picker::new(Arc::new(model)))),
                    None,
                ),
                // A file the engine's reader refuses is one today's
                // importer refuses too (`gltf-acceptance.test.ts`): today
                // the asset has no scene and draws the missing-model box.
                Err(e) => {
                    let why = format!("the engine cannot load the glTF model {id}: {}", e.0);
                    (ModelAsset::Missing, Some(why))
                }
            },
            ModelKind::Unloadable => {
                let why = format!(
                    "the engine cannot load the 3D model {id}: {}",
                    String::from_utf8_lossy(bytes)
                );
                (ModelAsset::Unloadable(why.as_str().into()), Some(why))
            }
        };
        self.forget(id);
        self.assets.insert(id.to_owned(), asset);
        error.map_or(Ok(()), Err)
    }

    pub fn forget(&mut self, id: &str) {
        self.next_asset_version = self.next_asset_version.wrapping_add(1);
        self.asset_versions
            .insert(id.to_owned(), self.next_asset_version);
        if let Some(ModelAsset::Gltf(old)) = self.assets.remove(id) {
            let ptr = Arc::as_ptr(&old) as usize;
            self.poses.retain(|((p, _, _), _)| *p != ptr);
        }
    }

    pub fn clear(&mut self) {
        self.assets.clear();
        self.poses.clear();
        self.histories.clear();
        self.asset_versions.clear();
    }

    /// The ids of the models held.
    pub fn ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.assets.keys().map(String::as_str).collect();
        ids.sort_unstable();
        ids
    }
}

/// A question's geometry host: the models, the request's editor camera,
/// and the model ids it met without having them.
pub struct GeomHost<'m> {
    pub models: &'m mut Models,
    pub camera: Option<Camera>,
    pub wanted: Vec<String>,
    /// The registry's mesh renderer definitions (`meshFields`: definition
    /// id to `renderer.assetField`, as `PM.layerDefinition` answers in the
    /// host, overrides by id and third-party definitions included); `None`
    /// when the request carries no registry (then only the built-in OBJ
    /// model layer is a mesh layer).
    pub mesh_fields: Option<HashMap<String, String>>,
    invalid_scenes: Vec<String>,
    scenes: SceneCache,
}

impl<'m> GeomHost<'m> {
    pub fn new(models: &'m mut Models, request: &Value) -> GeomHost<'m> {
        GeomHost {
            models,
            camera: request_camera(request),
            wanted: Vec::new(),
            mesh_fields: request_mesh_fields(request),
            invalid_scenes: request
                .as_object()
                .and_then(|r| r.get("invalidSceneIds"))
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            scenes: SceneCache::default(),
        }
    }
}

impl LayerHost for GeomHost<'_> {
    fn geometry(&mut self) -> Option<&mut dyn Geometry3d> {
        Some(self)
    }

    fn mesh_asset_field(&self, definition: &pm_model::JsString) -> Option<String> {
        match &self.mesh_fields {
            Some(fields) => definition.as_str().and_then(|id| fields.get(id)).cloned(),
            None => (definition.as_str() == Some("powermove.3d.obj-model"))
                .then(|| "assetId".to_owned()),
        }
    }
}

/// The request's `meshFields` (`{definitionId: assetField}`), when present.
pub fn request_mesh_fields(request: &Value) -> Option<HashMap<String, String>> {
    let fields = request.as_object()?.get("meshFields")?.as_object()?;
    Some(
        fields
            .iter()
            .filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_owned())))
            .collect(),
    )
}

impl Geometry3d for GeomHost<'_> {
    fn invalid_scene(&self, id: &str) -> bool {
        self.invalid_scenes.iter().any(|invalid| invalid == id)
    }
    fn model(&mut self, id: &str) -> ModelAsset {
        match self.models.assets.get(id) {
            Some(a) => a.clone(),
            None => {
                if !self.wanted.iter().any(|w| w == id) {
                    self.wanted.push(id.to_owned());
                }
                ModelAsset::Missing
            }
        }
    }

    fn view_camera(&self) -> Option<Camera> {
        self.camera
    }

    fn source_mesh(
        &mut self,
        key: u64,
        build: &mut dyn FnMut() -> TriangleMesh,
    ) -> Arc<TriangleMesh> {
        if let Some(m) = self.models.meshes.get(&key) {
            return m.clone();
        }
        if self.models.meshes.len() >= MAX_SOURCE_MESHES {
            self.models.meshes.clear();
        }
        let m = Arc::new(build());
        self.models.meshes.insert(key, m.clone());
        m
    }

    fn posed(&mut self, picker: &Arc<Picker>, world: &Mat4, time: f64) -> Arc<Posed> {
        let key = (
            Arc::as_ptr(picker) as usize,
            world.map(f64::to_bits),
            time.to_bits(),
        );
        if let Some(i) = self.models.poses.iter().position(|(k, _)| *k == key) {
            let entry = self.models.poses.remove(i);
            let posed = entry.1.clone();
            self.models.poses.push(entry);
            return posed;
        }
        let posed = Arc::new(picker.pose(world, time));
        if self.models.poses.len() >= MAX_POSES {
            self.models.poses.remove(0);
        }
        self.models.poses.push((key, posed.clone()));
        posed
    }

    fn scenes(&mut self) -> &mut SceneCache {
        &mut self.scenes
    }

    fn model_version(&self, id: &str) -> u64 {
        self.models.asset_versions.get(id).copied().unwrap_or(0)
    }

    fn order_scene(&mut self, key: &str, versions: &[(String, u64)], scene: &mut Scene) {
        // An unanswered model request has not completed a runtime sync yet.
        if !self.wanted.is_empty() {
            return;
        }
        let history = self.models.histories.entry(key.to_owned()).or_default();
        history.retain(|(id, version)| {
            versions
                .iter()
                .any(|(now_id, now_version)| now_id == id && now_version == version)
        });
        for (id, version) in versions {
            if !history.iter().any(|(old, _)| old == id) {
                history.push((id.clone(), *version));
            }
        }
        let mut by_id: HashMap<_, _> = std::mem::take(&mut scene.objects)
            .into_iter()
            .map(|object| (object.id.clone(), object))
            .collect();
        scene.objects = history
            .iter()
            .filter_map(|(id, _)| by_id.remove(id))
            .collect();
    }
}

fn mat(v: Option<&Value>) -> Option<Mat4> {
    let a = v?.as_array()?;
    if a.len() != 16 {
        return None;
    }
    let mut m = [0.0; 16];
    for (dst, src) in m.iter_mut().zip(a) {
        *dst = src.as_f64()?;
    }
    Some(m)
}

/// The request's editor camera (`camera: {world, view?, projection,
/// projectionInverse, orthographic, fov, zoom, near, far}`): the view
/// camera exactly as the renderer holds it, and its lens for questions that
/// resize it.
pub fn request_camera(request: &Value) -> Option<Camera> {
    let c = request.as_object()?.get("camera")?.as_object()?;
    let n = |k: &str| c.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
    let lens = if matches!(c.get("orthographic"), Some(Value::Bool(true))) {
        Lens::Orthographic {
            zoom: n("zoom"),
            near: n("near"),
            far: n("far"),
        }
    } else {
        Lens::Perspective {
            fov: n("fov"),
            zoom: n("zoom"),
            near: n("near"),
            far: n("far"),
        }
    };
    let world = mat(c.get("world"))?;
    let mut camera = Camera::new(lens, world, 1.0);
    if let Some(view) = mat(c.get("view")) {
        camera.view = view;
    }
    if let (Some(p), Some(pi)) = (mat(c.get("projection")), mat(c.get("projectionInverse"))) {
        camera.projection = p;
        camera.projection_inverse = pi;
    }
    Some(camera)
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use pm_geom3d::query::{Facing, Object, Shape};

    fn scene(ids: &[&str]) -> Scene {
        Scene {
            objects: ids
                .iter()
                .map(|id| Object {
                    id: (*id).to_owned(),
                    visible: true,
                    world: pm_geom3d::gltf::math::IDENTITY,
                    shape: Shape::Mesh {
                        mesh: pm_geom3d::query::missing_box(),
                        facing: Facing::DEFAULT,
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn equal_distance_picks_follow_persistent_runtime_insertion_history() {
        let mut models = Models::default();
        let request = pm_model::json::parse("{}").unwrap();
        let camera = Camera::new(
            Lens::Perspective {
                fov: 45.0,
                zoom: 1.0,
                near: 0.01,
                far: 1000.0,
            },
            pm_geom3d::query::camera_world([0.0, 0.0, 5.0], [0.0; 3], [0.0, 1.0, 0.0]),
            1.0,
        );
        let mut host = GeomHost::new(&mut models, &request);
        let mut objects = scene(&["a", "b"]);
        host.order_scene(
            "composition:C",
            &[("a".into(), 1), ("b".into(), 1)],
            &mut objects,
        );
        assert_eq!(objects.pick(&camera, 0.0, 0.0, None), Some("a"));

        // Document reorder alone does not change Map insertion order.
        let mut objects = scene(&["b", "a"]);
        host.order_scene(
            "composition:C",
            &[("b".into(), 1), ("a".into(), 1)],
            &mut objects,
        );
        assert_eq!(objects.pick(&camera, 0.0, 0.0, None), Some("a"));

        // Replacing a's source removes and reinserts it after b.
        let mut objects = scene(&["b", "a"]);
        host.order_scene(
            "composition:C",
            &[("b".into(), 1), ("a".into(), 2)],
            &mut objects,
        );
        assert_eq!(objects.pick(&camera, 0.0, 0.0, None), Some("b"));

        // The next presented frame sees a restored source. Because the
        // previous frame synchronized version 2, restoring version 1
        // recreates a after b even though the document has its old value.
        let mut objects = scene(&["a", "b"]);
        host.order_scene(
            "composition:C",
            &[("a".into(), 1), ("b".into(), 1)],
            &mut objects,
        );
        assert_eq!(objects.pick(&camera, 0.0, 0.0, None), Some("b"));

        // Removing and then re-adding b moves it to the back.
        let mut objects = scene(&["a"]);
        host.order_scene("composition:C", &[("a".into(), 2)], &mut objects);
        let mut objects = scene(&["b", "a"]);
        host.order_scene(
            "composition:C",
            &[("b".into(), 1), ("a".into(), 2)],
            &mut objects,
        );
        assert_eq!(objects.pick(&camera, 0.0, 0.0, None), Some("a"));
    }
}
