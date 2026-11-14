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
mod binds;
pub mod capability;
pub mod compat;
pub mod compat_crop;
pub mod composite;
pub mod coverage;
mod doc_walk;
pub mod effects;
mod error;
mod exec;
pub mod expr;
pub mod footage;
mod frame_prefix;
pub mod glsl;
pub mod js;
pub mod media;
pub mod motion;
mod nearest_half;
pub mod offline_card;
pub mod parity;
mod pipelines;
pub mod plan;
mod renderer;
mod raster_roi;
mod request;
pub mod retime;
pub mod reuse;
pub mod sampling;
pub mod scene3d;
pub mod scopes;
pub mod shader_layers;
pub mod slices;
pub mod space25d;
pub mod stills;
pub mod text;
pub mod three;
mod upload;
pub mod vector;

pub use assets::{AssetResolver, MapResolver, SourcePaths};
pub use capability::{
    Feature, Unsupported, unsupported, unsupported_for, unsupported_with_definitions,
    unsupported_with_fonts, unsupported_with_media,
};
pub use effects::{Effects, builtin_definitions};
pub use error::{LayerFailure, ReadCause, RenderError, SvgFallback, UnreadFrame};
pub use renderer::{
    CommandBracket, Frame, FrameFootage, HOST_COMPILE_WAITS, LayerPixels, LayerQuad, LayerRaster,
    MediaPreparation, PassThrough, Rendered, Renderer, SnapshotRows, TextRaster,
    MaterialCaptureRequest, MaterialPixels, MaterialCaptureStats,
};
pub use request::{OutputKind, OutputTone, RenderRequest};
pub use shader_layers::all_builtin_definitions;

mod parallel;
mod pass_facts;
