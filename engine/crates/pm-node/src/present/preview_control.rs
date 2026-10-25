//! Tracks explicit paused-preview requests for each viewer session. The
//! render thread owns this state; presented frames may complete only the
//! still-current generation.

use std::collections::BTreeMap;

use pm_protocol::common::{Revision, Seq, Ticks};
use pm_protocol::preview::{PreviewAction, PreviewControl, PreviewPhase, PreviewStatus};

use super::render::Key;

#[derive(Debug, Clone)]
struct Pending {
    control: PreviewControl,
}

pub struct PresentedFrame<'a> {
    pub revision: Revision,
    pub composition: &'a str,
    pub time: Ticks,
    pub seq: Seq,
    pub generation: Seq,
}

#[derive(Default)]
pub struct PreviewControls {
    pending: BTreeMap<Key, Pending>,
    last_generation: BTreeMap<Key, Seq>,
}

impl PreviewControls {
    pub fn control(&mut self, key: Key, control: PreviewControl) -> Option<PreviewStatus> {
        if self
            .last_generation
            .get(&key)
            .is_some_and(|last| *last >= control.generation)
        {
            return None;
        }
        self.last_generation.insert(key, control.generation);
        let phase = match control.action {
            PreviewAction::Render => PreviewPhase::Queued,
            PreviewAction::Cancel => PreviewPhase::Cancelled,
        };
        let status = status(&control, phase, None, None);
        if control.action == PreviewAction::Cancel {
            self.pending.remove(&key);
        } else {
            self.pending.insert(key, Pending { control });
        }
        Some(status)
    }

    /// Called for the descriptor of the frame actually sent to this viewer.
    pub fn presented(&mut self, key: Key, frame: PresentedFrame<'_>) -> Option<PreviewStatus> {
        let p = self.pending.get(&key)?;
        if p.control.cache_only
            || frame.generation != p.control.generation
            || frame.revision < p.control.revision
            || frame.composition != p.control.composition.as_str()
            || frame.time != p.control.time
        {
            return None;
        }
        let p = self.pending.remove(&key)?;
        Some(status(
            &p.control,
            PreviewPhase::Ready,
            Some(frame.seq),
            None,
        ))
    }

    /// A cache-only preview is ready without presenting a viewer frame.
    pub fn ready_cache(
        &mut self,
        key: Key,
        revision: Revision,
        generation: Seq,
    ) -> Option<PreviewStatus> {
        let p = self.pending.get(&key)?;
        if !p.control.cache_only
            || p.control.generation != generation
            || revision < p.control.revision
        {
            return None;
        }
        let p = self.pending.remove(&key)?;
        Some(status(&p.control, PreviewPhase::Ready, None, None))
    }

    pub fn failed(
        &mut self,
        key: Key,
        revision: Revision,
        generation: Option<Seq>,
        error: String,
    ) -> Option<PreviewStatus> {
        let p = self.pending.get(&key)?;
        if revision < p.control.revision || generation != Some(p.control.generation) {
            return None;
        }
        let p = self.pending.remove(&key)?;
        Some(status(&p.control, PreviewPhase::Failed, None, Some(error)))
    }

    pub fn cancelled(&mut self, key: Key) -> Option<PreviewStatus> {
        let p = self.pending.remove(&key)?;
        Some(status(&p.control, PreviewPhase::Cancelled, None, None))
    }

    pub fn forget(&mut self, key: Key) {
        self.pending.remove(&key);
        self.last_generation.remove(&key);
    }

    pub fn current(&self, key: Key) -> Option<&PreviewControl> {
        self.pending.get(&key).map(|p| &p.control)
    }
}

fn status(
    control: &PreviewControl,
    phase: PreviewPhase,
    frame_seq: Option<Seq>,
    error: Option<String>,
) -> PreviewStatus {
    PreviewStatus {
        generation: control.generation,
        automatic: false,
        revision: control.revision,
        composition: control.composition.clone(),
        time: control.time,
        phase,
        frame_seq,
        error,
        cache_only: control.cache_only,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{CompositionId, SessionId};
    use pm_protocol::host::PortId;

    fn key() -> Key {
        (PortId(1), SessionId::new(2).unwrap())
    }
    fn control(generation: u64, revision: u64, action: PreviewAction) -> PreviewControl {
        PreviewControl {
            generation: Seq::new(generation).unwrap(),
            revision: Revision::new(revision).unwrap(),
            composition: CompositionId::new("comp"),
            time: Ticks::new(42).unwrap(),
            shading: pm_protocol::frame::Shading3d::Rendered,
            action,
            cache_only: false,
            size: None,
            camera: None,
        }
    }
    fn frame(generation: u64, revision: u64, seq: u64) -> PresentedFrame<'static> {
        PresentedFrame {
            revision: Revision::new(revision).unwrap(),
            composition: "comp",
            time: Ticks::new(42).unwrap(),
            seq: Seq::new(seq).unwrap(),
            generation: Seq::new(generation).unwrap(),
        }
    }

    #[test]
    fn old_generation_cannot_complete_restarted_preview() {
        let mut tracker = PreviewControls::default();
        tracker
            .control(key(), control(1, 3, PreviewAction::Render))
            .unwrap();
        tracker
            .control(key(), control(2, 4, PreviewAction::Render))
            .unwrap();
        assert!(tracker.presented(key(), frame(1, 3, 5)).is_none());
        let ready = tracker.presented(key(), frame(2, 4, 6)).unwrap();
        assert_eq!(ready.generation.get(), 2);
        assert_eq!(ready.frame_seq.unwrap().get(), 6);
    }

    #[test]
    fn cache_ready_needs_current_generation_and_never_claims_a_frame() {
        let mut tracker = PreviewControls::default();
        let mut first = control(1, 3, PreviewAction::Render);
        first.cache_only = true;
        tracker.control(key(), first).unwrap();
        assert!(tracker.presented(key(), frame(1, 3, 5)).is_none());
        let mut second = control(2, 4, PreviewAction::Render);
        second.cache_only = true;
        tracker.control(key(), second).unwrap();
        assert!(
            tracker
                .ready_cache(key(), Revision::new(3).unwrap(), Seq::new(1).unwrap())
                .is_none()
        );
        let ready = tracker
            .ready_cache(key(), Revision::new(4).unwrap(), Seq::new(2).unwrap())
            .unwrap();
        assert_eq!(ready.phase, PreviewPhase::Ready);
        assert!(ready.cache_only);
        assert!(ready.frame_seq.is_none());
        assert!(tracker.current(key()).is_none());
    }

    #[test]
    fn cancel_and_failure_end_only_current_request() {
        let mut tracker = PreviewControls::default();
        tracker
            .control(key(), control(1, 3, PreviewAction::Render))
            .unwrap();
        assert_eq!(
            tracker
                .control(key(), control(2, 3, PreviewAction::Cancel))
                .unwrap()
                .phase,
            PreviewPhase::Cancelled
        );
        assert!(
            tracker
                .failed(
                    key(),
                    Revision::new(3).unwrap(),
                    Some(Seq::new(1).unwrap()),
                    "old".into()
                )
                .is_none()
        );
        tracker
            .control(key(), control(3, 4, PreviewAction::Render))
            .unwrap();
        assert_eq!(
            tracker
                .failed(
                    key(),
                    Revision::new(4).unwrap(),
                    Some(Seq::new(3).unwrap()),
                    "The preview could not be rendered".into()
                )
                .unwrap()
                .phase,
            PreviewPhase::Failed
        );
    }
}
