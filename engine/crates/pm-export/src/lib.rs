//! Encoders, muxers, verification (plan section 17).
//!
//! * [`settings`]: what an export produces (codec, container, tags, alpha, dither,
//!   audio, timecode, destination).
//! * [`convert`]: the encoder input on the CPU — matrix, siting-aware chroma,
//!   deterministic [`dither`], codable-range clipping — into `y416`, `v216`, `420v`.
//! * [`pipeline`]: frames and audio into a [`pipeline::Sink`]; test-only sinks capture
//!   encoder input (`test-sinks` feature).
//! * `mac` (macOS): VideoToolbox ProRes, H.264, and HEVC with session-property color
//!   tags, AVAssetWriter MOV/MP4 with section 17 timescales and a `tmcd` track, then
//!   [`verify`] and an [`atomic`] move into place.
//! * [`webm`]: VP9 and VP8 (with alpha) through libvpx and Opus through libopus,
//!   muxed by FFmpeg, verified by demuxing and decoding.
//! * [`png`]: PNG sequences and stills, 8 or 16-bit, verified losslessly.
//! * [`export`]: one entry point for every format ([`OutputSink`]).
//! * [`mp4box`]: the container reader verification uses.
//! * [`render`]: the job engine's export and agent-still pipeline: capability
//!   routing, pm-render frames (sliced through a [`render::GpuGate`]), pm-audio's
//!   offline mix, and the encoder above.

pub mod atomic;
pub mod blender;
pub mod convert;
pub mod dither;
pub mod gpu_time;
pub mod mp4box;
pub(crate) mod par;
pub mod pipeline;
pub mod png;
pub mod render;
pub mod settings;
pub mod timescale;
pub mod verify;
pub mod webm;

#[cfg(target_os = "macos")]
pub mod mac;

use std::path::PathBuf;

pub use convert::{EncoderInput, WorkingFrame};
pub use pipeline::{Pipeline, Sink};
pub use settings::*;
pub use verify::VerifyReport;

/// A delivered, verified output: a file, or for PNG sequences a folder.
#[derive(Clone, Debug)]
pub struct Delivered {
    pub path: PathBuf,
    pub report: VerifyReport,
}

/// The sink of one export, by container.
pub enum OutputSink {
    #[cfg(target_os = "macos")]
    VideoToolbox(Box<mac::VideoToolboxSink>),
    Webm(Box<webm::WebmSink>),
    Png(Box<png::PngSink>),
}

impl OutputSink {
    /// Opens the encoder, muxer, and temporary output `spec` asks for.
    pub fn new(spec: &ExportSpec) -> Result<OutputSink, Error> {
        match spec.container {
            Container::Mov | Container::Mp4 => {
                #[cfg(target_os = "macos")]
                {
                    Ok(OutputSink::VideoToolbox(Box::new(
                        mac::VideoToolboxSink::new(spec)?,
                    )))
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err(Error::InvalidSpec(
                        "MOV and MP4 exports need VideoToolbox (macOS)".into(),
                    ))
                }
            }
            Container::Webm => Ok(OutputSink::Webm(Box::new(webm::WebmSink::new(spec)?))),
            Container::PngSequence | Container::PngStill => {
                Ok(OutputSink::Png(Box::new(png::PngSink::new(spec)?)))
            }
        }
    }
}

impl Sink for OutputSink {
    type Output = Delivered;

    fn video(&mut self, index: u64, input: &EncoderInput) -> Result<(), Error> {
        match self {
            #[cfg(target_os = "macos")]
            OutputSink::VideoToolbox(s) => s.video(index, input),
            OutputSink::Webm(s) => s.video(index, input),
            OutputSink::Png(s) => s.video(index, input),
        }
    }

    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        match self {
            #[cfg(target_os = "macos")]
            OutputSink::VideoToolbox(s) => s.audio(interleaved),
            OutputSink::Webm(s) => s.audio(interleaved),
            OutputSink::Png(s) => s.audio(interleaved),
        }
    }

    fn finish(self) -> Result<Delivered, Error> {
        match self {
            #[cfg(target_os = "macos")]
            OutputSink::VideoToolbox(s) => s.finish(),
            OutputSink::Webm(s) => s.finish(),
            OutputSink::Png(s) => s.finish(),
        }
    }
}

/// An export in progress.
pub type Export = Pipeline<OutputSink>;

/// Starts an export of any format: validates, prepares the temporary output next to
/// the destination, and opens the encoder and muxer.
pub fn export(spec: ExportSpec) -> Result<Export, Error> {
    let sink = OutputSink::new(&spec)?;
    Pipeline::new(spec, sink)
}

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
