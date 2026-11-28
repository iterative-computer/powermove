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

use std::collections::HashMap;

use pm_model::Node;

use crate::js_sem::{JsResult, JsThrow};
use crate::math;
use crate::node;
use crate::parenting::{Role3d, transform_aim};
use crate::scope::{Scope, flag, js_hypot, js_max, same};
use crate::transform::Mat2d;

/// A column-major 4×4 matrix.
pub type Mat4 = [f64; 16];
/// A column-major 3×3 homography.
pub type Mat3 = [f64; 9];

/// A camera in Studio scene units (Y up, looking down its local −Z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera3d {
    pub position: [f64; 3],
    pub target: [f64; 3],
    pub roll: f64,
    /// Vertical field of view, in degrees, before zoom.
    pub fov: f64,
    pub zoom: f64,
    pub ortho: bool,
    /// Full vertical span, before zoom.
    pub ortho_height: f64,
}

/// The pixel-space bridge shared by flat 3D layers and Studio extrusions.
pub fn scene_units_per_pixel(height: f64) -> f64 {
    6.0 / height.max(1.0)
}

pub fn pixel_to_scene(width: f64, height: f64) -> Mat4 {
    let u = scene_units_per_pixel(height);
    [
        u,
        0.0,
        0.0,
        0.0,
        0.0,
        -u,
        0.0,
        0.0,
        0.0,
        0.0,
        -u,
        0.0,
        -width * u / 2.0,
        3.0,
        0.0,
        1.0,
    ]
}

/// Use legacy framing for the editor when only flat planes need a camera.
pub fn uses_default_25d_camera<'a>(layers: impl IntoIterator<Item = &'a Node>) -> bool {
    let mut planes = false;
    for layer in layers {
        if Role3d::of(layer) == Some(Role3d::Object) {
            return false;
        }
        if flag(layer, "threeD")
            && Role3d::of(layer).is_none()
            && !node::is_str(layer.get("type"), "null")
        {
            let data = layer.get("d").and_then(|d| d.get("extrusion"));
            if data.is_some_and(|d| flag(d, "enabled")) {
                return false;
            }
            planes = true;
        }
    }
    planes
}

