//! Times a kernel creates or moves (section 5, rule 4, with amendment 1).
//!
//! A kernel echoes the stored `f64` of every time it leaves alone. A time it
//! computes is done in ticks on the composition's exact grid and written with
//! [`IngestRate::seconds_of`]: `n / fps` (the document's own float grid) for a
//! frame start, `ticks / TICKS` off the grid. A document whose rate does not
//! resolve (no `fps`, a non-positive one) keeps the legacy float arithmetic.

use pm_model::IngestRate;
use pm_time::Tick;

/// The time grid of one composition.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    rate: Option<IngestRate>,
    /// `PM.proj.fps` as a number (the legacy arithmetic's divisor).
    fps: f64,
}

impl Clock {
    pub fn new(rate: Option<IngestRate>, fps: f64) -> Clock {
        Clock { rate, fps }
    }

    /// The composition's float rate as the legacy code divides by.
    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// The ticks of a stored time, when the rate resolves and the time is
    /// finite.
    pub fn ticks(&self, seconds: f64) -> Option<Tick> {
        self.rate?.time(seconds).ok().map(|t| t.ticks)
    }

    /// The float to store for `ticks`.
    pub fn seconds(&self, ticks: Tick) -> Option<f64> {
        Some(self.rate?.seconds_of(ticks))
    }

    /// The start of frame `n` (`n / PM.proj.fps` in the legacy code).
    pub fn frame(&self, n: i64) -> f64 {
        self.rate
            .and_then(|r| r.rate.frame_start(n).ok().map(|t| r.seconds_of(t)))
            .unwrap_or(n as f64 / self.fps)
    }

    /// `time - from` for a key a kernel creates at composition time `time`
    /// on a layer that starts at `from` (the layer's local time).
    pub fn local(&self, time: f64, from: f64) -> f64 {
        match (self.ticks(time), self.ticks(from)) {
            (Some(t), Some(f)) => t
                .checked_sub(f)
                .and_then(|d| self.seconds(d))
                .unwrap_or(time - from),
            _ => time - from,
        }
    }
}

/// `t + from - to` for a key moved from a layer starting at `from` (on
/// `source`'s grid) to one starting at `to` (on `target`'s grid): echoes `t`
/// when the two starts are the same instant.
pub fn shifted(t: f64, from: f64, to: f64, source: &Clock, target: &Clock) -> f64 {
    let ticks = (source.ticks(t), source.ticks(from), target.ticks(to));
    if let (Some(k), Some(f), Some(g)) = ticks {
        if f == g {
            return t;
        }
        if let Some(moved) = k.checked_add(f).and_then(|x| x.checked_sub(g))
            && let Some(s) = target.seconds(moved)
        {
            return s;
        }
    }
    t + from - to
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(fps: f64) -> Clock {
        Clock::new(IngestRate::from_document(None, Some(fps)).ok(), fps)
    }

    #[test]
    fn frames_are_the_documents_float_grid() {
        for fps in [24.0, 25.0, 29.97, 30.0, 59.94, 60.0] {
            let c = clock(fps);
            for n in [0i64, 1, 7, 299, 1800, 100_000] {
                assert_eq!(c.frame(n), n as f64 / fps, "{fps} {n}");
            }
        }
    }

    #[test]
    fn local_times_snap_to_frames() {
        let c = clock(30.0);
        // 0.3 - 0.1 is 0.2 - 1 ulp in floats; on the grid it is frame 6.
        assert_ne!(0.3 - 0.1, 0.2);
        assert_eq!(c.local(0.3, 0.1), 6.0 / 30.0);
        // Off-grid starts keep their offset in ticks.
        let off = c.local(1.0, 0.012_345);
        assert!((off - (1.0 - 0.012_345)).abs() < 1e-9);
    }

    #[test]
    fn unchanged_shift_echoes_the_time() {
        let c = clock(29.97);
        let t = 0.1 + 0.2;
        assert_eq!(shifted(t, 1.5, 1.5, &c, &c), t);
        assert_eq!(shifted(10.0 / 29.97, 2.0 / 29.97, 1.0 / 29.97, &c, &c), 11.0 / 29.97);
    }

    #[test]
    fn unresolved_rates_keep_float_arithmetic() {
        let c = clock(0.0);
        assert_eq!(c.local(1.0, 0.25), 0.75);
        assert_eq!(shifted(1.0, 0.5, 0.25, &c, &c), 1.25);
    }
}
