//! The 3D scene pass (plan section 8, "3D layers"; sections 9 and 10;
//! amendment 11): the native port of today's three.js r180 rendering of 3D
//! object layers (`compositor.ts`, `core/scene3d/runtime.ts` and
//! `service.ts`), legacy `scene3d` layers, and legacy OBJ mesh layers.
//!
//! - [`model`]: the evaluated scene (`SceneRuntime.sync` and
//!   `compositionRuntime`): objects, meshes, `MeshStandardMaterial`s with
//!   maps and placement, lights, cameras, environment, editor looks.
//! - [`layers`]: the scene of a composition pass, depth sets and ordering
//!   (`compositionDepthIds`, `compositionOrderedLayers`), the per-layer
//!   plans, and the static capability check.
//! - [`describe`]: the passes and draws three.js issues for a layer.
//! - [`lights`]: `WebGLLights` and the light shadows.
//! - [`gpu`]: executes the draws (MSAA, shared prepass, shadow maps).
//! - [`textures`]: material maps, shadow-map samplers, LTC tables.
//! - [`legacy`]: legacy `scene3d` and OBJ mesh layers.
//!
//! A 3D layer's output is premultiplied, E-encoded RGBA8 (the WebGL
//! canvas), drawn into the 2D graph per layer so masks, effects, opacity,
//! and blending stay per layer; with linear blending on it is converted
//! by [`linear`] before compositing (section 9).

pub mod describe;
pub mod gpu;
pub mod layers;
pub mod legacy;
pub mod lights;
pub mod linear;
pub mod model;
pub mod textures;

use std::sync::Arc;

pub use gpu::{MathMode, Rendered3d, SAMPLES, Scene3dRenderer};
pub use model::{
    Camera3d, Light3d, LightKind, Look, Material3d, MaterialKind, Materials, Mesh3d, Object3d,
    Projection, Scene3d, Texture3d,
};

pub(crate) use layers::{Pass3d, check, order_pass, plan_layer};
pub use layers::{View3d, scene_from_json};

/// Planning options of the 3D pass: the view override (editor camera,
/// looks, Local View) and the mesh layer definitions known to the host.
#[derive(Debug, Clone)]
pub struct Options3d {
    pub view: Option<View3d>,
    /// Definition snapshots of mesh layers (`renderer.kind === 'mesh'`), by
    /// definition id. The built-in OBJ model layer is known by default.
    pub mesh_definitions: Arc<std::collections::HashMap<String, legacy::MeshDefinition>>,
}

impl Default for Options3d {
    fn default() -> Self {
        Options3d {
            view: None,
            mesh_definitions: Arc::new(legacy::builtin_mesh_definitions()),
        }
    }
}

/// What a 3D render draws.
#[derive(Debug, Clone, PartialEq)]
pub enum Render3d {
    /// One 3D object layer of a composition pass: the shared prepass of its
    /// depth set, then its own color.
    Layer {
        /// `scene.objects` index of the layer's object.
        object: usize,
        /// `scene.objects` indices of its depth set.
        depth_set: Vec<usize>,
        /// The bottom layer of the set draws the scene background.
        background: bool,
    },
    /// A legacy `scene3d` layer: the whole scene with its own background.
    Whole,
    /// A legacy OBJ mesh layer (fixed shading).
    ObjMesh(legacy::ObjMeshPlan),
}

/// One 3D render in the frame's plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Object3dPlan {
    pub layer: String,
    pub scene: Arc<Scene3d>,
    pub kind: Render3d,
}

impl Object3dPlan {
    /// The content key of the render (section 8, "Content keys"): every
    /// quality-affecting input of the scene and of what this render draws
    /// (geometry by source hash, materials, maps by file and its size and
    /// modification time, lights, camera, environment, look, canvas size),
    /// and the 3D pass's translation revision. Equal keys render equal
    /// pixels on one device and math mode.
    pub fn content_key(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"pm-render scene3d content 1");
        h.update(format!("{:?}", self.kind).as_bytes());
        h.update(format!("{:?}", self.scene).as_bytes());
        // Map images are files: their version is part of the content.
        for o in &self.scene.objects {
            for m in &o.meshes {
                let list: Vec<&Material3d> = match &m.materials {
                    Materials::Single(x) => vec![x.as_ref()],
                    Materials::Slots(v) => v.iter().collect(),
                };
                for mat in list {
                    for (_, t) in mat.maps.all() {
                        if let Some(t) = t
                            && let Ok(meta) = std::fs::metadata(&t.path)
                        {
                            h.update(meta.len().to_le_bytes());
                            if let Ok(time) = meta.modified()
                                && let Ok(d) = time.duration_since(std::time::UNIX_EPOCH)
                            {
                                h.update(d.as_nanos().to_le_bytes());
                            }
                        }
                    }
                }
            }
        }
        h.finalize().into()
    }
}
