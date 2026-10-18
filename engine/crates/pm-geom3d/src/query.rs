//! Picks and bounds on a placed 3D scene with three.js r180's semantics:
//! the questions `SceneRuntime.pick`, `SceneRuntime.bounds`,
//! `SceneRuntime.worldBounds`, and `Box3.setFromObject` answer in today's
//! app (`core/scene3d/runtime.ts`, `core/scene3d/service.ts`), so the
//! engine answers 3D picks, bounds, and model-group bounds without
//! three.js (plan section 8, appendix C rows C9 and C10).
//!
//! A [`Scene`] lists the runtime's objects (`runtime.objects`) in order,
//! each with its world matrix, its visibility (its own and its
//! ancestors'), and its shape:
//!
//! - a [`Shape::Mesh`]: a `THREE.Mesh` over triangle geometry (scene
//!   sources, OBJ meshes, the missing-model box) with its material or
//!   slot list;
//! - a [`Shape::Model`]: an imported glTF model at its pose, picked through
//!   [`crate::gltf::pick`] (the kernel the glTF lane validated on 7,584
//!   rays) under `sync`'s material rule;
//! - a [`Shape::Light`]: nothing to pick or bound.
//!
//! `Mesh.raycast` is reproduced step by step: the bounding sphere (unless
//! it contains the ray origin), the bounding box in local space, then every
//! triangle of the draw range, or of each group when the material is an
//! array (a group whose material is missing throws three.js's
//! `TypeError`, a mesh without groups under an array material has no
//! hits), with `Ray.intersectTriangle` and the material side. The closest
//! hit whose object passes `pick`'s filter (visible chain, depth-set ids,
//! not every material at opacity 0) wins.
//!
//! Known differences from today's runtime, all history-dependent there:
//! hits at exactly equal distances keep scene order rather than the
//! runtime's insertion history; a skinned model's bounding volumes are
//! those of the current pose (three.js caches the first pose it saw); an
//! unindexed bounding box is always present (three.js computes it on the
//! first bounds question; the test only prunes).

use std::sync::Arc;

use crate::Geometry;
use crate::gltf::math::{self, Mat4, Vec3};
use crate::gltf::pick::{Bvh, Picker, Posed, Ray, Side, intersect_triangle};
use crate::gltf::{Mode, ObjectKind};

/// three.js's `TypeError` message when a group's material is missing.
pub const MISSING_GROUP_MATERIAL: &str = "Cannot read properties of undefined (reading 'side')";

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `Vector3.normalize()`: `divideScalar(length() || 1)`.
pub fn normalize(v: Vec3) -> Vec3 {
    let l = length(v);
    let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
    let s = 1.0 / l;
    [v[0] * s, v[1] * s, v[2] * s]
}

/// JavaScript `Math.min` of two numbers (NaN propagates).
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// JavaScript `Math.max` of two numbers (NaN propagates).
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b || (a == b && a.is_sign_positive()) {
        a
    } else {
        b
    }
}

// ── cameras ──────────────────────────────────────────────────────────────

/// `Matrix4.lookAt(eye, target, up)`'s rotation.
fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    let mut z = sub(eye, target);
    if dot(z, z) == 0.0 {
        z[2] = 1.0;
    }
    z = normalize(z);
    let mut x = cross(up, z);
    if dot(x, x) == 0.0 {
        if up[2].abs() == 1.0 {
            z[0] += 0.0001;
        } else {
            z[2] += 0.0001;
        }
        z = normalize(z);
        x = cross(up, z);
    }
    x = normalize(x);
    let y = cross(z, x);
    let mut m = math::IDENTITY;
    m[0] = x[0];
    m[4] = y[0];
    m[8] = z[0];
    m[1] = x[1];
    m[5] = y[1];
    m[9] = z[1];
    m[2] = x[2];
    m[6] = y[2];
    m[10] = z[2];
    m
}

/// A camera's `matrixWorld` after `position.set(p)`, `up.set(up)`,
/// `lookAt(target)`, `updateMatrixWorld()` (through the quaternion, as
/// three.js stores the rotation).
pub fn camera_world(position: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    let q = math::quaternion_from_rotation(&look_at(position, target, up));
    math::compose(position, q, [1.0, 1.0, 1.0])
}

