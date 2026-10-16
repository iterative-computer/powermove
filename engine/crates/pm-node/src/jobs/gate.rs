//! The job engine's GPU gate (contract 12's class order, contract 13's
//! slicing, amendment 12).
//!
//! Every GPU submission of every lane (agent snapshots, agent exports, user
//! jobs) passes [`Gate::admit`]: one submission is in flight across the job
//! engine at a time, a lane never submits while a higher class waits, and
//! while the interactive engine plays, [`JobSlicer`] sizes each submission to
//! the slicing budget and aligns it to the gap after the interactive engine's
//! frame. The renderer cuts a frame into bands of rows when the gate admits
//! fewer units than a frame has (`pm_export::render::render_frame`).
//!
//! **Measurement.** The renderer reports each submission's GPU time from
//! pass timestamps (`pm_export::gpu_time::GpuSpan`), which leaves out the
//! wait behind the interactive engine's frames; on a device without
//! timestamps it reports wall time from submission to completion, which
//! includes that wait.
//!
//! **Calibration.** Per-unit costs are calibrated from whole frames
//! submitted while nothing plays and from one-unit probes of a pass first
//! met during playback (the least such sample: contention only adds time);
//! never from a pass's first submission, which carries one-time costs.
//! Sliced submissions feed the measured-over-predicted correction instead.
//!
//! **Over the 2 ms bound.** Two things keep a pass's submissions under the
//! bound while the interactive engine plays, besides the prediction:
//!
//! - A unit predicted above the bound is never submitted on that
//!   prediction. If the pass's last uncontended whole frame took more than
//!   the bound, it waits until playback stops: it is known to be expensive.
//!   Otherwise it gets one-unit probes, so a contended estimate can come
//!   down: at once, then after [`OVERSIZE_PROBE_INTERVAL`], doubling, at
//!   most [`OVERSIZE_PROBE_LIMIT`] per playback, then it waits.
//! - Banding by rows does not make every pass cheaper: a precomposition or a
//!   3D layer renders whole for any region, so its bands cost about as much
//!   as its whole frame while whole-frame calibration predicts a fraction.
//!   A pass whose GPU-timed submissions measure over the bound
//!   [`OVER_BOUND_LIMIT`] times in one playback (its first submission
//!   aside) waits until playback stops.
//!
//! So a pass submits at most `OVER_BOUND_LIMIT` measured over-bound units
//! per playback besides its cold first submission, and none once its
//! uncontended frame is known to be over the bound and its unit predicted
//! over it. Without GPU timestamps only the probe limit applies.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pm_engine::clock::{host_now, since};
use pm_engine::slicing::{Decision, JobSlicer, PassKey, PendingPass, PlanError};
use pm_export::render::GpuGate;
use pm_protocol::slicing::{DeadlineSignal, SlicingPolicy};

/// Lanes, highest class first (contract 12): agent snapshots, agent exports,
/// user jobs.
pub const LANES: usize = 3;
pub const SNAPSHOT_LANE: usize = 0;
pub const AGENT_EXPORT_LANE: usize = 1;
pub const USER_LANE: usize = 2;

/// Longest single wait before conditions are checked again (signals and
/// completions notify sooner).
const POLL: Duration = Duration::from_millis(20);
/// The first back-off between one-unit probes of a pass predicted over the
/// bound (doubling after each probe).
pub const OVERSIZE_PROBE_INTERVAL: Duration = Duration::from_millis(100);
/// One-unit probes of a pass predicted over the bound per playback.
pub const OVERSIZE_PROBE_LIMIT: u32 = 4;
/// GPU-timed submissions of a pass measured over the bound per playback
/// before the pass waits for playback to stop.
pub const OVER_BOUND_LIMIT: u32 = 3;

/// One pass in the current playback.
#[derive(Debug, Clone, Copy, Default)]
struct PlaybackPass {
    probes: u32,
    last_probe: Option<Instant>,
    /// GPU-timed submissions measured over the bound.
    over_bound: u32,
}

