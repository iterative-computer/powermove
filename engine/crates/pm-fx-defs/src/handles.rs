//! Declarative viewer controls. Coordinates are layer pixels, never viewport pixels.
use crate::Parameter;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointBinding {
    pub x: String,
    pub y: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Handle {
    Point {
        point: PointBinding,
    },
    Line {
        start: PointBinding,
        end: PointBinding,
        #[serde(default)]
        stops: Option<String>,
        #[serde(default)]
        radial: Option<String>,
    },
    Angle {
        center: PointBinding,
        angle: String,
    },
    Radius {
        center: PointBinding,
        radius: String,
    },
    Rect {
        start: PointBinding,
        end: PointBinding,
    },
}
impl Handle {
    pub fn points(&self) -> Vec<&PointBinding> {
        match self {
            Self::Point { point } => vec![point],
            Self::Line { start, end, .. } | Self::Rect { start, end } => vec![start, end],
            Self::Angle { center, .. } | Self::Radius { center, .. } => vec![center],
        }
    }
}
/// Validate against the winning definition before any viewer reads it. Unknown
/// keys and nonnumeric position/distance bindings are errors, not silent fallbacks.
pub fn parse(raw: Option<&Value>, params: &[Parameter]) -> Result<Vec<Handle>, String> {
    let Some(raw) = raw else { return Ok(vec![]) };
    let handles: Vec<Handle> =
        serde_json::from_value(raw.clone()).map_err(|e| format!("viewerHandles: {e}"))?;
    if handles.len() > 32 {
        return Err("viewerHandles: at most 32 controls".into());
    }
    let check = |key: &str, kind: &str| {
        params
            .iter()
            .any(|p| p.k == key && p.kind.as_deref().unwrap_or("number") == kind)
            .then_some(())
            .ok_or_else(|| format!("viewerHandles: {key} must bind a {kind} parameter"))
    };
    for handle in &handles {
        for p in handle.points() {
            check(&p.x, "number")?;
            check(&p.y, "number")?;
            if p.x == p.y {
                return Err("viewerHandles: point axes must differ".into());
            }
        }
        match handle {
            Handle::Line { stops, radial, .. } => {
                if let Some(k) = stops {
                    check(k, "gradientStops")?;
                }
                if let Some(k) = radial {
                    check(k, "number")?;
                }
            }
            Handle::Angle { angle, .. } => check(angle, "number")?,
            Handle::Radius { radius, .. } => check(radius, "number")?,
            _ => (),
        }
    }
    Ok(handles)
}
/// An invertible layer-to-window transform, including composition zoom/pan.
#[derive(Clone, Copy, Debug)]
pub struct Mapping(pub [f64; 6]);
impl Mapping {
    pub fn screen(self, p: [f64; 2]) -> [f64; 2] {
        let m = self.0;
        [
            m[0] * p[0] + m[2] * p[1] + m[4],
            m[1] * p[0] + m[3] * p[1] + m[5],
        ]
    }
    pub fn local(self, p: [f64; 2]) -> Option<[f64; 2]> {
        let m = self.0;
        let d = m[0] * m[3] - m[1] * m[2];
        if !d.is_finite() || d.abs() < 1e-12 {
            return None;
        }
        let x = p[0] - m[4];
        let y = p[1] - m[5];
        Some([(m[3] * x - m[2] * y) / d, (m[0] * y - m[1] * x) / d])
    }
}
pub fn constrain(p: [f64; 2], center: [f64; 2]) -> [f64; 2] {
    let x = p[0] - center[0];
    let y = p[1] - center[1];
    let r = x.hypot(y);
    let a = (y.atan2(x) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
    [center[0] + r * a.cos(), center[1] + r * a.sin()]
}
/// Projection is performed in screen space: the off-line delete threshold is
/// 24 points at every zoom, including nonuniformly transformed layers.
pub fn project(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let d = [b[0] - a[0], b[1] - a[1]];
    let n = d[0] * d[0] + d[1] * d[1];
    let t = if n < 1e-12 {
        0.
    } else {
        ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / n
    };
    (
        t.clamp(0., 1.),
        (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1]),
    )
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    pub position: f64,
    pub color: [f64; 4],
}
pub fn add_stop(stops: &mut Vec<Stop>, position: f64) -> usize {
    let t = position.clamp(0., 1.);
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    let right = stops.partition_point(|s| s.position < t);
    let a = stops.get(right.saturating_sub(1));
    let b = stops.get(right).or_else(|| stops.last());
    let color = match (a, b) {
        (Some(a), Some(b)) => {
            let u = if b.position > a.position {
                (t - a.position) / (b.position - a.position)
            } else {
                0.
            };
            std::array::from_fn(|i| a.color[i] + (b.color[i] - a.color[i]) * u)
        }
        _ => [0., 0., 0., 1.],
    };
    stops.insert(right, Stop { position: t, color });
    right
}
pub fn move_stop(stops: &mut [Stop], index: usize, position: f64) {
    if let Some(s) = stops.get_mut(index) {
        s.position = position.clamp(0., 1.);
    }
}
pub fn delete_stop(stops: &mut Vec<Stop>, index: usize) -> bool {
    if stops.len() > 2 && index < stops.len() {
        stops.remove(index);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn mapping_round_trip_under_layer_transform_and_zoom() {
        for zoom in [0.1, 1., 4.] {
            let m = Mapping([0.6 * zoom, 0.8 * zoom, -1.6 * zoom, 1.2 * zoom, 37., -16.]);
            let p = [125., 73.];
            let q = m.local(m.screen(p)).unwrap();
            assert!((q[0] - p[0]).abs() < 1e-9);
            assert!((q[1] - p[1]).abs() < 1e-9);
        }
        assert!(Mapping([0.; 6]).local([1., 2.]).is_none());
    }
    #[test]
    fn stops_add_move_delete_and_minimum() {
        let mut s = vec![
            Stop {
                position: 0.,
                color: [0., 0., 0., 1.],
            },
            Stop {
                position: 1.,
                color: [1.; 4],
            },
        ];
        let i = add_stop(&mut s, 0.25);
        assert_eq!(s[i].color, [0.25, 0.25, 0.25, 1.]);
        move_stop(&mut s, i, 0.7);
        assert_eq!(s[i].position, 0.7);
        assert!(delete_stop(&mut s, i));
        assert!(!delete_stop(&mut s, 0));
        assert_eq!(project([50., 25.], [0., 0.], [100., 0.]), (0.5, 25.));
    }
    #[test]
    fn declarations_validate_keys_types_and_unknown_fields() {
        let params:Vec<Parameter>=serde_json::from_value(json!([{"k":"x","label":"X","def":0},{"k":"y","label":"Y","def":0},{"k":"tint","label":"Tint","type":"color","def":"#ffffff"}])).unwrap();
        assert!(
            parse(
                Some(&json!([{"kind":"point","point":{"x":"x","y":"y"}}])),
                &params
            )
            .is_ok()
        );
        for raw in [
            json!([{"kind":"point","point":{"x":"missing","y":"y"}}]),
            json!([{"kind":"point","point":{"x":"tint","y":"y"}}]),
            json!([{"kind":"point","point":{"x":"x","y":"y"},"oops":1}]),
        ] {
            assert!(parse(Some(&raw), &params).is_err());
        }
    }
}

pub const MAX_STOPS: usize = 16;
pub fn validate_stops(raw: &Value) -> Result<Vec<Stop>, String> {
    let stops: Vec<Stop> =
        serde_json::from_value(raw.clone()).map_err(|e| format!("gradientStops: {e}"))?;
    if !(2..=MAX_STOPS).contains(&stops.len())
        || stops.iter().any(|s| {
            !s.position.is_finite()
                || !(0. ..=1.).contains(&s.position)
                || s.color
                    .iter()
                    .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        })
    {
        return Err(
            "gradientStops: expected 2..16 finite positions and RGBA channels in 0..1".into(),
        );
    }
    Ok(stops)
}
/// Portable scalar uniforms avoid backend-specific uniform-array packing.
pub fn gradient_glsl(key: &str) -> String {
    let mut out = format!("uniform float u_{key}Count;\n");
    for i in 0..MAX_STOPS {
        out += &format!("uniform float u_{key}P{i}; uniform vec4 u_{key}C{i};\n");
    }
    out += &format!("vec4 pm_{key}(float t) {{ vec4 c=u_{key}C0; float p=u_{key}P0;\n");
    for i in 1..MAX_STOPS {
        out += &format!(
            "if(u_{key}Count>{}.5){{if(t<=u_{key}P{i}) return mix(c,u_{key}C{i},clamp((t-p)/max(u_{key}P{i}-p,1e-8),0.,1.)); c=u_{key}C{i};p=u_{key}P{i};}}\n",
            i
        );
    }
    out + "return c; }\n"
}

/// All arguments are screen points, so the removal threshold is independent of
/// parent transforms, nonuniform layer scale and composition zoom.
pub fn drag_stop(
    stops: &[Stop],
    index: usize,
    pointer: [f64; 2],
    start: [f64; 2],
    end: [f64; 2],
) -> (Vec<Stop>, bool) {
    let (t, distance) = project(pointer, start, end);
    let mut out = stops.to_vec();
    let removed = distance > 24. && delete_stop(&mut out, index);
    if !removed {
        move_stop(&mut out, index, t);
    }
    (out, removed)
}
#[cfg(test)]
mod drag_tests {
    use super::*;
    #[test]
    fn off_line_threshold_is_exactly_24_screen_points_at_every_zoom() {
        let mut stops = vec![
            Stop {
                position: 0.,
                color: [0.; 4],
            },
            Stop {
                position: 1.,
                color: [1.; 4],
            },
        ];
        add_stop(&mut stops, 0.5);
        for zoom in [0.25, 1., 4.] {
            let m = Mapping([0., 2. * zoom, -0.5 * zoom, 0., 200., 100.]);
            let a = m.screen([0., 0.]);
            let b = m.screen([100., 0.]);
            let p = m.screen([50., 0.]);
            for (distance, expected) in [(23.999, false), (24., false), (24.001, true)] {
                let (out, removed) = drag_stop(&stops, 1, [p[0] + distance, p[1]], a, b);
                assert_eq!(removed, expected);
                assert_eq!(out.len(), if expected { 2 } else { 3 });
            }
            let (out, removed) = drag_stop(&stops[..2], 0, [p[0] + 100., p[1]], a, b);
            assert!(!removed);
            assert_eq!(out.len(), 2);
        }
    }
}