/// `Matrix4.makePerspective` (WebGL clip space).
fn make_perspective(left: f64, right: f64, top: f64, bottom: f64, near: f64, far: f64) -> Mat4 {
    let x = 2.0 * near / (right - left);
    let y = 2.0 * near / (top - bottom);
    let a = (right + left) / (right - left);
    let b = (top + bottom) / (top - bottom);
    let c = -(far + near) / (far - near);
    let d = (-2.0 * far * near) / (far - near);
    [
        x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, a, b, c, -1.0, 0.0, 0.0, d, 0.0,
    ]
}

/// `Matrix4.makeOrthographic` (WebGL clip space).
fn make_orthographic(left: f64, right: f64, top: f64, bottom: f64, near: f64, far: f64) -> Mat4 {
    let x = 2.0 / (right - left);
    let y = 2.0 / (top - bottom);
    let a = -(right + left) / (right - left);
    let b = -(top + bottom) / (top - bottom);
    let c = -2.0 / (far - near);
    let d = -(far + near) / (far - near);
    [
        x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, c, 0.0, a, b, d, 1.0,
    ]
}

/// A camera's lens as `updateProjectionMatrix` reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lens {
    /// `PerspectiveCamera` (`fov` in degrees).
    Perspective { fov: f64, zoom: f64, near: f64, far: f64 },
    /// `OrthographicCamera` with `resizeCamera`'s frustum (±3 × aspect, ±3).
    Orthographic { zoom: f64, near: f64, far: f64 },
}

impl Lens {
    /// `projectionMatrix` at `aspect` (`resizeCamera(w, h)`: `w / max(1, h)`).
    pub fn projection(&self, aspect: f64) -> Mat4 {
        match *self {
            Lens::Perspective {
                fov,
                zoom,
                near,
                far,
            } => {
                let deg2rad = std::f64::consts::PI / 180.0;
                let top = near * libm::tan(deg2rad * 0.5 * fov) / zoom;
                let height = 2.0 * top;
                let width = aspect * height;
                let left = -0.5 * width;
                make_perspective(left, left + width, top, top - height, near, far)
            }
            Lens::Orthographic { zoom, near, far } => {
                let (l, r, t, b) = (-3.0 * aspect, 3.0 * aspect, 3.0, -3.0);
                let dx = (r - l) / (2.0 * zoom);
                let dy = (t - b) / (2.0 * zoom);
                let cx = (r + l) / 2.0;
                let cy = (t + b) / 2.0;
                make_orthographic(cx - dx, cx + dx, cy + dy, cy - dy, near, far)
            }
        }
    }

    pub fn near(&self) -> f64 {
        match *self {
            Lens::Perspective { near, .. } | Lens::Orthographic { near, .. } => near,
        }
    }

    pub fn far(&self) -> f64 {
        match *self {
            Lens::Perspective { far, .. } | Lens::Orthographic { far, .. } => far,
        }
    }
}

/// `resizeCamera`'s aspect: `w / max(1, h)`.
pub fn aspect(w: f64, h: f64) -> f64 {
    w / js_max(1.0, h)
}

/// A camera as a pick or projection uses it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub lens: Lens,
    /// `matrixWorld`.
    pub world: Mat4,
    /// `matrixWorldInverse`.
    pub view: Mat4,
    /// `projectionMatrix` as last updated.
    pub projection: Mat4,
    /// `projectionMatrixInverse`.
    pub projection_inverse: Mat4,
}

impl Camera {
    /// A camera at `world` whose projection was last updated at `aspect`.
    pub fn new(lens: Lens, world: Mat4, aspect: f64) -> Camera {
        let projection = lens.projection(aspect);
        Camera {
            lens,
            world,
            view: math::invert(&world),
            projection,
            projection_inverse: math::invert(&projection),
        }
    }

    /// `resizeCamera(w, h, camera)`: the projection updated at `w / max(1, h)`.
    pub fn resized(&self, w: f64, h: f64) -> Camera {
        let projection = self.lens.projection(aspect(w, h));
        Camera {
            projection,
            projection_inverse: math::invert(&projection),
            ..*self
        }
    }

    fn orthographic(&self) -> bool {
        matches!(self.lens, Lens::Orthographic { .. })
    }

    /// `Vector3.unproject(camera)`.
    fn unproject(&self, v: Vec3) -> Vec3 {
        math::apply(&self.world, math::apply(&self.projection_inverse, v))
    }

