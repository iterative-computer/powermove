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

pub mod assets;
pub mod capability;
mod error;
mod exec;
pub mod expr;
pub mod js;
pub mod media;
mod pipelines;
pub mod plan;
mod renderer;
mod request;
pub mod scene3d;
pub mod three;

pub use assets::{AssetResolver, MapResolver, SourcePaths};
pub use capability::{Feature, Unsupported, unsupported, unsupported_with_media};
pub use error::{LayerFailure, RenderError};
pub use renderer::{Frame, Rendered, Renderer};
pub use request::RenderRequest;
