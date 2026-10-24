//! High-precision reference renders for the engine's quality gates
//! (docs/engine/gate-policy.md).
//!
//! Every value here is computed in f64 from the standard or the authored
//! formula it names, never from the engine's own rendering or color code,
//! so an engine bug cannot also be in its reference. Gates compare the
//! engine and today's captures against these references with [`compare`].
//!
//! | Module | What it computes |
//! |---|---|
//! | [`coverage`] | Exact area coverage of filled paths and glyph outlines, supersampled per pixel for the fill rule |
//! | [`composite`] | Porter-Duff operators and the W3C separable blend modes on premultiplied colors |
//! | [`gaussian`] | The exact separable Gaussian blur |
//! | [`stroke`] | The region a stroke covers, from its joins, caps and half-width |
//! | [`color`] | Transfer functions, H.273 matrices, ranges and primaries, ICC matrix/TRC profiles |
//! | [`compare`] | Pixel classes, error statistics, the gate verdict, geometry and preview == export checks |
#![forbid(unsafe_code)]

pub mod color;
pub mod compare;
pub mod composite;
pub mod coverage;
pub mod gaussian;
pub mod stroke;
