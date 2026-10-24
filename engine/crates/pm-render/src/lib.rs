//! The renderer (plan section 8; Phase 2 tracer bullet).
//!
//! `pm-render` turns a document, a composition, and a time into pixels on
//! the GPU, for the interactive engine (viewer frames through the bridge
//! presenter) and the job engine (exports, agent captures) alike. This is
//! the minimal renderer of Phase 2: solid, image, and video layers,
//! transforms with today's sampling, opacity, normal blending, precomps,
//! groups with the compositing boundary rules, composition backgrounds, one
//! `layer3d` primitive lit by a sun, Standard color, region of interest,
//! scale, 16- and 32-bit precision, the depth and object-id surface, and a
//! conservative capability check ([`unsupported`]) for routing everything
//! else to the legacy engine. Design notes: `docs/engine/pm-render.md`.
//!
//! - [`Renderer`]: the headless API (document, composition, time, request →
//!   texture or readback).
//! - [`unsupported`], [`unsupported_with_media`]: capability routing.
//! - [`plan`]: the render graph of a frame, built through `pm-eval`.
//! - [`RenderRequest`]: contract 2's request as the graph sees it.
//! - [`effects`]: effects and transitions through the GLSL pipeline, with
//!   definition snapshots (contract 10) and the GPU watchdog.
//! - [`shader_layers`]: shader layers, extension fragment layers (Studio
//!   Cube included), and the decision 19 fallback runner.

pub mod assets;
pub mod capability;
pub mod compat;
pub mod compat_crop;
pub mod composite;
pub mod coverage;
pub mod effects;
mod binds;
mod error;
mod exec;
pub mod expr;
pub mod footage;
mod frame_prefix;
pub mod glsl;
pub mod js;
pub mod media;
pub mod motion;
pub mod parity;
mod pipelines;
pub mod plan;
mod renderer;
mod request;
pub mod retime;
pub mod reuse;
pub mod sampling;
pub mod scene3d;
pub mod shader_layers;
pub mod space25d;
pub mod stills;
pub mod text;
pub mod three;
pub mod vector;

pub use assets::{AssetResolver, MapResolver, SourcePaths};
pub use capability::{
    Feature, Unsupported, unsupported, unsupported_for, unsupported_with_definitions,
    unsupported_with_fonts, unsupported_with_media,
};
pub use effects::{Effects, builtin_definitions};
pub use error::{LayerFailure, RenderError};
pub use renderer::{
    CommandBracket, Frame, HOST_COMPILE_WAITS, LayerPixels, LayerQuad, LayerRaster, PassThrough,
    Rendered, Renderer, SnapshotRows, SvgWaiting, TextRaster,
};
pub use request::{RenderRequest, VideoEnd};
pub use shader_layers::all_builtin_definitions;

mod pass_facts;
