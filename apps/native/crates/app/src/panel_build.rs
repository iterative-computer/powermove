//! Deterministic timing primitives for the dot matrix panel creation effect.
//! Times are seconds since the corresponding agent event.
//! Build lifecycle events are shared by generated and extension panel slots.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage { Wake, Sketch, Ink, Rest, Erase, Ready }

pub fn stage(event: &str) -> Option<Stage> {
    Some(match event {
        "start" => Stage::Wake,
        "plan" => Stage::Sketch,
        "commit" => Stage::Ink,
        "rest" => Stage::Rest,
        "remove" | "cancel" => Stage::Erase,
        "ready" => Stage::Ready,
        _ => return None,
    })
}

pub fn fade(seconds: f32, duration: f32) -> f32 {
    (seconds / duration).clamp(0., 1.)
}

/// Solve x(t) before evaluating y(t) for cubic-bezier(.2,.9,.3,1).
pub fn ink_ease(progress: f32) -> f32 {
    let x = progress.clamp(0., 1.);
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..24 {
        let t = (lo + hi) * 0.5;
        let bx = 3. * (1.-t)*(1.-t)*t*0.2 + 3.*(1.-t)*t*t*0.3 + t*t*t;
        if bx < x { lo = t; } else { hi = t; }
    }
    let t = (lo + hi) * 0.5;
    3.*(1.-t)*(1.-t)*t*0.9 + 3.*(1.-t)*t*t + t*t*t
}

pub fn reveal(seconds: f32, reduced: bool) -> f32 {
    if reduced { fade(seconds - 0.120, 0.160) }
    else { fade(seconds - 0.080, 0.200) }
}

