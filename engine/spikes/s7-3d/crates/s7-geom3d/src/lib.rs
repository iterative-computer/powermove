//! `pm-geom3d` prototype (spike S7, deliverable 3): glTF decode, three.js-compatible
//! animation, CPU skinning and morphs, BVH build and refit, and picks against deformed
//! geometry. Builds natively and for wasm32-unknown-unknown (see [`wasm`] for the C ABI
//! the benchmark drives from V8).

pub mod anim;
pub mod bvh;
pub mod image;
pub mod lazy;
pub mod model;
pub mod scene;
pub mod wasm;

pub use model::{LoadOptions, Model, load};
pub use scene::{Camera, Hit, Layer, Prepared, Scene};
