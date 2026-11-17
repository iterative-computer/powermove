//! Compiles and program builds that failed for a reason outside their
//! source: a crashed compiler, one past its deadline, one out of resources.
//! (A compiler with every slot taken is no failure: the request waits its
//! turn, [`pm_glsl::Admission`], and is never counted here.)

use pm_glsl::{Failure, FailureStage};
use std::time::{Duration, Instant};

/// The wait after the first such failure; each consecutive one doubles it.
const FIRST: Duration = Duration::from_millis(100);
/// The longest wait.
const LONGEST: Duration = Duration::from_secs(4);
/// Failures of one kind, for one unchanged source, before the compiler is
/// given up on for it (a visible, terminal refusal until a retry).
pub const LIMIT: u32 = 5;

/// The kinds with their own budget.
const KINDS: [FailureStage; 3] = [
    FailureStage::CompilerProcess,
    FailureStage::Deadline,
    FailureStage::CompilerResources,
];

/// A failure retried after its backoff, so a compiler that keeps failing is
/// not started again by every draw, and given up after [`LIMIT`] failures of
/// one kind. A changed source has its own entry and starts afresh.
#[derive(Clone, Debug)]
pub(crate) struct Retry {
    failure: Failure,
    /// Failures so far, per kind ([`KINDS`]).
    counts: [u32; KINDS.len()],
    consecutive: u32,
    after: Instant,
}

impl Retry {
    /// The backoff after `failure`, following `previous` (consecutive
    /// failures of the same source).
    pub(crate) fn after(previous: Option<&Retry>, failure: Failure) -> Retry {
        let mut counts = previous.map_or([0; KINDS.len()], |r| r.counts);
        if let Some(i) = KINDS.iter().position(|k| *k == failure.stage) {
            counts[i] = counts[i].saturating_add(1);
        }
        let consecutive = previous.map_or(1, |r| r.consecutive.saturating_add(1));
        let wait = FIRST
            .saturating_mul(1 << (consecutive - 1).min(16))
            .min(LONGEST);
        Retry {
            failure,
            counts,
            consecutive,
            after: Instant::now() + wait,
        }
    }

    /// Failures of the last failure's kind so far.
    pub(crate) fn failures(&self) -> u32 {
        KINDS
            .iter()
            .position(|k| *k == self.failure.stage)
            .map_or(0, |i| self.counts[i])
    }
    /// Whether the last failure's kind used up its budget.
    pub(crate) fn exhausted(&self) -> bool {
        self.failures() >= LIMIT
    }
    /// Ends the wait now (tests: the next attempt without sleeping).
    #[cfg(test)]
    pub(crate) fn expire(&mut self) {
        self.after = Instant::now();
    }
    /// The failure a draw reports while the backoff runs.
    pub(crate) fn waiting(&self) -> Option<&Failure> {
        (Instant::now() < self.after).then_some(&self.failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_doubles_to_its_cap() {
        let crashed = || Failure::new(FailureStage::CompilerProcess, "crashed");
        let mut retry = Retry::after(None, crashed());
        let mut waits = vec![];
        for _ in 0..8 {
            waits.push(
                retry
                    .after
                    .duration_since(Instant::now())
                    .as_millis()
                    .div_ceil(100)
                    * 100,
            );
            retry = Retry::after(Some(&retry), crashed());
        }
        assert_eq!(waits, [100, 200, 400, 800, 1600, 3200, 4000, 4000]);
    }

    /// Each kind of failure has its own budget: four crashes and a deadline
    /// are not five of one.
    #[test]
    fn each_kind_has_its_own_budget() {
        let failure = |stage| Failure::new(stage, "x");
        let mut retry = Retry::after(None, failure(FailureStage::CompilerProcess));
        for _ in 1..LIMIT - 1 {
            retry = Retry::after(Some(&retry), failure(FailureStage::CompilerProcess));
        }
        retry = Retry::after(Some(&retry), failure(FailureStage::Deadline));
        assert_eq!(retry.failures(), 1);
        assert!(!retry.exhausted());
        retry = Retry::after(Some(&retry), failure(FailureStage::CompilerProcess));
        assert!(retry.exhausted(), "the fifth crash");
    }
}
