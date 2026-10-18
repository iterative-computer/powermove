//! 3D geometry: generators, OBJ meshes, depth sets and ordering (plan
//! sections 7 and 8).
//!
//! Every generator is three.js r180's, computed in f64 in its operation
//! order and stored as f32, so buffers equal three.js's bit for bit
//! (`tests/three_primitives.rs` checks them against three.js itself):
//! primitives ([`primitives`]), lathe and extrude ([`shapes`], with an
//! [`earcut`] port), mesh sources ([`source`]), and `computeVertexNormals`
//! ([`normals`]). [`obj`] parses Wavefront OBJ files as the kernel's
//! `parseObj` does for legacy mesh layers; [`layering`] holds the 3D depth
//! sets and composition ordering of plan section 8. glTF, BVH picking, and
//! recipe parts are the glTF lane's (the S7 prototype,
//! `engine/spikes/s7-3d/crates/s7-geom3d`).

pub mod earcut;
pub mod layering;
pub mod normals;
pub mod obj;
pub mod primitives;
pub mod shapes;
pub mod source;

pub use primitives::{Geometry, Group, Parameters, Primitive};
pub use source::{MeshSource, Source};