    /// `Vector3.project(camera)`.
    pub fn project(&self, v: Vec3) -> Vec3 {
        math::apply(&self.projection, math::apply(&self.view, v))
    }

    /// `Raycaster.setFromCamera(ndc, camera)`.
    pub fn ray(&self, x: f64, y: f64) -> Ray {
        if self.orthographic() {
            let (near, far) = (self.lens.near(), self.lens.far());
            let origin = self.unproject([x, y, (near + far) / (near - far)]);
            // `transformDirection`: the upper 3 × 3, normalized.
            let m = &self.world;
            let d = [-m[8], -m[9], -m[10]];
            Ray {
                origin,
                direction: normalize(d),
            }
        } else {
            let origin = [self.world[12], self.world[13], self.world[14]];
            let p = self.unproject([x, y, 0.5]);
            Ray {
                origin,
                direction: normalize(sub(p, origin)),
            }
        }
    }
}

// ── boxes ────────────────────────────────────────────────────────────────

/// A `Box3` (`min > max` when empty).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Box3 {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Box3 {
    fn default() -> Self {
        Box3::EMPTY
    }
}

impl Box3 {
    pub const EMPTY: Box3 = Box3 {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };

    /// `Box3.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.max[0] < self.min[0] || self.max[1] < self.min[1] || self.max[2] < self.min[2]
    }

    /// `Box3.expandByPoint`.
    pub fn expand(&mut self, p: Vec3) {
        for k in 0..3 {
            self.min[k] = js_min(self.min[k], p[k]);
            self.max[k] = js_max(self.max[k], p[k]);
        }
    }

    /// `Box3.union`.
    pub fn union(&mut self, b: &Box3) {
        for k in 0..3 {
            self.min[k] = js_min(self.min[k], b.min[k]);
            self.max[k] = js_max(self.max[k], b.max[k]);
        }
    }

    /// `Box3.applyMatrix4`: the box of the eight transformed corners (an
    /// empty box stays empty).
    pub fn transformed(&self, m: &Mat4) -> Box3 {
        if self.is_empty() {
            return *self;
        }
        let (lo, hi) = (self.min, self.max);
        let mut out = Box3::EMPTY;
        for p in [
            [lo[0], lo[1], lo[2]],
            [lo[0], lo[1], hi[2]],
            [lo[0], hi[1], lo[2]],
            [lo[0], hi[1], hi[2]],
            [hi[0], lo[1], lo[2]],
            [hi[0], lo[1], hi[2]],
            [hi[0], hi[1], lo[2]],
            [hi[0], hi[1], hi[2]],
        ] {
            out.expand(math::apply(m, p));
        }
        out
    }

    fn from_pair(b: (Vec3, Vec3)) -> Box3 {
        Box3 {
            min: b.0,
            max: b.1,
        }
    }
}

/// `Ray.intersectBox(box) !== null`, as r180 writes it.
fn intersects_box(r: &Ray, b: &Box3) -> bool {
    let (o, d) = (r.origin, r.direction);
    let (invx, invy, invz) = (1.0 / d[0], 1.0 / d[1], 1.0 / d[2]);
    let (mut tmin, mut tmax);
    if invx >= 0.0 {
        tmin = (b.min[0] - o[0]) * invx;
        tmax = (b.max[0] - o[0]) * invx;
    } else {
        tmin = (b.max[0] - o[0]) * invx;
        tmax = (b.min[0] - o[0]) * invx;
    }
    let (tymin, tymax) = if invy >= 0.0 {
        ((b.min[1] - o[1]) * invy, (b.max[1] - o[1]) * invy)
    } else {
        ((b.max[1] - o[1]) * invy, (b.min[1] - o[1]) * invy)
    };
    if tmin > tymax || tymin > tmax {
        return false;
    }
    if tymin > tmin || tmin.is_nan() {
        tmin = tymin;
    }
    if tymax < tmax || tmax.is_nan() {
        tmax = tymax;
    }
    let (tzmin, tzmax) = if invz >= 0.0 {
        ((b.min[2] - o[2]) * invz, (b.max[2] - o[2]) * invz)
    } else {
        ((b.max[2] - o[2]) * invz, (b.min[2] - o[2]) * invz)
    };
    if tmin > tzmax || tzmin > tmax {
        return false;
    }
    if tzmax < tmax || tmax.is_nan() {
        tmax = tzmax;
    }
    let _ = tmin;
    tmax >= 0.0
}

