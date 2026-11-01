//! Playing-frame present schedule: when to ask for a present, and when the
//! picture is then expected on glass.
//!
//! The requested present time must never depend on the lag estimate. The
//! estimate is sampled as `presentedTime − requested`, i.e. the compositor's
//! own delay. If the request were placed at `target + estimate` (as an earlier
//! schedule did), every sample would contain the previous estimate plus the
//! compositor delay, and the estimate would climb one refresh per window
//! without bound.
use std::collections::VecDeque;

/// Samples kept per viewer: about one second of 30 fps content.
const WINDOW: usize = 31;
/// Upper bound for one sample and for the estimate, in refreshes. A
/// windowed `CAMetalLayer` reaches glass 1–3 refreshes after its latch.
const MAX_REFRESHES: f64 = 4.;
/// Prior until enough samples exist: latched on the target vsync and shown
/// one refresh later (the common case at 120 Hz).
const PRIOR_REFRESHES: f64 = 1.5;
const MIN_SAMPLES: usize = 3;

/// Sliding-window median of the compositor delay (`presentedTime −
/// requested`). Bounded per sample and in total; old samples age out, so a
/// burst of slow presents cannot ratchet it.
#[derive(Debug, Default)]
pub(crate) struct PresentLag {
    samples: VecDeque<f64>,
}
impl PresentLag {
    /// Records one shown present. Samples earlier than the request (clock
    /// noise aside) or absurdly late are discarded or clamped.
    pub fn record(&mut self, requested: f64, display: f64, interval: f64) {
        let delay = display - requested;
        if !delay.is_finite() || delay < -0.0005 {
            return;
        }
        if self.samples.len() == WINDOW {
            self.samples.pop_front();
        }
        self.samples
            .push_back(delay.clamp(0., MAX_REFRESHES * interval));
    }
    /// Expected `presentedTime − requested`.
    pub fn estimate(&self, interval: f64) -> f64 {
        if self.samples.len() < MIN_SAMPLES {
            return PRIOR_REFRESHES * interval;
        }
        let mut v: Vec<_> = self.samples.iter().copied().collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2].min(MAX_REFRESHES * interval)
    }
}

