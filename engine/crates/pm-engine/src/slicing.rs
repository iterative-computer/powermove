//! GPU time slicing in the job engine (contract 13, amendment 12).
//!
//! wgpu has no preemption, so the job engine bounds each command buffer's
//! GPU time before it submits it: work is described as passes of uniform
//! units (rows or tiles of a band), each pass has a per-unit cost calibrated
//! from GPU timestamps, and a correction factor taken from the 90th
//! percentile of measured over predicted command-buffer time keeps the
//! prediction honest when other GPU clients slow everything down (S8: a 90%
//! fill of 2 ms overshot under load, a 1 ms target did not). [`plan_slice`]
//! packs units into one command buffer under the target and the alignment
//! deadline; [`JobSlicer`] combines the governor, the cost model, and the
//! in-flight limit into one decision per submission.

use std::collections::{HashMap, VecDeque};

use pm_protocol::common::HostMicros;
use pm_protocol::slicing::{DeadlineSignal, SliceBudget, SlicingGovernor, SlicingPolicy};

use crate::clock;

/// Identifies a pass (a pipeline at a size) whose units cost the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PassKey(pub u64);

/// Measured-over-predicted ratios kept for the correction factor.
pub const RATIO_WINDOW: usize = 64;
/// Ratios needed before the percentile replaces the running maximum.
pub const RATIO_MIN_SAMPLES: usize = 8;
/// Which percentile of the ratios corrects predictions.
pub const CORRECTION_PERCENTILE: f64 = 0.9;
/// Bounds on the correction factor, so one wild sample cannot stall or
/// unleash the job engine.
pub const CORRECTION_MIN: f64 = 0.25;
pub const CORRECTION_MAX: f64 = 16.0;
/// Weight of a new calibration sample in a pass's per-unit cost.
pub const CALIBRATION_WEIGHT: f64 = 0.25;

/// Per-pass unit costs and the correction factor.
#[derive(Debug, Clone)]
pub struct CostModel {
    per_unit_us: HashMap<PassKey, f64>,
    ratios: VecDeque<f64>,
    correction: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        Self::new()
    }
}

impl CostModel {
    pub fn new() -> Self {
        Self {
            per_unit_us: HashMap::new(),
            ratios: VecDeque::with_capacity(RATIO_WINDOW),
            correction: 1.0,
        }
    }

    /// Records a GPU-timestamped run of `units` units of `pass`.
    pub fn calibrate(&mut self, pass: PassKey, units: u32, measured_us: f64) {
        if units == 0 || !(measured_us.is_finite() && measured_us > 0.0) {
            return;
        }
        let sample = measured_us / f64::from(units);
        self.per_unit_us
            .entry(pass)
            .and_modify(|cost| *cost += CALIBRATION_WEIGHT * (sample - *cost))
            .or_insert(sample);
    }

    pub fn is_calibrated(&self, pass: PassKey) -> bool {
        self.per_unit_us.contains_key(&pass)
    }

    /// Uncorrected cost of one unit.
    pub fn raw_unit_us(&self, pass: PassKey) -> Option<f64> {
        self.per_unit_us.get(&pass).copied()
    }

    /// Corrected cost of one unit.
    pub fn unit_us(&self, pass: PassKey) -> Option<f64> {
        self.raw_unit_us(pass).map(|cost| cost * self.correction)
    }

    pub fn correction(&self) -> f64 {
        self.correction
    }

    /// Feeds back a submitted command buffer: its uncorrected prediction and
    /// the GPU time it actually took.
    pub fn observe(&mut self, raw_predicted_us: f64, measured_us: f64) {
        if !(raw_predicted_us.is_finite()
            && raw_predicted_us > 0.0
            && measured_us.is_finite()
            && measured_us >= 0.0)
        {
            return;
        }
        if self.ratios.len() == RATIO_WINDOW {
            self.ratios.pop_front();
        }
        self.ratios.push_back(measured_us / raw_predicted_us);
        let mut sorted: Vec<f64> = self.ratios.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let factor = if sorted.len() < RATIO_MIN_SAMPLES {
            // Too few samples for a percentile: be pessimistic.
            sorted.last().copied().unwrap_or(1.0).max(1.0)
        } else {
            let rank = ((sorted.len() as f64) * CORRECTION_PERCENTILE).ceil() as usize;
            sorted[rank.clamp(1, sorted.len()) - 1]
        };
        self.correction = factor.clamp(CORRECTION_MIN, CORRECTION_MAX);
    }
}

