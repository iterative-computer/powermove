//! The canonical document model (docs/rust-engine.md, section 6; contract 5).
//!
//! * [`json`]: JSON with JavaScript semantics (f64 numbers, UTF-16 strings),
//!   `JSON.parse`-compatible parsing, and the canonical serialization.
//! * [`hash`]: the canonical document hash.
#![forbid(unsafe_code)]

pub mod hash;
pub mod json;