pub fn playback(playing: bool) -> (f32, u32) {
    if playing { (0.4, 30) } else { (1., 60) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn events() {
        assert_eq!(stage("plan"), Some(Stage::Sketch));
        assert_eq!(stage("commit"), Some(Stage::Ink));
        assert_eq!(stage("ready"), Some(Stage::Ready));
        assert_eq!(stage("cancel"), Some(Stage::Erase));
        assert_eq!(stage("unknown"), None);
    }
    #[test]
    fn easing() {
        assert!(ink_ease(0.) < 0.00001);
        assert!(ink_ease(1.) > 0.99999);
        assert!(ink_ease(0.5) > 0.85);
        let mut previous = 0.;
        for i in 0..=100 {
            let next = ink_ease(i as f32 / 100.);
            assert!(next >= previous);
            previous = next;
        }
    }
    #[test]
    fn reduced_reveal() {
        assert_eq!(reveal(0.12, true), 0.);
        assert_eq!(reveal(0.28, true), 1.);
        assert_eq!(reveal(0.08, false), 0.);
    }
    #[test]
    fn playback_budget() { assert_eq!(playback(true), (0.4, 30)); }
}

/// Slot-local event clock, retained across GPUI renders.
#[derive(Clone, Debug)]
pub struct State {
    pub explicit: bool,
    pub planned: std::collections::BTreeSet<usize>,
    pub stage: Stage,
    pub started: std::time::Instant,
    pub changed: std::time::Instant,
    pub row: usize,
    pub pending: std::collections::VecDeque<usize>,
    pub complete: bool,
    pub committed: std::collections::BTreeMap<usize, std::time::Instant>,
}
impl Default for State {
    fn default() -> Self {
        let now = std::time::Instant::now();
        Self { explicit: false, planned: Default::default(), stage: Stage::Wake, started: now, changed: now, row: 0, pending: Default::default(), complete: false, committed: Default::default() }
    }
}
impl State {
    pub fn event(&mut self, event: &str, row: usize) {
        if let Some(stage) = stage(event) {
            if stage == Stage::Ink { self.committed.insert(row, std::time::Instant::now()); }
            self.stage = stage;
            self.row = row;
            self.changed = std::time::Instant::now();
        }
    }
}

impl State {
    /// Publications are committed data. Queue every newly published control;
    /// a short plan phase precedes each ink instead of dropping burst arrivals.
    pub fn published(&mut self, rows: impl IntoIterator<Item = usize>, complete: bool) {
        self.pending.extend(rows);
        self.complete = complete;
        if self.stage == Stage::Ready { self.event("start", self.row); }
    }
    pub fn tick(&mut self) {
        let age = self.changed.elapsed().as_secs_f32();
        if self.explicit {
            if self.stage == Stage::Erase || self.stage == Stage::Ready { return; }
            if (self.stage == Stage::Wake && age >= 0.24) || (self.stage == Stage::Ink && age >= 0.58) || self.stage == Stage::Rest || self.stage == Stage::Sketch {
                if let Some(row) = self.pending.pop_front() { self.event("commit", row); return; }
                else if self.complete { self.event("ready", self.row); return; }
                else if !self.planned.is_empty() && self.stage == Stage::Wake { self.event("plan", *self.planned.first().unwrap()); }
            }
            if age >= 2. && self.stage != Stage::Rest && self.pending.is_empty() && !self.complete { self.event("rest", self.row); }
            return;
        }
        match self.stage {
            Stage::Wake | Stage::Rest if !self.pending.is_empty() && (self.stage == Stage::Rest || age >= 0.24) => {
                let row = self.pending.pop_front().unwrap();
                self.event("plan", row);
            }
            Stage::Sketch if age >= 0.65 => self.event("commit", self.row),
            Stage::Ink if age >= 0.82 => {
                if let Some(row) = self.pending.pop_front() { self.event("plan", row); }
                else if self.complete { self.event("ready", self.row); }
                else if age >= 2. { self.event("rest", self.row); }
            }
            Stage::Wake | Stage::Rest if self.complete && self.pending.is_empty() => self.event("ready", self.row),
            _ => {}
        }
    }
}

#[cfg(test)]
mod wiring_tests {
    use super::*;
    #[test]
    fn publications_queue_every_ink_before_ready() {
        let mut s = State::default();
        s.published([0, 1], true);
        s.changed -= std::time::Duration::from_millis(241);
        for row in 0..2 {
            s.tick();
            assert_eq!((s.stage, s.row), (Stage::Sketch, row));
            s.changed -= std::time::Duration::from_millis(651);
            s.tick();
            assert_eq!(s.stage, Stage::Ink);
            s.changed -= std::time::Duration::from_millis(821);
            s.tick();
        }
        assert_eq!(s.stage, Stage::Ready);
    }
}


impl State {
    pub fn resolve(&self, now: std::time::Instant) -> f32 {
        if self.stage == Stage::Ready { pm_design::dots::resolve(now.saturating_duration_since(self.changed).as_secs_f32()) } else { 0. }
    }
    pub fn animating(&self, now: std::time::Instant, reduced: bool) -> bool {
        let elapsed=now.saturating_duration_since(self.changed).as_secs_f32();
        match self.stage {
            Stage::Ready => elapsed < 0.7,
            Stage::Erase => elapsed < 0.24,
            _ => !reduced || !self.pending.is_empty() || self.complete || elapsed < 0.28,
        }
    }
}
#[cfg(test)] mod dots_tests {
    use super::*;
    #[test] fn all_controls_resolve_on_one_clock() {
        let now=std::time::Instant::now(); let mut s=State::default();
        s.committed.insert(0,now-std::time::Duration::from_secs(3));
        s.committed.insert(1,now); assert_eq!(s.resolve(now),0.);
        s.event("ready",1); let middle=s.changed+std::time::Duration::from_millis(350);
        assert!((s.resolve(middle)-0.5).abs()<0.0001);
        assert_eq!(s.resolve(s.changed+std::time::Duration::from_millis(700)),1.);
    }
    #[test] fn idle_schedules_zero_frames() {
        let mut s=State::default();s.event("ready",0);
        assert!(!s.animating(s.changed+std::time::Duration::from_secs(1),false));
        s.event("rest",0); assert!(!s.animating(s.changed+std::time::Duration::from_secs(1),true));
    }
}

#[cfg(test)]
mod explicit_build_tests {
    use super::*;
    fn elapsed(s: &mut State, ms: u64) { s.changed -= std::time::Duration::from_millis(ms); s.tick(); }
    #[test] fn plans_wait_for_real_commits_and_idle_breathes() {
        let mut s = State::default(); s.explicit = true; s.planned.insert(0);
        elapsed(&mut s,241); assert_eq!(s.stage,Stage::Sketch);
        elapsed(&mut s,2100); assert_eq!(s.stage,Stage::Rest); assert!(s.committed.is_empty());
        s.pending.push_back(0); s.tick(); assert_eq!(s.stage,Stage::Ink);
        assert!(s.committed.contains_key(&0));
    }
    #[test] fn extension_handoff_waits_for_first_render_and_queued_ink() {
        let mut s = State::default(); s.explicit = true; s.pending.extend([0,1]);
        elapsed(&mut s,241); assert_eq!(s.row,0);
        elapsed(&mut s,581); assert_eq!(s.row,1);
        elapsed(&mut s,2100); assert_eq!(s.stage,Stage::Rest);
        s.complete = true; s.tick(); assert_eq!(s.stage,Stage::Ready);
    }
    #[test] fn cancel_stops_queued_commits() {
        let mut s = State::default(); s.explicit = true; s.pending.push_back(0);
        s.event("cancel",0); elapsed(&mut s,400);
        assert_eq!(s.stage,Stage::Erase); assert!(s.committed.is_empty());
    }
}
