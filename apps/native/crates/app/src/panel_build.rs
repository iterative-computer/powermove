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
