//! Ticks, rationals, frame math, and timecode (docs/rust-engine.md, section 5).
//!
//! # Units
//!
//! * A [`Tick`] is 1/705,600,000 second (the flick). 705,600,000 = 2⁹·3²·5⁵·7², so
//!   every supported frame rate ([`SUPPORTED_RATES`]) and every common sample rate
//!   from 8 kHz to 192 kHz divides it exactly. The `i64` range is about ±414 years.
//! * Rates are exact [`Rational`]s wrapped in [`Rate`] (positive, at most one frame
//!   per tick).
//!
//! # Frame math
//!
//! All frame math runs in `i128` with Euclidean division, exactly as section 5 states:
//!
//! ```text
//! frame_start(n) = ceil(n × den × TICKS / num)
//! frame_of(t)    = (t × num).div_euclid(den × TICKS)
//! ```
//!
//! `frame_of(frame_start(n)) == n` holds for every valid rate and every `n`, negative
//! frames included, and `frame_start(n)` is the first tick of frame `n`
//! (`frame_of(frame_start(n) - 1) == n - 1`).
//!
//! # Boundaries
//!
//! * Media timebases that do not divide the flick convert with round-half-even
//!   ([`ticks_from_timebase`], [`timebase_from_ticks`]).
//! * Float seconds from the document convert through [`ticks_from_seconds`]
//!   (exact nearest tick) and back through [`seconds_from_ticks`] (correctly
//!   rounded). The frame-aligned ingest rule lives in `pm-model::ingest`.
//! * Document frame rates snap through [`rate_from_document`] (section 5, boundary
//!   rule 3).
//! * Timecode, including SMPTE drop-frame at 29.97 and 59.94, is in [`timecode`].
#![forbid(unsafe_code)]

mod convert;
mod error;
mod rate;
mod rational;
mod snap;
pub mod timecode;

pub use convert::{
    seconds_from_ticks, ticks_from_seconds, ticks_from_timebase, timebase_from_ticks,
};
pub use error::TimeError;
pub use rate::{Rate, SUPPORTED_RATES, SUPPORTED_SAMPLE_RATES};
pub use rational::{Rational, round_half_even_div};
pub use snap::{
    DocumentRate, NTSC_MULTIPLES, RateSource, SNAP_INTEGER_RATES, SNAP_RELATIVE_TOLERANCE,
    rate_from_document, snap_float_rate,
};
pub use timecode::{Timecode, TimecodeFormat};

/// Engine time: 1/705,600,000 second.
pub type Tick = i64;

/// Ticks in one second (the flick).
pub const TICKS_PER_SECOND: i64 = 705_600_000;
