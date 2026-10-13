//! The renderer's document mirror and synchronous evaluator (docs/rust-engine.md
//! sections 6 and 7; section 26, Phase 1).
//!
//! The renderer stays the authority for the document. It sends every write as
//! canonical binary patches (contract 5, `canonicalV1`) and asks its
//! synchronous questions here: the kernel's `anim.*`, `groups.bounds`,
//! `space3d.*`, `render.gl.bounds` and `pick` for 2D layers, `textLayout`,
//! raster metrics, and the editor helpers. Answers come from `pm-eval` (with
//! `pm-expr` for expressions and `pm-text` for level 1 text), the same code the
//! engine processes run natively, so native and WebAssembly answers are
//! bit-identical (`tests/identity.rs`).
//!
//! * [`engine`]: [`Engine`], plain Rust, tested natively.
//! * [`expr`]: the production expression engine (`pm-expr` reading the
//!   mirror's nodes in place).
//! * [`reply`]: JSON replies with exact numbers.
//! * `abi` (wasm32 only): the module's C ABI exports.

pub mod engine;
pub mod expr;
pub mod reply;

#[cfg(target_family = "wasm")]
mod abi;

pub use engine::{ApplyError, Engine, Stats};
