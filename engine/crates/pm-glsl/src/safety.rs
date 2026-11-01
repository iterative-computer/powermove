//! Device-owner recovery seam for untrusted draws. This is an opt-in host
//! protocol, not a claim that killing a compiler cancels a submitted GPU job.
//! The owner must drive it off the UI thread and keep polling under the wall
//! deadline; every trait method must be nonblocking or externally supervised.
//! A backend unable to retire a wedged GPU safely needs a GPU service process.

use std::{
    collections::HashSet,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use crate::{Failure, FailureStage, Key};

/// Opaque generation-bound ticket. A device owner maps it to its native fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Submission(u64);

impl Submission {
    /// Process-local stable ID for native fence lookup and diagnostics.
    pub fn id(self) -> u64 {
        self.0
    }
}

static NEXT_SUBMISSION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Pending,
    Complete,
    DeviceLost,
}

/// The implementation owns the device, surfaces, native fences and source
/// archive. `retire` must invalidate all of them before `recreate` runs.
/// Neither method may wait indefinitely for a failed GPU to become idle.
pub trait DeviceOwner {
    fn poll(&mut self, submission: Submission) -> Result<Completion, String>;
    fn quarantine(&mut self, source: Key, reason: &Failure) -> Result<(), String>;
    fn retire(&mut self) -> Result<(), String>;
    fn recreate(&mut self) -> Result<(), String>;
}

struct Active {
    ticket: Submission,
    source: Key,
    deadline: Instant,
}

/// One in-flight untrusted draw per guard makes fault attribution exact.
/// Reuse this guard across device recreation; dropping it loses its in-memory
/// quarantine. The owner must persist quarantine alongside unchanged source.
#[derive(Default)]
pub struct DrawGuard {
    active: Option<Active>,
    quarantined: HashSet<Key>,
    unavailable: bool,
}

