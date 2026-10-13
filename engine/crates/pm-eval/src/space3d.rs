//! 2.5D layer space (`legacy/core/space-3d.ts`, plan section 8 "2.5D").
//!
//! Flat layers share a perspective space: positive Z goes away from the
//! viewer, and the default camera is a 50 mm lens on a 36 mm film gate
//! centered on the composition. 4×4 matrices are column-major
//! (`m[column * 4 + row]`), as in the TypeScript and three.js. Homographies
//! (`planeMatrix`) are column-major 3×3 matrices mapping layer pixels to
//! composition pixels; GPU rendering, outlines, and picking share them.
//!
//! The outermost 3D group owns the lens of everything inside it
//! (`perspectiveAmount`), and `depthOrderedLayers` sorts runs of 3D layers
//! back to front by surface, keeping coplanar artwork in timeline order.

use pm_model::Node;

use crate::js_sem::{JsResult, JsThrow};
use crate::math;
use crate::node;
use crate::scope::{Scope, flag, js_hypot, js_max, same};
use crate::transform::Mat2d;

/// A column-major 4×4 matrix.
pub type Mat4 = [f64; 16];
/// A column-major 3×3 homography.
pub type Mat3 = [f64; 9];

/// The 3D transform channels and their defaults (`CHANNELS_3D`).
pub const CHANNELS_3D: [(&str, f64); 9] = [
    ("perspective", 50.0),
    ("position.z", 0.0),
    ("anchor.z", 0.0),
    ("scale.z", 100.0),
    ("rotation.x", 0.0),
    ("rotation.y", 0.0),
    ("orientation.x", 0.0),
    ("orientation.y", 0.0),
    ("orientation.z", 0.0),
];

pub const IDENTITY4: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// `multiply(a, b)`: `a · b`.
pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0; 16];
    let mut c = 0;
    while c < 16 {
        for r in 0..4 {
            out[c + r] =
                a[r] * b[c] + a[4 + r] * b[c + 1] + a[8 + r] * b[c + 2] + a[12 + r] * b[c + 3];
        }
        c += 4;
    }
    out
}

/// `translation(x, y, z)`.
pub fn translation(x: f64, y: f64, z: f64) -> Mat4 {
    let mut m = IDENTITY4;
    m[12] = x;
    m[13] = y;
    m[14] = z;
    m
}

/// `rotation(axis, degrees)`: about X (0), Y (1), or Z (2).
pub fn rotation(axis: usize, degrees: f64) -> Mat4 {
    let mut m = IDENTITY4;
    let a = (axis + 1) % 3;
    let b = (axis + 2) % 3;
    let c = math::cos(math::deg_to_rad(degrees));
    let s = math::sin(math::deg_to_rad(degrees));
    m[a * 4 + a] = c;
    m[b * 4 + b] = c;
    m[a * 4 + b] = s;
    m[b * 4 + a] = -s;
    m
}

/// A 2D affine matrix in 3D (`local3D` of a flat layer).
pub fn lift(m: &Mat2d) -> Mat4 {
    [
        m[0], m[1], 0.0, 0.0, m[2], m[3], 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, m[4], m[5], 0.0, 1.0,
    ]
}

/// `affinePlane(m)`: a front-facing plane with a constant perspective divide
/// is exactly affine; tilted or behind-camera planes give `None`.
pub fn affine_plane(m: &Mat3) -> Option<Mat2d> {
    if m[2] != 0.0 || m[5] != 0.0 || m[8] <= 0.0001 || !m.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some([
        m[0] / m[8],
        m[1] / m[8],
        m[3] / m[8],
        m[4] / m[8],
        m[6] / m[8],
        m[7] / m[8],
    ])
}

/// `projectPoint(m, p)`.
pub fn project_point(m: &Mat3, x: f64, y: f64) -> (f64, f64) {
    let w = m[2] * x + m[5] * y + m[8];
    (
        (m[0] * x + m[3] * y + m[6]) / w,
        (m[1] * x + m[4] * y + m[7]) / w,
    )
}

