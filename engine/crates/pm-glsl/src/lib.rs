//! The GLSL pipeline for effects, transitions, and extension fragment
//! layers (plan section 11, "The pipeline"; amendment 9).
//!
//! Legacy GLSL ES 3.00, as the kernel wraps it (`kernel/glsl.ts`,
//! `kernel/extension-layers.ts`), becomes naga IR that wgpu runs with
//! `ShaderSource::Naga`. This is the production form of spike S3's naga
//! path (`docs/engine/spikes/S3.md`): every
//! stage, polyfill, SPIR-V rewrite, limit, and the bind check, without the
//! SPIRV-Cross comparison path. Design notes: `docs/engine/effects.md`.
//!
//! - [`translate`], [`prepare`]: the pipeline ([`pipeline`]).
//! - [`Cache`]: step 6, by source hash and back end, in memory and on disk,
//!   with translation isolated on a large-stack thread under a deadline.
//! - [`Failure`]: the stage and the diagnostics in the author's lines (step
//!   7); [`FailureStage::webgl_refuses`] separates programs WebGL refuses
//!   too (today they render nothing) from engine-only failures (the
//!   decision 19 fallback).
//! - [`reflect`]: the default uniform block and samplers, for binding
//!   uniforms with GL's rules.
//! - [`fuzz`]: mutants and resource bombs (the `pm-glsl-fuzz` target).

pub mod cache;
pub mod compile;
pub mod errors;
pub mod fuzz;
pub mod limits;
pub mod normalize;
pub mod pipeline;
pub mod reflect;
pub mod rewrite;

pub use cache::{Cache, Key};
pub use errors::Diagnostic;
pub use pipeline::{
    Failure, FailureStage, Prepared, PreparedStage, Program, StageModule, VARYINGS, VERTEX,
    prepare, translate,
};
pub use rewrite::{SAMPLER_BINDING_OFFSET, SplitBinding};

/// Descriptor sets of the two stages' resources: fragment resources in
/// set 0, the vertex stage's default uniform block in set 1.
pub use compile::{FRAGMENT_SET, VERTEX_SET};
