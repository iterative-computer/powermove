//! GPU time slicing and deadline signals (contract 13, section 8 "Device").
//!
//! During playback the job engine splits its GPU work into command buffers
//! of at most 2 ms (predicted to a 1 ms target), keeps one in flight, runs at
//! utility QoS, and submits a slice only into the gap after the interactive
//! engine's frame completes (frame alignment). It yields when the interactive
//! engine reports a deadline miss. Blender pauses during playback. These are
//! the numbers spike S8 measured (plan section 26, amendment 12;
//! `docs/engine/spikes/S8.md`, "Recommended contract 13 parameters").

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{HostMicros, PlaybackRate, Seq};
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// Signals from the interactive engine to the job engine. Playback start,
/// stop, misses, and quality steps go through main (which also moves the
/// leases, contract 8); per-frame `frameDone` goes on the direct port main
/// hands the two engines (S8: 4 ms of relay latency cut export throughput by
/// 24 to 55%).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "signal"
)]
pub enum DeadlineSignal {
    PlaybackStarted {
        at: HostMicros,
        rate: PlaybackRate,
        frame_budget_us: u32,
    },
    PlaybackStopped {
        at: HostMicros,
    },
    /// The interactive engine missed a presentation deadline.
    DeadlineMiss {
        at: HostMicros,
        frame_seq: Seq,
        overrun_us: u32,
    },
    /// Sustained load: the interactive engine stepped down its quality ladder.
    QualityStepDown {
        at: HostMicros,
    },
    /// The interactive engine's frame for `tick` finished its GPU work at
    /// `at`; its next GPU submission is planned at `next_submit`. The job
    /// engine fits its slices into the gap before `next_submit` (frame
    /// alignment, S8 recommendation 3).
    FrameDone {
        at: HostMicros,
        frame_seq: Seq,
        tick: HostMicros,
        next_submit: HostMicros,
        /// The frame completed after its deadline.
        late: bool,
    },
}

/// Blender worker behavior during playback. S8 measured that a worker
/// count does not bound GPU bursts and a `SIGSTOP` duty cycle leaks them, so
/// main always sends `pause` for playback; `throttle` remains for
/// non-playback load shedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum BlenderThrottle {
    Pause,
    /// At most this many concurrent Blender workers.
    Throttle {
        max_workers: u8,
    },
}

/// The job engine's slicing policy (host → job engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SlicingPolicy {
    /// Hard bound on any job-engine command buffer, and on any single
    /// dispatch in it, during playback (2 ms).
    pub max_command_buffer_us: u32,
    /// What the cost model predicts each command buffer to (1 ms). Filling to
    /// 90% of 2 ms overshot the bound once other GPU clients were active;
    /// a 1 ms target kept every buffer at 2.01 ms or less (S8).
    pub target_command_buffer_us: u32,
    /// Job-engine command buffers in flight during playback (1).
    pub max_in_flight: u8,
    /// A slice must be predicted to end this long before the interactive
    /// engine's next submission (1 ms).
    pub align_guard_us: u32,
    /// After a deadline miss, the job engine submits nothing for this long.
    pub yield_after_miss_us: u32,
    /// Blender during playback.
    pub blender: BlenderThrottle,
}

impl SlicingPolicy {
    pub const DEFAULT: Self = Self {
        max_command_buffer_us: 2_000,
        target_command_buffer_us: 1_000,
        max_in_flight: 1,
        align_guard_us: 1_000,
        yield_after_miss_us: 100_000,
        blender: BlenderThrottle::Pause,
    };
}

/// The ceiling on `max_command_buffer_us` (section 8).
pub const MAX_COMMAND_BUFFER_US: u32 = 2_000;
/// Longest yield after a deadline miss. Longer would starve jobs whenever
/// playback stutters; S8 tunes the default below it.
pub const MAX_YIELD_AFTER_MISS_US: u32 = 1_000_000;
/// Largest alignment guard: half a 60 fps frame. A larger guard would leave
/// no gap at 120 fps.
pub const MAX_ALIGN_GUARD_US: u32 = 8_000;
/// Most job-engine command buffers in flight during playback.
pub const MAX_IN_FLIGHT: u8 = 4;
/// Frames the job engine waits for at playback start before it trusts the
/// interactive engine's phase (S8).
pub const ALIGN_WARMUP_FRAMES: u32 = 2;
/// Frame budgets without a `frameDone` after which aligned slicing falls back
/// to reactive slicing, so a silent interactive engine cannot starve jobs.
pub const ALIGN_STALE_FRAMES: u64 = 8;