impl DrawGuard {
    /// Reserve before submitting. A refusal must never reach the GPU.
    /// Supply the owner's monotonic clock, also used for every `poll`.
    /// `source` must identify unchanged authored bytes across device resets,
    /// e.g. `Key::new(source, line_offset, "draw-quarantine")`, rather than a
    /// cache key whose driver context changes when the device is recreated.
    pub fn begin(
        &mut self,
        source: Key,
        now: Instant,
        timeout: Duration,
    ) -> Result<Submission, Failure> {
        if self.unavailable {
            return Err(Failure::new(
                FailureStage::DeviceRecovery,
                "device remains retired after failed recovery",
            ));
        }
        if self.quarantined.contains(&source) {
            return Err(Failure::new(
                FailureStage::DrawQuarantined,
                "unchanged source is quarantined; source bytes retained",
            ));
        }
        if self.active.is_some() {
            return Err(Failure::new(
                FailureStage::DrawBusy,
                "a guarded draw is already in flight",
            ));
        }
        let deadline = now
            .checked_add(timeout)
            .filter(|_| !timeout.is_zero())
            .ok_or_else(|| {
                Failure::new(
                    FailureStage::DrawDeadline,
                    "invalid or expired draw deadline",
                )
            })?;
        let serial = NEXT_SUBMISSION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| {
                Failure::new(
                    FailureStage::DeviceRecovery,
                    "submission generation exhausted",
                )
            })?;
        let ticket = Submission(serial);
        self.active = Some(Active {
            ticket,
            source,
            deadline,
        });
        Ok(ticket)
    }

    /// Poll completion before readback. After failure the ticket is invalid;
    /// a subsequent good draw can begin only after successful recovery.
    pub fn poll(
        &mut self,
        owner: &mut impl DeviceOwner,
        ticket: Submission,
        now: Instant,
    ) -> Result<Completion, Failure> {
        let active = self
            .active
            .as_ref()
            .filter(|a| a.ticket == ticket)
            .ok_or_else(|| Failure::new(FailureStage::Draw, "stale or foreign draw submission"))?;
        let failure = if now >= active.deadline {
            Failure::new(
                FailureStage::DrawDeadline,
                "GPU completion deadline exceeded",
            )
        } else {
            match owner.poll(ticket) {
                Ok(Completion::Complete) => {
                    self.active = None;
                    return Ok(Completion::Complete);
                }
                Ok(Completion::Pending) => return Ok(Completion::Pending),
                Ok(Completion::DeviceLost) => Failure::new(
                    FailureStage::DeviceLost,
                    "GPU device lost during guarded draw",
                ),
                Err(e) => Failure::new(FailureStage::DeviceLost, e),
            }
        };
        let active = self.active.take().expect("validated active submission");
        self.quarantined.insert(active.source);
        self.unavailable = true;
        // Even a failed archive/notification cannot skip device retirement.
        let archive = owner.quarantine(active.source, &failure);
        let retirement = owner.retire();
        if let Err(e) = retirement.and(archive).and_then(|_| owner.recreate()) {
            return Err(Failure::new(
                FailureStage::DeviceRecovery,
                format!("{}; recovery failed: {e}", failure.stage.name()),
            ));
        }
        self.unavailable = false;
        Err(failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Double {
        status: Completion,
        fail: Option<&'static str>,
        calls: Vec<&'static str>,
        archived: Vec<Key>,
    }
    impl DeviceOwner for Double {
        fn poll(&mut self, _: Submission) -> Result<Completion, String> {
            self.calls.push("poll");
            if self.fail == Some("poll") {
                Err("poll error".into())
            } else {
                Ok(self.status)
            }
        }
        fn quarantine(&mut self, source: Key, _: &Failure) -> Result<(), String> {
            self.calls.push("quarantine");
            self.archived.push(source);
            if self.fail == Some("quarantine") {
                Err("archive error".into())
            } else {
                Ok(())
            }
        }
        fn retire(&mut self) -> Result<(), String> {
            self.calls.push("retire");
            if self.fail == Some("retire") {
                Err("retire error".into())
            } else {
                Ok(())
            }
        }
        fn recreate(&mut self) -> Result<(), String> {
            self.calls.push("recreate");
            if self.fail == Some("recreate") {
                Err("recreate error".into())
            } else {
                Ok(())
            }
        }
    }
    fn owner(status: Completion) -> Double {
        Double {
            status,
            fail: None,
            calls: Vec::new(),
            archived: Vec::new(),
        }
    }
    fn key(source: &str) -> Key {
        Key::new(source, 0, "draw-guard-test")
    }

    #[test]
    fn pending_draws_complete_without_touching_device_lifecycle() {
        let mut guard = DrawGuard::default();
        let mut owner = owner(Completion::Pending);
        let now = Instant::now();
        let ticket = guard
            .begin(key("valid"), now, Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            guard
                .begin(key("other"), now, Duration::from_secs(1))
                .unwrap_err()
                .stage,
            FailureStage::DrawBusy
        );
        assert_eq!(
            guard.poll(&mut owner, ticket, now).unwrap(),
            Completion::Pending
        );
        owner.status = Completion::Complete;
        assert_eq!(
            guard.poll(&mut owner, ticket, now).unwrap(),
            Completion::Complete
        );
        assert_eq!(owner.calls, ["poll", "poll"]);
        assert!(guard.poll(&mut owner, ticket, now).is_err());
        assert!(
            guard
                .begin(key("valid"), now, Duration::from_secs(1))
                .is_ok()
        );
    }

    #[test]
    fn timeout_quarantines_exact_source_and_next_good_draw_recovers() {
        let mut guard = DrawGuard::default();
        let mut owner = owner(Completion::Pending);
        let now = Instant::now();
        let ticket = guard
            .begin(key("non-terminating"), now, Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            guard
                .poll(&mut owner, ticket, now + Duration::from_secs(1))
                .unwrap_err()
                .stage,
            FailureStage::DrawDeadline
        );
        assert_eq!(owner.calls, ["quarantine", "retire", "recreate"]);
        assert_eq!(owner.archived, [key("non-terminating")]);
        assert_eq!(
            guard
                .begin(key("non-terminating"), now, Duration::from_secs(1))
                .unwrap_err()
                .stage,
            FailureStage::DrawQuarantined
        );
        let next = guard
            .begin(key("valid"), now, Duration::from_secs(1))
            .unwrap();
        assert!(guard.poll(&mut owner, ticket, now).is_err());
        owner.status = Completion::Complete;
        assert_eq!(
            guard.poll(&mut owner, next, now).unwrap(),
            Completion::Complete
        );
    }

    #[test]
    fn device_loss_and_poll_errors_use_the_same_recovery_order() {
        for fail in [None, Some("poll")] {
            let mut guard = DrawGuard::default();
            let mut owner = owner(Completion::DeviceLost);
            owner.fail = fail;
            let now = Instant::now();
            let ticket = guard
                .begin(key("fault"), now, Duration::from_secs(1))
                .unwrap();
            assert_eq!(
                guard.poll(&mut owner, ticket, now).unwrap_err().stage,
                FailureStage::DeviceLost
            );
            assert_eq!(owner.calls, ["poll", "quarantine", "retire", "recreate"]);
        }
    }

    #[test]
    fn recovery_errors_fail_closed_and_retire_even_if_archiving_fails() {
        for failure in ["quarantine", "retire", "recreate"] {
            let mut guard = DrawGuard::default();
            let mut owner = owner(Completion::DeviceLost);
            owner.fail = Some(failure);
            let now = Instant::now();
            let ticket = guard
                .begin(key("fault"), now, Duration::from_secs(1))
                .unwrap();
            assert_eq!(
                guard.poll(&mut owner, ticket, now).unwrap_err().stage,
                FailureStage::DeviceRecovery
            );
            assert!(owner.calls.contains(&"retire"));
            assert_eq!(
                guard
                    .begin(key("valid"), now, Duration::from_secs(1))
                    .unwrap_err()
                    .stage,
                FailureStage::DeviceRecovery
            );
        }
    }

    #[test]
    fn invalid_deadline_never_reserves_a_submission() {
        let mut guard = DrawGuard::default();
        let now = Instant::now();
        for duration in [Duration::ZERO, Duration::MAX] {
            assert_eq!(
                guard.begin(key("valid"), now, duration).unwrap_err().stage,
                FailureStage::DrawDeadline
            );
        }
        assert!(
            guard
                .begin(key("valid"), now, Duration::from_secs(1))
                .is_ok()
        );
    }

    #[test]
    fn tickets_from_another_device_guard_cannot_poll_this_device() {
        let now = Instant::now();
        let mut first = DrawGuard::default();
        let mut second = DrawGuard::default();
        let a = first
            .begin(key("first"), now, Duration::from_secs(1))
            .unwrap();
        let b = second
            .begin(key("second"), now, Duration::from_secs(1))
            .unwrap();
        assert_ne!(a.id(), b.id());
        let mut owner = owner(Completion::Complete);
        assert!(second.poll(&mut owner, a, now).is_err());
        assert!(owner.calls.is_empty());
        assert_eq!(
            second.poll(&mut owner, b, now).unwrap(),
            Completion::Complete
        );
    }
}
