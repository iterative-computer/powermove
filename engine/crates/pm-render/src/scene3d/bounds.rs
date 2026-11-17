//! Where a 3D object layer's effects find its picture, in the viewer and in
//! exports alike (effects that read layer coordinates, `u_layerUV`, ask).
//! The studio draws each object layer as a picture of the whole frame, so
//! the frame is its bounds. Without the studio, `GL.bounds` is the
//! composition's 3D scene projected through the render camera
//! (`compositionRuntime(PM, T).bounds(PM.proj.w, PM.proj.h, [L.id],
//! camera)`), with the model assets the 3D pass draws.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use pm_eval::bounds::LayerHost;
use pm_eval::parenting::Role3d;
use pm_eval::query3d::{Geometry3d, ModelAsset, SceneCache};
use pm_eval::{JsResult, Scope};
use pm_geom3d::gltf::pick::Picker;
use pm_geom3d::query::{Camera, TriangleMesh};
use pm_model::Node;

use super::layers::DocumentModels;
use super::model::{ModelLookup, ModelSource};
use crate::assets::AssetResolver;
use crate::effects::plan::Bounds3d;

/// Where a 3D object layer's effects find its picture at `time`; `None`
/// for a layer that is not a 3D object (its bounds need no 3D scene).
pub(crate) fn object_bounds<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    assets: &dyn AssetResolver,
    studio: bool,
) -> JsResult<Option<Bounds3d>> {
    if Role3d::of(layer) != Some(Role3d::Object) {
        return Ok(None);
    }
    if studio {
        return Ok(Some(Bounds3d::Frame));
    }
    let models = DocumentModels {
        root: scope.comp.root(),
        assets,
    };
    let mut host = SceneGeometry {
        models: &models,
        scenes: SceneCache::default(),
    };
    scope
        .bounds_with_measure(layer, time, &mut None, &mut host)
        .map(|b| Some(Bounds3d::Scene(b)))
}

/// The render camera's view of the composition's scene, with the
/// document's model files read as the 3D pass reads them.
struct SceneGeometry<'a> {
    models: &'a dyn ModelSource,
    scenes: SceneCache,
}

impl Geometry3d for SceneGeometry<'_> {
    fn model(&mut self, id: &str) -> ModelAsset {
        match self.models.model(id) {
            ModelLookup::Missing => ModelAsset::Missing,
            ModelLookup::Obj { key, mesh } => cached((0, key), || {
                let g = pm_geom3d::Geometry {
                    indices: (0..mesh.vertex_count as u32).collect(),
                    positions: mesh.positions.clone(),
                    normals: mesh.normals.clone(),
                    uvs: mesh.texcoords.clone(),
                    indexed: false,
                    groups: Vec::new(),
                };
                ModelAsset::Mesh(
                    Arc::new(TriangleMesh::new(&g)),
                    mesh.source_vertex_count,
                    Arc::new(g),
                )
            }),
            ModelLookup::Gltf(asset) => cached((1, asset.key), || {
                ModelAsset::Gltf(Arc::new(Picker::new(asset.model.clone())))
            }),
        }
    }

    fn view_camera(&self) -> Option<Camera> {
        None
    }

    fn scenes(&mut self) -> &mut SceneCache {
        &mut self.scenes
    }
}

impl LayerHost for SceneGeometry<'_> {
    fn geometry(&mut self) -> Option<&mut dyn Geometry3d> {
        Some(self)
    }
}

/// Query meshes and pickers per model file version, so a frame does not
/// rebuild them.
fn cached(key: (u8, u64), build: impl FnOnce() -> ModelAsset) -> ModelAsset {
    static CACHE: OnceLock<Mutex<HashMap<(u8, u64), ModelAsset>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(a) = cache.lock().unwrap().get(&key) {
        return a.clone();
    }
    let asset = build();
    let mut c = cache.lock().unwrap();
    if c.len() >= 16 {
        c.clear();
    }
    c.insert(key, asset.clone());
    asset
}
