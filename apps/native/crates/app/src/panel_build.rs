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
    for y in (3..size[1].min(4096.) as usize).step_by(6) {
        for x in (3..size[0].min(4096.) as usize).step_by(6) {
            let center = [x as f32, y as f32];
            if !inside_slot(center, size, 8.) { continue; }
            let pencil = row.is_some_and(|r| (center[1] - r).abs() < 3.4);
            let strength = if reduced { fade(age, 0.12) } else { wake };
            dots.push(Dot { center, radius: if pencil { 1.45 } else { 0.9 },
                alpha: dim * if pencil { 0.38 * strength } else { if light { 0.06 } else { 0.08 } * wake },
                orange: pencil });
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
        assert!(dots.iter().any(|d| d.orange && d.alpha == 0.38));
    }
    #[test]
    fn light_and_playback_field_budget() {
        let dots = draw_dots([120., 80.], 1., None, true, true, true);
        assert!(dots.iter().all(|d| (d.alpha - 0.024).abs() < 0.00001));
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
}
impl Default for State {
    fn default() -> Self {
        let now = std::time::Instant::now();
        Self { stage: Stage::Wake, started: now, changed: now, row: 0 }
    }
}
impl State {
    pub fn event(&mut self, event: &str, row: usize) {
        if let Some(stage) = stage(event) {
            self.stage = stage;
            self.row = row;
            self.changed = std::time::Instant::now();
        }
    }
}