impl Validate for SlicingPolicy {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Slicing;
        ensure(
            self.max_command_buffer_us > 0 && self.max_command_buffer_us <= MAX_COMMAND_BUFFER_US,
            C,
            || "command buffers during playback are at most 2 ms".into(),
        )?;
        ensure(
            self.target_command_buffer_us > 0
                && self.target_command_buffer_us <= self.max_command_buffer_us,
            C,
            || "the prediction target is positive and within the command-buffer bound".into(),
        )?;
        ensure((1..=MAX_IN_FLIGHT).contains(&self.max_in_flight), C, || {
            format!("1 to {MAX_IN_FLIGHT} command buffers in flight")
        })?;
        ensure(self.align_guard_us <= MAX_ALIGN_GUARD_US, C, || {
            format!("the alignment guard is at most {MAX_ALIGN_GUARD_US} µs")
        })?;
        ensure(
            self.yield_after_miss_us <= MAX_YIELD_AFTER_MISS_US,
            C,
            || format!("the yield after a miss is at most {MAX_YIELD_AFTER_MISS_US} µs"),
        )?;
        ensure(
            !matches!(self.blender, BlenderThrottle::Throttle { max_workers: 0 }),
            C,
            || "a Blender throttle allows at least one worker; use pause for none".into(),
        )
    }
}

impl Validate for DeadlineSignal {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Slicing;
        match *self {
            DeadlineSignal::PlaybackStarted {
                rate,
                frame_budget_us,
                ..
            } => {
                ensure(rate.is_valid() && rate.num != 0, C, || {
                    "playback starts at a non-zero rate".into()
                })?;
                ensure(frame_budget_us > 0, C, || {
                    "the frame budget is positive".into()
                })
            }
            DeadlineSignal::DeadlineMiss { overrun_us, .. } => ensure(overrun_us > 0, C, || {
                "a miss overruns by at least 1 µs".into()
            }),
            DeadlineSignal::FrameDone {
                tick, next_submit, ..
            } => ensure(next_submit > tick, C, || {
                "the next submission comes after the finished frame's tick".into()
            }),
            DeadlineSignal::PlaybackStopped { .. } | DeadlineSignal::QualityStepDown { .. } => {
                Ok(())
            }
        }
    }
}

/// What the job engine may submit next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceBudget {
    /// No playback: submit freely.
    Unlimited,
    /// Playback: command buffers predicted to at most `target_us` and never
    /// above `max_us` of GPU time, at most `max_in_flight` at once. When
    /// `deadline` is set (aligned), a slice must be predicted to finish by it;
    /// `None` means reactive slicing (the interactive engine's phase is
    /// stale).
    Sliced {
        max_us: u32,
        target_us: u32,
        max_in_flight: u8,
        deadline: Option<HostMicros>,
    },
    /// Aligned slicing: the interactive engine's current frame is not done
    /// (or its phase is not known yet). Submit nothing until the next
    /// `frameDone`, or until this instant at the latest, then ask again.
    WaitForFrame { until: HostMicros },
    /// Yield until the given instant (after a miss or quality step-down).
    YieldUntil(HostMicros),
}

#[derive(Debug, Clone, Copy)]
struct Phase {
    /// When the last frame finished.
    done_at: HostMicros,
    /// The interactive engine's next planned submission.
    next_submit: HostMicros,
}

/// Job-engine state machine: feed it signals, ask it for a budget before
/// every submission.
#[derive(Debug, Clone)]
pub struct SlicingGovernor {
    policy: SlicingPolicy,
    playing: Option<Playing>,
    yield_until: Option<HostMicros>,
}

#[derive(Debug, Clone, Copy)]
struct Playing {
    started_at: HostMicros,
    frame_budget_us: u32,
    frames_done: u32,
    phase: Option<Phase>,
}

impl SlicingGovernor {
    pub fn new(policy: SlicingPolicy) -> Self {
        Self {
            policy,
            playing: None,
            yield_until: None,
        }
    }

    pub fn policy(&self) -> &SlicingPolicy {
        &self.policy
    }

    /// Replaces the policy (an `Event::Slicing` from the host). Playback
    /// state and an active yield are kept.
    pub fn set_policy(&mut self, policy: SlicingPolicy) {
        self.policy = policy;
    }

    pub fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    pub fn on_signal(&mut self, signal: &DeadlineSignal) {
        match *signal {
            DeadlineSignal::PlaybackStarted {
                at,
                frame_budget_us,
                ..
            } => {
                self.playing = Some(Playing {
                    started_at: at,
                    frame_budget_us,
                    frames_done: 0,
                    phase: None,
                });
            }
            DeadlineSignal::PlaybackStopped { .. } => {
                self.playing = None;
                self.yield_until = None;
            }
            DeadlineSignal::DeadlineMiss { at, .. } | DeadlineSignal::QualityStepDown { at } => {
                let until = at
                    .get()
                    .saturating_add(self.policy.yield_after_miss_us as u64);
                let until = HostMicros::new(until).unwrap_or(HostMicros::MAX);
                self.yield_until =
                    Some(self.yield_until.map_or(until, |current| current.max(until)));
            }
            DeadlineSignal::FrameDone {
                at, next_submit, ..
            } => {
                if let Some(playing) = &mut self.playing {
                    playing.frames_done = playing.frames_done.saturating_add(1);
                    // Signals can arrive out of order across two ports; keep the latest plan.
                    let newer = playing
                        .phase
                        .is_none_or(|phase| next_submit >= phase.next_submit);
                    if newer {
                        playing.phase = Some(Phase {
                            done_at: at,
                            next_submit,
                        });
                    }
                }
            }
        }
    }