/// The units left in one pass. Passes are dependent: a pass starts only
/// after every unit of the previous one is submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingPass {
    pub pass: PassKey,
    pub units: u32,
}

/// Units of one pass in a planned command buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlicePart {
    /// Index into the pending passes.
    pub index: usize,
    pub units: u32,
}

/// One planned command buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceSpec {
    pub parts: Vec<SlicePart>,
    /// Corrected prediction.
    pub predicted_us: f64,
    /// Uncorrected prediction (feed it back with [`CostModel::observe`]).
    pub raw_predicted_us: f64,
}

/// Work the planner cannot slice.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("pass {0:?} has no calibrated cost; calibrate it before playback")]
    Uncalibrated(PassKey),
    /// A single unit is predicted above the hard bound: band the pass finer.
    #[error(
        "one unit of pass {pass:?} is predicted at {unit_us:.0} µs, above the {max_us} µs bound"
    )]
    UnitTooLarge {
        pass: PassKey,
        unit_us: f64,
        max_us: u32,
    },
}

/// Packs units into one command buffer predicted to at most `target_us`.
/// When not even one unit fits the target, one unit is taken if it fits
/// `hard_us` (the 2 ms bound, or less inside an alignment window). Returns
/// `None` when nothing fits the window (wait for the next gap).
pub fn plan_slice(
    model: &CostModel,
    passes: &[PendingPass],
    target_us: f64,
    hard_us: f64,
    max_us: u32,
) -> Result<Option<SliceSpec>, PlanError> {
    let mut parts = Vec::new();
    let mut predicted = 0.0;
    let mut raw = 0.0;
    for (index, pending) in passes.iter().enumerate() {
        if pending.units == 0 {
            continue;
        }
        let raw_unit = model
            .raw_unit_us(pending.pass)
            .ok_or(PlanError::Uncalibrated(pending.pass))?;
        let unit = raw_unit * model.correction();
        if unit > f64::from(max_us) {
            return Err(PlanError::UnitTooLarge {
                pass: pending.pass,
                unit_us: unit,
                max_us,
            });
        }
        let room = (target_us - predicted).max(0.0);
        let mut take = ((room / unit).floor() as u64).min(u64::from(pending.units)) as u32;
        if take == 0 && parts.is_empty() && unit <= hard_us {
            take = 1;
        }
        if take == 0 {
            break;
        }
        parts.push(SlicePart { index, units: take });
        predicted += unit * f64::from(take);
        raw += raw_unit * f64::from(take);
        if take < pending.units {
            break;
        }
    }
    Ok((!parts.is_empty()).then_some(SliceSpec {
        parts,
        predicted_us: predicted,
        raw_predicted_us: raw,
    }))
}

/// What the job engine does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// No playback: submit whole frames.
    Unrestricted,
    /// Submit this command buffer.
    Submit(SliceSpec),
    /// Wait for the interactive engine's next `frameDone`, or until this
    /// instant at the latest.
    WaitForFrame { until: HostMicros },
    /// Yield until this instant (after a miss).
    Yield { until: HostMicros },
    /// The in-flight limit is reached; wait for a completion.
    InFlightFull,
}

/// The job engine's submission gate: governor, cost model, in-flight limit.
#[derive(Debug, Clone)]
pub struct JobSlicer {
    governor: SlicingGovernor,
    model: CostModel,
    in_flight: u8,
}

impl JobSlicer {
    pub fn new(policy: SlicingPolicy) -> Self {
        Self {
            governor: SlicingGovernor::new(policy),
            model: CostModel::new(),
            in_flight: 0,
        }
    }

    pub fn governor(&self) -> &SlicingGovernor {
        &self.governor
    }

    pub fn model(&self) -> &CostModel {
        &self.model
    }

    pub fn model_mut(&mut self) -> &mut CostModel {
        &mut self.model
    }

