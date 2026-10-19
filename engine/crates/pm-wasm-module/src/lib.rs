//! The pm-wasm WebAssembly module (`pm_wasm.wasm`).
//!
//! The module's C ABI exports (`pm-wasm`'s `abi`, wasm32 only) and its
//! allocator live in [`pm_wasm`]; a cdylib exports its dependencies'
//! `#[unsafe(no_mangle)]` functions. This crate only makes the cdylib, so the
//! release profile's LTO applies to it (docs/engine/release-build.md).

// Links pm-wasm (and with it the exports) into the cdylib.
use pm_wasm as _;