impl PlaybackPass {
    /// How long until the next probe may go (`None`: no more probes).
    fn probe_due(&self) -> Option<Duration> {
        if self.probes >= OVERSIZE_PROBE_LIMIT {
            return None;
        }
        let Some(last) = self.last_probe else {
            return Some(Duration::ZERO);
        };
        let interval = OVERSIZE_PROBE_INTERVAL * (1u32 << (self.probes.max(1) - 1).min(8));
        Some(interval.saturating_sub(last.elapsed()))
    }
}

#[derive(Debug, Clone, Copy)]
struct Admission {
    lane: usize,
    sliced: bool,
    calibrate: bool,
    raw_predicted_us: f64,
}

/// Why an admission waits for playback to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    KnownOverBound,
    ProbeLimit,
    MeasuredOverBound,
}

struct State {
    slicer: JobSlicer,
    policy: SlicingPolicy,
    /// Passes submitted at least once: the first submission of a pass
    /// carries one-time costs (pipeline and texture creation, first decode)
    /// and does not calibrate.
    seen: HashSet<u64>,
    playing: bool,
    waiting: [u32; LANES],
    current: Option<Admission>,
    /// Per pass: GPU time of its last uncontended whole frame (submitted
    /// while nothing played, not its first submission), in µs.
    uncontended_frame_us: HashMap<u64, f64>,
    /// Per pass, in the current playback.
    playback: HashMap<u64, PlaybackPass>,
    /// Counters for status and tests.
    stats: GateStats,
}

/// What the gate did.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateStats {
    pub submissions: u64,
    pub sliced_submissions: u64,
    pub calibrations: u64,
    /// Submissions admitted while the interactive engine played.
    pub during_playback: u64,
    /// Largest measured submission during playback, in µs.
    pub max_playback_us: f64,
    /// Submissions measured with GPU timestamps (the rest: wall time).
    pub gpu_timed: u64,
    /// GPU-timed submissions during playback measured over the bound.
    pub over_bound_during_playback: u64,
    /// Largest GPU-timed submission during playback, in µs.
    pub max_playback_gpu_us: f64,
    /// Submissions that waited for playback to stop.
    pub waited_for_stop: u64,
    /// The last corrected unit cost that was above the bound, in µs.
    pub last_unit_too_large_us: f64,
    /// Single-unit probes of passes predicted above the bound.
    pub oversize_probes: u64,
    /// Admissions that waited for playback to stop because the pass's
    /// uncontended whole frame is over the bound (never probed).
    pub known_over_bound: u64,
    /// Admissions that waited for playback to stop because the pass used its
    /// probes for this playback.
    pub probe_limit_reached: u64,
    /// Admissions that waited for playback to stop because the pass measured
    /// over the bound `OVER_BOUND_LIMIT` times in this playback.
    pub measured_over_bound: u64,
    /// Admissions released without a measurement (the render failed).
    pub abandoned: u64,
}

