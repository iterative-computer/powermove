//! GPU time slicing and deadline signals (contract 13, section 8 "Device").
//!
//! During playback main signals the job engine, which splits its GPU work
//! into command buffers of at most 2 ms, runs at lower QoS, and yields when
//! the interactive engine reports a deadline miss. Blender jobs pause or
//! throttle during playback. Spike S8 proves the policy; its numbers are
//! drafts until S8 passes.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{HostMicros, PlaybackRate, Seq};
use crate::validate::{ensure, Validate, Violation};
use crate::version::ContractId;

/// Signals relayed by main from the interactive engine to the job engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "signal")]
pub enum DeadlineSignal {
    PlaybackStarted { at: HostMicros, rate: PlaybackRate, frame_budget_us: u32 },
    PlaybackStopped { at: HostMicros },
    /// The interactive engine missed a presentation deadline.
    DeadlineMiss { at: HostMicros, frame_seq: Seq, overrun_us: u32 },
    /// Sustained load: the interactive engine stepped down its quality ladder.
    QualityStepDown { at: HostMicros },
}

/// Blender worker behavior during playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum BlenderThrottle {
    Pause,
    /// At most this many concurrent Blender workers.
    Throttle { max_workers: u8 },
}

/// The job engine's slicing policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SlicingPolicy {
    /// Longest job-engine command buffer during playback (2 ms).
    pub max_command_buffer_us: u32,
    /// After a deadline miss, the job engine submits nothing for this long.
    pub yield_after_miss_us: u32,
    /// Blender during playback.
    pub blender: BlenderThrottle,
}

impl SlicingPolicy {
    pub const DEFAULT: Self = Self { max_command_buffer_us: 2_000, yield_after_miss_us: 100_000, blender: BlenderThrottle::Pause };
}

/// Longest yield after a deadline miss. Longer would starve jobs whenever
/// playback stutters; S8 tunes the default below it.
pub const MAX_YIELD_AFTER_MISS_US: u32 = 1_000_000;

impl Validate for SlicingPolicy {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Slicing;
        ensure(self.max_command_buffer_us > 0 && self.max_command_buffer_us <= 2_000, C, || {
            "command buffers during playback are at most 2 ms".into()
        })?;
        ensure(self.yield_after_miss_us <= MAX_YIELD_AFTER_MISS_US, C, || {
            format!("the yield after a miss is at most {MAX_YIELD_AFTER_MISS_US} µs")
        })?;
        ensure(!matches!(self.blender, BlenderThrottle::Throttle { max_workers: 0 }), C, || {
            "a Blender throttle allows at least one worker; use pause for none".into()
        })
    }
}

impl Validate for DeadlineSignal {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Slicing;
        match *self {
            DeadlineSignal::PlaybackStarted { rate, frame_budget_us, .. } => {
                ensure(rate.is_valid() && rate.num != 0, C, || "playback starts at a non-zero rate".into())?;
                ensure(frame_budget_us > 0, C, || "the frame budget is positive".into())
            }
            DeadlineSignal::DeadlineMiss { overrun_us, .. } => ensure(overrun_us > 0, C, || "a miss overruns by at least 1 µs".into()),
            DeadlineSignal::PlaybackStopped { .. } | DeadlineSignal::QualityStepDown { .. } => Ok(()),
        }
    }
}

/// What the job engine may submit next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceBudget {
    /// No playback: submit freely.
    Unlimited,
    /// Playback: submit command buffers of at most this many microseconds of GPU time.
    Sliced { max_us: u32 },
    /// Yield until the given instant.
    YieldUntil(HostMicros),
}

/// Job-engine state machine: feed it signals, ask it for a budget.
#[derive(Debug, Clone)]
pub struct SlicingGovernor {
    policy: SlicingPolicy,
    playing: bool,
    yield_until: Option<HostMicros>,
}

impl SlicingGovernor {
    pub fn new(policy: SlicingPolicy) -> Self {
        Self { policy, playing: false, yield_until: None }
    }

