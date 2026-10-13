//! wgpu device, pools, shader compilation, interop, time slicing (section 8)
//!
//! See docs/rust-engine.md.
//!
//! - [`device`]: the engine's Metal device and its features;
//! - [`target`]: working render targets (`rgba16float`, `rgba32float`) and
//!   their pool;
//! - `glsl` (feature `glsl`): the GLSL ES pipeline to naga IR (three.js
//!   programs, amendment 11);
//! - [`readback`]: texture readback for tests, captures, and the CPU
//!   encoder-input reference.
//!
//! It also holds the two halves of the interop trait (contract 3) that
//! exist so far:
//!
//! - [`decode_in`] imports decoded frames from `pm-media` into wgpu without
//!   copies and runs the section 13 normalization;
//! - [`bridge_out`] exports IOSurface-backed viewer surfaces and runs the
//!   viewer's display transform.

#[cfg(feature = "decode-in")]
pub mod decode_in;

pub mod device;
#[cfg(feature = "glsl")]
pub mod glsl;
pub mod readback;
pub mod target;

pub use device::{DeviceError, Gpu};
pub use target::{Target, TargetPool, working_format};

#[cfg(target_os = "macos")]
pub mod bridge_out;