/// `sphere.containsPoint(origin) || ray.intersectSphere(sphere) !== null`.
fn reaches_sphere(r: &Ray, center: Vec3, radius: f64) -> bool {
    let v = sub(center, r.origin);
    if dot(v, v) <= radius * radius {
        return true;
    }
    let tca = dot(v, r.direction);
    let d2 = dot(v, v) - tca * tca;
    let r2 = radius * radius;
    if d2 > r2 {
        return false;
    }
    let thc = (r2 - d2).sqrt();
    let t1 = tca + thc;
    t1 >= 0.0
}

// ── shapes ───────────────────────────────────────────────────────────────

/// A material as a raycast and `pick`'s filter read it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Facing {
    pub side: Side,
    pub opacity: f64,
}

impl Facing {
    /// `new MeshStandardMaterial()` (and the missing model's basic material).
    pub const DEFAULT: Facing = Facing {
        side: Side::Front,
        opacity: 1.0,
    };
}

/// A mesh's `material`: one material or an array (slots).
#[derive(Debug, Clone, PartialEq)]
pub enum Materials {
    Single(Facing),
    Slots(Vec<Facing>),
}

impl Materials {
    /// `pick`'s filter: every material at opacity ≤ 0.
    fn invisible(&self) -> bool {
        match self {
            Materials::Single(f) => f.opacity <= 0.0,
            Materials::Slots(list) => list.iter().all(|f| f.opacity <= 0.0),
        }
    }
}

/// Triangle geometry prepared for raycasts: positions, the draw range's
/// triangles and each group's, their BVHs, and the bounding volumes.
#[derive(Debug)]
pub struct TriangleMesh {
    positions: Vec<Vec3>,
    /// The draw range (one material).
    whole: (Vec<[u32; 3]>, Bvh),
    /// Every group's triangles, with their group index (array materials).
    grouped: (Vec<[u32; 3]>, Vec<u32>, Bvh),
    /// `(material_index, non-empty)` per group.
    groups: Vec<(u32, bool)>,
    /// `geometry.boundingSphere`.
    pub sphere: (Vec3, f64),
    /// `geometry.boundingBox`.
    pub bbox: Box3,
}

