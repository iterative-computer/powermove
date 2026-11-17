// Port of distortion-waves 0.1.0 model.ts. Copyright (c) 2026 Jakub Antalik; MIT.
use crate::{WaveStyle, WaveType};
use std::f64::consts::{FRAC_PI_2, PI};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
impl Rect {
    pub fn validate(self) -> Result<(), crate::ValidationError> {
        if [self.x, self.y, self.w, self.h]
            .iter()
            .any(|v| !v.is_finite())
            || self.w <= 0.
            || self.h <= 0.
        {
            return Err(crate::ValidationError(
                "source must be finite with positive dimensions",
            ));
        }
        Ok(())
    }
}
pub(crate) fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
pub fn bezier_ease(curve: [f64; 4], x: f64) -> f64 {
    if x <= 0. {
        return 0.;
    }
    if x >= 1. {
        return 1.;
    }
    let b = |s: f64, a: f64, c: f64| {
        let r = 1. - s;
        3. * r * r * s * a + 3. * r * s * s * c + s * s * s
    };
    let [x1, y1, x2, y2] = curve;
    let mut lo = 0.;
    let mut hi = 1.;
    for _ in 0..24 {
        let s = (lo + hi) / 2.;
        if b(s, x1.clamp(0., 1.), x2.clamp(0., 1.)) < x {
            lo = s;
        } else {
            hi = s;
        }
    }
    b((lo + hi) / 2., y1, y2)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Front {
    pub ox: f64,
    pub oy: f64,
    pub tilt: f64,
    pub spine: f64,
    pub radius: f64,
    pub moved: f64,
    pub squash: f64,
    pub time: f64,
    pub energy: f64,
    pub swell: f64,
    pub width: f64,
    pub height: f64,
    pub gravity: f64,
    pub glow: f64,
    pub clear_radius: f64,
    pub hair_clear_radius: f64,
    pub rim: f64,
    pub sparkle: f64,
    pub flying: f64,
    pub launched: bool,
    pub has_cleared: bool,
    pub ring: bool,
    pub sideways: bool,
    pub hold: f64,
}
impl Front {
    pub fn at(
        progress: f64,
        source: Rect,
        width: f64,
        height: f64,
        s: &WaveStyle,
        focus: Option<Point>,
    ) -> Self {
        let dur = s.duration();
        let t = progress.clamp(0., 1.) * dur;
        let since = t - s.anticipation;
        let gathered = smooth(0., s.anticipation.max(0.001), t);
        let x = (since / s.travel).clamp(0., 1.);
        let moved = bezier_ease(s.easing, x);
        let ring = s.kind == WaveType::Center;
        let sideways = s.kind == WaveType::Horizontal;
        let squash = if ring { 1. } else { 1.4 };
        let mx = source.x + source.w / 2.;
        let my = source.y + source.h / 2.;
        let start = source.w.min(source.h) / 2.;
        let length = if sideways { source.h } else { source.w };
        let spine = if ring {
            0.
        } else {
            (length / 2. - start).max(0.)
        };
        let turned = if sideways { FRAC_PI_2 } else { 0. };
        let ax = turned.cos();
        let ay = turned.sin();
        let lean = if ring {
            0.
        } else {
            focus.map_or(0., |f| {
                let off = if sideways { f.y - my } else { f.x - mx };
                (off / (length / 2.).max(1.)).clamp(-1., 1.)
            })
        };
        let shift = lean * length / 2. * s.orientation;
        let full_lean = lean * s.orientation * 0.45;
        let tilt = turned + full_lean;
        let opening = smooth(0., 0.45, moved);
        let cs = tilt.cos();
        let sn = tilt.sin();
        let mut reach: f64 = 0.;
        for i in 0..4 {
            let dx = if i & 1 != 0 { width } else { 0. } - mx - shift * ax;
            let dy = if i & 2 != 0 { height } else { 0. } - my - shift * ay;
            reach = reach.max(((cs * dx + sn * dy) * squash).hypot(-sn * dx + cs * dy));
        }
        let crossed = reach + (s.swell + 60.) * squash;
        let settling = 1. - smooth(0.2, 0.55, x);
        let launch = start + s.launch_gap.max(0.);
        let radius = if since < 0. {
            launch + s.inhale * (1. - gathered)
        } else {
            launch + (crossed + 100. - launch) * moved
        };
        let glow = (0.5 * s.gather_glow.max(0.)).min(1.);
        let clear_start = start - s.rim_softness;
        let clear_end = reach + (s.rim_softness + s.swell) * squash;
        let facing = smooth(0., 0.1, moved);
        Self {
            ox: mx + shift * facing * ax,
            oy: my + shift * facing * ay,
            tilt: turned + full_lean * facing,
            spine: spine * (1. - opening),
            radius,
            moved,
            squash: 1. + (squash - 1.) * smooth(0., 0.5, moved),
            time: t,
            energy: smooth(0., 0.05, x) * settling,
            swell: s.swell * smooth(0., 0.3, moved),
            width,
            height,
            gravity: s.pull * gathered * (1. - smooth(0., 0.22, since)),
            glow: if since < 0. {
                glow * gathered
            } else {
                (glow + (1. - glow) * smooth(0., 0.07, x)) * settling
            },
            clear_radius: clear_start
                + (clear_end - clear_start)
                    * bezier_ease(
                        [0.42, 0., 0.58, 1.],
                        (since - s.linger) / s.rim_fade.max(0.01),
                    ),
            hair_clear_radius: clear_start
                + (clear_end - clear_start)
                    * bezier_ease(
                        [0.42, 0., 0.58, 1.],
                        (since - s.linger - s.hairline_linger) / s.hairline_fade.max(0.01),
                    ),
            rim: s.rim_strength * smooth(0., 0.1, x) * (1. - smooth(0.9, 1., t / dur)),
            sparkle: smooth(0., s.anticipation.max(0.001) * 0.35, t),
            flying: since.max(0.),
            launched: since > 0.,
            has_cleared: radius > crossed,
            ring,
            sideways,
            hold: if ring { start } else { 0. },
        }
    }
    pub(crate) fn uniforms(self) -> [f32; 19] {
        [
            self.ox,
            self.oy,
            self.spine,
            self.radius,
            self.squash,
            self.time,
            self.energy,
            self.swell,
            self.width,
            self.height,
            self.gravity,
            self.glow,
            self.clear_radius,
            self.rim,
            self.tilt,
            if self.ring { 1. } else { 0. },
            self.hold,
            if self.launched { 1. } else { 0. },
            self.hair_clear_radius,
        ]
        .map(|v| v as f32)
    }
}
pub(crate) fn rim_lumps(time: f64, width: f64, height: f64, r: f64, pulse: f64) -> [f32; 21] {
    let hash = |i: f64, a: f64, b: f64| {
        let v = (i * a + b).sin() * 43758.5453;
        v - v.floor()
    };
    let ex = (width / 2. - r).max(0.);
    let ey = (height / 2. - r).max(0.);
    let perimeter = 4. * (ex + ey + 0.5 * PI * r);
    let mut out = [0.; 21];
    for i in 0..7 {
        let k = i as f64;
        let a = hash(k, 12.9898, 1.3);
        let b = hash(k, 78.233, 4.1);
        let c = hash(k, 39.425, 2.7);
        out[i * 3] =
            (perimeter * (k + 0.35 * a) / 7. + time * (-22. + 44. * b) * (1. + 2. * pulse)) as f32;
        out[i * 3 + 1] = (perimeter / 7. * (0.2 + 0.25 * c)) as f32;
        let breath = (0.25 + 0.3 * pulse) * (time * (1.1 + 1.2 * a) * (1. + pulse) + k * 2.4).sin();
        out[i * 3 + 2] = ((0.2 + 2.6 * b) * (1. + breath)) as f32;
    }
    out
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Particle {
    index: f64,
    column: f64,
    heading: f64,
    pace: f64,
    drift: f64,
    sway: f64,
    sway_rate: f64,
    twinkle_rate: f64,
    phase: f64,
    life: f64,
    size: f64,
    brightness: f64,
    hop: f64,
    hop_offset: f64,
}
impl Particle {
    pub fn new(k: usize, s: &WaveStyle) -> Self {
        let i = (k + 1) as f64;
        let scatter = |step: f64| (i * step) % 1.;
        let small = scatter(0.5497004779);
        Self {
            index: i,
            column: (scatter(0.7548776662) * 2. - 1.) * 0.92,
            heading: scatter(0.7548776662) * 2. * PI,
            pace: 0.35 + 0.72 * scatter(0.8191725134).sqrt(),
            drift: 15. + 50. * scatter(0.6710436067),
            sway: 3. + 11. * scatter(0.4142135624),
            sway_rate: 2.5 + 3.5 * scatter(0.7320508076),
            twinkle_rate: 8. + 12. * scatter(0.2360679775),
            phase: scatter(0.6180339887) * 2. * PI,
            life: s.particle_life * (0.4 + 0.6 * scatter(0.6457513111)),
            size: s.particle_size * (0.55 + 0.45 * small * small),
            brightness: 0.3 + 0.7 * scatter(0.6055512755),
            hop: 0.16 + 0.14 * scatter(0.1231056256),
            hop_offset: scatter(0.472135955),
        }
    }
    pub fn at(self, f: &Front, src: Rect, s: &WaveStyle) -> [f32; 4] {
        let twinkle = 0.5 + 0.5 * (f.time * self.twinkle_rate + self.phase).sin();
        let glint = self.brightness * (0.7 + 0.3 * twinkle);
        let spread = s.particle_spread.clamp(0., 1.);
        let mx = src.x + src.w / 2.;
        let my = src.y + src.h / 2.;
        let hash = |n: f64, salt: f64| {
            let v = (self.index * 12.9898 + n * 78.233 + salt * 37.719).sin() * 43758.5453;
            v - v.floor()
        };
        let (x, y, a) = if !f.launched {
            let hops = f.time / self.hop + self.hop_offset;
            let count = hops.floor();
            let along = hops - count;
            let hopping = smooth(0., 0.25, along) * (1. - smooth(0.55, 1., along));
            let ending = 1. - smooth(s.anticipation * 0.75, s.anticipation, f.time);
            let a = f.sparkle * hopping * ending * glint * s.gather_dots;
            if f.ring {
                let angle = hash(count, 1.) * 2. * PI;
                let nearest = src.w.min(src.h) / 2. + s.particle_gap + s.particle_flight;
                let away = nearest + (s.particle_reach - nearest).max(0.) * hash(count, 2.);
                let d = away - s.particle_flight * along * along;
                (mx + angle.cos() * d, my + angle.sin() * d, a)
            } else {
                let half = if f.sideways { src.h } else { src.w } / 2.;
                let face = if f.sideways { src.w } else { src.h } / 2.;
                let x = (hash(count, 1.) * 2. - 1.) * (half + 24.);
                let nearest = s.particle_gap + s.particle_flight;
                let away = nearest + (s.particle_reach - nearest).max(0.) * hash(count, 2.);
                let corner = src.w.min(src.h) / 2.;
                let far = face + away;
                let landing = x.clamp(-half + corner, half - corner);
                let sx = if f.sideways { mx + far } else { mx + x };
                let sy = if f.sideways { my + x } else { my - far };
                let dx = if f.sideways { mx + face } else { mx + landing } - sx;
                let dy = if f.sideways { my + landing } else { my - face } - sy;
                let len = dx.hypot(dy).max(1.);
                let travelled = s.particle_flight * along * along;
                (sx + dx / len * travelled, sy + dy / len * travelled, a)
            }
        } else {
            let burning = 1. - smooth(self.life * 0.55, self.life, f.flying);
            let a = burning * glint;
            let sway = self.sway
                * (self.sway_rate * f.flying + self.phase).sin()
                * (f.flying / 0.25).min(1.);
            if f.ring {
                let edge = src.w.min(src.h) / 2.;
                let d = edge + ((f.radius - edge) * self.pace + self.drift * f.flying) * spread;
                let dx = self.heading.cos();
                let dy = self.heading.sin();
                (f.ox + dx * d - dy * sway, f.oy + dy * d + dx * sway, a)
            } else {
                let start = src.w.min(src.h) / 2.;
                let widening = 1. + 0.22 * f.moved * self.pace * spread;
                let xx =
                    self.column * if f.sideways { src.h } else { src.w } / 2. * widening + sway;
                let h = (start + (f.radius - start) * self.pace * spread)
                    * (1. - 0.3 * self.column * self.column)
                    + self.drift * f.flying * spread;
                let cs = f.tilt.cos();
                let sn = f.tilt.sin();
                (f.ox + cs * xx + sn * h, f.oy + sn * xx - cs * h, a)
            }
        };
        [
            x as f32,
            y as f32,
            ((self.size * 10.).floor() + a.min(0.999)) as f32,
            0.,
        ]
    }
}

/// Interruptible clock. Finishing yields one event; scrub and interrupted runs do not.
#[derive(Clone, Debug, Default)]
pub struct WaveClock {
    progress: f64,
    target: Option<bool>,
    finish: Option<bool>,
    from: f64,
    elapsed: f64,
}
impl WaveClock {
    pub fn progress(&self) -> f64 {
        self.progress
    }
    pub fn is_running(&self) -> bool {
        self.target.is_some()
    }
    pub fn set_revealed(&mut self, revealed: bool) {
        self.finish = None;
        self.from = self.progress;
        self.elapsed = 0.;
        if self.progress != if revealed { 1. } else { 0. } {
            self.target = Some(revealed);
        } else {
            self.target = None;
        }
    }
    pub fn play(&mut self) {
        self.progress = 0.;
        self.set_revealed(true);
    }
    pub fn reverse(&mut self) {
        self.set_revealed(false);
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn scrub(&mut self, value: f64) -> Result<(), crate::ValidationError> {
        if !value.is_finite() {
            return Err(crate::ValidationError("scrub must be finite"));
        }
        self.progress = value.clamp(0., 1.);
        self.target = None;
        self.finish = None;
        Ok(())
    }
    pub fn tick(&mut self, seconds: f64, s: &WaveStyle, reduced_motion: bool) {
        let Some(target) = self.target else { return };
        if !seconds.is_finite() || seconds <= 0. {
            return;
        }
        let to = if target { 1. } else { 0. };
        let duration = if reduced_motion {
            0.3 * (to - self.from).abs()
        } else if target {
            s.duration()
        } else {
            s.duration() / s.return_speed.max(0.05)
        };
        self.elapsed += seconds;
        let x = if duration > 0. {
            self.elapsed / duration
        } else {
            1.
        };
        let eased = if reduced_motion || target {
            x.clamp(0., 1.)
        } else {
            bezier_ease([0.22, 1., 0.36, 1.], x)
        };
        self.progress = self.from + (to - self.from) * eased;
        if x >= 1. {
            self.progress = to;
            self.target = None;
            self.finish = Some(target);
        }
    }
    pub fn take_finish(&mut self) -> Option<bool> {
        self.finish.take()
    }
}