    pub fn in_flight(&self) -> u8 {
        self.in_flight
    }

    pub fn set_policy(&mut self, policy: SlicingPolicy) {
        self.governor.set_policy(policy);
    }

    pub fn on_signal(&mut self, signal: &DeadlineSignal) {
        self.governor.on_signal(signal);
    }

    pub fn decide(&self, now: HostMicros, passes: &[PendingPass]) -> Result<Decision, PlanError> {
        match self.governor.budget(now) {
            SliceBudget::Unlimited => Ok(Decision::Unrestricted),
            SliceBudget::YieldUntil(until) => Ok(Decision::Yield { until }),
            SliceBudget::WaitForFrame { until } => Ok(Decision::WaitForFrame { until }),
            SliceBudget::Sliced {
                max_us,
                target_us,
                max_in_flight,
                deadline,
            } => {
                if self.in_flight >= max_in_flight {
                    return Ok(Decision::InFlightFull);
                }
                let window = deadline.map_or(f64::INFINITY, |d| clock::since(d, now) as f64);
                let hard = f64::from(max_us).min(window);
                let target = f64::from(target_us).min(window);
                match plan_slice(&self.model, passes, target, hard, max_us)? {
                    Some(spec) => Ok(Decision::Submit(spec)),
                    // Nothing fits before the interactive engine's next submission.
                    None => Ok(Decision::WaitForFrame {
                        until: deadline.unwrap_or(now),
                    }),
                }
            }
        }
    }

    /// A command buffer was submitted.
    pub fn submitted(&mut self) {
        self.in_flight = self.in_flight.saturating_add(1);
    }

    /// A command buffer completed after `measured_us` of GPU time.
    pub fn completed(&mut self, raw_predicted_us: f64, measured_us: f64) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.model.observe(raw_predicted_us, measured_us);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{PlaybackRate, Seq};

    const BLUR: PassKey = PassKey(1);
    const SOURCE: PassKey = PassKey(2);

    fn at(us: u64) -> HostMicros {
        HostMicros::new(us).unwrap()
    }

    fn model() -> CostModel {
        let mut model = CostModel::new();
        model.calibrate(SOURCE, 100, 500.0); // 5 µs per unit
        model.calibrate(BLUR, 10, 400.0); // 40 µs per unit
        model
    }

    #[test]
    fn calibration_blends_samples() {
        let mut model = CostModel::new();
        model.calibrate(BLUR, 10, 400.0);
        model.calibrate(BLUR, 10, 800.0);
        assert_eq!(model.raw_unit_us(BLUR), Some(50.0));
        model.calibrate(BLUR, 0, 1.0);
        model.calibrate(BLUR, 1, f64::NAN);
        assert_eq!(model.raw_unit_us(BLUR), Some(50.0));
    }

    #[test]
    fn correction_follows_the_90th_percentile_of_measured_over_predicted() {
        let mut model = CostModel::new();
        model.observe(1000.0, 1500.0);
        assert_eq!(model.correction(), 1.5, "pessimistic before 8 samples");
        let mut model = CostModel::new();
        for i in 0..20 {
            // 18 samples at 1.0, 2 at 2.0: p90 is 1.0... then a third outlier tips it.
            model.observe(1000.0, if i < 18 { 1000.0 } else { 2000.0 });
        }
        assert_eq!(model.correction(), 1.0);
        model.observe(1000.0, 2000.0);
        assert_eq!(model.correction(), 2.0);
        for _ in 0..RATIO_WINDOW {
            model.observe(1000.0, 100.0);
        }
        assert_eq!(model.correction(), CORRECTION_MIN);
    }

    #[test]
    fn plans_fill_the_target_in_pass_order() {
        let model = model();
        let passes = [
            PendingPass {
                pass: SOURCE,
                units: 100,
            },
            PendingPass {
                pass: BLUR,
                units: 100,
            },
        ];
        let spec = plan_slice(&model, &passes, 1000.0, 2000.0, 2000)
            .unwrap()
            .unwrap();
        // 100 source units (500 µs) then 12 blur units (480 µs).
        assert_eq!(
            spec.parts,
            vec![
                SlicePart {
                    index: 0,
                    units: 100
                },
                SlicePart {
                    index: 1,
                    units: 12
                }
            ]
        );
        assert!((spec.predicted_us - 980.0).abs() < 1e-9);
    }

