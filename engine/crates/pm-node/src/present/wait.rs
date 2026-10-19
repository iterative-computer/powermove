//! A viewer frame waiting for pixels only the document's window draws: a
//! level 1 SVG bucket (Chromium's `<img>` raster, decision S1) or a
//! program only the WebGL fallback runner draws (decision 19).
//!
//! The frame is not drawn with a stand-in: the view keeps its render-ahead
//! slot and draws that exact frame when the pixels arrive. Every way out is
//! prompt, so a window that cannot answer never freezes the picture:
//!
//! - the pixels arrive (the view draws them);
//! - the window answers that it could not draw them (`SvgFailed`, a refused
//!   SVG band, `ShaderFallbackMessage::Failed`): the frame is drawn at once
//!   without them (an SVG image draws nothing, a runner program draws
//!   nothing or is skipped, as today's compositor treats a layer that
//!   throws or a program WebGL refuses), and transient runner failures are
//!   asked for again after the renderer's backoff;
//! - an edit, a view change, or a transport discontinuity makes the wait
//!   obsolete (the next frame asks for what it needs);
//! - nothing arrives within the bound ([`Bounds`]), counted from when the
//!   pixels were first asked for: the wait times out and the frame is
//!   drawn without them, exactly as for a failure.
//!
//! docs/engine/perf.md, "SVG buckets and runner pixels in the viewer".

use pm_protocol::svg_raster::SvgRasterKey;

/// What a view waits for.
#[derive(Debug, Clone, PartialEq)]
pub enum Awaited {
    /// Level 1 SVG buckets the frame needs.
    Svg(Vec<SvgRasterKey>),
    /// Fallback runner requests (their keys).
    Fallback(Vec<String>),
}

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
    pub on: Awaited,
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

    /// Whether the window's answer for SVG bucket `key` ends this wait.
    pub fn waits_for_svg(&self, key: &SvgRasterKey) -> bool {
        matches!(&self.on, Awaited::Svg(keys) if keys.contains(key))
    }

    /// Whether this wait is for the fallback runner.
    pub fn is_fallback(&self) -> bool {
        matches!(self.on, Awaited::Fallback(_))
    }
}

/// The longest waits (host µs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub svg: i64,
    pub fallback: i64,
}

impl Bounds {
    /// The defaults ([`super::svg::VIEW_WAIT_US`] for SVG buckets; the
    /// runner's own compile and draw deadlines, [`super::svg::ASK_AGAIN_US`],
    /// for runner programs), each capped at `cap` (tests).
    pub fn new(cap: Option<i64>) -> Bounds {
        let cap = cap.unwrap_or(i64::MAX).max(0);
        Bounds {
            svg: super::svg::VIEW_WAIT_US.min(cap),
            fallback: super::svg::ASK_AGAIN_US.min(cap),
        }
    }
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

    fn key(w: u32) -> SvgRasterKey {
        SvgRasterKey {
            content: "ab".repeat(32),
            width: w,
            height: 2,
        }
    }

    fn wait(on: Awaited) -> ExternalWait {
        ExternalWait {
            until: 1_000,
            revision: 3,
            seq: 4,
            epoch: 5,
            on,
        }
    }

    #[test]
    fn a_wait_holds_until_its_deadline_for_the_same_frame_only() {
        let w = wait(Awaited::Svg(vec![key(2)]));
        assert_eq!(w.state(999, 3, 4, 5), State::Waiting);
        assert_eq!(w.state(1_000, 3, 4, 5), State::TimedOut);
        assert_eq!(w.state(0, 4, 4, 5), State::Obsolete, "an edit");
        assert_eq!(w.state(0, 3, 5, 5), State::Obsolete, "a view change");
        assert_eq!(w.state(0, 3, 4, 6), State::Obsolete, "a discontinuity");
    }

    #[test]
    fn answers_end_only_the_waits_they_answer() {
        let svg = wait(Awaited::Svg(vec![key(2), key(4)]));
        assert!(svg.waits_for_svg(&key(4)));
        assert!(!svg.waits_for_svg(&key(8)));
        assert!(!svg.is_fallback());
        let runner = wait(Awaited::Fallback(vec!["k".into()]));
        assert!(runner.is_fallback());
        assert!(!runner.waits_for_svg(&key(2)));
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
    fn bounds_default_to_the_protocol_waits_and_can_be_capped() {
        let b = Bounds::new(None);
        assert_eq!(b.svg, super::super::svg::VIEW_WAIT_US);
        assert_eq!(b.fallback, super::super::svg::ASK_AGAIN_US);
        assert_eq!(
            Bounds::new(Some(300_000)),
            Bounds {
                svg: 300_000,
                fallback: 300_000
            }
        );
    }
}
