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
use pm_geom3d::query::{Camera, Lens, TriangleMesh};
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
                        ModelAsset::Mesh(Arc::new(TriangleMesh::new(&g)), mesh.source_vertex_count),
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
                Err(e) => {
                    let why = format!("the engine cannot load the glTF model {id}: {}", e.0);
                    (ModelAsset::Unloadable(why.as_str().into()), Some(why))
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
        if let Some(ModelAsset::Gltf(old)) = self.assets.remove(id) {
            let ptr = Arc::as_ptr(&old) as usize;
            self.poses.retain(|((p, _, _), _)| *p != ptr);
        }
    }

    pub fn clear(&mut self) {
        self.assets.clear();
        self.poses.clear();
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
    scenes: SceneCache,
}

impl<'m> GeomHost<'m> {
    pub fn new(models: &'m mut Models, request: &Value) -> GeomHost<'m> {
        GeomHost {
            models,
            camera: request_camera(request),
            wanted: Vec::new(),
            mesh_fields: request_mesh_fields(request),
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
