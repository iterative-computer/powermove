//! The rendering side of pm-vector (feature `render`; design notes in
//! docs/engine/vector.md).
//!
//! - [`color`]: Canvas2D color strings.
//! - [`scene`]: coverage scenes at output resolution (`compat` level 2) and
//!   their atlas layout.
//! - [`glyphs`]: text glyphs drawn as paths, as coverage items.
//! - [`svg`]: SVG images through usvg and resvg.
//! - [`svg_depth`]: nesting safety for SVG documents.

pub mod color;
pub mod glyphs;
pub mod scene;
pub mod svg;
pub mod svg_depth;

/// A raster: premultiplied RGBA8, top-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