impl TriangleMesh {
    /// From a generator's or a mesh source's geometry.
    pub fn new(g: &Geometry) -> TriangleMesh {
        let positions: Vec<Vec3> = g
            .positions
            .chunks_exact(3)
            .map(|v| [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])])
            .collect();
        let count = if g.indexed {
            g.indices.len()
        } else {
            positions.len()
        };
        let vertex = |j: usize| -> u32 {
            if g.indexed {
                g.indices.get(j).copied().unwrap_or(u32::MAX)
            } else {
                j as u32
            }
        };
        let triangles = |start: usize, end: usize| -> Vec<[u32; 3]> {
            let mut out = Vec::new();
            let mut j = start;
            while j < end {
                out.push([vertex(j), vertex(j + 1), vertex(j + 2)]);
                j += 3;
            }
            out
        };
        // `drawRange` is (0, Infinity).
        let whole_tris = triangles(0, count);
        let mut grouped_tris = Vec::new();
        let mut owners = Vec::new();
        let mut groups = Vec::new();
        for (gi, group) in g.groups.iter().enumerate() {
            let start = group.start as usize;
            let end = count.min(group.start as usize + group.count as usize);
            let t = if start < end {
                triangles(start, end)
            } else {
                Vec::new()
            };
            groups.push((group.material_index, start < end));
            owners.extend(std::iter::repeat_n(gi as u32, t.len()));
            grouped_tris.extend(t);
        }
        let safe = |tris: &[[u32; 3]]| -> Vec<[u32; 3]> {
            // Out-of-range indices read `undefined` in three.js (NaN
            // positions that never hit): leave them out of the BVH.
            if positions.is_empty() {
                return Vec::new();
            }
            tris.iter()
                .map(|t| {
                    if t.iter().all(|&v| (v as usize) < positions.len()) {
                        *t
                    } else {
                        [0, 0, 0]
                    }
                })
                .collect()
        };
        let whole_safe = safe(&whole_tris);
        let grouped_safe = safe(&grouped_tris);
        let mut bbox = Box3::EMPTY;
        for p in &positions {
            bbox.expand(*p);
        }
        let sphere = if bbox.is_empty() {
            ([0.0; 3], 0.0)
        } else {
            let c = [
                (bbox.min[0] + bbox.max[0]) * 0.5,
                (bbox.min[1] + bbox.max[1]) * 0.5,
                (bbox.min[2] + bbox.max[2]) * 0.5,
            ];
            let mut max = 0f64;
            for p in &positions {
                let d = sub(c, *p);
                max = max.max(dot(d, d));
            }
            (c, max.sqrt())
        };
        TriangleMesh {
            whole: (whole_safe.clone(), Bvh::build(&positions, &whole_safe)),
            grouped: (
                grouped_safe.clone(),
                owners,
                Bvh::build(&positions, &grouped_safe),
            ),
            groups,
            positions,
            sphere,
            bbox,
        }
    }

    /// `Mesh.raycast` with `matrixWorld = world`: the closest hit's world
    /// distance, or three.js's `TypeError` for a group without a material.
    fn raycast(&self, world: &Mat4, materials: &Materials, ray: &Ray) -> Result<Option<f64>, ()> {
        let center = math::apply(world, self.sphere.0);
        let radius = self.sphere.1 * math::max_scale_on_axis(world);
        if !reaches_sphere(ray, center, radius) {
            return Ok(None);
        }
        let local = ray.apply(&math::invert(world));
        if !intersects_box(&local, &self.bbox) {
            return Ok(None);
        }
        let p = &self.positions;
        let found = match materials {
            Materials::Single(f) => {
                let (tris, bvh) = &self.whole;
                bvh.closest(&local, |t| triangle(&local, p, tris[t as usize], f.side))
            }
            Materials::Slots(list) => {
                if self
                    .groups
                    .iter()
                    .any(|(m, nonempty)| *nonempty && list.get(*m as usize).is_none())
                {
                    return Err(());
                }
                let (tris, owners, bvh) = &self.grouped;
                bvh.closest(&local, |t| {
                    let group = self.groups[owners[t as usize] as usize].0;
                    let side = list[group as usize].side;
                    triangle(&local, p, tris[t as usize], side)
                })
            }
        };
        Ok(found.map(|(t, _)| {
            let point = math::apply(world, local.at(t));
            length(sub(point, ray.origin))
        }))
    }
}

/// `checkIntersection`'s triangle test with the material side.
fn triangle(r: &Ray, p: &[Vec3], t: [u32; 3], side: Side) -> Option<f64> {
    let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
    if side == Side::Back {
        intersect_triangle(r, c, b, a, true)
    } else {
        intersect_triangle(r, a, b, c, side == Side::Front)
    }
}

/// `sync`'s material rule over an imported model's meshes.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelMaterials {
    /// `useSourceMaterials` without slots: the loader's materials.
    Source,
    /// The object's material on every mesh (points and lines keep theirs).
    Single(Facing),
    /// Slots: an array material on meshes without groups (no hits).
    Slots(Vec<Facing>),
}

/// A placed object's geometry.
pub enum Shape {
    Mesh {
        mesh: Arc<TriangleMesh>,
        materials: Materials,
    },
    Model {
        picker: Arc<Picker>,
        posed: Arc<Posed>,
        materials: ModelMaterials,
    },
    /// Lights: `Object3D.raycast` does nothing; bounds skip lights.
    Light,
}

/// One entry of `runtime.objects`.
pub struct Object {
    pub id: String,
    /// `object.visible` and every ancestor's.
    pub visible: bool,
    /// `matrixWorld` (for a model, the layer matrix the asset root sits under).
    pub world: Mat4,
    pub shape: Shape,
}

/// Why a question has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    /// three.js throws `TypeError` (`MISSING_GROUP_MATERIAL`).
    MissingGroupMaterial,
}

/// A runtime's objects.
#[derive(Default)]
pub struct Scene {
    pub objects: Vec<Object>,
}

impl Scene {
    fn model_facing(model: &crate::gltf::Model, i: usize) -> Facing {
        let ObjectKind::Drawable(d) = &model.objects[i].kind else {
            return Facing::DEFAULT;
        };
        let m = &model.materials[d.material];
        Facing {
            side: if m.double_sided {
                Side::Double
            } else {
                Side::Front
            },
            opacity: m.opacity,
        }
    }