/// `inversePlane(m)`, or `None` when the determinant is not finite or
/// `|det| < 1e-10`.
pub fn inverse_plane(m: &Mat3) -> Option<Mat3> {
    let [a, d, g, b, e, h, c, f, i] = *m;
    let aa = e * i - f * h;
    let bb = f * g - d * i;
    let cc = d * h - e * g;
    let det = a * aa + b * bb + c * cc;
    if !det.is_finite() || det.abs() < 1e-10 {
        return None;
    }
    Some([
        aa / det,
        bb / det,
        cc / det,
        (c * h - b * i) / det,
        (a * i - c * g) / det,
        (b * g - a * h) / det,
        (b * f - c * e) / det,
        (c * d - a * f) / det,
        (a * e - b * d) / det,
    ])
}

/// `inverse3D(m)`: the inverse of an affine 4×4 matrix.
pub fn inverse3d(m: &Mat4) -> Option<Mat4> {
    let linear = inverse_plane(&[m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]])?;
    let [a, b, c, d, e, f, g, h, i] = linear;
    let (x, y, z) = (m[12], m[13], m[14]);
    Some([
        a,
        b,
        c,
        0.0,
        d,
        e,
        f,
        0.0,
        g,
        h,
        i,
        0.0,
        -a * x - d * y - g * z,
        -b * x - e * y - h * z,
        -c * x - f * y - i * z,
        1.0,
    ])
}

