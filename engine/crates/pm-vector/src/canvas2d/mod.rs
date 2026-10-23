//! Chromium's accelerated Canvas2D, emulated for `compat` level 1
//! (docs/engine/vector.md).

pub mod analytic;
pub mod blink;
pub mod context;
pub mod convex;
pub mod filter;
pub mod mask;
pub mod msaa;
pub mod paint;
pub mod shape;
pub mod skpath;
pub mod stroke;
pub mod svg;
pub mod svg_text;
pub mod tess;

mod native;
pub mod text;
