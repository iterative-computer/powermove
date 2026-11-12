//! Which reading of a file's timestamps an asset's stored times address.
//!
//! Trims, speed changes and time-remap keys are source times. What a source
//! time selects is decided per asset, never per file: an asset made before
//! the `timing` member existed keeps the reading its edits were authored
//! against ([`ClipTiming::Legacy`]); an asset imported with the member reads
//! the file by its presentation ([`ClipTiming::Presentation`]).

use crate::index::FrameIndex;

/// The value of an asset record's `timing` member for
/// [`ClipTiming::Presentation`].
pub const ASSET_TIMING: u64 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClipTiming {
    /// No `timing` member. Picture: source time t shows the frame whose
    /// container presentation time covers t, as the earlier app's decoder
    /// placed it (the first frame holds before its own time; a sample
    /// straddling the start of the first media edit is not shown and the
    /// next one starts at the edit's start). Sound: source time t is
    /// program sample t; a stream's start offset is not applied.
    #[default]
    Legacy,
    /// `timing: 2`. Source time 0 is the start of the file's presentation
    /// and every stream keeps its offset from it.
    Presentation,
}

impl ClipTiming {
    /// The reading an asset record's `timing` member names. Absent, or any
    /// value this build does not know, is the reading of older projects.
    pub fn of_asset(timing: Option<f64>) -> ClipTiming {
        match timing {
            Some(n) if n == ASSET_TIMING as f64 => ClipTiming::Presentation,
            _ => ClipTiming::Legacy,
        }
    }

    /// The `timing` member a new import writes.
    pub fn imported() -> u64 {
        ASSET_TIMING
    }
}

/// The legacy reading of a frame index: each presented frame's source time
/// in index units, and the first frame that can show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyMap {
    pub pts: Vec<i64>,
    /// Frames before this one are never shown (the straddled sample).
    pub head: usize,
}

impl LegacyMap {
    pub fn of(index: &FrameIndex) -> LegacyMap {
        let gap = index.edits.head_gap;
        LegacyMap {
            pts: index.frames.iter().map(|f| f.pts - gap).collect(),
            head: usize::from(gap > 0 && index.frames.len() > 1),
        }
    }
}
