//! Probe, demux, decode, index (section 13).
//!
//! The original is the truth; every source is indexed; frames reach the GPU
//! at native depth with color and chroma metadata. See docs/rust-engine.md
//! section 13 and docs/engine/media.md.
//!
//! * [`index`]: frame indexes from container tables, scanning only where
//!   none exist.
//! * [`time`]: the nominal rate and section 5 rule 7 source frame selection.
//! * [`probe`]: streams, color, timing and priming, decode cost, peaks.
//! * [`hash`]: background BLAKE3 content hashes and the sampled fingerprint.
//! * [`decode`]: VideoToolbox, FFmpeg, dav1d, and paired alpha decoders,
//!   exact random access, pooled sessions, the decode service.
//! * [`cache`]: the decoded frame cache under a lease.
//! * [`image`]: still image readers (ImageIO, the `image` crate, OpenEXR).
//! * [`audio`]: streaming audio decode to program samples (section 16).

pub mod apple;
pub mod audio;
pub mod cache;
pub mod compat_video;
pub mod dav1d;
pub mod decode;
pub mod error;
pub mod facts;
pub mod ff;
pub mod frame;
pub mod hash;
pub mod image;
pub mod index;
pub mod interlace;
pub mod interpret;
pub mod pool;
pub mod preview;
pub mod probe;
pub mod sys;
pub mod time;
pub mod vt;

pub use error::{MediaError, Result};