    pub fn on_signal(&mut self, signal: &DeadlineSignal) {
        match *signal {
            DeadlineSignal::PlaybackStarted { .. } => self.playing = true,
            DeadlineSignal::PlaybackStopped { .. } => {
                self.playing = false;
                self.yield_until = None;
            }
            DeadlineSignal::DeadlineMiss { at, .. } | DeadlineSignal::QualityStepDown { at } => {
                let until = at.get().saturating_add(self.policy.yield_after_miss_us as u64);
                let until = HostMicros::new(until).unwrap_or(HostMicros::MAX);
                self.yield_until = Some(self.yield_until.map_or(until, |current| current.max(until)));
            }
        }
    }

    pub fn budget(&self, now: HostMicros) -> SliceBudget {
        if let Some(until) = self.yield_until
            && now < until
        {
            return SliceBudget::YieldUntil(until);
        }
        if self.playing { SliceBudget::Sliced { max_us: self.policy.max_command_buffer_us } } else { SliceBudget::Unlimited }
    }

    pub fn blender_during_playback(&self) -> Option<BlenderThrottle> {
        self.playing.then_some(self.policy.blender)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(us: u64) -> HostMicros {
        HostMicros::new(us).unwrap()
    }

    #[test]
    fn governor_slices_during_playback_and_yields_after_misses() {
        let mut governor = SlicingGovernor::new(SlicingPolicy::DEFAULT);
        assert_eq!(governor.budget(at(0)), SliceBudget::Unlimited);
        governor.on_signal(&DeadlineSignal::PlaybackStarted { at: at(10), rate: PlaybackRate::NORMAL, frame_budget_us: 16_666 });
        assert_eq!(governor.budget(at(20)), SliceBudget::Sliced { max_us: 2_000 });
        assert_eq!(governor.blender_during_playback(), Some(BlenderThrottle::Pause));
        governor.on_signal(&DeadlineSignal::DeadlineMiss { at: at(1_000), frame_seq: Seq::ZERO, overrun_us: 900 });
        assert_eq!(governor.budget(at(50_000)), SliceBudget::YieldUntil(at(101_000)));
        assert_eq!(governor.budget(at(101_000)), SliceBudget::Sliced { max_us: 2_000 });
        governor.on_signal(&DeadlineSignal::PlaybackStopped { at: at(200_000) });
        assert_eq!(governor.budget(at(200_001)), SliceBudget::Unlimited);
        assert_eq!(governor.blender_during_playback(), None);
    }

    #[test]
    fn policy_caps_command_buffers_at_2ms() {
        assert!(SlicingPolicy::DEFAULT.validate().is_ok());
        assert!(SlicingPolicy { max_command_buffer_us: 2_001, ..SlicingPolicy::DEFAULT }.validate().is_err());
        assert!(SlicingPolicy { max_command_buffer_us: 0, ..SlicingPolicy::DEFAULT }.validate().is_err());
        assert!(SlicingPolicy { yield_after_miss_us: 1_000_001, ..SlicingPolicy::DEFAULT }.validate().is_err());
        assert!(SlicingPolicy { blender: BlenderThrottle::Throttle { max_workers: 0 }, ..SlicingPolicy::DEFAULT }.validate().is_err());
        assert!(SlicingPolicy { blender: BlenderThrottle::Throttle { max_workers: 1 }, ..SlicingPolicy::DEFAULT }.validate().is_ok());
    }

    #[test]
    fn deadline_signals_validate() {
        let started = |num, frame_budget_us| DeadlineSignal::PlaybackStarted { at: at(1), rate: PlaybackRate { num, den: 1 }, frame_budget_us };
        assert!(started(1, 16_666).validate().is_ok());
        assert!(started(-2, 16_666).validate().is_ok(), "reverse playback");
        assert!(started(0, 16_666).validate().is_err());
        assert!(started(1, 0).validate().is_err());
        assert!(DeadlineSignal::DeadlineMiss { at: at(1), frame_seq: Seq::ZERO, overrun_us: 0 }.validate().is_err());
    }
}
