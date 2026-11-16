//! Deterministic timing primitives for the Light sketch panel effect.
//! Times are seconds since the corresponding agent event.
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

/// A slot-local brush. Integrating in bounded steps keeps the spring stable
/// when a background window resumes after a long pause.
#[derive(Clone, Copy, Debug, Default)]
pub struct Brush { pub position: [f32; 2], velocity: [f32; 2] }
impl Brush {
    pub fn advance(&mut self, target: [f32; 2], seconds: f32) {
        let mut remaining = seconds.clamp(0., 1.);
        while remaining > 0. {
            let dt = remaining.min(1. / 120.);
            for axis in 0..2 {
                self.velocity[axis] += (120. * (target[axis] - self.position[axis]) - 14. * self.velocity[axis]) * dt;
                self.position[axis] += self.velocity[axis] * dt;
            }
            remaining -= dt;
        }
    }
}

/// Analytic accumulation decay is independent of the rendering frame rate.
pub fn afterglow(age: f32) -> f32 { (-3.712 * age.max(0.)).exp() }

/// Test a dot against the rounded slot before submitting paint primitives.
pub fn inside_slot(point: [f32; 2], size: [f32; 2], radius: f32) -> bool {
    if size.iter().any(|v| !v.is_finite() || *v <= 0.) || point.iter().any(|v| !v.is_finite()) { return false; }
    let r = radius.max(0.).min(size[0].min(size[1]) / 2.);
    if point[0] < 0. || point[1] < 0. || point[0] > size[0] || point[1] > size[1] { return false; }
    let dx = point[0] - point[0].clamp(r, size[0] - r);
    let dy = point[1] - point[1].clamp(r, size[1] - r);
    dx * dx + dy * dy <= r * r
}

#[cfg(test)]
mod brush_tests {
    use super::*;
    #[test]
    fn spring_settles() {
        let mut brush = Brush::default();
        for _ in 0..180 { brush.advance([100., 50.], 1. / 60.); }
        assert!((brush.position[0] - 100.).abs() < 0.01);
        assert!((brush.position[1] - 50.).abs() < 0.01);
    }
    #[test]
    fn rounded_clip() {
        assert!(!inside_slot([0., 0.], [100., 80.], 8.));
        assert!(inside_slot([8., 0.], [100., 80.], 8.));
        assert!(inside_slot([50., 40.], [100., 80.], 8.));
        assert!(!inside_slot([101., 40.], [100., 80.], 8.));
        assert!(!inside_slot([f32::NAN, 0.], [100., 80.], 8.));
    }
    #[test]
    fn decay_matches_reference() {
        assert!((afterglow(1. / 60.) - 0.94).abs() < 0.0001);
        assert!((afterglow(1. / 30.) - afterglow(1. / 60.).powi(2)).abs() < 0.0001);
    }
}

/// Renderer-independent primitives; coordinates are local to the reserved slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Dot { pub center: [f32; 2], pub radius: f32, pub alpha: f32, pub orange: bool }

/// Stable dot field shared by native painting and headless frame assertions.
/// Reduced motion lights the whole planned track without moving a brush.
pub fn draw_dots(size: [f32; 2], age: f32, row: Option<f32>, light: bool, reduced: bool, playing: bool) -> Vec<Dot> {
    let mut dots = Vec::new();
    if size.iter().any(|v| !v.is_finite() || *v <= 0.) { return dots; }
    let dim = playback(playing).0;
    let wake = fade(age, 0.24);
    for y in (6..(size[1]-6.).max(6.).min(4096.) as usize).step_by(12) {
        for x in (6..(size[0]-6.).max(6.).min(4096.) as usize).step_by(12) {
            let center = [x as f32, y as f32];
            if !inside_slot(center, size, 8.) { continue; }
            dots.push(Dot { center, radius: 0.5,
                alpha: dim * (if light { 0.05 } else { 0.06 }) * wake, orange: false });
        }
    }
    if let Some(y) = row {
        dots.extend(sketch_dots(size, y, dim * if reduced { fade(age, 0.12) } else { wake }));
    }
    dots
}

/// Pencil anatomy stays within the same content inset as actual controls.
pub fn sketch_dots(size: [f32; 2], y: f32, strength: f32) -> Vec<Dot> {
    let width = (size[0] - 32.).max(0.);
    let mut dots = Vec::new();
    for (start, end, jitter) in [(0., 0.22, true), (0.38, 0.82, false), (0.88, 0.94, false)] {
        let mut x = 16. + width * start;
        let mut index = 0;
        while x <= 16. + width * end {
            let cy = y + if jitter { (index as f32 * 1.7).sin() } else { 0. };
            if inside_slot([x, cy], size, 8.) {
                dots.push(Dot { center: [x, cy], radius: 0.85, alpha: 0.25 * strength, orange: true });
            }
            index += 1;
            x += if start > 0.8 { (width * 0.03).max(1.) } else { 6. };
        }
    }
    dots
}

