//! Font registry, subsetting, shaping, and layout (plan section 12, with the
//! section 26 amendment 10).
//!
//! - `compat`: level 1 layout, today's Canvas2D text layout reproduced
//!   exactly (the Rust port of Blink's `measureText` over HarfRust in
//!   `blink`, the `text-layout.ts` wrapping, raster.ts baselines and carets).
//! - `level2` (feature `level2`): layout on Parley with grapheme-aware
//!   breaking and font metrics.
//! - `registry` (feature `system`, macOS): the native font registry
//!   (CoreText matching as Blink does it, project fonts, enumeration, the
//!   fallback manifest).
//! - `answers`: the registry boundary as data, so `pm-wasm` lays text out
//!   from recorded answers and metrics-and-shaping subsets (`subset`).
//! - `corpus`: the S4 corpus and its comparison with Canvas2D captures.
//! - `render` (feature `render`): glyph rendering with the engine-owned
//!   stem-darkening profiles, the text scene, and the CPU coverage and paint
//!   paths (`docs/engine/text-render.md`).
//!
//! Layout never calls a platform API; native and WASM builds run the same
//! code over the same answers and produce bit-identical results.

pub mod answers;
pub mod blink;
pub mod compat;
pub mod corpus;
pub mod fallback_cache;
pub mod font;
#[cfg(feature = "level2")]
pub mod level2;
pub mod registry;
#[cfg(feature = "render")]
pub mod render;
pub mod sanitize;
pub mod sfnt;
pub mod subset;
