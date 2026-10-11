//! The canonical document model (docs/rust-engine.md, section 6; contract 5).
//!
//! * [`json`]: JSON with JavaScript semantics (f64 numbers, UTF-16 strings),
//!   `JSON.parse`-compatible parsing, and the canonical serialization.
//! * [`hash`]: the canonical document hash.
//! * [`patch`]: leaf patches with the exact semantics of `src/shared/patch.ts`.
//! * [`doc`]: the revisioned document tree with id-keyed reconciliation.
#![forbid(unsafe_code)]

pub mod doc;
pub mod hash;
pub mod json;
pub mod patch;

pub use doc::{ApplyOutcome, Document, Node, NodeKind};
pub use json::{JsString, Map, Value};
pub use patch::{Patch, PatchError, PatchErrorKind, PathPart};