    /// The closest hit of one object that `pick`'s per-hit filter accepts
    /// (the object's visible chain is checked by the caller).
    fn object_hit(o: &Object, ray: &Ray) -> Result<Option<f64>, QueryError> {
        match &o.shape {
            Shape::Light => Ok(None),
            Shape::Mesh { mesh, materials } => {
                let d = mesh
                    .raycast(&o.world, materials, ray)
                    .map_err(|()| QueryError::MissingGroupMaterial)?;
                Ok(d.filter(|_| !materials.invisible()))
            }
            Shape::Model {
                picker,
                posed,
                materials,
            } => {
                let model = &picker.model;
                let is_mesh = |i: usize| match &model.objects[i].kind {
                    ObjectKind::Drawable(d) => model.geometries[d.geometry].mode == Mode::Triangles,
                    _ => false,
                };
                let facing = |i: usize| -> Option<Facing> {
                    if !is_mesh(i) {
                        return Some(Scene::model_facing(model, i));
                    }
                    match materials {
                        ModelMaterials::Source => Some(Scene::model_facing(model, i)),
                        ModelMaterials::Single(f) => Some(*f),
                        ModelMaterials::Slots(_) => None,
                    }
                };
                let side = |i: usize| facing(i).map_or(Side::Front, |f| f.side);
                let skip = |i: usize| match facing(i) {
                    None => true,
                    Some(f) => f.opacity <= 0.0,
                };
                Ok(picker
                    .pick(posed, ray, 0.0, f64::INFINITY, &side, &skip)
                    .map(|h| h.distance))
            }
        }
    }

    /// `runtime.pick(x, y, ids, camera)`: the id of the closest object hit
    /// along the camera ray through `(x, y)` in normalized device
    /// coordinates, among `ids` when given.
    pub fn pick(
        &self,
        camera: &Camera,
        x: f64,
        y: f64,
        ids: Option<&[&str]>,
    ) -> Result<Option<&str>, QueryError> {
        let ray = camera.ray(x, y);
        let mut best: Option<(f64, &str)> = None;
        for o in &self.objects {
            let wanted = o.visible && ids.is_none_or(|ids| ids.contains(&o.id.as_str()));
            // Every object is raycast (three.js raycasts before filtering),
            // so a missing group material throws whatever the filter says.
            let can_throw = matches!(&o.shape, Shape::Mesh { materials: Materials::Slots(_), .. });
            if !wanted && !can_throw {
                continue;
            }
            let hit = Scene::object_hit(o, &ray)?;
            if !wanted {
                continue;
            }
            if let Some(d) = hit
                && best.is_none_or(|(b, _)| d < b)
            {
                best = Some((d, o.id.as_str()));
            }
        }
        Ok(best.map(|(_, id)| id))
    }

    /// `runtime.worldBounds(ids)`: the union of the visible meshes' geometry
    /// boxes in world space (lights, points, and lines excluded).
    pub fn world_bounds(&self, ids: Option<&[&str]>) -> Box3 {
        let mut b = Box3::EMPTY;
        for o in &self.objects {
            if !o.visible || ids.is_some_and(|ids| !ids.contains(&o.id.as_str())) {
                continue;
            }
            match &o.shape {
                Shape::Light => {}
                Shape::Mesh { mesh, .. } => b.union(&mesh.bbox.transformed(&o.world)),
                Shape::Model { picker, posed, .. } => {
                    let model = &picker.model;
                    for i in model.traversal() {
                        if let ObjectKind::Drawable(d) = &model.objects[i].kind
                            && model.geometries[d.geometry].mode == Mode::Triangles
                            && let Some(g) = model.geometries[d.geometry].bounds()
                        {
                            b.union(&Box3::from_pair(g).transformed(&posed.world[i]));
                        }
                    }
                }
            }
        }
        b
    }