    pub fn budget(&self, now: HostMicros) -> SliceBudget {
        if let Some(until) = self.yield_until
            && now < until
        {
            return SliceBudget::YieldUntil(until);
        }
        let Some(playing) = self.playing else {
            return SliceBudget::Unlimited;
        };
        let sliced = |deadline| SliceBudget::Sliced {
            max_us: self.policy.max_command_buffer_us,
            target_us: self.policy.target_command_buffer_us,
            max_in_flight: self.policy.max_in_flight,
            deadline,
        };
        let stale_after = ALIGN_STALE_FRAMES.saturating_mul(playing.frame_budget_us as u64);
        let last_heard = playing.phase.map_or(playing.started_at, |p| p.done_at);
        let stale_at = HostMicros::new(last_heard.get().saturating_add(stale_after))
            .unwrap_or(HostMicros::MAX);
        if now >= stale_at {
            // The interactive engine went quiet: fall back to reactive slicing.
            return sliced(None);
        }
        match playing.phase {
            Some(phase) if playing.frames_done >= ALIGN_WARMUP_FRAMES => {
                let deadline = phase
                    .next_submit
                    .get()
                    .saturating_sub(self.policy.align_guard_us as u64);
                let deadline = HostMicros::new(deadline).unwrap_or(HostMicros::ZERO);
                if now < deadline {
                    sliced(Some(deadline))
                } else {
                    SliceBudget::WaitForFrame { until: stale_at }
                }
            }
            _ => SliceBudget::WaitForFrame { until: stale_at },
        }
    }

