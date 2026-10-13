//! wgpu device, pools, shader compilation, interop, time slicing (section 8)
//!
//! See docs/rust-engine.md.
//!
//! This crate currently holds the two halves of the interop trait
//! (contract 3) that exist so far:
//!
//! - [`decode_in`] imports decoded frames from `pm-media` into wgpu without
//!   copies and runs the section 13 normalization;
//! - [`bridge_out`] exports IOSurface-backed viewer surfaces and runs the
//!   viewer's display transform.

pub mod decode_in;

#[cfg(target_os = "macos")]
pub mod bridge_out;
