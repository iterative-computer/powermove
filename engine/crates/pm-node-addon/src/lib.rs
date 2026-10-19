//! The engine processes' Node-API addon (`native/pm_node.node`).
//!
//! Everything lives in [`pm_node`]; with the `node` feature its `#[napi]`
//! items register themselves (napi-rs constructors) and the `napi` crate
//! exports `napi_register_module_v1`, the one symbol Node looks up. This
//! crate only makes the cdylib, so the release profile's LTO applies to it
//! (docs/engine/release-build.md).

// Links pm-node (and with it the napi registrations) into the cdylib.
use pm_node as _;
