//! Fly navigation (Blender's View ▸ Navigation ▸ Fly, Shift+`): the view
//! flies continuously; the pointer's offset from the viewer's centre steers
//! (further out turns faster, a small dead zone in the middle holds course);
//! W/S and the scroll wheel change the forward speed, which persists; A/D
//! strafe, E/Q (and R/F) climb and descend along world vertical; Shift is
//! faster, Option slower. Pure state and maths: the viewer feeds it pointer,
//! keys and time, and reads the pose.
use pm_scene3d::navigation::{Pose, Vec3, add, cross, length, mul, normalize, sub};

/// Degrees per second at the edge of the viewer.
pub const MAX_TURN: f64 = 90.;
/// The share of the half-viewer around the centre that doesn't steer.
pub const DEAD_ZONE: f64 = 0.08;
/// Steepest pitch, degrees (looking straight up or down has no heading).
const MAX_PITCH: f64 = 89.;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Held {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fly {
    /// The pose the flight started from: Cancel returns here exactly.
    pub start: Pose,
    pub position: Vec3,
    /// Heading and pitch, degrees, as a camera's rotation y and x.
    pub yaw: f64,
    pub pitch: f64,
    /// Forward speed, scene units per second (negative flies backward).
    pub speed: f64,
    /// One speed step, and the strafe/climb speed at rest: scaled to the
    /// scene so a product and a landscape both feel right.
    pub step: f64,
    /// Where the view looks, as the pose's target distance.
    pub focus: f64,
    /// The pointer's offset from the viewer centre, −1…1 on each axis
    /// (x right, y down), in the viewer's own proportions.
    pub pointer: [f64; 2],
    pub held: Held,
    pub fast: bool,
    pub slow: bool,
}

/// The view direction for (pitch, yaw) degrees: −Z turned by pitch about X,
/// then yaw about Y, as a camera's rotation x and y compose.
pub fn forward(pitch: f64, yaw: f64) -> Vec3 {
    let (a, b) = (pitch.to_radians(), yaw.to_radians());
    [-a.cos() * b.sin(), a.sin(), -a.cos() * b.cos()]
}
/// (pitch, yaw) degrees looking along `f`.
pub fn angles(f: Vec3) -> (f64, f64) {
    let f = normalize(f);
    (f[1].clamp(-1., 1.).asin().to_degrees(), (-f[0]).atan2(-f[2]).to_degrees())
}
fn steer(offset: f64) -> f64 {
    let o = offset.clamp(-1., 1.);
    if o.abs() <= DEAD_ZONE {
        return 0.;
    }
    // Linear beyond the dead zone: further from the centre turns faster.
    o.signum() * (o.abs() - DEAD_ZONE) / (1. - DEAD_ZONE)
}

impl Fly {
    pub fn new(start: Pose) -> Self {
        let d = sub(start.target, start.position);
        let focus = length(d).max(1e-3);
        let (pitch, yaw) = angles(d);
        Self {
            start,
            position: start.position,
            yaw,
            pitch,
            speed: 0.,
            // A tenth of the distance to what it looked at, per second.
            step: (focus * 0.1).max(1e-3),
            focus,
            pointer: [0.; 2],
            held: Held::default(),
            fast: false,
            slow: false,
        }
    }
    fn gear(&self) -> f64 {
        if self.fast {
            4.
        } else if self.slow {
            0.25
        } else {
            1.
        }
    }
    /// W or a scroll notch away (+1), S or toward (−1): one speed step.
    pub fn accelerate(&mut self, notches: f64) {
        self.speed += notches * self.step;
        if self.speed.abs() < self.step * 1e-6 {
            self.speed = 0.;
        }
    }
    /// Advances `dt` seconds.
    pub fn tick(&mut self, dt: f64) {
        let dt = dt.clamp(0., 0.1);
        self.yaw -= steer(self.pointer[0]) * MAX_TURN * dt;
        self.pitch = (self.pitch - steer(self.pointer[1]) * MAX_TURN * dt).clamp(-MAX_PITCH, MAX_PITCH);
        let f = forward(self.pitch, self.yaw);
        let gear = self.gear();
        self.position = add(self.position, mul(f, self.speed * gear * dt));
        // Strafe on the level and climb along world vertical, at the current
        // speed or one step when flying slowly or not at all.
        let pace = self.speed.abs().max(self.step) * gear * dt;
        let right = normalize(cross(f, [0., 1., 0.]));
        let side = f64::from(self.held.right as u8) - f64::from(self.held.left as u8);
        let lift = f64::from(self.held.up as u8) - f64::from(self.held.down as u8);
        if length(right) > 1e-9 {
            self.position = add(self.position, mul(right, side * pace));
        }
        self.position = add(self.position, [0., lift * pace, 0.]);
    }
    /// The view now: where it stands and what it looks at.
    pub fn pose(&self) -> Pose {
        Pose {
            position: self.position,
            target: add(self.position, mul(forward(self.pitch, self.yaw), self.focus)),
            zoom: self.start.zoom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn start() -> Pose {
        Pose { position: [0., 0., 10.], target: [0., 0., 0.], zoom: 1. }
    }
    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }
    #[test]
    fn entering_keeps_the_view_exactly() {
        let f = Fly::new(Pose { position: [3., 2., 6.], target: [0.5, 0.2, -1.], zoom: 1. });
        let p = f.pose();
        assert!(close(p.position, [3., 2., 6.]));
        assert!(close(p.target, [0.5, 0.2, -1.]), "{:?}", p.target);
    }
    #[test]
    fn forward_follows_the_view_and_speed_persists() {
        let mut f = Fly::new(start());
        f.accelerate(2.);
        for _ in 0..10 {
            f.tick(0.1);
        }
        // Looking down −Z at two steps (2 × 1.0) per second for 1 s.
        assert!(close(f.position, [0., 0., 8.]), "{:?}", f.position);
        f.tick(0.1);
        assert!(f.position[2] < 8., "it keeps flying");
        f.accelerate(-2.);
        let at = f.position;
        f.tick(0.1);
        assert!(close(f.position, at), "speed back to zero stops it");
    }
    #[test]
    fn climbing_is_world_vertical_even_looking_down() {
        let mut f = Fly::new(Pose { position: [0., 5., 5.], target: [0., 0., 0.], zoom: 1. });
        f.held.up = true;
        let at = f.position;
        f.tick(0.1);
        let d = sub(f.position, at);
        assert!(d[0].abs() < 1e-12 && d[2].abs() < 1e-12 && d[1] > 0., "{d:?}");
    }
    #[test]
    fn the_pointer_steers_harder_further_out_and_not_in_the_dead_zone() {
        let mut f = Fly::new(start());
        f.pointer = [DEAD_ZONE * 0.5, 0.];
        f.tick(0.1);
        assert_eq!(f.yaw, 0.);
        let mut near = Fly::new(start());
        near.pointer = [0.4, 0.];
        near.tick(0.1);
        let mut far = Fly::new(start());
        far.pointer = [0.9, 0.];
        far.tick(0.1);
        // Right of centre turns right (yaw decreases), further turns faster.
        assert!(near.yaw < 0. && far.yaw < near.yaw, "{} {}", near.yaw, far.yaw);
    }
    #[test]
    fn shift_is_faster_and_option_slower() {
        let mut a = Fly::new(start());
        let mut b = a.clone();
        let mut c = a.clone();
        for f in [&mut a, &mut b, &mut c] {
            f.accelerate(1.);
        }
        b.fast = true;
        c.slow = true;
        for f in [&mut a, &mut b, &mut c] {
            f.tick(0.1);
        }
        let moved = |f: &Fly| 10. - f.position[2];
        assert!(moved(&b) > moved(&a) && moved(&a) > moved(&c));
    }
}
