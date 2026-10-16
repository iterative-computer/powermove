//! 3D geometry: generators, glTF, BVH, depth sets (sections 7 and 8).
//!
//! Phase 2 holds what the renderer's 3D primitive needs: three.js r180's
//! primitive geometry ([`primitives`]), bit-identical to three.js's buffers.
//! [`gltf`] decodes imported models as three.js r180's GLTFLoader does, with
//! today's import checks and normalization (Phase 3b). BVH picking and recipe
//! parts follow (the S7 prototype, `engine/spikes/s7-3d/crates/s7-geom3d`).

pub mod gltf;
pub mod primitives;

pub use primitives::{Geometry, Parameters, Primitive};
