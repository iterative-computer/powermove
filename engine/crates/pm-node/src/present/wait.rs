//! A viewer frame waiting for pixels only the document's window draws: a
//! program only the WebGL fallback runner draws (decision 19).
//!
//! The frame is not drawn with a stand-in: the view keeps its render-ahead
//! slot and draws that exact frame when the pixels arrive. Every way out is
//! prompt, so a window that cannot answer never freezes the picture:
//!
//! - the pixels arrive (the view draws them);
//! - the window answers that it could not draw them
//!   (`ShaderFallbackMessage::Failed`): the frame is drawn at once without
//!   them (a runner program draws nothing or is skipped, as today's
//!   compositor treats a program WebGL refuses), and transient runner
//!   failures are asked for again after the renderer's backoff;
//! - an edit, a view change, or a transport discontinuity makes the wait
//!   obsolete (the next frame asks for what it needs);
//! - nothing arrives within the bound ([`bound`]), counted from when the
//!   pixels were first asked for: the wait times out and the frame is
//!   drawn without them, exactly as for a failure.
//!
//! docs/engine/perf.md, "Runner pixels in the viewer".

/// The longest a viewer frame waits for the runner (host µs): its own
/// compile and draw deadlines.
pub const RUNNER_WAIT_US: i64 = 10_000_000;

/// A view's frame waiting for its window.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalWait {
    /// Host µs after which the wait times out.
    pub until: i64,
    /// The document revision, view sequence and clock epoch the frame was
    /// drawn for.
    pub revision: u64,
    pub seq: u64,
    pub epoch: u32,
    /// The fallback runner requests (their keys) it waits for.
    pub requests: Vec<String>,
}

/// Where a wait stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Still waiting: nothing to draw yet.
    Waiting,
    /// The frame it waits for is no longer the one to draw.
    Obsolete,
    /// Nothing arrived in time: drawn without what it waited for.
    TimedOut,
}

impl ExternalWait {
    /// The wait at `now` for a view at (`revision`, `seq`, `epoch`).
    pub fn state(&self, now: i64, revision: u64, seq: u64, epoch: u32) -> State {
        if self.revision != revision || self.seq != seq || self.epoch != epoch {
            State::Obsolete
        } else if self.until > now {
            State::Waiting
        } else {
            State::TimedOut
        }
    }
}

/// The longest wait ([`RUNNER_WAIT_US`]), capped at `cap` (tests).
pub fn bound(cap: Option<i64>) -> i64 {
    RUNNER_WAIT_US.min(cap.unwrap_or(i64::MAX).max(0))
}

/// When a wait for pixels asked at `asked` (each `None` when asked now)
/// times out: `bound` after the earliest ask, so drawing the frame again
/// (another answer, a backoff's end) never extends it.
pub fn deadline(asked: impl IntoIterator<Item = Option<i64>>, now: i64, bound: i64) -> i64 {
    asked
        .into_iter()
        .map(|at| at.unwrap_or(now).min(now))
        .min()
        .unwrap_or(now)
        .saturating_add(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_holds_until_its_deadline_for_the_same_frame_only() {
        let w = ExternalWait {
            until: 1_000,
            revision: 3,
            seq: 4,
            epoch: 5,
            requests: vec!["k".into()],
        };
        assert_eq!(w.state(999, 3, 4, 5), State::Waiting);
        assert_eq!(w.state(1_000, 3, 4, 5), State::TimedOut);
        assert_eq!(w.state(0, 4, 4, 5), State::Obsolete, "an edit");
        assert_eq!(w.state(0, 3, 5, 5), State::Obsolete, "a view change");
        assert_eq!(w.state(0, 3, 4, 6), State::Obsolete, "a discontinuity");
    }

    #[test]
    fn the_deadline_counts_from_the_earliest_ask() {
        assert_eq!(deadline([Some(100), None, Some(50)], 200, 10), 60);
        assert_eq!(deadline([None], 200, 10), 210);
        assert_eq!(deadline([], 200, 10), 210);
        // An ask stamped later than now (clock skew) counts from now.
        assert_eq!(deadline([Some(300)], 200, 10), 210);
    }

    #[test]
    fn the_bound_defaults_to_the_runner_wait_and_can_be_capped() {
        assert_eq!(bound(None), RUNNER_WAIT_US);
        assert_eq!(bound(Some(300_000)), 300_000);
    }
}
