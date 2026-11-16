//! Timeline-only dot reveal. The panel lane owns the shared sampling and shader.
use crate::{
    RowId,
    render::{Frame, Rect, Scene},
    v4,
};
use pm_design::{Tokens, dots};
use std::collections::VecDeque;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Focus,
    Create,
    Key,
    Set,
    Remove,
    Held,
    Dropped,
    End,
}
#[derive(Clone, Debug)]
pub struct Event {
    pub layer: String,
    pub property: Option<String>,
    pub kind: Kind,
    pub times: Vec<f64>,
    pub step: u64,
    pub committed: bool,
}
#[derive(Clone, Debug)]
struct Active {
    event: Event,
    started: u64,
    committed: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct DotGroup {
    pub bounds: Rect,
    pub indices: Vec<usize>,
    pub detail: f32,
    pub resolve: f32,
    pub pending: bool,
    pub reduced: bool,
}
#[derive(Default)]
pub struct AgentInk {
    events: VecDeque<Active>,
}
const REFINE_MS: u64 = (dots::REFINE_SECONDS * 1000.) as u64;
const RESOLVE_MS: u64 = (dots::RESOLVE_SECONDS * 1000.) as u64;
impl AgentInk {
    pub fn receive(&mut self, event: Event, now: u64) {
        if event.kind == Kind::End {
            self.events.retain(|a| a.event.kind != Kind::Held);
            return;
        }
        if event.kind != Kind::Held {
            self.events
                .retain(|a| a.event.layer != event.layer || a.event.kind != Kind::Held);
        }
        if !matches!(event.kind, Kind::Create | Kind::Key | Kind::Held) {
            return;
        }
        let started = self
            .events
            .iter()
            .find(|a| a.event.step == event.step && a.event.kind != Kind::Held)
            .map_or(now, |a| a.started);
        let committed = event.committed.then_some(now);
        if let Some(old) = self
            .events
            .iter_mut()
            .find(|a| a.event.layer == event.layer && a.event.kind == event.kind)
        {
            *old = Active {
                event,
                started,
                committed,
            };
            return;
        }
        if self.events.len() == 6 {
            self.events.pop_front();
        }
        self.events.push_back(Active {
            event,
            started,
            committed,
        });
    }
    pub fn commit(&mut self, step: u64, now: u64) {
        for a in &mut self.events {
            if a.event.step == step {
                a.committed = Some(now);
            }
        }
    }
    pub fn empty(&self) -> bool {
        self.events.is_empty()
    }
    fn resolve_at(a: &Active) -> Option<u64> {
        a.committed.map(|t| t.max(a.started + REFINE_MS))
    }
    pub fn moving(&mut self, now: u64) -> bool {
        self.moving_with_motion(now, false)
    }
    pub fn moving_with_motion(&mut self, now: u64, reduced: bool) -> bool {
        if reduced {
            self.events.retain(|a| {
                a.event.kind == Kind::Held || a.committed.is_none_or(|t| now < t + 160)
            });
            return self
                .events
                .iter()
                .any(|a| a.event.kind != Kind::Held && a.committed.is_some_and(|t| now < t + 160));
        }
        self.events.retain(|a| {
            a.event.kind == Kind::Held || Self::resolve_at(a).is_none_or(|t| now < t + RESOLVE_MS)
        });
        self.events.iter().any(|a| {
            a.event.kind != Kind::Held
                && (now < a.started + REFINE_MS
                    || Self::resolve_at(a).is_some_and(|t| now < t + RESOLVE_MS))
        })
    }
    /// Indices borrow the ordinary frame; native painting never clones its ops.
    pub fn groups(&self, frame: &Frame, scene: &Scene, t: &Tokens, now: u64) -> Vec<DotGroup> {
        let mut groups: Vec<DotGroup> = Vec::new();
        let v = frame.viewport;
        for a in &self.events {
            let age = now.saturating_sub(a.started) as f32 / 1000.;
            let resolve = Self::resolve_at(a).map_or(0., |at| {
                dots::resolve(now.saturating_sub(at) as f32 / 1000.)
            });
            let resolve = if t.accessibility.reduce_motion {
                a.committed.map_or(0., |at| {
                    dots::resolve(now.saturating_sub(at) as f32 / 160. * dots::RESOLVE_SECONDS)
                })
            } else {
                resolve
            };
            if resolve >= 1. && a.event.kind != Kind::Held {
                continue;
            }
            let clip = format!("clip:{}", a.event.layer);
            let label = format!("clip-content:{}", a.event.layer);
            let mut indices = Vec::new();
            let mut bounds: Option<Rect> = None;
            for (i, op) in frame.ops.iter().enumerate() {
                if op.covered {
                    continue;
                }
                let matches = if a.event.kind == Kind::Create {
                    op.id == clip || op.id == label
                } else if a.event.kind == Kind::Key && op.id.starts_with("key:") {
                    serde_json::from_str::<(String, String)>(&op.id[4..])
                        .ok()
                        .and_then(|(row, _)| v4::parse_row(&row).ok())
                        .is_some_and(|row| {
                            let layer = match row {
                                RowId::Layer(id)
                                | RowId::Property { layer_id: id, .. }
                                | RowId::Effect { layer_id: id, .. } => id,
                            };
                            let time = v.scroll_time
                                + (op.bounds.x + op.bounds.width / 2. - v.layer_column) / v.pps;
                            layer == a.event.layer
                                && (a.event.times.is_empty()
                                    || a.event.times.iter().any(|t| {
                                        (*t - time).abs() <= 0.5 / scene.fps.max(1.) + 0.000_001
                                    }))
                        })
                } else {
                    false
                };
                if !matches {
                    continue;
                }
                let mut b = op.bounds;
                if let Some(c) = op.clip {
                    let x = b.x.max(c.x);
                    let y = b.y.max(c.y);
                    b = Rect::new(
                        x,
                        y,
                        (b.x + b.width).min(c.x + c.width) - x,
                        (b.y + b.height).min(c.y + c.height) - y,
                    );
                }
                let x = b.x.max(v.layer_column);
                let y = b.y.max(t.metrics.ruler as f64);
                b = Rect::new(
                    x,
                    y,
                    (b.x + b.width).min(v.width) - x,
                    (b.y + b.height).min(v.height) - y,
                );
                if b.width <= 0. || b.height <= 0. {
                    continue;
                }
                indices.push(i);
                bounds = Some(match bounds {
                    None => b,
                    Some(old) => {
                        let x = old.x.min(b.x);
                        let y = old.y.min(b.y);
                        Rect::new(
                            x,
                            y,
                            (old.x + old.width).max(b.x + b.width) - x,
                            (old.y + old.height).max(b.y + b.height) - y,
                        )
                    }
                });
            }
            if a.event.kind == Kind::Held {
                let row = v4::row_id(&RowId::Layer(a.event.layer.clone()));
                if let Some(top) = scene.scroll_for(&row) {
                    let y = t.metrics.ruler as f64 + top - v.scroll_y
                        + t.metrics.track_row as f64 / 2.
                        - 1.35;
                    if y >= t.metrics.ruler as f64 && y < v.height {
                        groups.push(DotGroup {
                            bounds: Rect::new(v.layer_column - 8., y, 2.7, 2.7),
                            indices: vec![],
                            detail: 0.,
                            resolve: 1.,
                            pending: true,
                            reduced: true,
                        });
                    }
                }
            } else if let Some(bounds) = bounds {
                let detail = if t.accessibility.reduce_motion {
                    4.
                } else {
                    dots::detail(age)
                };
                // One filter per step bounds all its strip/key members together.
                if let Some(group) = groups
                    .iter_mut()
                    .find(|g| !g.pending && g.detail == detail && g.resolve == resolve)
                {
                    let x = group.bounds.x.min(bounds.x);
                    let y = group.bounds.y.min(bounds.y);
                    group.bounds = Rect::new(
                        x,
                        y,
                        (group.bounds.x + group.bounds.width).max(bounds.x + bounds.width) - x,
                        (group.bounds.y + group.bounds.height).max(bounds.y + bounds.height) - y,
                    );
                    group.indices.extend(indices);
                } else {
                    groups.push(DotGroup {
                        bounds,
                        indices,
                        detail,
                        resolve,
                        pending: false,
                        reduced: t.accessibility.reduce_motion,
                    });
                }
            }
        }
        groups
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: Kind) -> Event {
        Event {
            layer: "L".into(),
            property: None,
            kind,
            times: vec![],
            step: 1,
            committed: true,
        }
    }
    #[test]
    fn no_run_allocates_or_schedules_nothing() {
        let mut s = AgentInk::default();
        assert!(s.empty());
        assert_eq!(s.events.capacity(), 0);
        assert!(!s.moving(0));
    }
    #[test]
    fn idle_has_zero_requested_frames_after_fifteen_seconds() {
        let mut s = AgentInk::default();
        s.receive(event(Kind::Create), 0);
        assert_eq!(
            (15_000..25_000)
                .step_by(50)
                .filter(|t| s.moving(*t))
                .count(),
            0
        );
    }
    #[test]
    fn queue_is_bounded() {
        let mut s = AgentInk::default();
        for n in 0..20 {
            let mut e = event(Kind::Key);
            e.layer = n.to_string();
            s.receive(e, n);
        }
        assert_eq!(s.events.len(), 6);
    }
    #[test]
    fn held_dot_is_static() {
        let mut s = AgentInk::default();
        s.receive(event(Kind::Held), 0);
        assert!(!s.moving(15_000));
        assert!(!s.empty());
    }
    #[test]
    fn dotted_wait_sleeps_until_commit() {
        let mut s = AgentInk::default();
        let mut e = event(Kind::Create);
        e.committed = false;
        s.receive(e, 0);
        assert!(s.moving(400));
        assert!(!s.moving(900));
        assert!(!s.empty());
        s.commit(1, 1000);
        assert!(s.moving(1000));
        assert!(!s.moving(1700));
        assert!(s.empty());
    }
}
