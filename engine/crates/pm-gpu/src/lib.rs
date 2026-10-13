//! wgpu device, pools, shader compilation, interop, time slicing (section 8)
//!
//! See docs/rust-engine.md.
//!
//! This crate currently holds the bridge-out half of the interop trait
//! (contract 3): [`bridge_out`] exports IOSurface-backed viewer surfaces and
//! runs the viewer's display transform.

#[cfg(target_os = "macos")]
pub mod bridge_out;
