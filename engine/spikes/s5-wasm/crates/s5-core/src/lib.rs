//! Spike S5 (docs/rust-engine.md, Phase 0): a minimal Rust mirror of the
//! document with transform channels and speed/influence keyframes, id-keyed
//! patch reconciliation, and an evaluator whose results are bit-identical on
//! native and wasm targets (`libm` for every transcendental).
//!
//! Out of scope by design: expressions, springs, discrete (text, color,
//! boolean) values, keys saved before the speed/influence model, 3D, effects,
//! masks, nested compositions. They are reported, never approximated.

pub mod ease;
pub mod json_scan;
pub mod mirror;
pub mod model;
pub mod patch;

pub use mirror::{ApplyStats, Mirror};
