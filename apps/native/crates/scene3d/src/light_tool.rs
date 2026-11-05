//! Host supplies picks in world coordinates; light previews are values for a
//! kernel gesture, never in-place writes to a live document.
use crate::{Result, fail, navigation::*};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    Reflect,
    Direct,
    Orbit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Area,
    Spot,
    Point,
    Sun,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adjust {
    Power,
    Size,
    Distance,
    Spread,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub point: Vec3,
    pub normal: Vec3,
    pub eye: Vec3,
}
pub trait SurfacePicker: Send + Sync {
    fn pick(&self, u: f64, v: f64) -> Result<Option<Hit>>;
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    pub position: Vec3,
    pub target: Vec3,
    pub intensity: f64,
    pub width: f64,
    pub height: f64,
    pub angle: f64,
}
impl Snapshot {
    pub fn validate(self) -> Result<()> {
        if !self
            .position
            .iter()
            .chain(self.target.iter())
            .chain([self.intensity, self.width, self.height, self.angle].iter())
            .all(|x| x.is_finite())
        {
            return fail("Non-finite light state");
        }
        Ok(())
    }
}
pub fn highlight_placement(point: Vec3, normal: Vec3, eye: Vec3, distance: f64) -> Result<Vec3> {
    if !point
        .iter()
        .chain(normal.iter())
        .chain(eye.iter())
        .all(|x| x.is_finite())
        || !distance.is_finite()
        || distance <= 0.
        || length(normal) < 1e-12
        || length(sub(eye, point)) < 1e-12
    {
        return fail("Invalid surface or light distance");
    }
    let n = normalize(normal);
    let v = normalize(sub(eye, point));
    Ok(add(point, mul(sub(mul(n, 2. * dot(v, n)), v), distance)))
}
pub fn aim(mut s: Snapshot, h: Hit, mode: Aim) -> Result<Snapshot> {
    s.validate()?;
    let distance = length(sub(s.position, s.target)).max(0.05);
    s.position = match mode {
        Aim::Reflect => highlight_placement(h.point, h.normal, h.eye, distance)?,
        Aim::Direct => {
            highlight_placement(h.point, h.normal, h.eye, distance)?;
            add(h.point, mul(normalize(h.normal), distance))
        }
        Aim::Orbit => return fail("Orbit uses pointer deltas"),
    };
    s.target = h.point;
    Ok(s)
}
pub fn adjust(mut s: Snapshot, kind: Kind, what: Adjust, value: f64) -> Result<Snapshot> {
    s.validate()?;
    if !value.is_finite() {
        return fail("Non-finite light adjustment");
    }
    match what {
        Adjust::Power => s.intensity = value.clamp(0., 10000.),
        Adjust::Size if kind == Kind::Area => {
            let k = value.max(0.001) / s.width.max(s.height).max(1e-6);
            s.width *= k;
            s.height *= k;
        }
        Adjust::Spread if kind == Kind::Spot => s.angle = value.clamp(1., 89.),
        Adjust::Distance if kind != Kind::Sun => {
            let offset = sub(s.position, s.target);
            let d0 = length(offset).max(1e-6);
            let d1 = value.max(0.05);
            if length(offset) < 1e-12 {
                return fail("Light and target coincide");
            }
            s.position = add(s.target, mul(normalize(offset), d1));
            s.intensity = (s.intensity * (d1 / d0).powi(2)).clamp(0., 10000.);
        }
        _ => {}
    }
    Ok(s)
}
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub id: String,
    pub before: Snapshot,
    pub after: Snapshot,
}
#[derive(Default)]
pub struct LightTool {
    gesture: Option<(String, Snapshot, Snapshot, Aim, [f64; 2])>,
}
impl LightTool {
    pub fn begin(&mut self, id: String, start: Snapshot, mode: Aim, u: f64, v: f64) -> Result<()> {
        start.validate()?;
        if self.gesture.is_some() {
            return fail("Light gesture already active");
        }
        if !u.is_finite() || !v.is_finite() {
            return fail("Invalid pointer");
        }
        self.gesture = Some((id, start, start, mode, [u, v]));
        Ok(())
    }
    pub fn moved(
        &mut self,
        u: f64,
        v: f64,
        picker: &dyn SurfacePicker,
    ) -> Result<Option<Snapshot>> {
        if !u.is_finite() || !v.is_finite() {
            return fail("Invalid pointer");
        }
        let Some((_, _, s, mode, last)) = &mut self.gesture else {
            return Ok(None);
        };
        if *mode == Aim::Orbit {
            let offset = sub(s.position, s.target);
            let r = length(offset);
            let theta = offset[0].atan2(offset[2]) - (u - last[0]) * std::f64::consts::TAU;
            let phi = if r > 0. {
                (offset[1] / r).clamp(-1., 1.).acos()
            } else {
                0.
            };
            let phi = (phi + (v - last[1]) * std::f64::consts::PI)
                .clamp(0.05, std::f64::consts::PI - 0.05);
            s.position = add(
                s.target,
                [
                    r * phi.sin() * theta.sin(),
                    r * phi.cos(),
                    r * phi.sin() * theta.cos(),
                ],
            );
            *last = [u, v];
        } else if let Some(h) = picker.pick(u, v)? {
            *s = aim(*s, h, *mode)?;
        }
        Ok(Some(*s))
    }
    pub fn end(&mut self) -> Option<Change> {
        self.gesture.take().and_then(|(id, before, after, _, _)| {
            (before != after).then_some(Change { id, before, after })
        })
    }
    pub fn cancel(&mut self) -> Option<(String, Snapshot)> {
        self.gesture.take().map(|(id, start, _, _, _)| (id, start))
    }
    pub fn active(&self) -> bool {
        self.gesture.is_some()
    }
}