/// Layer-space bounds, as `GL.bounds` returns them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl<'d> Scope<'_, 'd> {
    /// `value(PM, L, key, T, fallback)`: the channel as a finite number, or
    /// `fallback` when the channel is missing (`L.p?.[key] == null`) or not
    /// finite.
    pub fn value3d(&mut self, layer: &'d Node, key: &str, time: f64, fallback: f64) -> JsResult<f64> {
        let p = node::get(layer, b"p");
        let prop = match p {
            node::Got::Node(p) => node::get(p, key.as_bytes()),
            _ => node::Got::Undefined,
        };
        if prop.is_nullish() {
            return Ok(fallback);
        }
        let v = self.num(layer, key, time)?;
        Ok(if v.is_finite() { v } else { fallback })
    }

    /// `local3D(PM, L, T)`.
    pub fn local3d(&mut self, layer: &'d Node, time: f64) -> JsResult<Mat4> {
        if !flag(layer, "threeD") {
            return Ok(lift(&self.local_matrix(layer, time)?));
        }
        let x = self.value3d(layer, "position.x", time, 0.0)?;
        let y = self.value3d(layer, "position.y", time, 0.0)?;
        let z = self.value3d(layer, "position.z", time, 0.0)?;
        let mut m = translation(x, y, z);
        for (axis, key) in [
            (2, "rotation"),
            (1, "rotation.y"),
            (0, "rotation.x"),
            (2, "orientation.z"),
            (1, "orientation.y"),
            (0, "orientation.x"),
        ] {
            let degrees = self.value3d(layer, key, time, 0.0)?;
            if degrees != 0.0 {
                m = multiply(&m, &rotation(axis, degrees));
            }
        }
        let mut scale = IDENTITY4;
        scale[0] = self.value3d(layer, "scale.x", time, 100.0)? / 100.0;
        scale[5] = self.value3d(layer, "scale.y", time, 100.0)? / 100.0;
        scale[10] = self.value3d(layer, "scale.z", time, 100.0)? / 100.0;
        scale[4] = math::tan(math::deg_to_rad(self.value3d(layer, "skew", time, 0.0)?)) * scale[5];
        let ax = self.value3d(layer, "anchor.x", time, 0.0)?;
        let ay = self.value3d(layer, "anchor.y", time, 0.0)?;
        let az = self.value3d(layer, "anchor.z", time, 0.0)?;
        Ok(multiply(&multiply(&m, &scale), &translation(-ax, -ay, -az)))
    }

    /// `parent3D(PM, L, T, parent = L.parent ? PM.L(L.parent) : null)`: the
    /// 3D space of a layer (enclosing groups and the parent chain, looked up
    /// with `PM.L`).
    pub fn parent3d(
        &mut self,
        layer: &'d Node,
        time: f64,
        parent: Option<Option<&'d Node>>,
    ) -> JsResult<Mat4> {
        let parent = match parent {
            Some(p) => p,
            None => self.parent_by_id(layer),
        };
        let mut walk = Walk3d {
            groups: Vec::new(),
            matrix: IDENTITY4,
        };
        walk.append_groups(self, layer, time)?;
        walk.append_parents(self, layer, parent, time)?;
        Ok(walk.matrix)
    }

    /// `world3D(PM, L, T, positioning)`. With `positioning`, the layer's own
    /// transform is only its Z translation (`L.threeD`).
    pub fn world3d(&mut self, layer: &'d Node, time: f64, positioning: bool) -> JsResult<Mat4> {
        let own = if positioning {
            let z = if flag(layer, "threeD") {
                self.value3d(layer, "position.z", time, 0.0)?
            } else {
                0.0
            };
            translation(0.0, 0.0, z)
        } else {
            self.local3d(layer, time)?
        };
        let parent = self.parent3d(layer, time, None)?;
        Ok(multiply(&parent, &own))
    }

    /// `is3DLayer(PM, layer)`: the layer's 3D switch or any enclosing 3D group.
    pub fn is_3d_layer(&mut self, layer: &'d Node) -> JsResult<bool> {
        if flag(layer, "threeD") {
            return Ok(true);
        }
        Ok(self
            .group_ancestors(layer)?
            .iter()
            .any(|g| flag(g, "threeD")))
    }

    /// `perspectiveAmount(PM, layer, time)`: the outermost 3D group's lens
    /// (or the layer's own), at least 0.001.
    pub fn perspective_amount(&mut self, layer: &'d Node, time: f64) -> JsResult<f64> {
        let groups = self.group_ancestors(layer)?;
        let owner = groups
            .iter()
            .rev()
            .find(|g| flag(g, "threeD"))
            .copied()
            .unwrap_or(layer);
        Ok(js_max(0.001, self.value3d(owner, "perspective", time, 50.0)?))
    }

    /// `planeMatrix(PM, L, T, positioning)`: layer pixels to composition
    /// pixels.
    pub fn plane_matrix(&mut self, layer: &'d Node, time: f64, positioning: bool) -> JsResult<Mat3> {
        if !self.is_3d_layer(layer)? {
            let m = self.world_matrix(layer, time)?;
            return Ok([m[0], m[1], 0.0, m[2], m[3], 0.0, m[4], m[5], 1.0]);
        }
        let m = self.world3d(layer, time, positioning)?;
        let (w, h) = self.comp_size();
        let cx = w / 2.0;
        let cy = h / 2.0;
        let f = js_max(1.0, w) * self.perspective_amount(layer, time)? / 36.0;
        Ok([
            m[0] + cx * m[2] / f,
            m[1] + cy * m[2] / f,
            m[2] / f,
            m[4] + cx * m[6] / f,
            m[5] + cy * m[6] / f,
            m[6] / f,
            m[12] + cx * m[14] / f,
            m[13] + cy * m[14] / f,
            1.0 + m[14] / f,
        ])
    }

    /// `planeContains(PM, L, T, x, y, b)`: whether a composition point hits
    /// the layer's bounds in front of the camera.
    pub fn plane_contains(&mut self, layer: &'d Node, time: f64, x: f64, y: f64, b: &Rect) -> JsResult<bool> {
        let m = self.plane_matrix(layer, time, false)?;
        let Some(inv) = inverse_plane(&m) else {
            return Ok(false);
        };
        let (px, py) = project_point(&inv, x, y);
        Ok(m[2] * px + m[5] * py + m[8] > 0.0001
            && px >= b.x0
            && px <= b.x1
            && py >= b.y0
            && py <= b.y1)
    }

    /// `depthOrderedLayers(PM, layers, T)`: within each run of 3D layers,
    /// surfaces (coplanar layers, in timeline order) sorted back to front by
    /// the depth of their first layer's anchor. Returns the input order when
    /// no layer is 3D.
    pub fn depth_ordered_layers(&mut self, layers: &[&'d Node], time: f64) -> JsResult<Vec<&'d Node>> {
        let mut flags = Vec::with_capacity(layers.len());
        for l in layers {
            flags.push(self.is_3d_layer(l)?);
        }
        let mut result: Vec<&'d Node> = layers.to_vec();
        if !flags.iter().any(|f| *f) {
            return Ok(result);
        }
        let mut start = 0;
        while start < result.len() {
            if !flags[start] {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < result.len() && flags[end] {
                end += 1;
            }
            struct Surface<'d> {
                plane: Option<[f64; 4]>,
                layers: Vec<&'d Node>,
                depth: f64,
            }
            let mut surfaces: Vec<Surface<'d>> = Vec::new();
            for &layer in &result[start..end] {
                let m = self.world3d(layer, time, false)?;
                let normal = [
                    m[1] * m[6] - m[2] * m[5],
                    m[2] * m[4] - m[0] * m[6],
                    m[0] * m[5] - m[1] * m[4],
                ];
                let length = js_hypot(&normal);
                let mut plane = None;
                if length > 1e-12 {
                    // A reflected layer still belongs to the same two-sided plane.
                    let first = normal
                        .iter()
                        .copied()
                        .find(|v| v.abs() > length * 1e-8)
                        .unwrap_or(1.0);
                    let sign = if first < 0.0 { -1.0 } else { 1.0 };
                    let n = normal.map(|v| v / length * sign);
                    plane = Some([n[0], n[1], n[2], n[0] * m[12] + n[1] * m[13] + n[2] * m[14]]);
                }
                if let Some(p) = plane
                    && let Some(existing) = surfaces.iter_mut().find(|s| {
                        s.plane.is_some_and(|q| {
                            (0..3).all(|i| (p[i] - q[i]).abs() < 1e-7) && (p[3] - q[3]).abs() < 1e-5
                        })
                    })
                {
                    existing.layers.push(layer);
                    continue;
                }
                let ax = self.value3d(layer, "anchor.x", time, 0.0)?;
                let ay = self.value3d(layer, "anchor.y", time, 0.0)?;
                let az = if flag(layer, "threeD") {
                    self.value3d(layer, "anchor.z", time, 0.0)?
                } else {
                    0.0
                };
                let depth = m[2] * ax + m[6] * ay + m[10] * az + m[14];
                surfaces.push(Surface {
                    plane,
                    layers: vec![layer],
                    depth,
                });
            }
            // `Array.prototype.sort` is stable; a NaN comparison counts as equal.
            surfaces.sort_by(|a, b| {
                let d = a.depth - b.depth;
                if d < 0.0 {
                    core::cmp::Ordering::Less
                } else if d > 0.0 {
                    core::cmp::Ordering::Greater
                } else {
                    core::cmp::Ordering::Equal
                }
            });
            let ordered: Vec<&'d Node> = surfaces.into_iter().flat_map(|s| s.layers).collect();
            result.splice(start..end, ordered);
            start = end;
        }
        Ok(result)
    }

    /// `poseValues3D(PM, L, world, T, parent)`: decompose a world matrix into
    /// the layer's position, rotation, scale, and XY skew channels in its
    /// parent's space. Throws (with the legacy messages) for a zero-scale
    /// parent or layer and for depth shear the channels cannot represent.
    pub fn pose_values3d(
        &mut self,
        layer: &'d Node,
        world: &Mat4,
        time: f64,
        parent: Option<&Mat4>,
    ) -> JsResult<Pose3d> {
        let parent = match parent {
            Some(p) => *p,
            None => self.parent3d(layer, time, None)?,
        };
        let inv = inverse3d(&parent).ok_or_else(|| {
            JsThrow::error("Increase the parent’s zero scale before changing its 3D hierarchy")
        })?;
        let m = multiply(&inv, world);
        let sx = js_hypot(&[m[0], m[1], m[2]]);
        if sx < 1e-9 {
            return Err(JsThrow::error("Increase zero scale before changing the 3D hierarchy"));
        }
        let x = [m[0] / sx, m[1] / sx, m[2] / sx];
        let xy = x[0] * m[4] + x[1] * m[5] + x[2] * m[6];
        let mut y = [m[4] - xy * x[0], m[5] - xy * x[1], m[6] - xy * x[2]];
        let sy = js_hypot(&y);
        if sy < 1e-9 {
            return Err(JsThrow::error("Increase zero scale before changing the 3D hierarchy"));
        }
        for v in &mut y {
            *v /= sy;
        }
        let z = [
            x[1] * y[2] - x[2] * y[1],
            x[2] * y[0] - x[0] * y[2],
            x[0] * y[1] - x[1] * y[0],
        ];
        let sz = z[0] * m[8] + z[1] * m[9] + z[2] * m[10];
        let shear_x = x[0] * m[8] + x[1] * m[9] + x[2] * m[10];
        let shear_y = y[0] * m[8] + y[1] * m[9] + y[2] * m[10];
        if js_max(shear_x.abs(), shear_y.abs()) > 1e-6 * js_max(1.0, sz.abs()) {
            return Err(JsThrow::error(
                "This 3D hierarchy has depth shear. Use uniform group scale before ungrouping or reparenting",
            ));
        }
        let ry = math::asin(js_max(-1.0, crate::scope::js_min(1.0, -x[2])));
        let regular = math::cos(ry).abs() > 1e-7;
        let rx = if regular { math::atan2(y[2], z[2]) } else { 0.0 };
        let rz = if regular {
            math::atan2(x[1], x[0])
        } else {
            math::atan2(-y[0], y[1])
        };
        let ax = self.value3d(layer, "anchor.x", time, 0.0)?;
        let ay = self.value3d(layer, "anchor.y", time, 0.0)?;
        let az = if flag(layer, "threeD") {
            self.value3d(layer, "anchor.z", time, 0.0)?
        } else {
            0.0
        };
        let deg = |r: f64| r * 180.0 / core::f64::consts::PI;
        Ok(Pose3d {
            position: [
                m[12] + m[0] * ax + m[4] * ay + m[8] * az,
                m[13] + m[1] * ax + m[5] * ay + m[9] * az,
                m[14] + m[2] * ax + m[6] * ay + m[10] * az,
            ],
            scale: [sx * 100.0, sy * 100.0, sz * 100.0],
            rotation: deg(rz),
            rotation_x: deg(rx),
            rotation_y: deg(ry),
            skew: deg(math::atan(xy / sy)),
        })
    }
}

/// `poseValues3D`'s channels (orientation is always 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose3d {
    /// `position.x`, `position.y`, `position.z`.
    pub position: [f64; 3],
    /// `scale.x`, `scale.y`, `scale.z` (percent).
    pub scale: [f64; 3],
    /// `rotation` (Z), degrees.
    pub rotation: f64,
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub skew: f64,
}