/// The job engine's submission gate, shared by every lane.
pub struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    pub fn new() -> Gate {
        Gate {
            state: Mutex::new(State {
                slicer: JobSlicer::new(SlicingPolicy::DEFAULT),
                policy: SlicingPolicy::DEFAULT,
                seen: HashSet::new(),
                playing: false,
                waiting: [0; LANES],
                current: None,
                uncontended_frame_us: HashMap::new(),
                playback: HashMap::new(),
                stats: GateStats::default(),
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A deadline signal (contract 13), after the engine's own slicer.
    pub fn signal(&self, signal: &DeadlineSignal) {
        let mut st = self.lock();
        st.slicer.on_signal(signal);
        match signal {
            DeadlineSignal::PlaybackStarted { .. } => {
                st.playing = true;
                st.playback.clear();
            }
            DeadlineSignal::PlaybackStopped { .. } => {
                st.playing = false;
                st.playback.clear();
            }
            _ => {}
        }
        drop(st);
        self.changed.notify_all();
    }

    pub fn policy(&self, policy: &SlicingPolicy) {
        let mut st = self.lock();
        st.slicer.set_policy(*policy);
        st.policy = *policy;
        drop(st);
        self.changed.notify_all();
    }

    pub fn is_playing(&self) -> bool {
        self.lock().playing
    }

    pub fn stats(&self) -> GateStats {
        self.lock().stats
    }

    /// Wakes every waiter (cancellation, shutdown).
    pub fn poke(&self) {
        self.changed.notify_all();
    }

    fn admit(&self, lane: usize, pass: u64, units: u32, cancel: &AtomicBool) -> Option<u32> {
        let mut st = self.lock();
        st.waiting[lane] += 1;
        let mut held: Option<Held> = None;
        let mut hold = |st: &mut State, why: Held| {
            if held.is_none() {
                match why {
                    Held::KnownOverBound => st.stats.known_over_bound += 1,
                    Held::ProbeLimit => st.stats.probe_limit_reached += 1,
                    Held::MeasuredOverBound => st.stats.measured_over_bound += 1,
                }
            }
            held = Some(why);
        };
        let grant = loop {
            if cancel.load(Ordering::Relaxed) {
                break None;
            }
            let higher_waiting = st.waiting[..lane].iter().any(|&w| w > 0);
            if st.current.is_some() || higher_waiting {
                st = self.wait(st, POLL);
                continue;
            }
            if st.playing
                && st
                    .playback
                    .get(&pass)
                    .is_some_and(|p| p.over_bound >= OVER_BOUND_LIMIT)
            {
                hold(&mut st, Held::MeasuredOverBound);
                st = self.wait(st, POLL);
                continue;
            }
            let now = host_now();
            let decision = st.slicer.decide(
                now,
                &[PendingPass {
                    pass: PassKey(pass),
                    units,
                }],
            );
            match decision {
                Ok(Decision::Unrestricted) => {
                    st.current = Some(Admission {
                        lane,
                        sliced: false,
                        calibrate: true,
                        raw_predicted_us: 0.0,
                    });
                    break Some(units);
                }
                Ok(Decision::Submit(spec)) => {
                    let n = spec.parts.first().map_or(1, |p| p.units).max(1);
                    st.slicer.submitted();
                    st.current = Some(Admission {
                        lane,
                        sliced: true,
                        calibrate: false,
                        raw_predicted_us: spec.raw_predicted_us,
                    });
                    st.stats.sliced_submissions += 1;
                    break Some(n);
                }
                Ok(Decision::WaitForFrame { until } | Decision::Yield { until }) => {
                    let ms = since(until, now).div_ceil(1000).max(1);
                    st = self.wait(st, POLL.min(Duration::from_millis(ms)));
                }
                Ok(Decision::InFlightFull) => st = self.wait(st, POLL),
                Err(PlanError::Uncalibrated(_)) => {
                    // One unit measures the pass; nothing larger is risked.
                    st.slicer.submitted();
                    st.current = Some(Admission {
                        lane,
                        sliced: true,
                        calibrate: true,
                        raw_predicted_us: f64::NAN,
                    });
                    st.stats.calibrations += 1;
                    break Some(1);
                }
                Err(PlanError::UnitTooLarge {
                    unit_us, max_us, ..
                }) => {
                    st.stats.last_unit_too_large_us = unit_us;
                    let known_over = st
                        .uncontended_frame_us
                        .get(&pass)
                        .is_some_and(|&us| us > f64::from(max_us));
                    if known_over || !st.playing {
                        hold(&mut st, Held::KnownOverBound);
                        st = self.wait(st, POLL);
                        continue;
                    }
                    let entry = st.playback.entry(pass).or_default();
                    match entry.probe_due() {
                        Some(Duration::ZERO) => {
                            // One unit re-measures the pass.
                            entry.probes += 1;
                            entry.last_probe = Some(Instant::now());
                            st.slicer.submitted();
                            st.current = Some(Admission {
                                lane,
                                sliced: true,
                                calibrate: true,
                                raw_predicted_us: f64::NAN,
                            });
                            st.stats.oversize_probes += 1;
                            break Some(1);
                        }
                        Some(due) => {
                            st = self.wait(st, POLL.min(due.max(Duration::from_millis(1))));
                        }
                        None => {
                            hold(&mut st, Held::ProbeLimit);
                            st = self.wait(st, POLL);
                        }
                    }
                }
            }
        };
        st.waiting[lane] -= 1;
        if grant.is_some() {
            st.stats.submissions += 1;
            if st.playing {
                st.stats.during_playback += 1;
            }
            if held.is_some() {
                st.stats.waited_for_stop += 1;
            }
        }
        drop(st);
        self.changed.notify_all();
        grant
    }

    fn wait<'a>(
        &self,
        st: std::sync::MutexGuard<'a, State>,
        timeout: Duration,
    ) -> std::sync::MutexGuard<'a, State> {
        self.changed
            .wait_timeout(st, timeout)
            .map(|(g, _)| g)
            .unwrap_or_else(|e| e.into_inner().0)
    }

    fn finished(&self, lane: usize, pass: u64, units: u32, measured_us: f64, gpu_timed: bool) {
        let mut st = self.lock();
        let Some(adm) = st.current.take() else {
            return;
        };
        debug_assert_eq!(adm.lane, lane);
        let measured = measured_us.max(1.0);
        let first = st.seen.insert(pass);
        if gpu_timed {
            st.stats.gpu_timed += 1;
        }
        if !adm.sliced && !first {
            st.uncontended_frame_us.insert(pass, measured);
        }
        if adm.calibrate && units > 0 && !first {
            let model = st.slicer.model_mut();
            if adm.sliced {
                // A probe while the interactive engine plays: contended.
                model.calibrate_contended(PassKey(pass), units, measured);
            } else {
                model.calibrate(PassKey(pass), units, measured);
            }
        }
        if adm.sliced {
            let raw = if adm.raw_predicted_us.is_nan() {
                measured
            } else {
                adm.raw_predicted_us
            };
            st.slicer.completed(raw, measured);
        }
        if st.playing {
            st.stats.max_playback_us = st.stats.max_playback_us.max(measured);
            if gpu_timed {
                st.stats.max_playback_gpu_us = st.stats.max_playback_gpu_us.max(measured);
                if adm.sliced && measured > f64::from(st.policy.max_command_buffer_us) {
                    st.stats.over_bound_during_playback += 1;
                    if !first {
                        st.playback.entry(pass).or_default().over_bound += 1;
                    }
                }
            }
        }
        drop(st);
        self.changed.notify_all();
    }

    /// The admitted submission failed before it could be measured: the gate
    /// is free again and nothing is calibrated from it.
    fn abandoned(&self, lane: usize) {
        let mut st = self.lock();
        let Some(adm) = st.current.take() else {
            return;
        };
        debug_assert_eq!(adm.lane, lane);
        if adm.sliced {
            st.slicer.abandoned();
        }
        st.stats.abandoned += 1;
        drop(st);
        self.changed.notify_all();
    }
}

/// One lane's view of the gate, as the renderer uses it.
#[derive(Clone)]
pub struct LaneGate {
    pub gate: Arc<Gate>,
    pub lane: usize,
}

impl GpuGate for LaneGate {
    fn admit(&self, pass: u64, units: u32, cancel: &AtomicBool) -> Option<u32> {
        self.gate.admit(self.lane, pass, units, cancel)
    }

    fn finished(&self, pass: u64, units: u32, measured_us: f64) {
        self.gate
            .finished(self.lane, pass, units, measured_us, false)
    }

    fn finished_timed(&self, pass: u64, units: u32, measured_us: f64, gpu_timed: bool) {
        self.gate
            .finished(self.lane, pass, units, measured_us, gpu_timed)
    }

    fn abandoned(&self, _pass: u64) {
        self.gate.abandoned(self.lane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{HostMicros, PlaybackRate, Seq};
    use std::time::Instant;

    fn lane(gate: &Arc<Gate>, lane: usize) -> LaneGate {
        LaneGate {
            gate: gate.clone(),
            lane,
        }
    }

    fn started() -> DeadlineSignal {
        DeadlineSignal::PlaybackStarted {
            at: host_now(),
            rate: PlaybackRate::NORMAL,
            frame_budget_us: 16_666,
        }
    }

    #[test]
    fn without_playback_whole_frames_pass_and_calibrate() {
        let gate = Arc::new(Gate::new());
        let g = lane(&gate, USER_LANE);
        let no = AtomicBool::new(false);
        // The first submission of a pass is cold and does not calibrate.
        assert_eq!(g.admit(9, 68, &no), Some(68));
        g.finished(9, 68, 6800.0);
        assert_eq!(gate.lock().slicer.model().raw_unit_us(PassKey(9)), None);
        assert_eq!(g.admit(9, 68, &no), Some(68));
        g.finished(9, 68, 680.0);
        let st = gate.lock();
        assert_eq!(st.slicer.model().raw_unit_us(PassKey(9)), Some(10.0));
        assert!(st.current.is_none());
    }

    #[test]
    fn during_playback_submissions_shrink_to_the_budget() {
        let gate = Arc::new(Gate::new());
        let g = lane(&gate, SNAPSHOT_LANE);
        let no = AtomicBool::new(false);
        // Calibrate at 100 µs a unit while idle (after the cold frame).
        for _ in 0..2 {
            assert_eq!(g.admit(1, 45, &no), Some(45));
            g.finished(1, 45, 4500.0);
        }
        gate.signal(&started());
        // No frameDone yet: reactive slicing under the 1 ms target.
        let n = g.admit(1, 45, &no).unwrap();
        assert!(
            (1..=10).contains(&n),
            "{n} units of 100 µs under a 1 ms target"
        );
        g.finished(1, n, 100.0 * f64::from(n));
        // An uncalibrated pass gets one-unit probes until one calibrates it
        // (the first is cold).
        for _ in 0..2 {
            assert_eq!(g.admit(2, 45, &no), Some(1));
            g.finished(2, 1, 120.0);
        }
        assert!(
            g.admit(2, 45, &no).unwrap() > 1,
            "calibrated at 120 µs a unit"
        );
        g.finished(2, 8, 960.0);
        let stats = gate.stats();
        assert_eq!(stats.calibrations, 2);
        assert!(stats.sliced_submissions >= 2);
        gate.signal(&DeadlineSignal::PlaybackStopped { at: host_now() });
        assert_eq!(g.admit(2, 45, &no), Some(45), "whole frames again");
        g.finished(2, 45, 45.0 * 120.0);
    }

    /// A pass first measured under contention (4 ms for one unit) would wait
    /// for playback to stop; probes once per interval bring it back.
    #[test]
    fn over_bound_passes_are_probed_during_playback_and_recover() {
        let gate = Arc::new(Gate::new());
        let g = lane(&gate, SNAPSHOT_LANE);
        let no = AtomicBool::new(false);
        gate.signal(&started());
        // Cold probe, then a contended probe that measures 4 ms for a unit.
        assert_eq!(g.admit(3, 26, &no), Some(1));
        g.finished(3, 1, 5000.0);
        assert_eq!(g.admit(3, 26, &no), Some(1));
        g.finished(3, 1, 4000.0);
        // Over the 2 ms bound: a probe at once, still contended; the next one
        // waits for the probe interval.
        assert_eq!(g.admit(3, 26, &no), Some(1));
        g.finished(3, 1, 4500.0);
        let started_at = Instant::now();
        assert_eq!(g.admit(3, 26, &no), Some(1));
        assert!(started_at.elapsed() >= OVERSIZE_PROBE_INTERVAL - Duration::from_millis(5));
        assert_eq!(gate.stats().oversize_probes, 2);
        // It measures 150 µs: the least contended sample becomes the cost.
        g.finished(3, 1, 150.0);
        assert_eq!(
            gate.lock().slicer.model().raw_unit_us(PassKey(3)),
            Some(150.0)
        );
        let n = g.admit(3, 26, &no).unwrap();
        assert!(n > 1, "sliced again: {n} units");
        g.finished(3, n, 150.0 * f64::from(n));
        // Without playback, over-bound passes are never probed: whole frames.
        gate.signal(&DeadlineSignal::PlaybackStopped { at: host_now() });
        assert_eq!(g.admit(3, 26, &no), Some(26));
        g.finished(3, 26, 26.0 * 150.0);
    }

    /// Admits on another thread; the result after `then` ran on this one.
    fn admit_while(
        g: &LaneGate,
        pass: u64,
        units: u32,
        after: Duration,
        then: impl FnOnce(),
    ) -> (Option<u32>, Duration) {
        let cancel = Arc::new(AtomicBool::new(false));
        let (g2, c2) = (g.clone(), cancel.clone());
        let started = Instant::now();
        let t = std::thread::spawn(move || {
            let r = g2.admit(pass, units, &c2);
            (r, started.elapsed())
        });
        std::thread::sleep(after);
        then();
        let out = t.join().unwrap();
        cancel.store(true, Ordering::Relaxed);
        out
    }

    /// A pass whose uncontended whole frame is over the bound is never
    /// probed while the interactive engine plays: it waits for playback to
    /// stop and then renders whole.
    #[test]
    fn passes_known_over_the_bound_are_never_probed_during_playback() {
        let gate = Arc::new(Gate::new());
        let g = lane(&gate, SNAPSHOT_LANE);
        let no = AtomicBool::new(false);
        // Idle: the cold frame, then 2 units in 6 ms (3 ms a unit).
        for _ in 0..2 {
            assert_eq!(g.admit(4, 2, &no), Some(2));
            g.finished(4, 2, 6000.0);
        }
        gate.signal(&started());
        let g2 = gate.clone();
        let (granted, waited) = admit_while(&g, 4, 2, Duration::from_millis(400), move || {
            assert_eq!(g2.stats().oversize_probes, 0, "never probed");
            g2.signal(&DeadlineSignal::PlaybackStopped { at: host_now() });
        });
        assert_eq!(granted, Some(2), "whole once playback stopped");
        assert!(waited >= Duration::from_millis(400), "{waited:?}");
        g.finished(4, 2, 6000.0);
        let stats = gate.stats();
        assert_eq!(stats.oversize_probes, 0);
        assert_eq!(stats.known_over_bound, 1);
        assert_eq!(stats.waited_for_stop, 1);
    }

    /// A pass first met during playback whose probes keep measuring over
    /// the bound is probed at most OVERSIZE_PROBE_LIMIT times, at doubling
    /// intervals, then waits for playback to stop.
    #[test]
    fn probes_of_a_pass_first_met_during_playback_back_off_and_stop() {
        let gate = Arc::new(Gate::new());
        let g = lane(&gate, SNAPSHOT_LANE);
        let no = AtomicBool::new(false);
        gate.signal(&started());
        // Cold, then the calibration probe: 4 ms for one unit.
        for us in [5000.0, 4000.0] {
            assert_eq!(g.admit(6, 26, &no), Some(1));
            g.finished(6, 1, us);
        }
        let mut gaps = vec![];
        let mut last = Instant::now();
        for _ in 0..OVERSIZE_PROBE_LIMIT {
            assert_eq!(g.admit(6, 26, &no), Some(1));
            gaps.push(last.elapsed());
            last = Instant::now();
            g.finished(6, 1, 4000.0);
        }
        assert_eq!(
            gate.stats().oversize_probes,
            u64::from(OVERSIZE_PROBE_LIMIT)
        );
        // The first goes at once; each later one waits twice as long.
        assert!(gaps[0] < Duration::from_millis(50), "{gaps:?}");
        for (i, gap) in gaps.iter().enumerate().skip(1) {
            let want = OVERSIZE_PROBE_INTERVAL * (1 << (i - 1));
            assert!(*gap + Duration::from_millis(5) >= want, "{gaps:?}");
        }
        let g2 = gate.clone();
        let (granted, _) = admit_while(&g, 6, 26, Duration::from_millis(1000), move || {
            assert_eq!(
                g2.stats().oversize_probes,
                u64::from(OVERSIZE_PROBE_LIMIT),
                "no probe past the limit"
            );
            g2.signal(&DeadlineSignal::PlaybackStopped { at: host_now() });
        });
        assert_eq!(granted, Some(26));
        // Uncontended, the whole frame takes 1.5 ms: within the bound.
        g.finished(6, 26, 1500.0);
        assert_eq!(gate.stats().probe_limit_reached, 1);
        // A new playback gives the pass its probes back (its unit is still
        // predicted over the bound, its whole frame is not).
        // (After the governor's wait for a first frame, 8 frame budgets.)
        gate.signal(&started());
        let t = Instant::now();
        assert_eq!(g.admit(6, 26, &no), Some(1));
        assert!(
            t.elapsed() < Duration::from_millis(300),
            "{:?}",
            t.elapsed()
        );
        g.finished(6, 1, 100.0);
        assert_eq!(
            gate.stats().oversize_probes,
            u64::from(OVERSIZE_PROBE_LIMIT) + 1
        );
    }

    /// Bands of a pass that renders whole for any region (a precomposition)
    /// cost far more than whole-frame calibration predicts: after
    /// OVER_BOUND_LIMIT GPU-timed submissions over the bound, the pass waits
    /// for playback to stop. Wall-time measurements never count.
    #[test]
    fn passes_measured_over_the_bound_wait_for_playback_to_stop() {
        let gate = Arc::new(Gate::new());
        let g = lane(&gate, SNAPSHOT_LANE);
        let no = AtomicBool::new(false);
        // Idle: 68 units in 1.36 ms, 20 µs a unit by whole frames.
        for _ in 0..2 {
            assert_eq!(g.admit(9, 68, &no), Some(68));
            g.finished_timed(9, 68, 1360.0, true);
        }
        gate.signal(&started());
        // Wall time over the bound (contention) does not count.
        let n = g.admit(9, 68, &no).unwrap();
        g.finished(9, n, 2600.0);
        assert_eq!(gate.stats().over_bound_during_playback, 0);
        // Every band costs 3 ms of GPU time, whatever its size.
        for _ in 0..OVER_BOUND_LIMIT {
            let n = g.admit(9, 68, &no).unwrap();
            g.finished_timed(9, n, 3000.0, true);
        }
        assert_eq!(
            gate.stats().over_bound_during_playback,
            u64::from(OVER_BOUND_LIMIT)
        );
        let g2 = gate.clone();
        let (granted, waited) = admit_while(&g, 9, 68, Duration::from_millis(300), move || {
            g2.signal(&DeadlineSignal::PlaybackStopped { at: host_now() });
        });
        assert_eq!(granted, Some(68), "whole frames once playback stopped");
        assert!(waited >= Duration::from_millis(300), "{waited:?}");
        g.finished_timed(9, 68, 1360.0, true);
        let stats = gate.stats();
        assert_eq!(stats.measured_over_bound, 1);
        assert_eq!(stats.waited_for_stop, 1);
        assert!(stats.max_playback_gpu_us >= 3000.0);
        // A new playback starts the count again.
        gate.signal(&started());
        let n = g.admit(9, 68, &no).unwrap();
        g.finished_timed(9, n, 500.0, true);
        assert_eq!(gate.stats().measured_over_bound, 1);
    }

    /// A pass whose uncontended whole frame fits the bound but whose
    /// corrected unit does not (contention raised the correction) is probed:
    /// no band of it can cost more than the whole frame.
    #[test]
    fn passes_within_the_bound_uncontended_are_probed_when_contention_inflates_them() {
        let gate = Arc::new(Gate::new());
        let g = lane(&gate, SNAPSHOT_LANE);
        let no = AtomicBool::new(false);
        for _ in 0..2 {
            assert_eq!(g.admit(8, 1, &no), Some(1));
            g.finished(8, 1, 1500.0);
        }
        gate.signal(&started());
        // 1.5 ms predicted, 3 ms measured: the correction doubles.
        assert_eq!(g.admit(8, 1, &no), Some(1));
        g.finished(8, 1, 3000.0);
        let t = Instant::now();
        assert_eq!(g.admit(8, 1, &no), Some(1));
        assert!(t.elapsed() < Duration::from_millis(50));
        g.finished(8, 1, 1400.0);
        let stats = gate.stats();
        assert_eq!(stats.oversize_probes, 1);
        assert_eq!(stats.known_over_bound, 0);
    }

    #[test]
    fn higher_lanes_go_first_and_one_submission_is_in_flight() {
        let gate = Arc::new(Gate::new());
        let user = lane(&gate, USER_LANE);
        let snap = lane(&gate, SNAPSHOT_LANE);
        let no = Arc::new(AtomicBool::new(false));
        assert_eq!(user.admit(1, 4, &no), Some(4));
        // The snapshot waits for the user's submission to complete.
        let order = Arc::new(Mutex::new(vec![]));
        let (s2, o2, n2) = (snap.clone(), order.clone(), no.clone());
        let t = std::thread::spawn(move || {
            s2.admit(2, 4, &n2).unwrap();
            o2.lock().unwrap().push("snapshot");
            s2.finished(2, 4, 10.0);
        });
        std::thread::sleep(Duration::from_millis(30));
        assert!(order.lock().unwrap().is_empty(), "one submission in flight");
        // A user lane asking again while the snapshot waits is held back.
        user.finished(1, 4, 10.0);
        let started = Instant::now();
        user.admit(1, 4, &no).unwrap();
        order.lock().unwrap().push("user");
        user.finished(1, 4, 10.0);
        t.join().unwrap();
        assert_eq!(*order.lock().unwrap(), vec!["snapshot", "user"]);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// A submission whose render fails frees the gate (and, while playing,
    /// its in-flight slot) without calibrating anything.
    #[test]
    fn an_abandoned_submission_frees_the_gate_for_every_lane() {
        let gate = Arc::new(Gate::new());
        let snap = lane(&gate, SNAPSHOT_LANE);
        let user = lane(&gate, USER_LANE);
        let no = AtomicBool::new(false);
        assert_eq!(snap.admit(1, 4, &no), Some(4));
        snap.abandoned(1);
        assert!(gate.lock().current.is_none());
        // Not counted as the pass's cold submission either.
        assert!(!gate.lock().seen.contains(&1));
        let t = Instant::now();
        assert_eq!(user.admit(2, 4, &no), Some(4));
        assert!(t.elapsed() < Duration::from_millis(100), "admitted at once");
        user.finished(2, 4, 40.0);
        // While playing, an abandoned probe gives its in-flight slot back.
        gate.signal(&started());
        assert_eq!(snap.admit(5, 26, &no), Some(1), "uncalibrated: one unit");
        assert_eq!(gate.lock().slicer.in_flight(), 1);
        snap.abandoned(5);
        assert_eq!(gate.lock().slicer.in_flight(), 0);
        assert_eq!(gate.lock().slicer.model().raw_unit_us(PassKey(5)), None);
        let t = Instant::now();
        assert_eq!(snap.admit(5, 26, &no), Some(1));
        assert!(t.elapsed() < Duration::from_millis(100), "admitted at once");
        snap.finished(5, 1, 100.0);
        assert_eq!(gate.stats().abandoned, 2);
    }

    #[test]
    fn cancel_releases_a_waiting_lane() {
        let gate = Arc::new(Gate::new());
        let a = lane(&gate, SNAPSHOT_LANE);
        let b = lane(&gate, USER_LANE);
        let no = AtomicBool::new(false);
        a.admit(1, 1, &no).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (b2, c2) = (b.clone(), cancel.clone());
        let t = std::thread::spawn(move || b2.admit(1, 1, &c2));
        std::thread::sleep(Duration::from_millis(30));
        cancel.store(true, Ordering::Relaxed);
        gate.poke();
        assert_eq!(t.join().unwrap(), None);
        a.finished(1, 1, 5.0);
    }

    #[test]
    fn aligned_slices_wait_for_the_gap_after_a_frame() {
        let gate = Arc::new(Gate::new());
        let g = lane(&gate, SNAPSHOT_LANE);
        let no = AtomicBool::new(false);
        for _ in 0..2 {
            g.admit(1, 10, &no).unwrap();
            g.finished(1, 10, 1000.0);
        }
        gate.signal(&started());
        // Two frames done: the governor aligns to the next submission gap.
        for seq in 0..2u64 {
            let now = host_now();
            gate.signal(&DeadlineSignal::FrameDone {
                at: now,
                frame_seq: Seq::new(seq + 1).unwrap(),
                tick: now,
                next_submit: HostMicros::new(now.get() + 8_000).unwrap(),
                late: false,
            });
        }
        let n = g.admit(1, 10, &no).unwrap();
        assert!(n >= 1);
        g.finished(1, n, 100.0 * f64::from(n));
        assert!(gate.stats().during_playback >= 1);
    }
}