/// When to ask for a playing frame, and when it is expected on glass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Plan {
    /// `presentDrawable:atTime:` argument.
    pub requested: f64,
    /// Expected `presentedTime`; the picture and audio are chosen for it.
    pub shown: f64,
}
/// Half a refresh before the target vsync, moved to a later refresh only if
/// that is already due (eligibility would otherwise become the commit time).
/// The compositor cannot latch the frame for an earlier vsync, and a request
/// exactly on the predicted grid line no longer slips a refresh when the real
/// vsync is a hair early. Only the target, the interval and `now` enter: never
/// the estimate and never a `presentedTime` phase. (Anchoring to the last
/// `presentedTime` was tried: at 60 Hz this ProMotion panel shows a frame a
/// near-constant time after its request rather than on a fixed grid, so the
/// glass phase fed back into the next request and random-walked.)
pub(crate) fn plan(target: f64, interval: f64, lag: &PresentLag, now: f64) -> Plan {
    let mut requested = target - interval / 2.;
    if requested < now + 0.001 {
        requested += ((now + 0.001 - requested) / interval).ceil() * interval;
    }
    Plan {
        requested,
        shown: requested + lag.estimate(interval),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift; no test dependency needed.
    struct Rng(u64);
    impl Rng {
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// WindowServer stand-in: the drawable becomes eligible at
    /// `max(requested, committed)`, the compositor needs `delay + jitter` after
    /// that, and the picture reaches glass on the next real vsync. The real grid
    /// is `phase` off the one the worker predicts. `miss` adds a refresh.
    struct Compositor {
        interval: f64,
        phase: f64,
        delay: f64,
        jitter: f64,
        miss: f64,
        rng: Rng,
    }
    impl Compositor {
        fn glass(&mut self, requested: f64, committed: f64) -> f64 {
            let ready =
                requested.max(committed) + self.delay + self.jitter * self.rng.unit() - self.phase;
            let slot = (ready / self.interval).ceil() * self.interval + self.phase;
            slot + if self.rng.unit() < self.miss {
                self.interval
            } else {
                0.
            }
        }
    }

    struct Run {
        estimates: Vec<f64>,
        /// glass − shown (prediction error) per frame after warm-up.
        errors: Vec<f64>,
        /// glass − target per frame.
        latency: Vec<f64>,
        /// Frames whose glass time is beyond the 250 ms flight window.
        lost: usize,
    }

    /// 60 s of 30 fps content. Frames are encoded two refreshes ahead of their
    /// target vsync; a `presentedTime` reaches the worker one refresh after
    /// glass. `schedule` maps (target, samples so far) → (requested, shown) and
    /// returns which instant the sample is measured from.
    fn simulate(
        hz: f64,
        compositor: &mut Compositor,
        mut schedule: impl FnMut(f64, f64) -> (f64, f64),
        mut observe: impl FnMut(f64, f64, f64),
    ) -> Run {
        let interval = 1. / hz;
        let step = (hz / 30.).round() * interval;
        let mut pending = VecDeque::<(f64, f64, f64, f64)>::new();
        let mut run = Run {
            estimates: vec![],
            errors: vec![],
            latency: vec![],
            lost: 0,
        };
        let mut target = 1.;
        while target < 61. {
            let now = target - 2. * interval;
            while pending.front().is_some_and(|p| p.3 + interval <= now) {
                let (t, requested, _, glass) = pending.pop_front().unwrap();
                observe(t, requested, glass);
            }
            let (requested, shown) = schedule(target, interval);
            let glass = compositor.glass(requested, now + 0.002);
            // The worker gives up on a presentedTime after 250 ms.
            if glass - target >= 0.25 {
                run.lost += 1;
            }
            run.estimates.push(shown - target);
            if target > 3. {
                run.errors.push(glass - shown);
                run.latency.push(glass - target);
            }
            pending.push_back((target, requested, shown, glass));
            target += step;
        }
        run
    }

    fn compositor(hz: f64, delay: f64, jitter: f64, seed: u64) -> Compositor {
        Compositor {
            interval: 1. / hz,
            phase: 0.0002,
            delay,
            jitter,
            miss: 0.03,
            rng: Rng(seed),
        }
    }

    fn quantile(v: &[f64], q: f64) -> f64 {
        let mut v = v.to_vec();
        v.sort_by(f64::total_cmp);
        v[((v.len() as f64 * q).ceil() as usize).clamp(1, v.len()) - 1]
    }

    fn run_current(hz: f64, c: &mut Compositor) -> Run {
        let lag = std::cell::RefCell::new(PresentLag::default());
        let interval = 1. / hz;
        simulate(
            hz,
            c,
            |target, interval| {
                let p = plan(target, interval, &lag.borrow(), target - 2. * interval);
                (p.requested, p.shown)
            },
            |_, requested, glass| lag.borrow_mut().record(requested, glass, interval),
        )
    }

    /// Bounded, converged, nothing lost, and errors within `slack` refreshes.
    /// Returns the fraction of frames that landed exactly where predicted.
    fn check(hz: f64, run: &Run, slack: f64) -> f64 {
        let interval = 1. / hz;
        let settled = &run.estimates[run.estimates.len() / 20..];
        let max = settled.iter().copied().fold(f64::MIN, f64::max);
        let min = settled.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            max - min <= interval + 1e-9,
            "{hz} Hz: estimate wanders {min}..{max}"
        );
        assert!(
            max <= MAX_REFRESHES * interval,
            "{hz} Hz: estimate {max} above the bound"
        );
        let end = *run.estimates.last().unwrap();
        let start = run.estimates[run.estimates.len() / 10];
        assert!((end - start).abs() <= interval + 1e-9, "{hz} Hz drifts");
        assert_eq!(run.lost, 0, "{hz} Hz: frames beyond the flight window");
        assert!(quantile(&run.latency, 1.) <= (MAX_REFRESHES + 0.5) * interval);
        assert!(
            run.errors
                .iter()
                .all(|e| e.abs() <= slack * interval + 1e-6),
            "{hz} Hz: a frame landed more than {slack} refreshes from its prediction"
        );
        let exact = run
            .errors
            .iter()
            .filter(|e| e.abs() < interval / 4.)
            .count();
        exact as f64 / run.errors.len() as f64
    }

    #[test]
    fn estimate_converges_and_stays_bounded_for_60_s_at_120_and_60_hz() {
        for hz in [120., 60.] {
            for seed in [1, 7, 0x9e37_79b9] {
                // Eligibility (request + delay + jitter) well inside a refresh:
                // only the 3% misses are off.
                let clean = if hz == 120. {
                    [0.006, 0.008]
                } else {
                    [0.0015, 0.004]
                };
                for delay in clean {
                    let run = run_current(hz, &mut compositor(hz, delay, 0.002, seed));
                    let exact = check(hz, &run, 1.);
                    assert!(exact >= 0.95, "{hz} Hz/{delay}: {exact} exact");
                }
                // A compositor that itself splits frames between two slots
                // (eligibility on a vsync line) or jitters by most of a
                // refresh: no estimator makes that exact, but it must stay
                // bounded and centred on the common slot.
                // Eligibility (request + delay) right on a vsync line.
                let split = if hz == 120. { 0.004 } else { 0.008 };
                for (delay, jitter) in [(split, 0.002), (0.004, 0.75 / hz)] {
                    let run = run_current(hz, &mut compositor(hz, delay, jitter, seed));
                    // Split slot plus a miss: two refreshes.
                    let exact = check(hz, &run, 2.);
                    assert!(exact >= 0.5, "{hz} Hz/{delay}/{jitter}: {exact} exact");
                }
            }
        }
    }

    /// The pre-fix schedule (request at target + p90 lag − ½ refresh, lag
    /// sampled as glass − target) in the same simulated compositor: proves
    /// the simulator reproduces the on-screen runaway, so the test above
    /// would have caught it.
    #[test]
    fn simulator_reproduces_the_feedback_runaway_of_the_old_schedule() {
        for (hz, at_least) in [(120., 0.25), (60., 0.05)] {
            let lag = std::cell::RefCell::new(VecDeque::<f64>::new());
            let p90 = |s: &VecDeque<f64>| {
                if s.len() < 3 {
                    return 0.;
                }
                let mut v: Vec<_> = s.iter().copied().collect();
                v.sort_by(f64::total_cmp);
                v[(v.len() * 9).div_ceil(10) - 1]
            };
            let mut c = compositor(hz, 0.008, 0.002, 3);
            // Same compositor, but one that never misses: the runaway does not
            // need misses at 120 Hz.
            if hz == 120. {
                c.miss = 0.;
            }
            let run = simulate(
                hz,
                &mut c,
                |target, interval| {
                    let shown = target + p90(&lag.borrow());
                    ((shown - interval / 2.).max(target), shown)
                },
                |target, _, glass| {
                    let mut s = lag.borrow_mut();
                    if s.len() == 15 {
                        s.pop_front();
                    }
                    s.push_back(glass - target);
                },
            );
            let worst = run.estimates.iter().copied().fold(0., f64::max);
            assert!(worst >= at_least, "{hz} Hz: old schedule stayed at {worst}");
            if hz == 120. {
                assert!(run.lost > 0, "120 Hz runaway leaves the flight window");
            }
        }
    }

    #[test]
    fn a_burst_of_slow_presents_does_not_ratchet_and_ages_out() {
        let i = 1. / 120.;
        let mut lag = PresentLag::default();
        assert_eq!(lag.estimate(i), PRIOR_REFRESHES * i);
        for _ in 0..WINDOW {
            lag.record(0., 1.5 * i, i);
        }
        assert_eq!(lag.estimate(i), 1.5 * i);
        // 40% slow: the median stays on the common slot.
        for _ in 0..12 {
            lag.record(0., 2.5 * i, i);
        }
        assert_eq!(lag.estimate(i), 1.5 * i);
        // A sustained shift moves it; a return moves it back.
        for _ in 0..WINDOW {
            lag.record(0., 2.5 * i, i);
        }
        assert_eq!(lag.estimate(i), 2.5 * i);
        for _ in 0..WINDOW {
            lag.record(0., 1.5 * i, i);
        }
        assert_eq!(lag.estimate(i), 1.5 * i);
        // Absurd samples are clamped; early ones are dropped.
        for _ in 0..WINDOW {
            lag.record(0., 10., i);
            lag.record(1., 0.5, i);
        }
        assert_eq!(lag.estimate(i), MAX_REFRESHES * i);
    }

    #[test]
    fn request_never_depends_on_the_estimate() {
        let i = 1. / 120.;
        let mut lag = PresentLag::default();
        let a = plan(10., i, &lag, 9.98);
        // A slow compositor raises the estimate (and the predicted glass
        // time) but never the request.
        for _ in 0..WINDOW {
            lag.record(9.9 - i / 2., 9.9 + 3. * i, i);
        }
        let b = plan(10., i, &lag, 9.98);
        assert_eq!(a.requested, b.requested);
        assert!(b.shown > a.shown);
    }

    #[test]
    fn request_is_half_a_refresh_before_the_target_and_in_the_future() {
        for hz in [120., 60.] {
            let i = 1. / hz;
            let p = plan(10., i, &PresentLag::default(), 10. - 2. * i);
            assert!((p.requested - (10. - i / 2.)).abs() < 1e-9);
            // A request that would already be due moves to a later refresh.
            let late = plan(10., i, &PresentLag::default(), 10.);
            assert!(late.requested >= 10.001 && late.requested < 10.001 + i);
            let slots = (late.requested - p.requested) / i;
            assert!(
                (slots - slots.round()).abs() < 1e-6,
                "stays on the target grid"
            );
        }
    }
}
