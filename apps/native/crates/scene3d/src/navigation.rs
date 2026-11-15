//! Transient editor viewpoint. Values here never write shot-camera channels.
use crate::{Result, fail};
use serde::{Deserialize, Serialize};
pub type Vec3 = [f64; 3];
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] + b[i])
}
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] - b[i])
}
pub fn mul(a: Vec3, s: f64) -> Vec3 {
    a.map(|x| x * s)
}
pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub fn length(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}
pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 0. { mul(a, 1. / l) } else { [0.; 3] }
}
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub position: Vec3,
    pub target: Vec3,
    pub zoom: f64,
    #[serde(default)]
    pub ortho: bool,
    #[serde(default)]
    pub roll: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lens {
    Perspective {
        fov: f64,
        aspect: f64,
        near: f64,
        far: f64,
    },
    Orthographic {
        width: f64,
        height: f64,
    },
}
impl Lens {
    pub fn validate(self) -> Result<()> {
        match self {
            Self::Perspective {
                fov,
                aspect,
                near,
                far,
            } => {
                if ![fov, aspect, near, far].iter().all(|v| v.is_finite())
                    || (fov <= 0. || fov >= 180.)
                    || aspect <= 0.
                    || near <= 0.
                    || far <= near
                {
                    return fail("Invalid perspective lens");
                }
            }
            Self::Orthographic { width, height } => {
                if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
                    return fail("Invalid orthographic lens");
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Select,
    Orbit,
    Pan,
    Dolly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Camera,
    Editor,
}
impl Pose {
    pub fn validate(self) -> Result<()> {
        if !self
            .position
            .iter()
            .chain(self.target.iter())
            .all(|x| x.is_finite())
            || !self.roll.is_finite()
            || !self.zoom.is_finite()
            || self.zoom <= 0.
        {
            return fail("Invalid editor pose");
        }
        Ok(())
    }
}
pub fn orbit(start: Pose, dx: f64, dy: f64) -> Pose {
    let o = sub(start.position, start.target);
    let r = length(o).max(0.05);
    let theta = o[0].atan2(o[2]) - dx * 0.008;
    let phi = if length(o) == 0. {
        0.
    } else {
        (o[1] / length(o)).clamp(-1., 1.).acos()
    };
    let phi = (phi - dy * 0.008).clamp(0.01, std::f64::consts::PI - 0.01);
    Pose {
        ortho: false,
        position: add(
            start.target,
            [
                r * phi.sin() * theta.sin(),
                r * phi.cos(),
                r * phi.sin() * theta.cos(),
            ],
        ),
        ..start
    }
}
pub fn view_lens(pose: Pose, lens: Lens) -> Lens {
    if pose.ortho {
        if let Lens::Perspective { fov, aspect, .. } = lens {
            let height =
                2. * length(sub(pose.position, pose.target)) * (fov.to_radians() / 2.).tan();
            return Lens::Orthographic {
                width: height * aspect,
                height,
            };
        }
    }
    lens
}
pub fn pan(start: Pose, dx: f64, dy: f64, lens: Lens, height: f64, right: Vec3, up: Vec3) -> Pose {
    let scale = match view_lens(start, lens) {
        Lens::Perspective { fov, .. } => {
            2. * length(sub(start.position, start.target)) * (fov.to_radians() / 2.).tan()
                / start.zoom
        }
        Lens::Orthographic { height, .. } => height / start.zoom,
    };
    let o = mul(add(mul(right, -dx), mul(up, dy)), scale / height.max(1.));
    Pose {
        position: add(start.position, o),
        target: add(start.target, o),
        ..start
    }
}
pub fn dolly(start: Pose, delta: f64, lens: Lens) -> Pose {
    let factor = (delta * 0.005).clamp(-2., 2.).exp();
    match view_lens(start, lens) {
        Lens::Orthographic { .. } => Pose {
            zoom: (start.zoom / factor).clamp(0.01, 1000.),
            ..start
        },
        Lens::Perspective { near, far, .. } => {
            let mut o = sub(start.position, start.target);
            let d = length(o);
            if d < 1e-9 {
                o = [0., 0., 1.];
            }
            let d = (d.max(0.05) * factor)
                .min(far * 0.8)
                .max(near * 2.)
                .max(0.05);
            Pose {
                position: add(start.target, mul(normalize(o), d)),
                ..start
            }
        }
    }
}
/// Blender's zoom step: one wheel notch (or numpad + / −) brings the view
/// 1.2 times closer, toward the view centre.
pub const ZOOM_STEP: f64 = 1.2;
/// Blender's numpad orbit step (Preferences ▸ Navigation ▸ Rotation Angle).
pub const ORBIT_STEP: f64 = 15.;
/// `notches` steps closer (negative: further), toward the view centre.
pub fn zoom_step(start: Pose, notches: f64, lens: Lens) -> Pose {
    let factor = ZOOM_STEP.powf(notches.clamp(-20., 20.));
    match view_lens(start, lens) {
        Lens::Orthographic { .. } => Pose {
            zoom: (start.zoom * factor).clamp(0.01, 1000.),
            ..start
        },
        Lens::Perspective { near, far, .. } => {
            let mut o = sub(start.position, start.target);
            if length(o) < 1e-9 {
                o = [0., 0., 1.];
            }
            let d = (length(o).max(0.05) / factor)
                .min(far * 0.8)
                .max(near * 2.)
                .max(0.05);
            Pose {
                position: add(start.target, mul(normalize(o), d)),
                ..start
            }
        }
    }
}
/// Turntable orbit by degrees around the view centre: `yaw` about world
/// vertical (positive turns the view to the right), `pitch` up and down
/// (positive looks from higher up).
pub fn orbit_by(start: Pose, yaw: f64, pitch: f64) -> Pose {
    orbit(start, yaw.to_radians() / 0.008, pitch.to_radians() / 0.008)
}
/// Blender's numpad views, from the side they look at (Blender is Z-up; this
/// scene is Y-up with the shot looking down −Z: Front looks down −Z).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}
/// Looking at the view centre from `side`, as far away as before.
pub fn side_view(start: Pose, side: Side) -> Pose {
    let d = length(sub(start.position, start.target)).max(0.05);
    // Straight up or down has no heading: a hair off, as orbit's own limit.
    let (s, c) = (0.01f64.sin(), 0.01f64.cos());
    let dir = match side {
        Side::Front => [0., 0., 1.],
        Side::Back => [0., 0., -1.],
        Side::Right => [1., 0., 0.],
        Side::Left => [-1., 0., 0.],
        Side::Top => [0., c, s],
        Side::Bottom => [0., -c, s],
    };
    Pose {
        ortho: true,
        roll: 0.,
        position: add(start.target, mul(dir, d)),
        ..start
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}
impl Bounds {
    pub fn validate(self) -> Result<()> {
        if !(0..3).all(|i| {
            self.min[i].is_finite() && self.max[i].is_finite() && self.min[i] <= self.max[i]
        }) {
            return fail("Invalid 3D bounds");
        }
        Ok(())
    }
    pub fn center(self) -> Vec3 {
        mul(add(self.min, self.max), 0.5)
    }
    pub fn radius(self) -> f64 {
        length(sub(self.max, self.min)) / 2.
    }
}
pub fn frame(start: Pose, b: Bounds, lens: Lens) -> Pose {
    let target = b.center();
    let radius = b.radius().max(0.1);
    let o = sub(start.position, start.target);
    let dir = if dot(o, o) < 1e-9 {
        [0., 0., 1.]
    } else {
        normalize(o)
    };
    match view_lens(start, lens) {
        Lens::Orthographic { width, height } => Pose {
            ortho: start.ortho,
            roll: start.roll,
            position: add(target, mul(dir, length(o).max(1.))),
            target,
            zoom: width.min(height) / 2. / (radius * 1.25),
        },
        Lens::Perspective { fov, aspect, .. } => {
            let vertical = ((fov.to_radians() / 2.).tan() / start.zoom).atan();
            let horizontal = (vertical.tan() * aspect).atan();
            Pose {
                position: add(
                    target,
                    mul(dir, radius * 1.25 / vertical.min(horizontal).sin()),
                ),
                target,
                ..start
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Gesture {
    start: Pose,
    view: View,
    x: f64,
    y: f64,
    mode: Mode,
    lens: Lens,
    height: f64,
    right: Vec3,
    up: Vec3,
}
#[derive(Default, Debug)]
pub struct Navigation {
    pub mode: Mode,
    pub view: View,
    gesture: Option<Gesture>,
}
impl Navigation {
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        pose: Pose,
        lens: Lens,
        button: u8,
        alt: bool,
        shift: bool,
        available: bool,
        x: f64,
        y: f64,
        height: f64,
        right: Vec3,
        up: Vec3,
    ) -> Result<bool> {
        if !available || self.gesture.is_some() || button > 1 {
            return Ok(false);
        }
        let mode = if button == 1 || alt {
            if shift { Mode::Pan } else { Mode::Orbit }
        } else {
            self.mode
        };
        if mode == Mode::Select {
            return Ok(false);
        }
        pose.validate()?;
        lens.validate()?;
        if ![x, y, height]
            .iter()
            .chain(right.iter())
            .chain(up.iter())
            .all(|x| x.is_finite())
        {
            return fail("Invalid pointer input");
        }
        self.gesture = Some(Gesture {
            start: pose,
            view: self.view,
            x,
            y,
            mode,
            lens,
            height,
            right,
            up,
        });
        self.view = View::Editor;
        Ok(true)
    }
    pub fn moved(&self, x: f64, y: f64, shift: bool) -> Result<Option<Pose>> {
        if !x.is_finite() || !y.is_finite() {
            return fail("Invalid pointer input");
        }
        Ok(self.gesture.map(|g| {
            let fine = if shift && g.mode != Mode::Pan {
                0.1
            } else {
                1.
            };
            let dx = (x - g.x) * fine;
            let dy = (y - g.y) * fine;
            match g.mode {
                Mode::Orbit => orbit(g.start, dx, dy),
                Mode::Pan => pan(g.start, dx, dy, g.lens, g.height, g.right, g.up),
                Mode::Dolly => dolly(g.start, dy, g.lens),
                Mode::Select => g.start,
            }
        }))
    }
    pub fn cancel(&mut self) -> Option<Pose> {
        self.gesture.take().map(|g| {
            self.view = g.view;
            g.start
        })
    }
    pub fn end(&mut self) {
        self.gesture = None;
    }
    pub fn active(&self) -> bool {
        self.gesture.is_some()
    }
    // Matches the host wheel event plus the frozen lens/pose snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn wheel(
        &mut self,
        start: Pose,
        lens: Lens,
        delta: f64,
        delta_mode: u8,
        height: f64,
        alt: bool,
        available: bool,
    ) -> Result<Option<Pose>> {
        if !available || (self.mode == Mode::Select && !alt) || self.active() {
            return Ok(None);
        }
        start.validate()?;
        lens.validate()?;
        if !delta.is_finite() || !height.is_finite() {
            return fail("Invalid wheel input");
        }
        let delta = delta
            * match delta_mode {
                1 => 16.,
                2 => height,
                _ => 1.,
            };
        self.view = View::Editor;
        Ok(Some(dolly(start, delta, lens)))
    }
}
