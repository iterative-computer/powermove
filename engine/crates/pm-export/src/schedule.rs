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
    fn offset(self, n: u64) -> i128 {
        round_half_even_div(
            i128::from(n)
                .saturating_mul(i128::from(self.rate.den()))
                .saturating_mul(i128::from(TICKS_PER_SECOND)),
            i128::from(self.rate.num()),
        )
    }
    pub fn tick(self, n: u64) -> i64 {
        i128::from(self.start).saturating_add(self.offset(n)).clamp(
            -i128::from(MAX_DOCUMENT_TICK),
            i128::from(MAX_DOCUMENT_TICK),
        ) as i64
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
