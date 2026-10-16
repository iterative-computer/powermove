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
#[derive(Debug, Clone, Default)]
pub struct Options3d {
    pub view: Option<View3d>,
    /// Definition snapshots of mesh layers (`renderer.kind === 'mesh'`), by
    /// definition id.
    pub mesh_definitions: Arc<std::collections::HashMap<String, legacy::MeshDefinition>>,
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
