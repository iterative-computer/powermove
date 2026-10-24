//! The rendering side of pm-vector (feature `render`; design notes in
//! docs/engine/vector.md).
//!
//! - [`color`]: Canvas2D color strings.
//! - [`scene`]: coverage scenes at output resolution (`compat` level 2) and
//!   their atlas layout.
//! - [`cpu`]: the `vello_cpu` coverage backend (the CPU fallback).
//! - [`svg`]: SVG images through usvg (drawn by Vello or resvg).
//! - [`svg_vello`]: SVG documents as Vello scenes (feature `gpu`).
//! - [`svg_depth`]: nesting safety for SVG documents; `svg_xml`: their XML
//!   read as Chromium reads it.

pub mod color;
pub mod cpu;
pub mod scene;
pub mod svg;
pub mod svg_depth;
mod svg_xml;
#[cfg(feature = "gpu")]
pub mod svg_vello;