    /// `runtime.bounds(w, h, ids, camera)`: the projected corners of the
    /// world box in front of the camera, clipped to the canvas.
    pub fn bounds(
        &self,
        w: f64,
        h: f64,
        ids: Option<&[&str]>,
        camera: &Camera,
    ) -> Option<Rect> {
        let camera = camera.resized(w, h);
        let b = self.world_bounds(ids);
        if b.is_empty() {
            return None;
        }
        let mut xs = Vec::with_capacity(8);
        let mut ys = Vec::with_capacity(8);
        for x in [b.min[0], b.max[0]] {
            for y in [b.min[1], b.max[1]] {
                for z in [b.min[2], b.max[2]] {
                    let v = camera.project([x, y, z]);
                    if v[2] < 1.0 {
                        xs.push((v[0] + 1.0) * w / 2.0);
                        ys.push((1.0 - v[1]) * h / 2.0);
                    }
                }
            }
        }
        if xs.is_empty() {
            return None;
        }
        let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, js_min);
        let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, js_max);
        let x0 = js_max(0.0, min(&xs));
        let x1 = js_min(w, max(&xs));
        let y0 = js_max(0.0, min(&ys));
        let y1 = js_min(h, max(&ys));
        (x1 > x0 && y1 > y0).then_some(Rect { x0, y0, x1, y1 })
    }

    /// `new Box3().setFromObject(object)` for the object `id` (no
    /// visibility test; points and lines included; a skinned mesh's posed
    /// vertices and an instanced mesh's instances).
    pub fn object_box(&self, id: &str) -> Option<Box3> {
        let o = self.objects.iter().find(|o| o.id == id)?;
        let mut b = Box3::EMPTY;
        match &o.shape {
            Shape::Light => {}
            Shape::Mesh { mesh, .. } => b.union(&mesh.bbox.transformed(&o.world)),
            Shape::Model { picker, posed, .. } => {
                let model = &picker.model;
                for i in model.traversal() {
                    if let ObjectKind::Drawable(d) = &model.objects[i].kind
                        && let Some(lb) =
                            crate::gltf::pose::local_box(model, i, d, &posed.pose, &posed.world)
                    {
                        b.union(&Box3::from_pair(lb).transformed(&posed.world[i]));
                    }
                }
            }
        }
        Some(b)
    }

    /// The camera-space depth of each object's origin (`compositionOrderedLayers`).
    pub fn origin(&self, id: &str) -> Option<Vec3> {
        let o = self.objects.iter().find(|o| o.id == id)?;
        Some([o.world[12], o.world[13], o.world[14]])
    }
}

