//! Encoders, muxers, verification (plan section 17).
//!
//! * [`settings`]: what an export produces (codec, container, tags, alpha, dither,
//!   audio, timecode, destination).
//! * [`convert`]: the encoder input on the CPU — matrix, siting-aware chroma,
//!   deterministic [`dither`], codable-range clipping — into `y416`, `v216`, `420v`.
//! * [`pipeline`]: frames and audio into a [`pipeline::Sink`]; test-only sinks capture
//!   encoder input (`test-sinks` feature).
//! * `mac` (macOS): VideoToolbox ProRes and H.264 with session-property color tags,
//!   AVAssetWriter MOV/MP4 with section 17 timescales and a `tmcd` track, then
//!   [`verify`] and an [`atomic`] move into place.
//! * [`mp4box`]: the container reader verification uses.

pub mod atomic;
pub mod convert;
pub mod dither;
pub mod mp4box;
pub mod pipeline;
pub mod settings;
pub mod timescale;
pub mod verify;

#[cfg(target_os = "macos")]
pub mod mac;

use std::path::PathBuf;

pub use convert::{EncoderInput, WorkingFrame};
pub use pipeline::{Pipeline, Sink};
pub use settings::*;
pub use verify::VerifyReport;

#[cfg(target_os = "macos")]
pub use mac::{Delivered, Export, export};

/// Export errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid export settings: {0}")]
    InvalidSpec(String),
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
    #[error("{} already exists", .0.display())]
    Exists(PathBuf),
    #[error("file error: {0}")]
    Io(String),
    #[error("encoder: {0}")]
    Encode(String),
    #[error("muxer: {0}")]
    Mux(String),
    #[error("verification failed: {0}")]
    Verify(String),
}
