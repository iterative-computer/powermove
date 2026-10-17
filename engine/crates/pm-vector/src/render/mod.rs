//! The rendering side of pm-vector (feature `render`; design notes in
//! docs/engine/vector.md).
//!
//! - [`color`]: Canvas2D color strings.
//! - [`scene`]: coverage scenes at output resolution (`compat` level 2) and
//!   their atlas layout.
//! - [`cpu`]: the `vello_cpu` coverage backend (the CPU fallback).
//! - [`svg`]: SVG images through usvg and resvg.

pub mod color;
pub mod cpu;
pub mod scene;
pub mod svg;