/// A canvas rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// The missing-model box (`new BoxGeometry(1, 1, 1)`).
pub fn missing_box() -> Arc<TriangleMesh> {
    static BOX: std::sync::OnceLock<Arc<TriangleMesh>> = std::sync::OnceLock::new();
    BOX.get_or_init(|| {
        Arc::new(TriangleMesh::new(
            &crate::Primitive::Box {
                width: 1.0,
                height: 1.0,
                depth: 1.0,
            }
            .geometry(),
        ))
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Parameters, Source};

    fn primitive(name: &str) -> Arc<TriangleMesh> {
        Arc::new(TriangleMesh::new(
            &Source::primitive(name, &Parameters::default())
                .unwrap()
                .geometry(),
        ))
    }

    fn camera() -> Camera {
        let world = camera_world([0.0, 0.0, 5.0], [0.0; 3], [0.0, 1.0, 0.0]);
        Camera::new(
            Lens::Perspective {
                fov: 45.0,
                zoom: 1.0,
                near: 0.01,
                far: 1000.0,
            },
            world,
            16.0 / 9.0,
        )
    }

    fn object(id: &str, mesh: Arc<TriangleMesh>, x: f64, z: f64, materials: Materials) -> Object {
        let mut world = math::IDENTITY;
        world[12] = x;
        world[14] = z;
        Object {
            id: id.into(),
            visible: true,
            world,
            shape: Shape::Mesh { mesh, materials },
        }
    }

    #[test]
    fn the_nearest_object_in_the_set_wins() {
        let scene = Scene {
            objects: vec![
                object("back", primitive("box"), 0.0, -2.0, Materials::Single(Facing::DEFAULT)),
                object("front", primitive("sphere"), 0.0, 0.0, Materials::Single(Facing::DEFAULT)),
            ],
        };
        let c = camera();
        assert_eq!(scene.pick(&c, 0.0, 0.0, None), Ok(Some("front")));
        assert_eq!(scene.pick(&c, 0.0, 0.0, Some(&["back"])), Ok(Some("back")));
        assert_eq!(scene.pick(&c, 0.9, 0.9, None), Ok(None));
    }

    #[test]
    fn invisible_and_transparent_objects_are_skipped() {
        let mut hidden = object("a", primitive("box"), 0.0, 0.0, Materials::Single(Facing::DEFAULT));
        hidden.visible = false;
        let clear = object(
            "b",
            primitive("box"),
            0.0,
            -2.0,
            Materials::Single(Facing {
                side: Side::Front,
                opacity: 0.0,
            }),
        );
        let behind = object("c", primitive("box"), 0.0, -4.0, Materials::Single(Facing::DEFAULT));
        let scene = Scene {
            objects: vec![hidden, clear, behind],
        };
        assert_eq!(scene.pick(&camera(), 0.0, 0.0, None), Ok(Some("c")));
    }

    #[test]
    fn slots_follow_three_groups() {
        // A box has 6 groups: one slot leaves 5 without a material (three.js
        // throws); a sphere has none (an array material finds nothing).
        let one = Materials::Slots(vec![Facing::DEFAULT]);
        let scene = Scene {
            objects: vec![object("box", primitive("box"), 0.0, 0.0, one.clone())],
        };
        assert_eq!(
            scene.pick(&camera(), 0.0, 0.0, None),
            Err(QueryError::MissingGroupMaterial)
        );
        let scene = Scene {
            objects: vec![object("s", primitive("sphere"), 0.0, 0.0, one)],
        };
        assert_eq!(scene.pick(&camera(), 0.0, 0.0, None), Ok(None));
        let six = Materials::Slots(vec![Facing::DEFAULT; 6]);
        let scene = Scene {
            objects: vec![object("box", primitive("box"), 0.0, 0.0, six)],
        };
        assert_eq!(scene.pick(&camera(), 0.0, 0.0, None), Ok(Some("box")));
    }

    #[test]
    fn back_faces_need_a_double_or_back_side() {
        // A plane faces +Z; seen from behind it is hit only when not front-sided.
        let plane = primitive("plane");
        let behind = camera_world([0.0, 0.0, -5.0], [0.0; 3], [0.0, 1.0, 0.0]);
        let lens = Lens::Perspective {
            fov: 45.0,
            zoom: 1.0,
            near: 0.01,
            far: 1000.0,
        };
        let c = Camera::new(lens, behind, 1.0);
        for (side, hit) in [(Side::Front, false), (Side::Double, true), (Side::Back, true)] {
            let scene = Scene {
                objects: vec![object(
                    "p",
                    plane.clone(),
                    0.0,
                    0.0,
                    Materials::Single(Facing { side, opacity: 1.0 }),
                )],
            };
            assert_eq!(scene.pick(&c, 0.0, 0.0, None).unwrap().is_some(), hit, "{side:?}");
        }
    }

    #[test]
    fn bounds_project_the_world_box() {
        let scene = Scene {
            objects: vec![object("box", primitive("box"), 0.0, 0.0, Materials::Single(Facing::DEFAULT))],
        };
        let r = scene.bounds(1920.0, 1080.0, Some(&["box"]), &camera()).unwrap();
        // Symmetric about the center.
        assert!((r.x0 + r.x1 - 1920.0).abs() < 1e-9);
        assert!((r.y0 + r.y1 - 1080.0).abs() < 1e-9);
        assert!(r.x1 > r.x0 && r.y1 > r.y0);
        assert_eq!(scene.bounds(1920.0, 1080.0, Some(&["nothing"]), &camera()), None);
        let b = scene.object_box("box").unwrap();
        assert_eq!(b.min, [-0.5; 3]);
        assert_eq!(b.max, [0.5; 3]);
    }

    #[test]
    fn orthographic_rays_are_parallel() {
        let world = camera_world([0.0, 0.0, 5.0], [0.0; 3], [0.0, 1.0, 0.0]);
        let c = Camera::new(
            Lens::Orthographic {
                zoom: 1.0,
                near: 0.01,
                far: 1000.0,
            },
            world,
            1.0,
        );
        let a = c.ray(-0.5, 0.2);
        let b = c.ray(0.5, -0.2);
        assert_eq!(a.direction, b.direction);
        assert!((a.direction[2] + 1.0).abs() < 1e-15);
        assert!((a.origin[0] + 1.5).abs() < 1e-9, "{:?}", a.origin);
    }
}
