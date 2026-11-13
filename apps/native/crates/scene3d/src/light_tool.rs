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
/// Source light creation from a surface pick. The caller adds this snapshot
/// inside the same kernel gesture as subsequent pointing, for one Undo.
pub fn new_light(kind: Kind, mode: Aim, hit: Hit) -> Result<Snapshot> {
    if mode == Aim::Orbit {
        return fail("Orbit needs an existing light");
    }
    let distance = (length(sub(hit.eye, hit.point)) * 0.9).max(1.5);
    let position = if mode == Aim::Direct {
        highlight_placement(hit.point, hit.normal, hit.eye, distance)?;
        add(hit.point, mul(normalize(hit.normal), distance))
    } else {
        highlight_placement(hit.point, hit.normal, hit.eye, distance)?
    };
    let size = distance * 0.6;
    Ok(Snapshot {
        position,
        target: hit.point,
        intensity: match kind {
            Kind::Area => 6.,
            Kind::Spot => 40.,
            Kind::Point => 25.,
            Kind::Sun => 3.,
        },
        width: if kind == Kind::Area { size } else { 2. },
        height: if kind == Kind::Area { size * 0.62 } else { 1.2 },
        angle: if kind == Kind::Spot { 30. } else { 45. },
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub on: bool,
    pub aim: Aim,
    pub kind: Kind,
    pub adding: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            on: false,
            aim: Aim::Reflect,
            kind: Kind::Area,
            adding: false,
        }
    }
}
impl State {
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.adding = false;
        }
    }
    pub fn choose_kind(&mut self, kind: Kind) {
        self.kind = kind;
        self.adding = true;
    }
}
#[derive(Default)]
pub struct Tuning {
    change: Option<Change>,
}
impl Tuning {
    pub fn begin(&mut self, id: String, snapshot: Snapshot) -> Result<()> {
        snapshot.validate()?;
        if self.change.is_some() {
            return fail("Light adjustment already active");
        }
        self.change = Some(Change {
            id,
            before: snapshot,
            after: snapshot,
        });
        Ok(())
    }
    pub fn set(&mut self, kind: Kind, what: Adjust, value: f64) -> Result<Option<Snapshot>> {
        let Some(c) = &mut self.change else {
            return Ok(None);
        };
        c.after = adjust(c.after, kind, what, value)?;
        Ok(Some(c.after))
    }
    pub fn commit(&mut self) -> Option<Change> {
        self.change.take().filter(|c| c.before != c.after)
    }
    pub fn cancel(&mut self) -> Option<(String, Snapshot)> {
        self.change.take().map(|c| (c.id, c.before))
    }
}

