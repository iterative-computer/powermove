//! Studio: Powermove's product-shot renderer (docs/3d-studio-redesign.md).
//!
//! Phase 0 renders every tier with one GPU path tracer:
//! - **Live**: one sample per pixel per frame, denoised (dragging, playback).
//! - **Refine**: keeps accumulating while nothing changes (paused).
//! - **Final**: a fixed sample count per frame (export, agent frames).

pub mod bvh;
pub mod gpu;
pub mod math;
pub mod mesh;
#[cfg(feature = "oidn")]
pub mod oidn;
pub mod scene;
pub mod splats;

pub mod extract;

pub mod service;

pub mod swatch;

pub mod status;