    pub fn blender_during_playback(&self) -> Option<BlenderThrottle> {
        self.playing.is_some().then_some(self.policy.blender)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(us: u64) -> HostMicros {
        HostMicros::new(us).unwrap()
    }

    fn started(us: u64) -> DeadlineSignal {
        DeadlineSignal::PlaybackStarted {
            at: at(us),
            rate: PlaybackRate::NORMAL,
            frame_budget_us: 16_666,
        }
    }

    fn done(seq: u64, done_at: u64, tick: u64) -> DeadlineSignal {
        DeadlineSignal::FrameDone {
            at: at(done_at),
            frame_seq: Seq::new(seq).unwrap(),
            tick: at(tick),
            next_submit: at(tick + 16_666),
            late: false,
        }
    }

    const SLICED: fn(Option<HostMicros>) -> SliceBudget = |deadline| SliceBudget::Sliced {
        max_us: 2_000,
        target_us: 1_000,
        max_in_flight: 1,
        deadline,
    };

    #[test]
    fn unlimited_without_playback() {
        let governor = SlicingGovernor::new(SlicingPolicy::DEFAULT);
        assert_eq!(governor.budget(at(0)), SliceBudget::Unlimited);
        assert_eq!(governor.blender_during_playback(), None);
    }

    #[test]
    fn waits_for_two_frames_then_aligns_to_the_next_submission() {
        let mut governor = SlicingGovernor::new(SlicingPolicy::DEFAULT);
        governor.on_signal(&started(10));
        assert!(governor.is_playing());
        assert_eq!(
            governor.blender_during_playback(),
            Some(BlenderThrottle::Pause)
        );
        let stale = 10 + 8 * 16_666;
        assert_eq!(
            governor.budget(at(20)),
            SliceBudget::WaitForFrame { until: at(stale) },
            "nothing before the phase is known"
        );
        governor.on_signal(&done(0, 3_000, 0));
        assert!(matches!(
            governor.budget(at(4_000)),
            SliceBudget::WaitForFrame { .. }
        ));
        governor.on_signal(&done(1, 19_500, 16_666));
        // Next submission at 33,332; guard 1 ms.
        assert_eq!(governor.budget(at(20_000)), SLICED(Some(at(32_332))));
        assert_eq!(
            governor.budget(at(32_332)),
            SliceBudget::WaitForFrame {
                until: at(19_500 + 8 * 16_666)
            },
            "inside the guard: wait for the next frame"
        );
        governor.on_signal(&done(2, 36_000, 33_332));
        assert_eq!(governor.budget(at(36_100)), SLICED(Some(at(48_998))));
    }

    #[test]
    fn out_of_order_frame_signals_keep_the_latest_plan() {
        let mut governor = SlicingGovernor::new(SlicingPolicy::DEFAULT);
        governor.on_signal(&started(0));
        governor.on_signal(&done(1, 19_500, 16_666));
        governor.on_signal(&done(0, 3_000, 0));
        assert_eq!(governor.budget(at(20_000)), SLICED(Some(at(32_332))));
    }

    #[test]
    fn a_silent_interactive_engine_falls_back_to_reactive_slicing() {
        let mut governor = SlicingGovernor::new(SlicingPolicy::DEFAULT);
        governor.on_signal(&started(0));
        assert_eq!(governor.budget(at(8 * 16_666)), SLICED(None));
        governor.on_signal(&done(0, 200_000, 190_000));
        governor.on_signal(&done(1, 210_000, 206_666));
        assert_eq!(governor.budget(at(211_000)), SLICED(Some(at(222_332))));
        assert_eq!(governor.budget(at(210_000 + 8 * 16_666)), SLICED(None));
    }

    #[test]
    fn misses_yield_and_stop_clears() {
        let mut governor = SlicingGovernor::new(SlicingPolicy::DEFAULT);
        governor.on_signal(&started(10));
        governor.on_signal(&DeadlineSignal::DeadlineMiss {
            at: at(1_000),
            frame_seq: Seq::ZERO,
            overrun_us: 900,
        });
        assert_eq!(
            governor.budget(at(50_000)),
            SliceBudget::YieldUntil(at(101_000))
        );
        // A step-down extends, never shortens, the active yield.
        governor.on_signal(&DeadlineSignal::QualityStepDown { at: at(500) });
        assert_eq!(
            governor.budget(at(50_000)),
            SliceBudget::YieldUntil(at(101_000))
        );
        governor.on_signal(&DeadlineSignal::QualityStepDown { at: at(2_000) });
        assert_eq!(
            governor.budget(at(101_500)),
            SliceBudget::YieldUntil(at(102_000))
        );
        governor.on_signal(&DeadlineSignal::PlaybackStopped { at: at(200_000) });
        assert_eq!(governor.budget(at(200_001)), SliceBudget::Unlimited);
        assert_eq!(governor.blender_during_playback(), None);
    }

    #[test]
    fn frame_signals_without_playback_are_ignored() {
        let mut governor = SlicingGovernor::new(SlicingPolicy::DEFAULT);
        governor.on_signal(&done(0, 3_000, 0));
        assert_eq!(governor.budget(at(4_000)), SliceBudget::Unlimited);
    }

    #[test]
    fn policy_bounds() {
        let ok = SlicingPolicy::DEFAULT;
        assert!(ok.validate().is_ok());
        let bad = [
            SlicingPolicy {
                max_command_buffer_us: 2_001,
                ..ok
            },
            SlicingPolicy {
                max_command_buffer_us: 0,
                ..ok
            },
            SlicingPolicy {
                target_command_buffer_us: 0,
                ..ok
            },
            SlicingPolicy {
                target_command_buffer_us: 2_001,
                ..ok
            },
            SlicingPolicy {
                max_in_flight: 0,
                ..ok
            },
            SlicingPolicy {
                max_in_flight: 5,
                ..ok
            },
            SlicingPolicy {
                align_guard_us: 8_001,
                ..ok
            },
            SlicingPolicy {
                yield_after_miss_us: 1_000_001,
                ..ok
            },
            SlicingPolicy {
                blender: BlenderThrottle::Throttle { max_workers: 0 },
                ..ok
            },
        ];
        for policy in bad {
            assert!(policy.validate().is_err(), "{policy:?}");
        }
        assert!(
            SlicingPolicy {
                blender: BlenderThrottle::Throttle { max_workers: 1 },
                target_command_buffer_us: 2_000,
                ..ok
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn deadline_signals_validate() {
        let started = |num, frame_budget_us| DeadlineSignal::PlaybackStarted {
            at: at(1),
            rate: PlaybackRate { num, den: 1 },
            frame_budget_us,
        };
        assert!(started(1, 16_666).validate().is_ok());
        assert!(started(-2, 16_666).validate().is_ok(), "reverse playback");
        assert!(started(0, 16_666).validate().is_err());
        assert!(started(1, 0).validate().is_err());
        assert!(
            DeadlineSignal::DeadlineMiss {
                at: at(1),
                frame_seq: Seq::ZERO,
                overrun_us: 0
            }
            .validate()
            .is_err()
        );
        assert!(done(0, 10, 5).validate().is_ok());
        assert!(
            DeadlineSignal::FrameDone {
                at: at(10),
                frame_seq: Seq::ZERO,
                tick: at(5),
                next_submit: at(5),
                late: false,
            }
            .validate()
            .is_err()
        );
    }
}