/// Where a light preset goes for a product centred at `c` with radius `r`,
/// seen from `eye`. Directions are the camera's: "left" is screen left and
/// "behind" is away from the camera.
pub struct Placement {
    pub at: Vec3,
    pub width: f64,
    pub height: f64,
    pub intensity: f64,
    pub name: &'static str,
}
/// The edge-highlight presets (the iPad film's missing look: thin bright
/// lines along chamfered edges):
/// - `rim_left` / `rim_right`: a tall, thin strip behind and beside the
///   product, so its silhouette edge catches a bright line;
/// - `top`: a large softbox overhead for a soft top gradient;
/// - `edge_card`: a narrow card beside the product, slightly toward the
///   camera, so the near chamfer (and a lens ring) catches a highlight;
/// - `strip`: a front-side strip, the usual product key.
pub fn preset_placement(preset: &str, c: Vec3, r: f64, eye: Vec3) -> Result<Placement> {
    let mut f = sub(c, eye);
    f[1] = 0.;
    let f = if length(f) < 1e-6 { [0., 0., -1.] } else { normalize(f) };
    let right = normalize(cross(f, [0., 1., 0.]));
    let up = [0., 1., 0.];
    let at = |side: f64, ahead: f64, lift: f64| {
        add(add(add(c, mul(right, side * r)), mul(f, ahead * r)), mul(up, lift * r))
    };
    Ok(match preset {
        "rim_left" => Placement {
            at: at(-2.2, 1.6, 0.3),
            width: 0.22 * r,
            height: 3.2 * r,
            intensity: 30.,
            name: "Rim Strip Left",
        },
        "rim_right" => Placement {
            at: at(2.2, 1.6, 0.3),
            width: 0.22 * r,
            height: 3.2 * r,
            intensity: 30.,
            name: "Rim Strip Right",
        },
        "top" => Placement {
            at: at(0., 0.2, 3.),
            width: 2.6 * r,
            height: 2.6 * r,
            intensity: 12.,
            name: "Top Softbox",
        },
        "edge_card" => Placement {
            at: at(2.4, -0.5, 0.5),
            width: 0.35 * r,
            height: 2.4 * r,
            intensity: 25.,
            name: "Edge Highlight Card",
        },
        "strip" => Placement {
            at: at(-2., -1.2, 1.),
            width: 0.3 * r,
            height: 3. * r,
            intensity: 15.,
            name: "Strip Light",
        },
        _ => return fail("Choose a light preset: rim_left, rim_right, top, edge_card or strip"),
    })
}
/// A light's colour for a colour temperature in kelvin (1000–40000 K), as
/// sRGB hex: 6500 K is about white, 3200 K tungsten, 9000 K cool daylight.
pub fn kelvin_hex(kelvin: f64) -> String {
    let t = kelvin.clamp(1000., 40000.) / 100.;
    let red = if t <= 66. {
        255.
    } else {
        329.698727446 * (t - 60.).powf(-0.1332047592)
    };
    let green = if t <= 66. {
        99.4708025861 * t.ln() - 161.1195681661
    } else {
        288.1221695283 * (t - 60.).powf(-0.0755148492)
    };
    let blue = if t >= 66. {
        255.
    } else if t <= 19. {
        0.
    } else {
        138.5177312231 * (t - 10.).ln() - 305.0447927307
    };
    let c = |v: f64| v.round().clamp(0., 255.) as u8;
    format!("#{:02X}{:02X}{:02X}", c(red), c(green), c(blue))
}
/// The colour temperature (100 K steps, 1000–12000 K) whose colour is
/// nearest a light's sRGB hex, for the Light panel's Temperature reading.
pub fn kelvin_of(hex: &str) -> Option<f64> {
    let rgb = |h: &str| -> Option<[i32; 3]> {
        let h = h.strip_prefix('#')?;
        if h.len() != 6 {
            return None;
        }
        let n = |i: usize| i32::from_str_radix(&h[i..i + 2], 16).ok();
        Some([n(0)?, n(2)?, n(4)?])
    };
    let want = rgb(hex)?;
    (10..=120)
        .map(|k| f64::from(k) * 100.)
        .min_by_key(|k| {
            let got = rgb(&kelvin_hex(*k)).unwrap_or_default();
            (0..3).map(|i| (got[i] - want[i]).pow(2)).sum::<i32>()
        })
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    #[test]
    fn rim_strips_sit_behind_and_beside_the_product_as_the_camera_sees_it() {
        // Camera on +z looking at the origin: screen right is +x, behind is -z.
        let eye = [0., 0., 5.];
        let left = preset_placement("rim_left", [0., 0., 0.], 1., eye).unwrap();
        let right = preset_placement("rim_right", [0., 0., 0.], 1., eye).unwrap();
        assert!(left.at[0] < -1. && left.at[2] < -1., "{:?}", left.at);
        assert!(right.at[0] > 1. && right.at[2] < -1., "{:?}", right.at);
        assert!(left.height > 10. * left.width, "a strip is tall and thin");
        let top = preset_placement("top", [0., 0., 0.], 1., eye).unwrap();
        assert!(top.at[1] > 2.);
        let card = preset_placement("edge_card", [0., 0., 0.], 1., eye).unwrap();
        assert!(card.at[0] > 1. && card.at[2] > 0., "beside, toward the camera");
        assert!(preset_placement("disco", [0.; 3], 1., eye).is_err());
    }
    #[test]
    fn colour_temperature_runs_warm_to_cool() {
        assert_eq!(kelvin_hex(6600.), "#FFFFFF");
        let warm = kelvin_hex(3200.);
        let cool = kelvin_hex(9000.);
        assert!(warm.starts_with("#FF") && &warm[5..] < "C0", "{warm}");
        assert!(cool.ends_with("FF") && &cool[1..3] < "E0", "{cool}");
        assert_eq!(kelvin_of(&kelvin_hex(3200.)), Some(3200.));
        assert_eq!(kelvin_of("#FFFFFF"), Some(6600.));
        assert_eq!(kelvin_of("nope"), None);
    }
}
