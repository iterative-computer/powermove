//! Program builds that failed for a reason outside their source.

use pm_glsl::Failure;
use std::time::{Duration, Instant};

/// The wait after the first such failure; each consecutive one doubles it.
const FIRST: Duration = Duration::from_millis(100);
/// The longest wait.
const LONGEST: Duration = Duration::from_secs(4);

/// A build that failed because of the compiler (busy, crashed, timed out):
/// never remembered as the source's failure, retried, but no sooner than
/// its backoff, so a compiler that keeps failing is not started again by
/// every draw. A changed source has its own entry and starts afresh.
#[derive(Clone, Debug)]
pub(crate) struct Retry {
    failure: Failure,
    failures: u32,
    after: Instant,
}

impl Retry {
    /// The backoff after `failure`, following `previous` (consecutive
    /// failures of the same source).
    pub(crate) fn after(previous: Option<&Retry>, failure: Failure) -> Retry {
        let failures = previous.map_or(1, |r| r.failures.saturating_add(1));
        let wait = FIRST
            .saturating_mul(1 << (failures - 1).min(16))
            .min(LONGEST);
        Retry {
            failure,
            failures,
            after: Instant::now() + wait,
        }
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
        let busy = || Failure::new(pm_glsl::FailureStage::CompilerBusy, "busy");
        let mut retry = Retry::after(None, busy());
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
            retry = Retry::after(Some(&retry), busy());
        }
        assert_eq!(waits, [100, 200, 400, 800, 1600, 3200, 4000, 4000]);
    }
}