#[cfg(test)]
mod draw_tests {
    use super::*;
    #[test]
    fn deterministic_clipped_field() {
        let dots = draw_dots([120., 80.], 0.24, Some(39.), false, false, false);
        assert_eq!(dots, draw_dots([120., 80.], 0.24, Some(39.), false, false, false));
        assert!(dots.iter().all(|d| inside_slot(d.center, [120., 80.], 8.)));
        assert!(dots.iter().any(|d| d.orange && d.alpha == 0.25));
    }
    #[test]
    fn light_and_playback_field_budget() {
        let dots = draw_dots([120., 80.], 1., None, true, true, true);
        assert!(dots.iter().all(|d| (d.alpha - 0.02).abs() < 0.00001));
        assert!(draw_dots([f32::NAN, 80.], 1., None, false, false, false).is_empty());
    }
}

/// Slot-local event clock, retained across GPUI renders.
#[derive(Clone, Debug)]
pub struct State {
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
        Self { stage: Stage::Wake, started: now, changed: now, row: 0, pending: Default::default(), complete: false, committed: Default::default() }
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
        match self.stage {
            Stage::Wake | Stage::Rest if !self.pending.is_empty() && (self.stage == Stage::Rest || age >= 0.24) => {
                let row = self.pending.pop_front().unwrap();
                self.event("plan", row);
            }
            Stage::Sketch if age >= 0.65 => self.event("commit", self.row),
            Stage::Ink if age >= 0.58 => {
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
            s.changed -= std::time::Duration::from_millis(581);
            s.tick();
        }
        assert_eq!(s.stage, Stage::Ready);
    }
}

/// Reduced motion cross-fades the final ink; it never grows or heats a fill.
/// The ink a row is written in: the accent while wet, drying to the theme's
/// text colour.
pub fn ink_color(elapsed: f32, accent: u32, ink: u32, reduced: bool) -> u32 {
    let dry = if reduced { 1. } else { ink_ease((elapsed - 0.28) / 0.3) };
    pm_design::mix(accent, ink, dry.clamp(0., 1.))
}

#[cfg(test)]
mod ink_tests {
    use super::*;
    #[test]
    fn reduced_motion_uses_final_ink_at_full_reveal() {
        let (accent, ink) = (0xf0580a, 0xf2f2f2);
        assert_eq!(ink_color(0.2, accent, ink, true), ink);
        assert_eq!(ink_color(0.2, accent, 0x111111, true), 0x111111);
        assert_eq!(ink_color(0., accent, ink, false), accent);
        assert_eq!(ink_color(0.58, accent, ink, false), ink);
    }
}

#[cfg(test)]
mod anatomy_tests {
    use super::*;
    #[test]
    fn pencil_has_three_inset_parts_and_a_jittered_label() {
        let dots = sketch_dots([300., 260.], 58., 1.);
        assert!(dots.iter().all(|d| d.center[0] >= 16. && d.center[0] <= 284.));
        for (lo, hi) in [(0.,0.22),(0.38,0.82),(0.88,0.94)] {
            assert!(dots.iter().any(|d| (16.+268.*lo..=16.+268.*hi).contains(&d.center[0])));
        }
        assert!(dots.iter().all(|d| (d.center[1]-58.).abs() <= 1.));
    }
}

/// Geometry is shared with the actual control's 38–82% track.
pub fn ink_track(width: f32, fraction: f32, age: f32, reduced: bool) -> [f32; 2] {
    let inner = (width - 32.).max(0.);
    let grow = if reduced { 1. } else { ink_ease(age / 0.28) };
    [16. + inner * 0.38, inner * 0.44 * fraction.clamp(0., 1.) * grow]
}
#[cfg(test)]
mod track_tests {
    use super::*;
    #[test]
    fn ink_ends_at_the_value_fraction_inside_the_control_track() {
        let [x, w] = ink_track(300., 0.5, 0.28, false);
        assert!((x - 117.84).abs() < 0.001);
        assert!((w - 58.96).abs() < 0.001);
        assert_eq!(ink_track(300., 0.5, 0., true), ink_track(300., 0.5, 0.28, true));
    }
}

/// The entire real control fades through the ink; no substitute geometry.
pub fn control_alpha(age: f32, reduced: bool) -> f32 {
    if reduced { reveal(age, true) } else { fade(age, 0.28) }
}
#[cfg(test)]
mod reveal_tests {
    use super::*;
    #[test]
    fn real_controls_fade_over_the_full_commit_window() {
        assert_eq!(control_alpha(0., false), 0.);
        assert!((control_alpha(0.14, false)-0.5).abs() < 0.00001);
        assert_eq!(control_alpha(0.28, false), 1.);
        assert_eq!(control_alpha(0.12, true), 0.);
        assert_eq!(control_alpha(0.28, true), 1.);
    }
}
