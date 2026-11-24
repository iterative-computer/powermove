//! Direct Walk/Fly navigation (Shift+`): move only while WASD / Arrow / Q / E
//! keys are held (no cruise-control glide after release) and look directly with
//! mouse movement deltas (no drift when the pointer stops). Scroll wheel or
//! +/- adjusts movement speed; Shift is faster, Option is slower. Pure state
//! and maths: the viewer feeds pointer deltas, keys, and time, and reads the
//! pose.
use pm_scene3d::navigation::{Pose, Vec3, add, cross, length, mul, normalize, sub};

/// Degrees turned per pixel of pointer movement.
pub const LOOK_SENSITIVITY: f64 = 0.22;
/// Steepest pitch, degrees (looking straight up or down has no heading).
const MAX_PITCH: f64 = 89.;
/// Minimum and maximum user speed multiplier (adjusted by scroll / +/-).
const MIN_SPEED_SCALE: f64 = 0.05;
const MAX_SPEED_SCALE: f64 = 25.;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Held {
    keys: u16,
}

impl Held {
    pub fn set(&mut self, key: &str, pressed: bool) {
        let bit = match key {
            "a" => 0,
            "left" => 1,
            "d" => 2,
            "right" => 3,
            "e" => 4,
            "r" => 5,
            "q" => 6,
            "f" => 7,
            "w" => 8,
            "up" => 9,
            "s" => 10,
            "down" => 11,
            _ => return,
        };
        let mask = 1u16 << bit;
        self.keys = if pressed {
            self.keys | mask
        } else {
            self.keys & !mask
        };
    }
    pub fn any(&self) -> bool {
        self.keys != 0
    }
    fn active(&self, mask: u16) -> f64 {
        f64::from(u8::from(self.keys & mask != 0))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fly {
    /// The pose the flight started from: Cancel returns here exactly.
    pub start: Pose,
    pub position: Vec3,
    /// Heading and pitch, degrees, as a camera's rotation y and x.
    pub yaw: f64,
    pub pitch: f64,
    /// User-adjustable movement speed multiplier (scroll wheel / +/-).
    pub speed_scale: f64,
    /// Base movement speed in scene units per second, scaled to the scene.
    pub step: f64,
    /// Where the view looks, as the pose's target distance.
    pub focus: f64,
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
    (
        f[1].clamp(-1., 1.).asin().to_degrees(),
        (-f[0]).atan2(-f[2]).to_degrees(),
    )
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
            speed_scale: 1.,
            // Responsive pace scaled to scene focus distance.
            step: (focus * 0.45).max(0.05),
            focus,
            held: Held::default(),
            fast: false,
            slow: false,
        }
    }
    fn gear(&self) -> f64 {
        if self.fast {
            3.
        } else if self.slow {
            0.25
        } else {
            1.
        }
    }
    /// Scroll wheel or +/- adjusts movement speed multiplier cleanly without
    /// imparting any persistent velocity or glide.
    pub fn adjust_speed(&mut self, notches: f64) {
        self.speed_scale = (self.speed_scale * 1.2f64.powf(notches))
            .clamp(MIN_SPEED_SCALE, MAX_SPEED_SCALE);
    }
    /// Direct mouse-look from pointer movement in pixels: stops turning the
    /// moment the pointer stops moving.
    pub fn look(&mut self, dx: f64, dy: f64) {
        self.yaw -= dx * LOOK_SENSITIVITY;
        self.pitch = (self.pitch - dy * LOOK_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }
    /// Advances `dt` seconds. Moves strictly while directional keys are held;
    /// releasing all keys stops immediately with zero glide.
    pub fn tick(&mut self, dt: f64) -> bool {
        let dt = dt.clamp(0., 0.1);
        if !self.held.any() || dt <= 0. {
            return false;
        }
        let f = forward(self.pitch, self.yaw);
        let right = normalize(cross(f, [0., 1., 0.]));
        let fwd = self.held.active(0b0011_0000_0000) - self.held.active(0b1100_0000_0000);
        let side = self.held.active(0b0000_0000_1100) - self.held.active(0b0000_0000_0011);
        let lift = self.held.active(0b0000_0011_0000) - self.held.active(0b0000_1100_0000);
        let mut dir = [0., lift, 0.];
        if fwd != 0. {
            dir = add(dir, mul(f, fwd));
        }
        if side != 0. && length(right) > 1e-9 {
            dir = add(dir, mul(right, side));
        }
        let mag = length(dir);
        if mag <= 1e-9 {
            return false;
        }
        let distance = self.step * self.speed_scale * self.gear() * dt;
        self.position = add(self.position, mul(dir, distance / mag));
        true
    }
    /// The view now: where it stands and what it looks at.
    pub fn pose(&self) -> Pose {
        Pose {
            ortho: false,
            roll: 0.,
            position: self.position,
            target: add(
                self.position,
                mul(forward(self.pitch, self.yaw), self.focus),
            ),
            zoom: self.start.zoom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn start() -> Pose {
        Pose {
            ortho: false,
            roll: 0.,
            position: [0., 0., 10.],
            target: [0., 0., 0.],
            zoom: 1.,
        }
    }
    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }
    #[test]
    fn entering_keeps_the_view_exactly() {
        let f = Fly::new(Pose {
            ortho: false,
            roll: 0.,
            position: [3., 2., 6.],
            target: [0.5, 0.2, -1.],
            zoom: 1.,
        });
        let p = f.pose();
        assert!(close(p.position, [3., 2., 6.]));
        assert!(close(p.target, [0.5, 0.2, -1.]), "{:?}", p.target);
    }
    #[test]
    fn forward_moves_only_while_held_with_no_glide() {
        let mut f = Fly::new(start());
        f.held.set("w", true);
        for _ in 0..10 {
            assert!(f.tick(0.1));
        }
        // Moved forward along −Z by 1s × step (4.5).
        assert!(close(f.position, [0., 0., 5.5]), "{:?}", f.position);
        // Releasing W stops immediately with zero residual glide.
        f.held.set("w", false);
        let at = f.position;
        assert!(!f.tick(0.1));
        assert!(close(f.position, at), "no glide after key release");
    }
    #[test]
    fn climbing_is_world_vertical_even_looking_down() {
        let mut f = Fly::new(Pose {
            ortho: false,
            roll: 0.,
            position: [0., 5., 5.],
            target: [0., 0., 0.],
            zoom: 1.,
        });
        f.held.set("e", true);
        let at = f.position;
        f.tick(0.1);
        let d = sub(f.position, at);
        assert!(
            d[0].abs() < 1e-12 && d[2].abs() < 1e-12 && d[1] > 0.,
            "{d:?}"
        );
    }
    #[test]
    fn mouse_look_is_direct_and_never_drifts_on_tick() {
        let mut f = Fly::new(start());
        f.look(20., 10.);
        let (yaw_after, pitch_after) = (f.yaw, f.pitch);
        assert!(yaw_after < 0. && pitch_after < 0.);
        // Ticking without further mouse motion never drifts yaw or pitch.
        assert!(!f.tick(0.1));
        assert_eq!(f.yaw, yaw_after);
        assert_eq!(f.pitch, pitch_after);
    }
    #[test]
    fn shift_is_faster_and_option_slower_and_scroll_adjusts_speed() {
        let mut a = Fly::new(start());
        let mut b = a.clone();
        let mut c = a.clone();
        let mut d = a.clone();
        b.fast = true;
        c.slow = true;
        d.adjust_speed(3.);
        for f in [&mut a, &mut b, &mut c, &mut d] {
            f.held.set("w", true);
            f.tick(0.1);
        }
        let moved = |f: &Fly| 10. - f.position[2];
        assert!(moved(&b) > moved(&a) * 2.5);
        assert!(moved(&c) < moved(&a) * 0.3);
        assert!(moved(&d) > moved(&a) * 1.5);
    }
}
