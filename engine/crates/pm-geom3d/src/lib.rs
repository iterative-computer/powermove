//! 3D geometry: generators, imported models, OBJ meshes, depth sets and
//! ordering (plan sections 7, 8, 13 and 25).
//!
//! Every generator is three.js r180's, computed in f64 in its operation
//! order and stored as f32, so buffers equal three.js's bit for bit
//! (`tests/three_primitives.rs` checks them against three.js itself):
//! primitives ([`primitives`]), lathe and extrude ([`shapes`], with an
//! [`earcut`] port), mesh sources ([`source`]), and `computeVertexNormals`
//! ([`normals`]). [`obj`] parses Wavefront OBJ files as the kernel's
//! `parseObj` does for legacy mesh layers and, with the `import` feature,
//! packs `prepareModelImport`'s OBJ + MTL + textures into a GLB; [`layering`]
//! holds the 3D depth sets and composition ordering of plan section 8.
//! [`gltf`] loads glTF and GLB models as three.js r180's `GLTFLoader` and
//! today's importer do, animates them with the `AnimationMixer` rule, and
//! picks posed models (skinned, morphed, instanced, points, lines).
//! [`model_import`] is the formats lane's import check: `loadGltf`'s checks
//! and normalization and the object graph three.js builds, with the facts
//! the importer reports (Phase 3b). Recipe parts are another lane's.

pub mod earcut;
pub mod gltf;
#[cfg(feature = "import")]
mod js;
pub mod layering;
pub mod model_import;
pub mod normals;
pub mod obj;
pub mod primitives;
pub mod query;
pub mod scene_validation;
pub mod shapes;
pub mod source;

pub use primitives::{Geometry, Group, Parameters, Primitive};
pub use source::{MeshSource, Source};
