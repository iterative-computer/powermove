//! napi-rs addon loaded in Electron main (plan section 4, `pm-bridge`).
//!
//! - Frame bridge (section 18): rendezvous and surface import, Phase 2.
//! - Font registry (section 12): before Phase 2 the native font registry runs
//!   in main through this addon (`fonts`), answering `pm-wasm`'s registry
//!   queries and serving metrics-and-shaping subsets. From Phase 2 the
//!   engine owns it and main only forwards.
//!
//! See docs/rust-engine.md.

#[cfg(target_os = "macos")]
mod fonts;
