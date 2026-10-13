//! Transfer functions, chroma, alpha, dither, and output tags (plan section 9).
//!
//! `pm-color` is the engine's color library. It builds on the normative f64
//! reference functions of contract 4 (`pm_protocol::color`) and adds what the
//! render graph, the source conversion, and the encoder input need:
//!
//! | Module | Section 9 topic |
//! |---|---|
//! | [`transfer`] | The display encoding E (sRGB piecewise, extended to all reals), E⁻¹, the Interpret alternatives, code values |
//! | [`standard`] | The Standard pipeline: hex colors, pass-through code values, and the Standard color invariants as checkable functions |
//! | [`ycbcr`] | Y′CbCr matrices, quantization, the codable range, clipping only at output |
//! | [`chroma`] | Chroma siting, siting-aware upsampling (exact at co-sited samples), output decimation, pass-through eligibility |
//! | [`alpha`] | Premultiplication rules for sources and outputs |
//! | [`linear`] | Linear-light helpers for linear blending (Phase 3a): input and output transforms and the HDR-safe blend formulas |
//! | [`dither`] | The deterministic blue-noise RPDF dither of section 17 |
//! | [`cicp`] | Output tags (amendment 7) and reading H.273 code points |
//! | [`key`] | Color components of render cache keys (contract 4 `ColorKey`) and the precision default rule (amendment 8) |
//! | [`wgsl`] | WGSL source for E, E⁻¹, and luma, shared by every shader so the GPU and these functions agree |
//!
//! Everything here is pure, deterministic, and free of platform code: the
//! same inputs give the same bits on every target (section 2, determinism).

pub mod alpha;
pub mod chroma;
pub mod cicp;
pub mod dither;
pub mod key;
pub mod linear;
pub mod standard;
pub mod transfer;
pub mod wgsl;
pub mod ycbcr;

pub use pm_protocol::color as contract;
