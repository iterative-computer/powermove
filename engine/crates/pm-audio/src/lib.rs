//! Audio: decode, mix, playback clock, export render, meters, waveforms
//! (plan section 16; the playback clock is contract 6, section 15).
//!
//! * [`source`]: decoded program samples in blocks, decoded ahead of
//!   playback through FFmpeg (`pm_media::audio`), no length limit.
//! * [`kernel`]: band-limited interpolation for sample-rate conversion,
//!   speed, and remaps (rubato's sinc design, at least 120 dB stopband,
//!   passband to 0.45·fs).
//! * [`graph`]: which clips sound and how their level and source time
//!   follow the document (the legacy `audioLayers` and `gainAt`).
//! * [`mixer`]: voices → strips → master in f32 at `audioSampleRate`, a
//!   pure function of the sample range.
//! * [`layout`]: channel layouts and down-mix matrices.
//! * [`offline`]: export render, PCM 24-bit and 32-bit float, 16-bit with
//!   TPDF dither, a streaming WAV/RF64 writer.
//! * [`meter`], [`waveform`]: level taps and waveform data.
//! * [`clock`]: clock samples from the device's presentation times.
//! * [`transport`]: play, pause, seek, loop, step, play around, scrub.
//! * [`playback`]: the render thread, the ring, and the output device
//!   (`cpal` on CoreAudio) that drive the master clock.
//!
//! See docs/rust-engine.md section 16 and docs/engine/pm-audio.md.

pub mod clock;
pub mod device;
pub mod error;
pub mod graph;
pub mod kernel;
pub mod layout;
pub mod meter;
pub mod mixer;
pub mod offline;
pub mod playback;
pub mod ring;
pub mod source;
pub mod transport;
pub mod waveform;

pub use error::{AudioError, Result};
pub use graph::{AssetResolver, MixGraph};
pub use mixer::{MixFormat, Mixer, RenderOptions};
pub use source::SourceCache;
