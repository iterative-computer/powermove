//! Event-driven toolbar agent status. Idle never schedules animation frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub step: String,
    pub started_at: u64,
    pub attention: bool,
    pub running: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub tasks: Vec<Task>,
    pub panel_open: bool,
    pub revision: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Idle,
    Working,
    Done,
    Attention,
}
#[derive(Clone, Debug, Default)]
pub struct Launcher {
    pub snapshot: Snapshot,
    pub unseen: bool,
    pub receiving_at: Option<u64>,
    pub done_at: Option<u64>,
    seen_attention: Vec<String>,
    initialized: bool,
}
impl Launcher {
    pub fn update(&mut self, next: Snapshot, now: u64) {
        if self
            .snapshot
            .tasks
            .iter()
            .any(|old| old.running && !next.tasks.iter().any(|t| t.id == old.id && t.running))
        {
            self.done_at = Some(now);
            self.unseen = !next.panel_open;
        }
        if ((next.revision != self.snapshot.revision
            && (self.initialized || next.tasks.iter().any(|t| t.running)))
            || (self.snapshot.panel_open && !next.tasks.is_empty()))
            && !next.panel_open
        {
            self.receiving_at = Some(now);
        }
        self.seen_attention
            .retain(|id| next.tasks.iter().any(|t| t.id == *id && t.attention));
        if next.panel_open {
            for task in next.tasks.iter().filter(|t| t.attention) {
                if !self.seen_attention.contains(&task.id) {
                    self.seen_attention.push(task.id.clone());
                }
            }
            self.unseen = false;
            self.receiving_at = None;
        }
        self.initialized = true;
        self.snapshot = next;
    }
    pub fn tone(&self) -> Tone {
        if self.snapshot.tasks.iter().any(|t| t.attention) {
            Tone::Attention
        } else if self.running_count() > 0 {
            Tone::Working
        } else if self.unseen {
            Tone::Done
        } else {
            Tone::Idle
        }
    }
    pub fn requests_frames(&self, now: u64, reduced: bool) -> bool {
        !reduced
            && (self.tone() == Tone::Working
                || (self.tone() == Tone::Attention
                    && self.running_count() > 0
                    && self.attention_unseen())
                || self.done_at.is_some_and(|at| now.saturating_sub(at) < 360)
                || self
                    .receiving_at
                    .is_some_and(|at| now.saturating_sub(at) < 420))
    }
    pub fn attention_unseen(&self) -> bool {
        self.snapshot
            .tasks
            .iter()
            .any(|t| t.attention && !self.seen_attention.contains(&t.id))
    }
    pub fn tooltip(&self, now: u64) -> String {
        let label = match self.tone() {
            Tone::Idle => "Ask the agent (⌘⇧K)",
            Tone::Working => "Agent is working",
            Tone::Done => "Agent finished · Unseen",
            Tone::Attention => "Agent needs your answer",
        };
        let mut s = label.to_owned();
        if let Some(t) = self.snapshot.tasks.iter().find(|t| t.running) {
            s += &format!(", running {}", elapsed(now.saturating_sub(t.started_at)));
        }
        if self.running_count() > 1 {
            s += &format!(", {} tasks running", self.running_count());
        }
        s
    }
    pub fn running_count(&self) -> usize {
        self.snapshot.tasks.iter().filter(|t| t.running).count()
    }
    pub fn popover_rows(&self, now: u64) -> Vec<(String, String, String)> {
        self.snapshot
            .tasks
            .iter()
            .filter(|t| t.running)
            .map(|t| {
                (
                    t.id.clone(),
                    if t.title.is_empty() {
                        "New thread".into()
                    } else {
                        t.title.clone()
                    },
                    format!(
                        "{} · {}",
                        if t.step.is_empty() {
                            "Working…"
                        } else {
                            &t.step
                        },
                        elapsed(now.saturating_sub(t.started_at))
                    ),
                )
            })
            .collect()
    }
}
pub fn elapsed(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {}s", s / 60, s % 60)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn task(id: &str) -> Task {
        Task {
            id: id.into(),
            title: format!("Task {id}"),
            started_at: 1000,
            running: true,
            ..Default::default()
        }
    }
    #[test]
    fn idle_requests_zero_frames() {
        let l = Launcher::default();
        for now in [0, 1000, u64::MAX] {
            assert!(!l.requests_frames(now, false));
        }
    }
    #[test]
    fn restored_idle_history_requests_zero_frames() {
        let mut launcher = Launcher::default();
        launcher.update(
            Snapshot {
                revision: 50,
                ..Default::default()
            },
            1000,
        );
        assert_eq!(launcher.receiving_at, None);
        assert!(!launcher.requests_frames(1000, false));
    }
    #[test]
    fn running_and_multi_task() {
        let mut l = Launcher::default();
        l.update(
            Snapshot {
                tasks: vec![task("a"), task("b")],
                ..Default::default()
            },
            1000,
        );
        assert_eq!(l.tone(), Tone::Working);
        assert!(l.requests_frames(2000, false));
        assert!(l.tooltip(81000).contains("running 1m 20s, 2 tasks running"));
        assert!(!l.requests_frames(2000, true));
    }
    #[test]
    fn done_unseen_then_seen_stops() {
        let mut l = Launcher::default();
        l.update(
            Snapshot {
                tasks: vec![task("a")],
                ..Default::default()
            },
            1000,
        );
        l.update(Snapshot::default(), 2000);
        assert_eq!(l.tone(), Tone::Done);
        assert!(l.unseen);
        assert!(l.requests_frames(2100, false));
        assert!(!l.requests_frames(2360, false));
        l.update(
            Snapshot {
                panel_open: true,
                ..Default::default()
            },
            2500,
        );
        assert_eq!(l.tone(), Tone::Idle);
        assert!(!l.requests_frames(2500, false));
    }
    #[test]
    fn attention_pulse_stops_when_seen() {
        let mut l = Launcher::default();
        let mut t = task("a");
        t.attention = true;
        l.update(
            Snapshot {
                tasks: vec![t],
                ..Default::default()
            },
            0,
        );
        assert_eq!(l.tone(), Tone::Attention);
        assert!(l.requests_frames(1000, false));
        let mut s = l.snapshot.clone();
        s.panel_open = true;
        l.update(s, 1000);
        assert!(!l.requests_frames(1000, false));
        let mut closed = l.snapshot.clone();
        closed.panel_open = false;
        l.update(closed, 1100);
        assert!(!l.attention_unseen());
        assert!(!l.requests_frames(1520, false));
    }
    #[test]
    fn consent_without_a_run_does_not_drive_frames() {
        let mut l = Launcher::default();
        l.update(
            Snapshot {
                tasks: vec![Task {
                    attention: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            0,
        );
        assert_eq!(l.tone(), Tone::Attention);
        assert_eq!(l.running_count(), 0);
        assert!(l.popover_rows(1000).is_empty());
        assert!(!l.requests_frames(1000, false));
    }
    #[test]
    fn popover_contents_and_receiving() {
        let mut l = Launcher::default();
        l.update(
            Snapshot {
                tasks: vec![task("a")],
                revision: 1,
                ..Default::default()
            },
            1000,
        );
        assert_eq!(
            l.popover_rows(81000),
            vec![("a".into(), "Task a".into(), "Working… · 1m 20s".into())]
        );
        assert_eq!(l.receiving_at, Some(1000));
        l.update(
            Snapshot {
                panel_open: true,
                ..Default::default()
            },
            2000,
        );
        assert_eq!(l.receiving_at, None);
    }
}

/// Electron's cubic-bezier(0.16, 1, 0.3, 1), inverted on x.
pub fn ease_out(progress: f64) -> f64 {
    let x = progress.clamp(0., 1.);
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..20 {
        let u = (lo + hi) * 0.5;
        let v = 1. - u;
        let at = 3. * v * v * u * 0.16 + 3. * v * u * u * 0.3 + u * u * u;
        if at < x {
            lo = u;
        } else {
            hi = u;
        }
    }
    let u = (lo + hi) * 0.5;
    1. - (1. - u).powi(3)
}

/// CSS ease-in-out used by the 1.4 second attention pulse.
pub fn pulse_opacity(progress: f64) -> f64 {
    let phase = progress.fract();
    let x = if phase < 0.5 {
        phase * 2.
    } else {
        (1. - phase) * 2.
    };
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..20 {
        let u = (lo + hi) * 0.5;
        let v = 1. - u;
        let at = 3. * v * v * u * 0.42 + 3. * v * u * u * 0.58 + u * u * u;
        if at < x {
            lo = u;
        } else {
            hi = u;
        }
    }
    let u = (lo + hi) * 0.5;
    let y = 3. * (1. - u) * u * u + u * u * u;
    1. - 0.65 * y
}