    #[test]
    fn a_unit_above_the_target_runs_alone_and_above_the_bound_is_refused() {
        let mut model = CostModel::new();
        model.calibrate(BLUR, 1, 1500.0);
        let passes = [PendingPass {
            pass: BLUR,
            units: 3,
        }];
        let spec = plan_slice(&model, &passes, 1000.0, 2000.0, 2000)
            .unwrap()
            .unwrap();
        assert_eq!(spec.parts, vec![SlicePart { index: 0, units: 1 }]);
        // Inside a 1.2 ms window it does not fit at all.
        assert_eq!(
            plan_slice(&model, &passes, 1000.0, 1200.0, 2000).unwrap(),
            None
        );
        model.calibrate(PassKey(9), 1, 2500.0);
        assert!(matches!(
            plan_slice(
                &model,
                &[PendingPass {
                    pass: PassKey(9),
                    units: 1
                }],
                1000.0,
                2000.0,
                2000
            ),
            Err(PlanError::UnitTooLarge { .. })
        ));
        assert_eq!(
            plan_slice(
                &model,
                &[PendingPass {
                    pass: PassKey(7),
                    units: 1
                }],
                1000.0,
                2000.0,
                2000
            ),
            Err(PlanError::Uncalibrated(PassKey(7)))
        );
    }

    #[test]
    fn the_slicer_aligns_limits_in_flight_and_learns() {
        let mut slicer = JobSlicer::new(SlicingPolicy::DEFAULT);
        *slicer.model_mut() = model();
        let passes = [PendingPass {
            pass: BLUR,
            units: 1000,
        }];
        assert_eq!(
            slicer.decide(at(0), &passes).unwrap(),
            Decision::Unrestricted
        );
        slicer.on_signal(&DeadlineSignal::PlaybackStarted {
            at: at(0),
            rate: PlaybackRate::NORMAL,
            frame_budget_us: 16_666,
        });
        assert!(matches!(
            slicer.decide(at(10), &passes).unwrap(),
            Decision::WaitForFrame { .. }
        ));
        for (seq, tick) in [(0u64, 0u64), (1, 16_666)] {
            slicer.on_signal(&DeadlineSignal::FrameDone {
                at: at(tick + 3_000),
                frame_seq: Seq::new(seq).unwrap(),
                tick: at(tick),
                next_submit: at(tick + 16_666),
                late: false,
            });
        }
        // Gap until 33,332 − 1,000 guard.
        let Decision::Submit(spec) = slicer.decide(at(20_000), &passes).unwrap() else {
            panic!("expected a slice")
        };
        assert_eq!(spec.parts[0].units, 25, "1 ms of 40 µs units");
        slicer.submitted();
        assert_eq!(
            slicer.decide(at(21_000), &passes).unwrap(),
            Decision::InFlightFull
        );
        // It ran 2x slower than predicted on a busy GPU: the model learns.
        slicer.completed(spec.raw_predicted_us, spec.raw_predicted_us * 2.0);
        let Decision::Submit(spec) = slicer.decide(at(22_000), &passes).unwrap() else {
            panic!("expected a slice")
        };
        assert_eq!(spec.parts[0].units, 12);
        // 600 µs before the guard: only what fits the window.
        let Decision::Submit(spec) = slicer.decide(at(31_732), &passes).unwrap() else {
            panic!("expected a slice")
        };
        assert_eq!(spec.parts[0].units, 7, "600 µs of 80 µs units");
        assert!(matches!(
            slicer.decide(at(32_300), &passes).unwrap(),
            Decision::WaitForFrame { .. }
        ));
        slicer.on_signal(&DeadlineSignal::DeadlineMiss {
            at: at(40_000),
            frame_seq: Seq::new(3).unwrap(),
            overrun_us: 500,
        });
        assert_eq!(
            slicer.decide(at(41_000), &passes).unwrap(),
            Decision::Yield { until: at(140_000) }
        );
    }
}
