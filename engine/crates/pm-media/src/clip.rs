//! Which reading of a file's timestamps an asset's stored times address.
//!
//! Trims, speed changes and time-remap keys are source times. What a source
//! time selects is decided per asset, never per file: an asset made before
//! the `timing` member existed keeps the reading its edits were authored
//! against ([`ClipTiming::Legacy`]); an asset imported with the member reads
//! the file by its presentation ([`ClipTiming::Presentation`]).

use std::cmp::Ordering;

use crate::index::FrameIndex;
use crate::probe::{MediaProbe, StreamKind};

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
    /// ([`Span`]) and every stream keeps its offset from it.
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

/// A container time: `pts` units of `timebase` (num, den) seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub pts: i64,
    pub timebase: [i64; 2],
}

impl Stamp {
    fn fraction(&self) -> (i128, i128) {
        (
            self.pts as i128 * self.timebase[0] as i128,
            (self.timebase[1] as i128).max(1),
        )
    }

    fn order(&self, other: &Stamp) -> Ordering {
        let ((a, b), (c, d)) = (self.fraction(), other.fraction());
        (a * d).cmp(&(c * b))
    }

    pub fn seconds(&self) -> f64 {
        let (n, d) = self.fraction();
        n as f64 / d as f64
    }

    /// This time in units of `timebase` seconds, to the nearest unit.
    pub fn in_units(&self, timebase: pm_time::Rational) -> i64 {
        let (n, d) = self.fraction();
        let (num, den) = (n * timebase.den() as i128, d * timebase.num() as i128);
        (2 * num + den).div_euclid(2 * den) as i64
    }
}

/// A file's presentation, as [`ClipTiming::Presentation`] reads it.
///
/// Clip time 0 is the earliest presented sample of the picture and of every
/// audio stream, after edit lists; each stream keeps its offset from it (a
/// late soundtrack is silent until it starts, a late picture shows nothing
/// until its first frame). One exception: a picture whose first frame
/// starts no more than one frame duration after that is the origin itself,
/// so a clip never opens on a blank frame because its sound leads by
/// milliseconds; the sound before that frame is not part of the clip.
///
/// The origin is the file's, not a selection's: switching an audio stream
/// off never moves a clip's picture or its other streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub origin: Stamp,
    /// The latest presented end of any stream, when the file says.
    pub end: Option<Stamp>,
}

impl Span {
    pub fn of(probe: &MediaProbe) -> Span {
        let picture = probe.video.as_ref().map(|v| {
            let at = |pts| Stamp {
                pts,
                timebase: v.timebase,
            };
            (at(v.start_pts), at(v.end_pts))
        });
        let sound: Vec<(Stamp, Option<Stamp>)> = probe
            .streams
            .iter()
            .filter(|s| s.kind == StreamKind::Audio && s.audio.is_some())
            .map(|s| {
                let start = s.start_pts.unwrap_or(0);
                let at = |pts| Stamp {
                    pts,
                    timebase: s.timebase,
                };
                (
                    at(start),
                    s.duration_pts.map(|d| at(start.saturating_add(d))),
                )
            })
            .collect();
        let earliest = picture
            .iter()
            .map(|p| p.0)
            .chain(sound.iter().map(|s| s.0))
            .min_by(Stamp::order);
        let zero = Stamp {
            pts: 0,
            timebase: [1, 1],
        };
        let mut origin = earliest.unwrap_or(zero);
        if let (Some((first, _)), Some(v)) = (picture, probe.video.as_ref()) {
            // first - origin <= one frame: (a/b - c/d) <= den/num.
            let within_a_frame = v.nominal_rate.as_ref().is_some_and(|nominal| {
                let ((a, b), (c, d)) = (first.fraction(), origin.fraction());
                let (num, den) = (nominal.rate.num() as i128, nominal.rate.den() as i128);
                (a * d - c * b) * num <= den * b * d
            });
            if within_a_frame {
                origin = first;
            }
        }
        // A stream that states no end of its own (Matroska) ends no later
        // than its container does.
        let container = sound.iter().any(|s| s.1.is_none()).then(|| {
            probe.duration_us.map(|d| Stamp {
                pts: probe.start_us.unwrap_or(0).saturating_add(d),
                timebase: [1, 1_000_000],
            })
        });
        let end = picture
            .iter()
            .map(|p| p.1)
            .chain(sound.iter().filter_map(|s| s.1))
            .chain(container.flatten())
            .max_by(Stamp::order);
        Span { origin, end }
    }

    /// The clip's length in seconds, when the file says where it ends.
    pub fn duration(&self) -> Option<f64> {
        self.end
            .map(|end| (end.seconds() - self.origin.seconds()).max(0.0))
    }
}