/// The legacy 50 mm / 36 mm horizontal film gate, expressed in scene units.
pub fn default_camera_25d(width: f64, height: f64) -> Camera3d {
    let focal = width.max(1.0) * 50.0 / 36.0;
    Camera3d {
        position: [0.0, 0.0, focal * scene_units_per_pixel(height)],
        target: [0.0; 3],
        roll: 0.0,
        fov: (2.0 * (height / (2.0 * focal)).atan()).to_degrees(),
        zoom: 1.0,
        ortho: false,
        ortho_height: 6.0,
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalized(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    if length > 0.0 {
        v.map(|x| x / length)
    } else {
        v
    }
}

impl Camera3d {
    fn forward(&self) -> [f64; 3] {
        normalized(std::array::from_fn(|i| self.target[i] - self.position[i]))
    }

    pub fn basis(&self) -> [[f64; 3]; 3] {
        let forward = self.forward();
        let right = cross(forward, [0.0, 1.0, 0.0]);
        let right = if dot(right, right) < 1e-12 {
            [1.0, 0.0, 0.0]
        } else {
            normalized(right)
        };
        let up = cross(right, forward);
        let (s, c) = self.roll.to_radians().sin_cos();
        [
            forward,
            std::array::from_fn(|i| right[i] * c + up[i] * s),
            std::array::from_fn(|i| up[i] * c - right[i] * s),
        ]
    }

    /// Recover the shot's roll from its parented world-space up axis.
    pub fn roll_from_up(&self, up: [f64; 3]) -> f64 {
        let [_, right, base_up] = Self { roll: 0.0, ..*self }.basis();
        let x = -dot(up, right);
        let y = dot(up, base_up);
        if x.hypot(y) < 1e-6 {
            0.0
        } else {
            x.atan2(y).to_degrees()
        }
    }

    /// Maps an affine layer plane to homogeneous composition pixels.
    /// Positive w is in front of the eye, including when viewing the back.
    pub fn project_plane(&self, world: &Mat4, width: f64, height: f64) -> Mat3 {
        let scene = multiply(&pixel_to_scene(width, height), world);
        let [forward, right, up] = self.basis();
        let lens_height = if self.ortho {
            self.ortho_height
        } else {
            2.0 * (self.fov.to_radians() / 2.0).tan()
        };
        let focal = height * self.zoom / lens_height;
        if self.ortho {
            let origin = std::array::from_fn(|i| scene[12 + i] - self.position[i]);
            if dot(origin, forward) <= 0.0
                && dot([scene[0], scene[1], scene[2]], forward).abs() < 1e-12
                && dot([scene[4], scene[5], scene[6]], forward).abs() < 1e-12
            {
                // A screen-parallel orthographic plane has a single depth.
                return [0., 0., 0., 0., 0., 0., 0., 0., -1.];
            }
        }
        let mut plane = [0.0; 9];
        for (i, c) in [0, 4, 12].into_iter().enumerate() {
            let v =
                std::array::from_fn(|j| scene[c + j] - if i == 2 { self.position[j] } else { 0.0 });
            let w = if self.ortho {
                if i == 2 { 1.0 } else { 0.0 }
            } else {
                dot(v, forward)
            };
            plane[i * 3] = focal * dot(v, right) + width / 2.0 * w;
            plane[i * 3 + 1] = -focal * dot(v, up) + height / 2.0 * w;
            plane[i * 3 + 2] = w;
        }
        plane
    }
}

/// The 3D transform channels and their defaults (`CHANNELS_3D`).
pub const CHANNELS_3D: [(&str, f64); 11] = [
    ("perspective", 50.0),
    ("position.z", 0.0),
    ("anchor.z", 0.0),
    ("scale.z", 100.0),
    ("rotation.x", 0.0),
    ("rotation.y", 0.0),
    ("orientation.x", 0.0),
    ("orientation.y", 0.0),
    ("orientation.z", 0.0),
    ("dofBlur", 0.0),
    ("dofFocus", 0.0),
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

fn camera_data(layer: &Node) -> Option<&Node> {
    layer.get("d")?.get("data")?.get("camera")
}

/// A pass's coplanar runs, independent of the camera used to sort them.
#[derive(Default)]
pub struct PlaneOrder {
    runs: Vec<PlaneRun>,
}

struct PlaneRun {
    start: usize,
    surfaces: Vec<PlaneSurface>,
    inactive: Vec<usize>,
}

#[derive(Default)]
struct PlaneSurface {
    plane: Option<[f64; 4]>,
    members: Vec<usize>,
    depth: f64,
    anchor: [f64; 3],
}

const PLANE_TOLERANCE: [f64; 4] = [1e-7, 1e-7, 1e-7, 1e-5];
// Power-of-two cells make scaling exact and each cell over twice as wide
// as the tolerance. A match can only be in this cell or its nearer neighbor.
const PLANE_CELL_SCALE: [f64; 4] = [4_194_304., 4_194_304., 4_194_304., 32_768.];
const INDEX_SURFACES_AT: usize = 16;
type PlaneCell = [i64; 4];
type SurfaceIndex = HashMap<PlaneCell, Vec<usize>>;

fn plane_cells(plane: [f64; 4]) -> (PlaneCell, PlaneCell) {
    let scaled: [f64; 4] = std::array::from_fn(|i| plane[i] * PLANE_CELL_SCALE[i]);
    let cell = scaled.map(|v| v.floor() as i64);
    let neighbor = std::array::from_fn(|i| {
        cell[i].saturating_add(if scaled[i] - scaled[i].floor() < 0.5 {
            -1
        } else {
            1
        })
    });
    (cell, neighbor)
}

fn matching_surface(
    surfaces: &[PlaneSurface],
    index: Option<&SurfaceIndex>,
    plane: [f64; 4],
) -> Option<usize> {
    let matches = |surface: &PlaneSurface| {
        surface
            .plane
            .is_some_and(|q| (0..4).all(|i| (plane[i] - q[i]).abs() < PLANE_TOLERANCE[i]))
    };
    let Some(index) = index else {
        return surfaces.iter().position(matches);
    };
    let (cell, neighbor) = plane_cells(plane);
    // Tolerance is nontransitive. Pick the earliest matching representative,
    // exactly as the timeline-order linear search does.
    (0..16)
        .filter_map(|bits| {
            let key = std::array::from_fn(|i| {
                if bits & (1 << i) == 0 {
                    cell[i]
                } else {
                    neighbor[i]
                }
            });
            index
                .get(&key)?
                .iter()
                .copied()
                .find(|&i| matches(&surfaces[i]))
        })
        .min()
}

impl PlaneOrder {
    /// Indices whose world transforms contributed to the cached surfaces.
    #[doc(hidden)]
    pub fn world_layers(&self) -> impl Iterator<Item = usize> + '_ {
        self.runs
            .iter()
            .filter(|run| {
                run.surfaces
                    .iter()
                    .map(|surface| surface.members.len())
                    .sum::<usize>()
                    > 1
            })
            .flat_map(|run| {
                run.surfaces
                    .iter()
                    .flat_map(|surface| surface.members.iter().copied())
            })
    }

    /// Order the same layer slice used to prepare the surfaces.
    pub fn project<'d>(
        &self,
        scope: &mut Scope<'_, 'd>,
        layers: &[&'d Node],
        time: f64,
    ) -> JsResult<Vec<&'d Node>> {
        let mut result = layers.to_vec();
        let camera = if self.runs.iter().any(|run| run.surfaces.len() > 1) {
            scope.projection_camera_3d(time)?
        } else {
            None
        };
        let forward = camera.map(|camera| camera.forward());
        for PlaneRun {
            start,
            surfaces,
            inactive,
        } in &self.runs
        {
            if surfaces.len() == 1 {
                if !inactive.is_empty() {
                    for (slot, &i) in (*start..).zip(surfaces[0].members.iter().chain(inactive)) {
                        result[slot] = layers[i];
                    }
                }
                continue;
            }
            let mut depths: Vec<_> = surfaces
                .iter()
                .map(|surface| {
                    let depth = match (camera, forward) {
                        (Some(camera), Some(forward)) => dot(
                            std::array::from_fn(|i| surface.anchor[i] - camera.position[i]),
                            forward,
                        ),
                        _ => surface.depth,
                    };
                    (depth, surface)
                })
                .collect();
            // Stable, including NaN comparisons (Array.prototype.sort).
            depths.sort_by(|a, b| {
                (a.0 - b.0)
                    .partial_cmp(&0.0)
                    .unwrap_or(core::cmp::Ordering::Equal)
            });
            let indices = depths
                .into_iter()
                .flat_map(|(_, s)| &s.members)
                .chain(inactive);
            for (slot, &i) in (*start..).zip(indices) {
                result[slot] = layers[i];
            }
        }
        Ok(result)
    }
}

impl<'d> Scope<'_, 'd> {
    /// `value(PM, L, key, T, fallback)`: the channel as a finite number, or
    /// `fallback` when the channel is missing (`L.p?.[key] == null`) or not
    /// finite.
    pub fn value3d(
        &mut self,
        layer: &'d Node,
        key: &str,
        time: f64,
        fallback: f64,
    ) -> JsResult<f64> {
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
        let key = (crate::scope::address(layer), time.to_bits());
        if let Some(m) = self.local_memo.as_ref().and_then(|m| m.local3d.get(&key)) {
            return Ok(*m);
        }
        let m = self.local3d_unmemoized(layer, time)?;
        if let Some(memo) = self.local_memo.as_mut() {
            memo.local3d.insert(key, m);
        }
        Ok(m)
    }

    fn local3d_unmemoized(&mut self, layer: &'d Node, time: f64) -> JsResult<Mat4> {
        if !flag(layer, "threeD") {
            return Ok(lift(&self.local_matrix(layer, time)?));
        }
        let x = self.value3d(layer, "position.x", time, 0.0)?;
        let y = self.value3d(layer, "position.y", time, 0.0)?;
        let z = self.value3d(layer, "position.z", time, 0.0)?;
        let mut m = translation(x, y, z);
        self.apply_local_rotation3d(layer, time, &mut m)?;
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

    /// Apply the layer's Rotation and Orientation in world3d's local order.
    fn apply_local_rotation3d(
        &mut self,
        layer: &'d Node,
        time: f64,
        m: &mut Mat4,
    ) -> JsResult<bool> {
        let mut rotated = false;
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
                *m = multiply(m, &rotation(axis, degrees));
                rotated = true;
            }
        }
        Ok(rotated)
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
        // Only the layer's own parent lookup is remembered.
        let key = parent
            .is_none()
            .then(|| (crate::scope::address(layer), time.to_bits()));
        if let Some(m) = key.and_then(|k| self.local_memo.as_ref()?.parent3d.get(&k).copied()) {
            return Ok(m);
        }
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
        if let (Some(k), Some(memo)) = (key, self.local_memo.as_mut()) {
            memo.parent3d.insert(k, walk.matrix);
        }
        Ok(walk.matrix)
    }

    /// `world3D(PM, L, T, positioning)`. With `positioning`, the layer's own
    /// transform is only its Z translation (`L.threeD`).
    pub fn world3d(&mut self, layer: &'d Node, time: f64, positioning: bool) -> JsResult<Mat4> {
        if positioning {
            let z = if flag(layer, "threeD") {
                self.value3d(layer, "position.z", time, 0.0)?
            } else {
                0.0
            };
            let parent = self.parent3d(layer, time, None)?;
            return Ok(multiply(&parent, &translation(0.0, 0.0, z)));
        }
        let key = (crate::scope::address(layer), time.to_bits());
        if let Some(world) = self
            .local_memo
            .as_ref()
            .and_then(|memo| memo.world3d.get(&key))
        {
            return Ok(*world);
        }
        let own = self.local3d(layer, time)?;
        let parent = self.parent3d(layer, time, None)?;
        let world = multiply(&parent, &own);
        if let Some(memo) = self.local_memo.as_mut() {
            memo.world3d.insert(key, world);
        }
        Ok(world)
    }

    /// `is3DLayer(PM, layer)`: the layer's 3D switch or any enclosing 3D group.
    pub fn is_3d_layer(&mut self, layer: &'d Node) -> JsResult<bool> {
        if flag(layer, "threeD") {
            return Ok(true);
        }
        if !node::truthy(layer.get("group")) {
            return Ok(false);
        }
        let key = crate::scope::address(layer);
        if let Some(value) = self
            .local_memo
            .as_ref()
            .and_then(|memo| memo.is_3d.get(&key))
        {
            return Ok(*value);
        }
        let value = self
            .group_ancestors(layer)?
            .iter()
            .any(|g| flag(g, "threeD"));
        if let Some(memo) = self.local_memo.as_mut() {
            memo.is_3d.insert(key, value);
        }
        Ok(value)
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
        Ok(js_max(
            0.001,
            self.value3d(owner, "perspective", time, 50.0)?,
        ))
    }

    /// 2.5D Depth of Field `(focus_z_px, blur_percent)` when `layer` has
    /// `threeD: true` and `dofBlur > 0`, else `None`.
    pub fn dof_25d(&mut self, layer: &'d Node, time: f64) -> JsResult<Option<(f64, f64)>> {
        if !flag(layer, "threeD") {
            return Ok(None);
        }
        let blur = self.value3d(layer, "dofBlur", time, 0.0)?;
        if !(blur > 0.0) {
            return Ok(None);
        }
        let focus = self.value3d(layer, "dofFocus", time, 0.0)?;
        Ok(Some((focus, blur.clamp(0.0, 100.0))))
    }

    /// Active perspective 3D camera Depth of Field `(focus_distance_scene_units, blur_percent)`
    /// when looking through the shot camera and `camera.blur > 0` (or legacy `camera.fStop > 0`).
    pub fn camera_dof_3d(&mut self, time: f64) -> JsResult<Option<(f64, f64)>> {
        if self.view_camera.is_some() {
            return Ok(None);
        }
        let key = time.to_bits();
        if let Some((last, dof)) = self.active_camera_dof
            && last == key
        {
            return Ok(dof);
        }
        let mut cam_layer = None;
        for layer in self.layers() {
            if Role3d::of(layer) == Some(Role3d::Camera) {
                let projection = camera_data(layer).and_then(|node| node.get("projection"));
                if (node::is_str(projection, "perspective")
                    || node::is_str(projection, "orthographic"))
                    && self.active(layer, time)?
                {
                    cam_layer = Some(layer);
                    break;
                }
            }
        }
        let dof = if let Some(layer) = cam_layer
            && let Some(node) = camera_data(layer)
            && !node::is_str(node.get("projection"), "orthographic")
        {
            let mut channel = |key: &str, fallback: f64| -> JsResult<f64> {
                let Some(prop) = node.get("p").and_then(|p| p.get(key)) else {
                    return Ok(fallback);
                };
                let n = self
                    .channel_p(layer, Some(prop), time, &format!("camera.{key}"))?
                    .to_number();
                Ok(if n.is_finite() { n } else { fallback })
            };
            let mut blur = channel("blur", 0.0)?;
            if !(blur > 0.0) {
                let f_stop = channel("fStop", 0.0)?;
                if f_stop > 0.0 {
                    blur = (90.0 / f_stop).clamp(1.0, 100.0);
                }
            }
            if blur > 0.0 {
                let mut focus = channel("focusDistance", 0.0)?;
                if !(focus > 0.05) {
                    let cam = self.camera_3d(layer, time)?;
                    let d: [f64; 3] = std::array::from_fn(|i| cam.target[i] - cam.position[i]);
                    focus = js_hypot(&d).max(0.5);
                }
                Some((focus.max(0.05), blur.clamp(0.0, 100.0)))
            } else {
                None
            }
        } else {
            None
        };
        self.active_camera_dof = Some((key, dof));
        Ok(dof)
    }

    /// The first enabled Camera in stack order, as in the Studio renderer.
    pub fn active_camera_3d(&mut self, time: f64) -> JsResult<Option<Camera3d>> {
        let key = time.to_bits();
        if let Some((last, camera)) = self.active_camera
            && last == key
        {
            return Ok(camera);
        }
        let mut camera = None;
        for layer in self.layers() {
            // Studio's saved camera schema always names its projection.
            // Legacy camera-shaped extensions keep the original plane matrix.
            if Role3d::of(layer) == Some(Role3d::Camera) {
                let projection = camera_data(layer).and_then(|node| node.get("projection"));
                if (node::is_str(projection, "perspective")
                    || node::is_str(projection, "orthographic"))
                    && self.active(layer, time)?
                {
                    camera = Some(self.camera_3d(layer, time)?);
                    break;
                }
            }
        }
        self.active_camera = Some((key, camera));
        Ok(camera)
    }

    /// A Studio Camera at this time, using the same parented eye/aim as meshes.
    pub fn camera_3d(&mut self, layer: &'d Node, time: f64) -> JsResult<Camera3d> {
        self.camera_3d_with_up(layer, time)
            .map(|(camera, _)| camera)
    }

    /// The evaluated camera and its world-space up vector before normalization.
    pub fn camera_3d_with_up(
        &mut self,
        layer: &'d Node,
        time: f64,
    ) -> JsResult<(Camera3d, [f64; 3])> {
        let node = camera_data(layer);
        let mut channel = |key: &str, fallback: f64| -> JsResult<f64> {
            let Some(prop) = node.and_then(|n| n.get("p")).and_then(|p| p.get(key)) else {
                return Ok(fallback);
            };
            let n = self
                .channel_p(layer, Some(prop), time, &format!("camera.{key}"))?
                .to_number();
            Ok(if n.is_finite() { n } else { fallback })
        };
        let aim = [
            channel("targetX", 0.0)?,
            channel("targetY", 0.0)?,
            channel("targetZ", 0.0)?,
        ];
        let fov = channel("fov", 45.0)?.clamp(1.0, 150.0);
        let zoom = channel("zoom", 1.0)?.clamp(0.001, 1000.0);
        let world = self.world3d(layer, time, false)?;
        let parent = self.parent3d(layer, time, None)?;
        let position = [world[12], world[13], world[14]];
        let mut target = transform_aim(&parent, &aim);
        let delta: [f64; 3] = std::array::from_fn(|i| target[i] - position[i]);
        let distance = js_hypot(&delta);
        let free_rotation = node::is_str(node.and_then(|n| n.get("autoOrient")), "off");
        if free_rotation || distance < 1e-6 {
            let forward = normalized([-world[8], -world[9], -world[10]]);
            let distance = if distance > 1e-6 { distance } else { 10.0 };
            target = std::array::from_fn(|i| position[i] + forward[i] * distance);
        }
        let mut camera = Camera3d {
            position,
            target,
            roll: 0.0,
            fov,
            zoom,
            ortho: node::is_str(node.and_then(|n| n.get("projection")), "orthographic"),
            ortho_height: 6.0,
        };
        let mut up = [world[4], world[5], world[6]];
        let mut local_rotation = IDENTITY4;
        if !free_rotation
            && distance >= 1e-6
            && flag(layer, "threeD")
            && self.apply_local_rotation3d(layer, time, &mut local_rotation)?
        {
            // Start with the legacy look-at framing, including any roll from
            // the parented Y axis before the camera's own rotation.
            let sy = self.value3d(layer, "scale.y", time, 100.0)? / 100.0;
            let skew_y = math::tan(math::deg_to_rad(self.value3d(layer, "skew", time, 0.0)?)) * sy;
            let base_up = std::array::from_fn(|i| parent[i] * skew_y + parent[4 + i] * sy);
            camera.roll = camera.roll_from_up(base_up);
            let [forward, right, up_base] = camera.basis();
            // The look-at basis maps local +X, +Y, and +Z to right, up,
            // and backward. Rotate local -Z and +Y within that basis.
            let to_world = |v: [f64; 3]| {
                std::array::from_fn(|i| right[i] * v[0] + up_base[i] * v[1] - forward[i] * v[2])
            };
            let [rx, ry, rz] = [local_rotation[8], local_rotation[9], local_rotation[10]];
            if [rx, ry, rz] != [0.0, 0.0, 1.0] {
                let forward = normalized(to_world([-rx, -ry, -rz]));
                camera.target = std::array::from_fn(|i| position[i] + forward[i] * distance);
            }
            up = to_world([local_rotation[4], local_rotation[5], local_rotation[6]]);
        }
        camera.roll = camera.roll_from_up(up);
        Ok((camera, up))
    }

    pub fn projection_camera_3d(&mut self, time: f64) -> JsResult<Option<Camera3d>> {
        match self.view_camera {
            Some(camera) => Ok(Some(camera)),
            None => self.active_camera_3d(time),
        }
    }

    /// `planeMatrix(PM, L, T, positioning)`: layer pixels to composition
    /// pixels.
    pub fn plane_matrix(
        &mut self,
        layer: &'d Node,
        time: f64,
        positioning: bool,
    ) -> JsResult<Mat3> {
        if !self.is_3d_layer(layer)? {
            let m = self.world_matrix(layer, time)?;
            return Ok([m[0], m[1], 0.0, m[2], m[3], 0.0, m[4], m[5], 1.0]);
        }
        let m = self.world3d(layer, time, positioning)?;
        let (w, h) = self.comp_size();
        if let Some(camera) = self.projection_camera_3d(time)? {
            return Ok(camera.project_plane(&m, w, h));
        }
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
    pub fn plane_contains(
        &mut self,
        layer: &'d Node,
        time: f64,
        x: f64,
        y: f64,
        b: &Rect,
    ) -> JsResult<bool> {
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
    pub fn depth_ordered_layers(
        &mut self,
        layers: &[&'d Node],
        time: f64,
    ) -> JsResult<Vec<&'d Node>> {
        self.depth_ordered_layers_inner(layers, time, false)
    }

    /// Renderer-only ordering using the drawable planes' surfaces. Keep
    /// inactive runs, inactive tails, and 2D barriers in their original slots.
    pub fn depth_ordered_draw_layers(
        &mut self,
        layers: &[&'d Node],
        time: f64,
    ) -> JsResult<Vec<&'d Node>> {
        self.depth_ordered_layers_inner(layers, time, true)
    }

    fn depth_ordered_layers_inner(
        &mut self,
        layers: &[&'d Node],
        time: f64,
        skip_inactive: bool,
    ) -> JsResult<Vec<&'d Node>> {
        let order = self.prepare_plane_order(layers, time, skip_inactive)?;
        order.project(self, layers, time)
    }

    /// Camera-independent coplanar surfaces of a drawing pass.
    #[doc(hidden)]
    pub fn prepare_plane_order(
        &mut self,
        layers: &[&'d Node],
        time: f64,
        skip_inactive: bool,
    ) -> JsResult<PlaneOrder> {
        let mut order = PlaneOrder::default();
        if layers.len() <= 1 {
            return Ok(order);
        }
        let layers = if skip_inactive {
            // No ordering is needed past the last drawable plane. Entire
            // inactive passes can skip the group ancestry lookups as well.
            let mut last = None;
            for (i, &layer) in layers.iter().enumerate().rev() {
                if !crate::evaluator::outside_layer_span(layer, time) && self.is_3d_layer(layer)? {
                    last = Some(i);
                    break;
                }
            }
            let Some(last) = last else {
                return Ok(order);
            };
            &layers[..=last]
        } else {
            layers
        };
        let mut flags = Vec::with_capacity(layers.len());
        for layer in layers {
            flags.push(self.is_3d_layer(layer)?);
        }
        let (width, height) = self.comp_size();
        let to_scene = pixel_to_scene(width, height);
        let mut start = 0;
        while start < layers.len() {
            if !flags[start] {
                start += 1;
                continue;
            }
            let end = start + flags[start..].iter().take_while(|&&flag| flag).count();
            let mut run = PlaneRun {
                start,
                surfaces: Vec::new(),
                inactive: Vec::new(),
            };
            let mut active = Vec::new();
            for i in start..end {
                if skip_inactive && crate::evaluator::outside_layer_span(layers[i], time) {
                    run.inactive.push(i);
                } else {
                    active.push(i);
                }
            }
            start = end;
            let Some(&last) = active.last() else {
                continue;
            };
            // Keep trailing inactive slots and 2D barriers in place. Earlier
            // inactive planes follow the sorted drawable planes in this run.
            run.inactive.retain(|&i| i < last);
            if active.len() == 1 {
                if !run.inactive.is_empty() {
                    run.surfaces.push(PlaneSurface {
                        members: active,
                        ..Default::default()
                    });
                    order.runs.push(run);
                }
                continue;
            }
            let mut index: Option<SurfaceIndex> = None;
            for i in active {
                let layer = layers[i];
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
                    && let Some(existing) = matching_surface(&run.surfaces, index.as_ref(), p)
                {
                    run.surfaces[existing].members.push(i);
                    continue;
                }
                if let Some(index) = &mut index
                    && let Some(p) = plane
                {
                    index
                        .entry(plane_cells(p).0)
                        .or_default()
                        .push(run.surfaces.len());
                }
                let ax = self.value3d(layer, "anchor.x", time, 0.0)?;
                let ay = self.value3d(layer, "anchor.y", time, 0.0)?;
                let az = if flag(layer, "threeD") {
                    self.value3d(layer, "anchor.z", time, 0.0)?
                } else {
                    0.0
                };
                let depth = m[2] * ax + m[6] * ay + m[10] * az + m[14];
                let anchor = transform_aim(&m, &[ax, ay, az]);
                run.surfaces.push(PlaneSurface {
                    plane,
                    members: vec![i],
                    depth,
                    anchor: transform_aim(&to_scene, &anchor),
                });
                if run.surfaces.len() == INDEX_SURFACES_AT && index.is_none() {
                    let mut cells = SurfaceIndex::new();
                    for (i, surface) in run.surfaces.iter().enumerate() {
                        if let Some(p) = surface.plane {
                            cells.entry(plane_cells(p).0).or_default().push(i);
                        }
                    }
                    index = Some(cells);
                }
            }
            order.runs.push(run);
        }
        Ok(order)
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
            return Err(JsThrow::error(
                "Increase zero scale before changing the 3D hierarchy",
            ));
        }
        let x = [m[0] / sx, m[1] / sx, m[2] / sx];
        let xy = x[0] * m[4] + x[1] * m[5] + x[2] * m[6];
        let mut y = [m[4] - xy * x[0], m[5] - xy * x[1], m[6] - xy * x[2]];
        let sy = js_hypot(&y);
        if sy < 1e-9 {
            return Err(JsThrow::error(
                "Increase zero scale before changing the 3D hierarchy",
            ));
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
        let rx = if regular {
            math::atan2(y[2], z[2])
        } else {
            0.0
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Comp, Evaluator};

    fn root(layers: serde_json::Value) -> Node {
        let value = serde_json::json!({"id":"c","w":800,"h":500,"fps":30,"dur":10,"layers":layers});
        Node::from_value(pm_model::json::parse(&value.to_string()).unwrap(), 1)
    }

    fn close(value: (f64, f64), expected: (f64, f64)) {
        assert!(
            (value.0 - expected.0).abs() < 1e-9,
            "{value:?} != {expected:?}"
        );
        assert!(
            (value.1 - expected.1).abs() < 1e-9,
            "{value:?} != {expected:?}"
        );
    }

    #[test]
    fn legacy_matrix_is_unchanged_and_default_free_view_matches_it() {
        let root = root(serde_json::json!([{
            "id":"plane","threeD":true,"on":true,"from":0,"dur":10,
            "p":{"position.x":{"v":420},"position.y":{"v":205},"position.z":{"v":125}}
        }]));
        let comp = Comp::at_node(&root, &root, "c", 1);
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let layer = &comp.layers()[0];
        let focal = 800.0 * 50.0 / 36.0;
        let legacy = scope.plane_matrix(layer, 0., false).unwrap();
        assert_eq!(
            legacy,
            [
                1.,
                0.,
                0.,
                0.,
                1.,
                0.,
                420. + 400. * 125. / focal,
                205. + 250. * 125. / focal,
                1. + 125. / focal,
            ]
        );
        scope.view_camera = Some(default_camera_25d(800., 500.));
        let free = scope.plane_matrix(layer, 0., false).unwrap();
        close(
            project_point(&free, 50., -12.),
            project_point(&legacy, 50., -12.),
        );
    }

    #[test]
    fn editor_eye_projects_pitched_planes_and_sorts_by_its_forward_axis() {
        let root = root(serde_json::json!([
            {"id":"a","threeD":true,"p":{"position.x":{"v":600},"position.y":{"v":250}}},
            {"id":"b","threeD":true,"p":{"position.x":{"v":200},"position.z":{"v":-100}}}
        ]));
        let comp = Comp::at_node(&root, &root, "c", 1);
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let [a, b] = [&comp.layers()[0], &comp.layers()[1]];
        let mut camera = Camera3d {
            position: [0., 6., 8.],
            target: [0.; 3],
            fov: 90.,
            ..default_camera_25d(800., 500.)
        };
        scope.view_camera = Some(camera);
        let plane = scope.plane_matrix(a, 0., false).unwrap();
        // (1, 1, 0) in scene space is 9.4 units forward, 1 right and 0.8 up.
        let u = scene_units_per_pixel(500.);
        close(
            project_point(&plane, 400. + 1. / u - 600., -1. / u),
            (400. + 250. / 9.4, 250. - 200. / 9.4),
        );

        camera.position = [10., 0., 0.];
        scope.view_camera = Some(camera);
        assert!(same(scope.depth_ordered_layers(&[a, b], 0.).unwrap()[0], a));
        scope.view_camera = None;
        assert!(same(scope.depth_ordered_layers(&[a, b], 0.).unwrap()[0], b));

        camera.position = [0., 0., -6.];
        scope.view_camera = Some(camera);
        let back = scope.plane_matrix(a, 0., false).unwrap();
        assert!(back[8] > 0.);
        assert!(project_point(&back, 0., 0.).0 < 400.);
        camera.target = [0., 0., -12.];
        scope.view_camera = Some(camera);
        let behind = scope.plane_matrix(a, 0., false).unwrap();
        assert!(behind[8] < 0.);
    }

    #[test]
    fn the_renderer_keeps_depth_sorting_for_a_pass_with_an_active_plane() {
        let root = root(serde_json::json!([
            {"id":"a","type":"shape","threeD":true,"from":0,"dur":1,"p":{}},
            {"id":"b","type":"shape","threeD":true,"from":0,"dur":3,
                "p":{"position.z":{"v":-100}}}
        ]));
        let comp = Comp::at_node(&root, &root, "c", 1);
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let [a, b] = [&comp.layers()[0], &comp.layers()[1]];
        let pass = [a, b];
        // At two seconds one plane still draws and needs the original order.
        let normal = scope.depth_ordered_layers(&pass, 2.).unwrap();
        assert!(same(normal[0], b));
        assert!(same(scope.depth_ordered_draw_layers(&pass, 2.).unwrap()[0], b));
        // At four seconds neither plane can draw. Keep their stored order.
        assert!(same(
            scope.depth_ordered_draw_layers(&pass, 4.).unwrap()[0],
            a
        ));
    }

    #[test]
    fn draw_order_skips_expired_transforms_and_keeps_2d_barriers() {
        let mut layers: Vec<_> = (0..3000)
            .map(|i| {
                serde_json::json!({
                    "id": format!("expired{i}"), "threeD": true, "from": 0, "dur": 1
                })
            })
            .collect();
        layers.extend([
            serde_json::json!({"id":"front","threeD":true,"p":{"position.z":{"v":40}}}),
            serde_json::json!({"id":"back","threeD":true,"p":{"position.z":{"v":10}}}),
            serde_json::json!({"id":"barrier","from":0,"dur":1}),
            serde_json::json!({"id":"other_run","threeD":true,"p":{"position.z":{"v":-40}}}),
        ]);
        let root = root(serde_json::json!(layers));
        let comp = Comp::at_node(&root, &root, "c", 1);
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        scope.memoize_transforms();
        let layers: Vec<_> = comp.layers().iter().collect();
        let ordered = scope.depth_ordered_draw_layers(&layers, 2.).unwrap();
        assert!(same(ordered[0], layers[3001]));
        assert!(same(ordered[1], layers[3000]));
        assert!(same(ordered[2], layers[0]));
        assert!(same(ordered[3002], layers[3002]));
        assert!(same(ordered[3003], layers[3003]));
        let memo = scope.local_memo.as_ref().unwrap();
        assert_eq!(memo.local3d.len(), 2);
        assert_eq!(memo.world3d.len(), 2);
    }

    #[test]
    fn surface_index_matches_a_linear_scan_across_cells_and_ties() {
        let planes = [[0., 0., 1., -1e-6], [0., 0., 1., 12e-6]]
            .into_iter()
            .chain((1..320).map(|i| [-2e-8, 0., 1., f64::from(i) * 0.001]));
        let mut index = SurfaceIndex::new();
        let mut surfaces = Vec::new();
        for plane in planes {
            index
                .entry(plane_cells(plane).0)
                .or_default()
                .push(surfaces.len());
            surfaces.push(PlaneSurface {
                plane: Some(plane),
                ..Default::default()
            });
        }
        let tie = [0., 0., 1., 5e-6];
        assert_eq!(matching_surface(&surfaces, Some(&index), tie), Some(0));
        for surface in &surfaces {
            let plane = surface.plane.unwrap();
            for adjustment in [-1.01, -0.99, 0., 0.99, 1.01] {
                let query = std::array::from_fn(|i| plane[i] + adjustment * PLANE_TOLERANCE[i]);
                assert_eq!(
                    matching_surface(&surfaces, Some(&index), query),
                    matching_surface(&surfaces, None, query),
                    "plane {query:?}"
                );
            }
        }
    }

    #[test]
    fn transform_memo_keys_world_and_local_by_time_and_keeps_positioning_separate() {
        let root = root(serde_json::json!([
            {"id":"g","type":"group","threeD":true,"p":{"position.z":{"v":10}}},
            {"id":"child","group":"g","p":{"position.x":{"v":42}}}
        ]));
        let comp = Comp::at_node(&root, &root, "c", 1);
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        scope.memoize_transforms();
        let layer = &comp.layers()[1];
        assert!(scope.is_3d_layer(layer).unwrap());
        assert!(scope.is_3d_layer(layer).unwrap());
        let world = scope.world3d(layer, 0., false).unwrap();
        assert_eq!(scope.world3d(layer, 0., false).unwrap(), world);
        assert_eq!(scope.world3d(layer, 1., false).unwrap(), world);
        assert_ne!(scope.world3d(layer, 0., true).unwrap(), world);
        let memo = scope.local_memo.as_ref().unwrap();
        let key = crate::scope::address(layer);
        assert_eq!(memo.is_3d.get(&key), Some(&true));
        for time in [0_f64, 1.] {
            let key = (key, time.to_bits());
            assert!(memo.local3d.contains_key(&key));
            assert_eq!(memo.world3d.get(&key), Some(&world));
        }
        assert_eq!(memo.world3d.len(), 2);
    }

    #[test]
    fn prepared_surfaces_keep_drawable_order_for_each_camera() {
        let root = root(serde_json::json!([
            {"id":"a","type":"shape","threeD":true,"from":0,"dur":3,"p":{}},
            {"id":"b","type":"shape","threeD":true,"from":0,"dur":3,
                "p":{"position.z":{"v":30}}},
            {"id":"expired","type":"shape","threeD":true,"from":0,"dur":1,
                "p":{"position.z":{"v":-50}}}
        ]));
        let comp = Comp::at_node(&root, &root, "c", 1);
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let layers: Vec<_> = comp.layers().iter().collect();
        let prepared = scope.prepare_plane_order(&layers, 2., true).unwrap();
        let mut camera = default_camera_25d(1000., 500.);
        for z in [6., -6.] {
            camera.position = [0., 0., z];
            scope.view_camera = Some(camera);
            let full = scope.depth_ordered_layers(&layers, 2.).unwrap();
            let visible = full
                .into_iter()
                .filter(|l| !same(l, layers[2]))
                .collect::<Vec<_>>();
            let cached = prepared.project(&mut scope, &layers, 2.).unwrap();
            assert!(visible.iter().zip(&cached).all(|(a, b)| same(a, b)));
            assert!(same(cached[2], layers[2]));
        }
    }

    #[test]
    fn camera_rotation_offsets_its_point_of_interest() {
        let [s, c] = [0.5, 3.0_f64.sqrt() / 2.0];
        let d = std::f64::consts::FRAC_1_SQRT_2;
        for (channel, expected_forward, expected_up) in [
            ("rotation.x", [-d * c, s, -d * c], [d * s, c, d * s]),
            ("rotation.y", [-d * (c + s), 0., -d * (c - s)], [0., 1., 0.]),
            ("rotation", [-d, 0., -d], [-d * s, c, d * s]),
            ("orientation.x", [-d * c, s, -d * c], [d * s, c, d * s]),
        ] {
            let root = root(serde_json::json!([
                {"id":"camera","type":"extension","threeD":true,"on":true,"from":0,"dur":10,
                 "p":{"position.x":{"v":6},"position.z":{"v":6},(channel):{"v":30}},
                 "d":{"definition":"powermove.3d.camera","data":{"camera":{"projection":"perspective","p":{}}}}}
            ]));
            let comp = Comp::at_node(&root, &root, "c", 1);
            let mut ev = Evaluator::new();
            let mut scope = Scope::open(&mut ev, &comp);
            let camera = scope.active_camera_3d(0.).unwrap().unwrap();
            let [forward, _, up] = camera.basis();
            let pose = scope.scene_camera(0.).unwrap();
            assert_eq!(pose.position, camera.position);
            assert_eq!(pose.target, camera.target);
            for i in 0..3 {
                assert!(
                    (forward[i] - expected_forward[i]).abs() < 1e-12,
                    "{channel}: {forward:?}"
                );
                assert!((up[i] - expected_up[i]).abs() < 1e-12, "{channel}: {up:?}");
                assert!((pose.up[i] - expected_up[i]).abs() < 1e-12, "{channel}: {pose:?}");
            }
            assert_eq!(camera.position, [6., 0., 6.]);
        }
    }

    #[test]
    fn free_rotation_and_coincident_aim_keep_the_world_transform() {
        for (mode, target, distance) in [("off", 0., 6.), ("target", 6., 10.)] {
            let root = root(serde_json::json!([
                {"id":"camera","type":"extension","threeD":true,"on":true,"from":0,"dur":10,
                 "p":{"position.z":{"v":6},"rotation.x":{"v":25},"rotation.y":{"v":31},
                      "rotation":{"v":17},"orientation.y":{"v":15}},
                 "d":{"definition":"powermove.3d.camera","data":{"camera":{"projection":"perspective",
                     "autoOrient":mode,"p":{"targetZ":{"v":target}}}}}}
            ]));
            let comp = Comp::at_node(&root, &root, "c", 1);
            let mut ev = Evaluator::new();
            let mut scope = Scope::open(&mut ev, &comp);
            let world = scope.world3d(&comp.layers()[0], 0., false).unwrap();
            let forward = normalized([-world[8], -world[9], -world[10]]);
            let camera = scope.active_camera_3d(0.).unwrap().unwrap();
            assert_eq!(camera.position, [0., 0., 6.]);
            assert_eq!(
                camera.target,
                std::array::from_fn(|i| camera.position[i] + forward[i] * distance)
            );
            assert_eq!(camera.roll, camera.roll_from_up([world[4], world[5], world[6]]));
            assert_eq!(scope.scene_camera(0.).unwrap().target, camera.target);
        }
    }

    #[test]
    fn zero_local_camera_rotation_preserves_legacy_look_at_bits() {
        let root = root(serde_json::json!([
            {"id":"rig","type":"null","threeD":true,"on":true,"from":0,"dur":10,
             "p":{"position.x":{"v":2.3},"rotation.x":{"v":23},"rotation":{"v":37}}},
            {"id":"camera","type":"extension","threeD":true,"on":true,"from":0,"dur":10,"parent":"rig",
             "p":{"position.z":{"v":6.4},"scale.y":{"v":73},"skew":{"v":12}},
             "d":{"definition":"powermove.3d.camera","data":{"camera":{"projection":"perspective",
                 "p":{"targetX":{"v":1.3},"targetY":{"v":-0.5},"targetZ":{"v":0.9}}}}}}
        ]));
        let comp = Comp::at_node(&root, &root, "c", 1);
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let layer = &comp.layers()[1];
        let world = scope.world3d(layer, 0., false).unwrap();
        let parent = scope.parent3d(layer, 0., None).unwrap();
        let position = [world[12], world[13], world[14]];
        let target = transform_aim(&parent, &[1.3, -0.5, 0.9]);
        let prior = Camera3d {
            position,
            target,
            ..default_camera_25d(800., 500.)
        };
        let roll = prior.roll_from_up([world[4], world[5], world[6]]);
        assert!(roll.abs() > 1.0);

        let camera = scope.active_camera_3d(0.).unwrap().unwrap();
        assert_eq!(
            camera.position.map(f64::to_bits),
            position.map(f64::to_bits)
        );
        assert_eq!(camera.target.map(f64::to_bits), target.map(f64::to_bits));
        assert_eq!(camera.roll.to_bits(), roll.to_bits());
    }

    #[test]
    fn plane_follows_only_the_enabled_parented_camera() {
        let root = root(serde_json::json!([
            {"id":"disabled","threeD":true,"on":false,"from":0,"dur":10,
             "d":{"definition":"powermove.3d.camera"}},
            {"id":"rig","type":"null","threeD":true,"on":true,"from":0,"dur":10,
             "p":{"position.x":{"v":2},"position.y":{"v":1}}},
            {"id":"camera","type":"extension","threeD":true,"on":true,"from":0,"dur":10,"parent":"rig",
             "p":{"position.z":{"v":6}},
             "d":{"definition":"powermove.3d.camera","data":{"camera":{"projection":"orthographic",
                 "p":{"targetX":{"v":0},"targetY":{"v":0},"targetZ":{"v":0},"zoom":{"v":2}}}}}},
            {"id":"plane","threeD":true,"on":true,"from":0,"dur":10,
             "p":{"position.x":{"v":400},"position.y":{"v":250}}}
        ]));
        let comp = Comp::at_node(&root, &root, "c", 1);
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let camera = scope.active_camera_3d(0.).unwrap().unwrap();
        assert_eq!(camera.position, [2., 1., 6.]);
        assert_eq!(camera.target, [2., 1., 0.]);
        assert!(camera.ortho);
        let plane = scope.plane_matrix(&comp.layers()[3], 0., false).unwrap();
        close(
            project_point(&plane, 0., 0.),
            (400. - 500. * 2. / 3., 250. + 500. / 3.),
        );
    }
}
