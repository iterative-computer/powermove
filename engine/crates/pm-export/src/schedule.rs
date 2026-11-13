//! Headless composition/work-area sampling at the requested export rate.
//! Shared with the job renderer; frame zero starts exactly at the authored in
//! point, independently of the composition's own rate.
use crate::Error;
use pm_time::{Rate, TICKS_PER_SECOND, round_half_even_div};

/// The legacy/native document tick boundary (exactly representable on the v1–3 wire).
pub const MAX_DOCUMENT_TICK: i64 = 9_007_199_254_740_991;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSchedule {
    pub start: i64,
    pub rate: Rate,
    pub frames: u64,
}
impl FrameSchedule {
    pub fn new(start: i64, end: i64, rate: Rate) -> Result<Self, Error> {
        if !(-MAX_DOCUMENT_TICK..=MAX_DOCUMENT_TICK).contains(&start)
            || !(-MAX_DOCUMENT_TICK..=MAX_DOCUMENT_TICK).contains(&end)
            || end <= start
        {
            return Err(Error::InvalidSpec(
                "empty or unrepresentable export range".into(),
            ));
        }
        let span = i128::from(end) - i128::from(start);
        let unit = i128::from(rate.den()) * i128::from(TICKS_PER_SECOND);
        // Authored frame counts use Math.round's positive half-up rule.
        let frames = ((2 * span * i128::from(rate.num()) + unit) / (2 * unit)).max(1);
        let out = Self {
            start,
            rate,
            frames: u64::try_from(frames)
                .map_err(|_| Error::InvalidSpec("too many export frames".into()))?,
        };
        if i128::from(start) + out.offset(out.frames) > i128::from(MAX_DOCUMENT_TICK) {
            return Err(Error::InvalidSpec(
                "export end frame exceeds document time".into(),
            ));
        }
        Ok(out)
    }
    fn offset(self, n: impl Into<i128>) -> i128 {
        round_half_even_div(
            n.into()
                .saturating_mul(i128::from(self.rate.den()))
                .saturating_mul(i128::from(TICKS_PER_SECOND)),
            i128::from(self.rate.num()),
        )
    }
    pub fn tick(self, n: u64) -> i64 {
        self.tick_of(self.offset(n))
    }
    /// The grid's tick `n` frames from the start, to either side of it.
    pub fn tick_at(self, n: i64) -> i64 {
        self.tick_of(self.offset(n))
    }
    fn tick_of(self, offset: i128) -> i64 {
        i128::from(self.start).saturating_add(offset).clamp(
            -i128::from(MAX_DOCUMENT_TICK),
            i128::from(MAX_DOCUMENT_TICK),
        ) as i64
    }
    /// The frames of `0..end` ticks on this grid, numbered from the
    /// schedule's first frame (frames before it are negative), with the
    /// schedule's own frames.
    pub fn grid(self, end: i64) -> std::ops::Range<i64> {
        let unit = i128::from(self.rate.den()) * i128::from(TICKS_PER_SECOND);
        let num = i128::from(self.rate.num());
        // The first frame at or after tick 0; the first at or after `end`.
        let first = -(i128::from(self.start) * num).div_euclid(unit);
        let last = -((i128::from(self.start) - i128::from(end)) * num).div_euclid(unit);
        let frames = i128::from(self.frames);
        (first.min(0) as i64)..(last.max(frames) as i64)
    }
    /// Absolute project sample bounds, contiguous even with a nonzero in point.
    pub fn samples(self, n: u64, sample_rate: u32) -> (i64, i64) {
        let sample = |tick: i64| {
            round_half_even_div(
                i128::from(tick) * i128::from(sample_rate),
                i128::from(TICKS_PER_SECOND),
            ) as i64
        };
        let a = sample(self.tick(n));
        let b = sample(self.tick(n.saturating_add(1)));
        (a, b.max(a))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_range_starting_on_the_grid_shares_the_whole_ranges_frames() {
        for rate in pm_time::SUPPORTED_RATES {
            let end = TICKS_PER_SECOND * 4;
            let whole = FrameSchedule::new(0, end, rate).unwrap();
            let start = whole.tick(37);
            let part = FrameSchedule::new(start, whole.tick(61), rate).unwrap();
            assert_eq!(part.frames, 24);
            // The composition's frames are the same ones, numbered from
            // either range's first (a last frame that starts inside the
            // composition counts, though the whole export rounds it away).
            let (all, grid) = (whole.grid(end), part.grid(end));
            assert_eq!(all.start, 0, "{rate:?}");
            assert!(all.end >= whole.frames as i64 && all.end <= whole.frames as i64 + 1);
            assert_eq!(grid, all.start - 37..all.end - 37, "{rate:?}");
            for n in grid {
                assert_eq!(part.tick_at(n), whole.tick_at(n + 37), "{rate:?} {n}");
                assert!((0..end).contains(&part.tick_at(n)), "{rate:?} {n}");
            }
        }
        // A range that leaves the composition keeps its own frames.
        let out =
            FrameSchedule::new(-TICKS_PER_SECOND, TICKS_PER_SECOND * 3, Rate::FPS_24).unwrap();
        assert_eq!(out.grid(TICKS_PER_SECOND * 2), 0..96);
    }
    #[test]
    fn all_rates_keep_nonzero_in_point_and_contiguous_audio() {
        for rate in pm_time::SUPPORTED_RATES {
            let start = TICKS_PER_SECOND * 3 + TICKS_PER_SECOND / 7;
            let s = FrameSchedule::new(start, start + TICKS_PER_SECOND * 2, rate).unwrap();
            assert_eq!(s.tick(0), start);
            for sr in [44100, 48000, 96000] {
                let mut next = s.samples(0, sr).0;
                let first = next;
                let mut total = 0;
                for n in 0..s.frames {
                    let (a, b) = s.samples(n, sr);
                    assert_eq!(a, next);
                    total += b - a;
                    next = b;
                }
                assert_eq!(total, next - first);
                let exact = round_half_even_div(
                    i128::from(s.tick(s.frames)) * i128::from(sr),
                    i128::from(TICKS_PER_SECOND),
                ) as i64;
                assert_eq!(next, exact);
            }
        }
    }
    #[test]
    fn half_frame_counts_and_invalid_ranges() {
        let half = TICKS_PER_SECOND / 60;
        assert_eq!(FrameSchedule::new(0, half, Rate::FPS_30).unwrap().frames, 1);
        assert_eq!(
            FrameSchedule::new(0, half * 3, Rate::FPS_30)
                .unwrap()
                .frames,
            2
        );
        assert!(FrameSchedule::new(1, 1, Rate::FPS_30).is_err());
        assert!(
            FrameSchedule::new(MAX_DOCUMENT_TICK - 1, MAX_DOCUMENT_TICK, Rate::FPS_30).is_err()
        );
    }
}