impl Pose3d {
    /// The channels as `poseValues3D` lists them.
    pub fn channels(&self) -> [(&'static str, f64); 13] {
        [
            ("position.x", self.position[0]),
            ("position.y", self.position[1]),
            ("position.z", self.position[2]),
            ("scale.x", self.scale[0]),
            ("scale.y", self.scale[1]),
            ("scale.z", self.scale[2]),
            ("rotation", self.rotation),
            ("rotation.x", self.rotation_x),
            ("rotation.y", self.rotation_y),
            ("orientation.x", 0.0),
            ("orientation.y", 0.0),
            ("orientation.z", 0.0),
            ("skew", self.skew),
        ]
    }
}

/// `parent3D`'s walk.
struct Walk3d {
    groups: Vec<Option<crate::comp::IdKey>>,
    matrix: Mat4,
}

impl Walk3d {
    fn append_parents<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        first: Option<&'d Node>,
        time: f64,
    ) -> JsResult<()> {
        let mut chain: Vec<&'d Node> = Vec::new();
        let mut seen: Vec<&'d Node> = vec![layer];
        let mut cur = first;
        while let Some(c) = cur {
            if seen.iter().any(|l| same(l, c)) || chain.len() >= 256 {
                break;
            }
            seen.push(c);
            chain.push(c);
            self.append_groups(scope, c, time)?;
            cur = scope.parent_by_id(c);
        }
        for item in chain.iter().rev() {
            let local = scope.local3d(item, time)?;
            self.matrix = multiply(&self.matrix, &local);
        }
        Ok(())
    }

    fn append_groups<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        time: f64,
    ) -> JsResult<()> {
        let groups = scope.group_ancestors(layer)?;
        for group in groups.iter().rev() {
            let id = crate::comp::IdKey::of(&node::get(group, b"id"));
            if self.groups.contains(&id) {
                continue;
            }
            self.groups.push(id);
            let first = scope.parent_by_id(group);
            self.append_parents(scope, group, first, time)?;
            let local = scope.local3d(group, time)?;
            self.matrix = multiply(&self.matrix, &local);
        }
        Ok(())
    }
}
