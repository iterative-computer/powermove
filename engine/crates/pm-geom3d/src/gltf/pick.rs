//! Raycasts against a posed model with three.js r180's semantics
//! (`Mesh.raycast` with its bounding-sphere and box pre-tests,
//! `SkinnedMesh` and morphed vertices through `getVertexPosition`,
//! `InstancedMesh` per instance, `Points` and `Line` with threshold 1
//! divided by the object's mean local scale, `Ray.intersectTriangle` with
//! the material side, closest hit first), as the S7 spike validated
//! (docs/engine/spikes/S7.md). Skinned bounding volumes are those of the
//! current pose (production keeps the first pose's sphere; S7 defect 5).
//!
//! Triangle drawables keep a BVH built once over the rest pose; posed
//! (skinned or morphed) drawables re-pose their vertices once per time and
//! refit it.

use std::sync::Arc;

use super::math::{self, Mat4, Vec3};
use super::pose::{self, Pose, Poser};
use super::{Model, ObjectKind};

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}
fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l == 0.0 { a } else { scale(a, 1.0 / l) }
}
fn distance_sq(a: Vec3, b: Vec3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

/// A ray (`direction` normalized).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Ray {
    /// `Ray.applyMatrix4`: the origin as a point, the direction through
    /// `transformDirection` (normalized).
    pub fn apply(&self, m: &Mat4) -> Ray {
        let (x, y, z) = (self.direction[0], self.direction[1], self.direction[2]);
        let d = [
            m[0] * x + m[4] * y + m[8] * z,
            m[1] * x + m[5] * y + m[9] * z,
            m[2] * x + m[6] * y + m[10] * z,
        ];
        Ray {
            origin: math::apply(m, self.origin),
            direction: normalize(d),
        }
    }

    pub fn at(&self, t: f64) -> Vec3 {
        add(self.origin, scale(self.direction, t))
    }

    /// `Ray.intersectsSphere`.
    fn hits_sphere(&self, center: Vec3, radius: f64) -> bool {
        let along = dot(sub(center, self.origin), self.direction);
        let d2 = if along < 0.0 {
            distance_sq(self.origin, center)
        } else {
            distance_sq(self.at(along), center)
        };
        d2 <= radius * radius
    }

    /// `Ray.intersectsBox`.
    fn hits_box(&self, lo: Vec3, hi: Vec3) -> bool {
        let mut tmin = f64::NEG_INFINITY;
        let mut tmax = f64::INFINITY;
        for k in 0..3 {
            let inv = 1.0 / self.direction[k];
            let (a, b) = if inv >= 0.0 {
                (
                    (lo[k] - self.origin[k]) * inv,
                    (hi[k] - self.origin[k]) * inv,
                )
            } else {
                (
                    (hi[k] - self.origin[k]) * inv,
                    (lo[k] - self.origin[k]) * inv,
                )
            };
            if a > tmax || tmin > b {
                return false;
            }
            if a > tmin || tmin.is_nan() {
                tmin = a;
            }
            if b < tmax || tmax.is_nan() {
                tmax = b;
            }
        }
        tmax >= 0.0
    }
}

/// three.js r180 `Ray.intersectTriangle` (the ray parameter).
pub fn intersect_triangle(r: &Ray, a: Vec3, b: Vec3, c: Vec3, cull: bool) -> Option<f64> {
    let edge1 = sub(b, a);
    let edge2 = sub(c, a);
    let normal = cross(edge1, edge2);
    let mut ddn = dot(r.direction, normal);
    let sign;
    if ddn > 0.0 {
        if cull {
            return None;
        }
        sign = 1.0;
    } else if ddn < 0.0 {
        sign = -1.0;
        ddn = -ddn;
    } else {
        return None;
    }
    let diff = sub(r.origin, a);
    let ddqxe2 = sign * dot(r.direction, cross(diff, edge2));
    if ddqxe2 < 0.0 {
        return None;
    }
    let dde1xq = sign * dot(r.direction, cross(edge1, diff));
    if dde1xq < 0.0 || ddqxe2 + dde1xq > ddn {
        return None;
    }
    let qdn = -sign * dot(diff, normal);
    if qdn < 0.0 {
        return None;
    }
    Some(qdn / ddn)
}

/// A triangle BVH (binned SAH, leaves of 4), refit for new positions.
#[derive(Debug, Clone, Default)]
pub(crate) struct Bvh {
    /// Node bounds and links: leaf `(first, count > 0)`, interior
    /// `(left child, 0)` with the right child after it.
    nodes: Vec<([f64; 3], [f64; 3], u32, u32)>,
    order: Vec<u32>,
}

const LEAF: usize = 4;
const BINS: usize = 12;

fn tri_box_from_positions(position: &impl Fn(usize) -> Vec3, t: &[u32; 3]) -> (Vec3, Vec3) {
    let (a, b, c) = (
        position(t[0] as usize),
        position(t[1] as usize),
        position(t[2] as usize),
    );
    (
        [
            a[0].min(b[0]).min(c[0]),
            a[1].min(b[1]).min(c[1]),
            a[2].min(b[2]).min(c[2]),
        ],
        [
            a[0].max(b[0]).max(c[0]),
            a[1].max(b[1]).max(c[1]),
            a[2].max(b[2]).max(c[2]),
        ],
    )
}

fn tri_box(p: &[Vec3], t: &[u32; 3]) -> (Vec3, Vec3) {
    tri_box_from_positions(&|i| p[i], t)
}

fn area(lo: Vec3, hi: Vec3) -> f64 {
    let e = sub(hi, lo);
    if e[0] < 0.0 {
        return 0.0;
    }
    e[0] * e[1] + e[1] * e[2] + e[2] * e[0]
}

impl Bvh {
    pub(crate) fn build(p: &[Vec3], tris: &[[u32; 3]]) -> Bvh {
        Self::build_from_positions(tris, |i| p[i])
    }

    /// Build from rest-pose positions without materializing a second f64
    /// array. The getter yields exactly the f64 values `build` would read.
    fn build_from_positions(tris: &[[u32; 3]], position: impl Fn(usize) -> Vec3) -> Bvh {
        let n = tris.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        let empty = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], 0, 0);
        if n == 0 {
            return Bvh {
                nodes: vec![empty],
                order,
            };
        }
        let boxes: Vec<(Vec3, Vec3)> = tris
            .iter()
            .map(|t| tri_box_from_positions(&position, t))
            .collect();
        let centers: Vec<Vec3> = boxes.iter().map(|(a, b)| scale(add(*a, *b), 0.5)).collect();
        let mut nodes = vec![([0.0; 3], [0.0; 3], 0u32, n as u32)];
        let mut stack = vec![0usize];
        while let Some(ni) = stack.pop() {
            let (first, count) = (nodes[ni].2 as usize, nodes[ni].3 as usize);
            let slice = &mut order[first..first + count];
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            let (mut clo, mut chi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for &t in slice.iter() {
                let (a, b) = boxes[t as usize];
                let c = centers[t as usize];
                for k in 0..3 {
                    lo[k] = lo[k].min(a[k]);
                    hi[k] = hi[k].max(b[k]);
                    clo[k] = clo[k].min(c[k]);
                    chi[k] = chi[k].max(c[k]);
                }
            }
            nodes[ni].0 = lo;
            nodes[ni].1 = hi;
            if count <= LEAF {
                continue;
            }
            let mut best = (f64::MAX, 0usize, 0usize);
            for axis in 0..3 {
                let extent = chi[axis] - clo[axis];
                if extent <= 0.0 {
                    continue;
                }
                let mut bl = [[f64::INFINITY; 3]; BINS];
                let mut bh = [[f64::NEG_INFINITY; 3]; BINS];
                let mut bn = [0usize; BINS];
                let s = BINS as f64 / extent;
                for &t in slice.iter() {
                    let b = (((centers[t as usize][axis] - clo[axis]) * s) as usize).min(BINS - 1);
                    let (a, c) = boxes[t as usize];
                    bn[b] += 1;
                    for k in 0..3 {
                        bl[b][k] = bl[b][k].min(a[k]);
                        bh[b][k] = bh[b][k].max(c[k]);
                    }
                }
                let mut left_area = [0f64; BINS];
                let mut left_n = [0usize; BINS];
                let (mut mn, mut mx, mut cnt) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], 0);
                for b in 0..BINS - 1 {
                    cnt += bn[b];
                    for k in 0..3 {
                        mn[k] = mn[k].min(bl[b][k]);
                        mx[k] = mx[k].max(bh[b][k]);
                    }
                    left_area[b] = area(mn, mx);
                    left_n[b] = cnt;
                }
                let (mut mn, mut mx, mut cnt) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], 0);
                for b in (1..BINS).rev() {
                    cnt += bn[b];
                    for k in 0..3 {
                        mn[k] = mn[k].min(bl[b][k]);
                        mx[k] = mx[k].max(bh[b][k]);
                    }
                    let cost = left_area[b - 1] * left_n[b - 1] as f64 + area(mn, mx) * cnt as f64;
                    if left_n[b - 1] > 0 && cnt > 0 && cost < best.0 {
                        best = (cost, axis, b);
                    }
                }
            }
            let mid = if best.0 < f64::MAX {
                let (axis, split) = (best.1, best.2);
                let s = BINS as f64 / (chi[axis] - clo[axis]);
                let (mut i, mut j) = (0, slice.len());
                while i < j {
                    let b = (((centers[slice[i] as usize][axis] - clo[axis]) * s) as usize)
                        .min(BINS - 1);
                    if b < split {
                        i += 1;
                    } else {
                        j -= 1;
                        slice.swap(i, j);
                    }
                }
                i
            } else {
                count / 2
            };
            let mid = if mid == 0 || mid == count {
                count / 2
            } else {
                mid
            };
            let left = nodes.len();
            nodes.push(([0.0; 3], [0.0; 3], first as u32, mid as u32));
            nodes.push((
                [0.0; 3],
                [0.0; 3],
                (first + mid) as u32,
                (count - mid) as u32,
            ));
            nodes[ni].2 = left as u32;
            nodes[ni].3 = 0;
            stack.push(left);
            stack.push(left + 1);
        }
        Bvh { nodes, order }
    }

    fn refit(&mut self, p: &[Vec3], tris: &[[u32; 3]]) {
        for i in (0..self.nodes.len()).rev() {
            let (_, _, first, count) = self.nodes[i];
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            if count > 0 {
                for &t in &self.order[first as usize..(first + count) as usize] {
                    let (a, b) = tri_box(p, &tris[t as usize]);
                    for k in 0..3 {
                        lo[k] = lo[k].min(a[k]);
                        hi[k] = hi[k].max(b[k]);
                    }
                }
            } else if !self.order.is_empty() {
                let (l, r) = (self.nodes[first as usize], self.nodes[first as usize + 1]);
                for k in 0..3 {
                    lo[k] = l.0[k].min(r.0[k]);
                    hi[k] = l.1[k].max(r.1[k]);
                }
            }
            self.nodes[i].0 = lo;
            self.nodes[i].1 = hi;
        }
    }

    fn slab(&self, i: usize, r: &Ray, inv: Vec3) -> Option<f64> {
        let (lo, hi, _, _) = self.nodes[i];
        if hi[0] < lo[0] {
            return None;
        }
        let mut tmin = 0.0f64;
        let mut tmax = f64::INFINITY;
        for k in 0..3 {
            let t1 = (lo[k] - r.origin[k]) * inv[k];
            let t2 = (hi[k] - r.origin[k]) * inv[k];
            let (a, b) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
            if a > tmin {
                tmin = a;
            }
            if b < tmax {
                tmax = b;
            }
        }
        (tmin <= tmax).then_some(tmin)
    }

    /// Every triangle the ray may hit, nearest boxes first, with `test`
    /// returning a ray parameter; the closest accepted hit.
    pub(crate) fn closest(
        &self,
        r: &Ray,
        mut test: impl FnMut(u32) -> Option<f64>,
    ) -> Option<(f64, u32)> {
        let inv = [
            1.0 / r.direction[0],
            1.0 / r.direction[1],
            1.0 / r.direction[2],
        ];
        let mut best = f64::INFINITY;
        let mut hit = None;
        self.slab(0, r, inv)?;
        let mut stack = vec![0usize];
        while let Some(ni) = stack.pop() {
            let (_, _, first, count) = self.nodes[ni];
            if count > 0 {
                for &t in &self.order[first as usize..(first + count) as usize] {
                    if let Some(d) = test(t)
                        && d < best
                    {
                        best = d;
                        hit = Some((d, t));
                    }
                }
                continue;
            }
            let (l, rr) = (first as usize, first as usize + 1);
            let tl = self.slab(l, r, inv).filter(|t| *t <= best);
            let tr = self.slab(rr, r, inv).filter(|t| *t <= best);
            match (tl, tr) {
                (Some(a), Some(b)) => {
                    let (near, far) = if a <= b { (l, rr) } else { (rr, l) };
                    stack.push(far);
                    stack.push(near);
                }
                (Some(_), None) => stack.push(l),
                (None, Some(_)) => stack.push(rr),
                (None, None) => {}
            }
        }
        hit
    }
}

/// How a drawable's material faces rays (`material.side`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Front,
    Back,
    Double,
}

/// One hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The drawable object of `model.objects`.
    pub object: usize,
    pub distance: f64,
    pub point: Vec3,
    /// Triangle, point, or segment start index.
    pub index: u32,
    pub instance: Option<u32>,
}

/// A posed drawable: positions, refit BVH, and (skinned) its bounding
/// sphere and box for the current pose.
struct Deformed {
    positions: Vec<Vec3>,
    bvh: Bvh,
    bounds: (Vec3, f64, Option<(Vec3, Vec3)>),
}

/// Rest-pose triangles and BVHs of a model's geometries, shared by every
/// layer using it.
pub struct Picker {
    pub model: Arc<Model>,
    tris: Vec<Option<(Vec<[u32; 3]>, Bvh)>>,
    /// `geometry.boundingSphere` per geometry.
    spheres: Vec<(Vec3, f64)>,
    /// Per object: a skinned mesh's `boundingBox` as the importer leaves it
    /// (`loadGltf`'s `Box3.setFromObject(scene)` computes it at the loaded
    /// pose; every runtime clone copies it and never recomputes it).
    loaded_boxes: Vec<Option<(Vec3, Vec3)>>,
}

/// A model posed under a layer at a time (re-posed vertices for skinned
/// and morphed drawables).
pub struct Posed {
    pub world: Vec<Mat4>,
    pub pose: Pose,
    reachable: Vec<bool>,
    /// Per object: posed positions and their refit BVH.
    deformed: Vec<Option<Deformed>>,
}

impl Picker {
    pub fn new(model: Arc<Model>) -> Picker {
        let tris = model
            .geometries
            .iter()
            .map(|g| {
                if g.mode != super::Mode::Triangles {
                    return None;
                }
                let n = g.vertex_count() as u32;
                // A triangle with an index past the last vertex reads
                // `undefined` positions in `Mesh.raycast`: never a hit.
                let t: Vec<[u32; 3]> = match &g.index {
                    Some(ix) => ix
                        .chunks_exact(3)
                        .map(|c| [c[0], c[1], c[2]])
                        .filter(|c| c.iter().all(|&v| v < n))
                        .collect(),
                    None => (0..n / 3).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect(),
                };
                let position = &g.position.data;
                let bvh = Bvh::build_from_positions(&t, |i| {
                    let at = i * 3;
                    [
                        f64::from(position[at]),
                        f64::from(position[at + 1]),
                        f64::from(position[at + 2]),
                    ]
                });
                Some((t, bvh))
            })
            .collect();
        let spheres = model
            .geometries
            .iter()
            .map(|g| g.bounding_sphere())
            .collect();
        // The loaded pose: `scene.updateMatrixWorld()` from the glTF scene.
        let rest = Pose::rest(&model);
        let rest_world = pose::world(&model, &rest, None);
        let loaded_boxes = model
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| match &o.kind {
                ObjectKind::Drawable(d) if d.skin.is_some() => {
                    pose::local_box(&model, i, d, &rest, &rest_world)
                }
                _ => None,
            })
            .collect();
        Picker {
            model,
            tris,
            spheres,
            loaded_boxes,
        }
    }

    /// A skinned drawable's `boundingBox` as every runtime clone of the
    /// imported model holds it (the loaded pose), in its local space.
    pub fn loaded_box(&self, object: usize) -> Option<(Vec3, Vec3)> {
        self.loaded_boxes.get(object).copied().flatten()
    }

    /// Poses the model under the layer matrix `layer` at mixer time `time`.
    pub fn pose(&self, layer: &Mat4, time: f64) -> Posed {
        let model = &self.model;
        let (parent, pose) = pose::placed(model, Pose::at(model, time), layer);
        let world = pose::world(model, &pose, Some(&parent));
        let reachable = pose::reachable(model);
        let mut deformed: Vec<Option<Deformed>> = (0..model.objects.len()).map(|_| None).collect();
        for (i, o) in model.objects.iter().enumerate() {
            let ObjectKind::Drawable(d) = &o.kind else {
                continue;
            };
            if !reachable[i] {
                continue;
            }
            let morphing = pose.influences[i].iter().any(|w| *w != 0.0)
                && model.geometries[d.geometry].morph.position.is_some();
            if d.skin.is_none() && !morphing {
                continue;
            }
            let Some((tris, bvh)) = &self.tris[d.geometry] else {
                continue;
            };
            let poser = Poser::new(model, i, d, &pose.influences[i], &world);
            let n = model.geometries[d.geometry].vertex_count();
            let p: Vec<Vec3> = (0..n).map(|v| poser.vertex(v)).collect();
            let mut b = bvh.clone();
            b.refit(&p, tris);
            let bounds = if d.skin.is_some() {
                let mut lo = [f64::INFINITY; 3];
                let mut hi = [f64::NEG_INFINITY; 3];
                for v in &p {
                    for ((l, h), x) in lo.iter_mut().zip(hi.iter_mut()).zip(v) {
                        *l = l.min(*x);
                        *h = h.max(*x);
                    }
                }
                let c = scale(add(lo, hi), 0.5);
                let r = p
                    .iter()
                    .map(|v| distance_sq(*v, c))
                    .fold(0.0, f64::max)
                    .sqrt();
                (c, r, Some((lo, hi)))
            } else {
                let (c, r) = self.spheres[d.geometry];
                (c, r, model.geometries[d.geometry].bounding_box)
            };
            deformed[i] = Some(Deformed {
                positions: p,
                bvh: b,
                bounds,
            });
        }
        Posed {
            world,
            pose,
            reachable,
            deformed,
        }
    }

    /// The closest hit along the world ray within `[near, far]`; `side`
    /// gives each drawable's material side, `skip` drawables to ignore
    /// (materials all at opacity 0, hidden). Skinned bounding volumes are
    /// those of the current pose.
    pub fn pick(
        &self,
        posed: &Posed,
        ray: &Ray,
        near: f64,
        far: f64,
        side: &dyn Fn(usize) -> Side,
        skip: &dyn Fn(usize) -> bool,
    ) -> Option<Hit> {
        self.pick_with(posed, ray, near, far, side, skip, false)
    }

    /// [`Picker::pick`] as today's runtime raycasts a clone of the imported
    /// model: a skinned mesh's box pre-test uses the box the importer
    /// computed at the loaded pose ([`Picker::loaded_box`]).
    pub fn pick_as_runtime(
        &self,
        posed: &Posed,
        ray: &Ray,
        near: f64,
        far: f64,
        side: &dyn Fn(usize) -> Side,
        skip: &dyn Fn(usize) -> bool,
    ) -> Option<Hit> {
        self.pick_with(posed, ray, near, far, side, skip, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn pick_with(
        &self,
        posed: &Posed,
        ray: &Ray,
        near: f64,
        far: f64,
        side: &dyn Fn(usize) -> Side,
        skip: &dyn Fn(usize) -> bool,
        loaded_boxes: bool,
    ) -> Option<Hit> {
        let model = &self.model;
        let mut best: Option<Hit> = None;
        let mut consider = |h: Hit| {
            if h.distance < near || h.distance > far {
                return;
            }
            if best.as_ref().is_none_or(|b| h.distance < b.distance) {
                best = Some(h);
            }
        };
        for (i, o) in model.objects.iter().enumerate() {
            let ObjectKind::Drawable(d) = &o.kind else {
                continue;
            };
            if !posed.reachable[i] || skip(i) {
                continue;
            }
            let g = &model.geometries[d.geometry];
            let world = &posed.world[i];
            match g.mode {
                super::Mode::Triangles => {
                    let s = side(i);
                    let cull = s == Side::Front;
                    let Some((tris, rest_bvh)) = &self.tris[d.geometry] else {
                        continue;
                    };
                    let (positions, bvh, (center, radius, mut bbox)): (Option<&[Vec3]>, &Bvh, _) =
                        match &posed.deformed[i] {
                            Some(x) => (Some(&x.positions), &x.bvh, x.bounds),
                            None => {
                                let (c, r) = self.spheres[d.geometry];
                                (None, rest_bvh, (c, r, g.bounding_box))
                            }
                        };
                    if loaded_boxes && d.skin.is_some() {
                        bbox = self.loaded_box(i);
                    }
                    let vertex = |k: u32| -> Vec3 {
                        match positions {
                            Some(p) => p[k as usize],
                            None => {
                                let v = &g.position.data[k as usize * 3..k as usize * 3 + 3];
                                [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]
                            }
                        }
                    };
                    let instances: Vec<(Option<u32>, Mat4)> = match &d.instances {
                        Some(inst) => inst
                            .matrices
                            .iter()
                            .enumerate()
                            .map(|(k, m)| {
                                let m: Mat4 = std::array::from_fn(|e| f64::from(m[e]));
                                (Some(k as u32), math::multiply(world, &m))
                            })
                            .collect(),
                        None => vec![(None, *world)],
                    };
                    for (instance, m) in instances {
                        let wc = math::apply(&m, center);
                        let wr = radius * math::max_scale_on_axis(&m);
                        if !ray.hits_sphere(wc, wr) {
                            continue;
                        }
                        let local = ray.apply(&math::invert(&m));
                        if let Some((lo, hi)) = bbox
                            && !local.hits_box(lo, hi)
                        {
                            continue;
                        }
                        let found = bvh.closest(&local, |t| {
                            let tri = tris[t as usize];
                            let (a, b, c) = (vertex(tri[0]), vertex(tri[1]), vertex(tri[2]));
                            if s == Side::Back {
                                intersect_triangle(&local, c, b, a, true)
                            } else {
                                intersect_triangle(&local, a, b, c, cull)
                            }
                        });
                        if let Some((t, index)) = found {
                            let point = math::apply(&m, local.at(t));
                            consider(Hit {
                                object: i,
                                distance: length(sub(point, ray.origin)),
                                point,
                                index,
                                instance,
                            });
                        }
                    }
                }
                mode => {
                    let s = model.objects[i].scale;
                    let threshold = 1.0 / ((s[0] + s[1] + s[2]) / 3.0);
                    let (center, radius) = self.spheres[d.geometry];
                    let wc = math::apply(world, center);
                    let wr = radius * math::max_scale_on_axis(world) + 1.0;
                    if !ray.hits_sphere(wc, wr) {
                        continue;
                    }
                    let local = ray.apply(&math::invert(world));
                    let positions = StaticPositions(&g.position.data);
                    let found = if mode == super::Mode::Points {
                        points_hit(
                            positions,
                            g.index.as_ref().map(|index| index.as_slice()),
                            &local,
                            threshold * threshold,
                            world,
                            ray,
                        )
                    } else {
                        line_hit(
                            positions,
                            g.index.as_ref().map(|index| index.as_slice()),
                            mode,
                            &local,
                            threshold * threshold,
                            world,
                            ray,
                        )
                    };
                    if let Some((p, index)) = found {
                        let point = math::apply(world, p);
                        consider(Hit {
                            object: i,
                            distance: length(sub(point, ray.origin)),
                            point,
                            index,
                            instance: None,
                        });
                    }
                }
            }
        }
        best
    }
}

/// Rest-pose f32 POSITION, read as the f64 tuples three.js raycasts. The
/// former temporary used `chunks_exact(3)` even if a hand-edited accessor
/// declared another item size.
#[derive(Clone, Copy)]
struct StaticPositions<'a>(&'a [f32]);

impl StaticPositions<'_> {
    fn len(&self) -> usize {
        self.0.len() / 3
    }

    fn get(&self, i: usize) -> Option<Vec3> {
        if i >= self.len() {
            return None;
        }
        let at = i * 3;
        let p = &self.0[at..at + 3];
        Some([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
    }
}

/// `Points.raycast`'s per-point test: the closest point on the ray.
fn points_hit(
    positions: StaticPositions<'_>,
    indices: Option<&[u32]>,
    r: &Ray,
    threshold_sq: f64,
    world: &Mat4,
    world_ray: &Ray,
) -> Option<(Vec3, u32)> {
    let mut best: Option<(f64, Vec3, u32)> = None;
    let count = indices.map_or(positions.len(), <[u32]>::len);
    for i in 0..count {
        let vi = indices.map_or(i, |ix| ix[i] as usize);
        // An index past the last vertex reads NaN: never within range.
        let Some(p) = positions.get(vi) else {
            continue;
        };
        let along = dot(sub(p, r.origin), r.direction);
        let closest = if along < 0.0 { r.origin } else { r.at(along) };
        if distance_sq(closest, p) < threshold_sq {
            let d = length(sub(math::apply(world, closest), world_ray.origin));
            if best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, closest, vi as u32));
            }
        }
    }
    best.map(|(_, p, i)| (p, i))
}

/// `Line.raycast` for segments, strips, and loops.
fn line_hit(
    positions: StaticPositions<'_>,
    indices: Option<&[u32]>,
    mode: super::Mode,
    r: &Ray,
    threshold_sq: f64,
    world: &Mat4,
    world_ray: &Ray,
) -> Option<(Vec3, u32)> {
    // An index past the last vertex reads `undefined`: NaN coordinates.
    let v = |i: usize| {
        positions
            .get(indices.map_or(i, |ix| ix[i] as usize))
            .unwrap_or([f64::NAN; 3])
    };
    let count = indices.map_or(positions.len(), <[u32]>::len);
    if count < 2 {
        return None;
    }
    let step = if mode == super::Mode::LineSegments {
        2
    } else {
        1
    };
    let segments = (0..count - 1)
        .step_by(step)
        .map(|i| (i, i + 1))
        .chain((mode == super::Mode::LineLoop).then_some((count - 1, 0)));
    let mut best: Option<(f64, Vec3, u32)> = None;
    for (a, b) in segments {
        let (d2, on_ray) = distance_sq_to_segment(r, v(a), v(b));
        if d2 > threshold_sq {
            continue;
        }
        let d = length(sub(math::apply(world, on_ray), world_ray.origin));
        if best.as_ref().is_none_or(|x| d < x.0) {
            best = Some((d, on_ray, a as u32));
        }
    }
    best.map(|(_, p, i)| (p, i))
}

/// `Ray.distanceSqToSegment`: the squared distance and the point on the ray.
fn distance_sq_to_segment(r: &Ray, v0: Vec3, v1: Vec3) -> (f64, Vec3) {
    let center = scale(add(v0, v1), 0.5);
    let dir = normalize(sub(v1, v0));
    let diff = sub(r.origin, center);
    let extent = length(sub(v0, v1)) * 0.5;
    let a01 = -dot(r.direction, dir);
    let b0 = dot(diff, r.direction);
    let b1 = -dot(diff, dir);
    let c = dot(diff, diff);
    let det = (1.0 - a01 * a01).abs();
    let (s0, d2);
    if det > 0.0 {
        let mut x0 = a01 * b1 - b0;
        let mut x1 = a01 * b0 - b1;
        let ext = extent * det;
        if x0 >= 0.0 {
            if x1 >= -ext {
                if x1 <= ext {
                    let inv = 1.0 / det;
                    x0 *= inv;
                    x1 *= inv;
                    d2 = x0 * (x0 + a01 * x1 + 2.0 * b0) + x1 * (a01 * x0 + x1 + 2.0 * b1) + c;
                } else {
                    x1 = extent;
                    x0 = (-(a01 * x1 + b0)).max(0.0);
                    d2 = -x0 * x0 + x1 * (x1 + 2.0 * b1) + c;
                }
            } else {
                x1 = -extent;
                x0 = (-(a01 * x1 + b0)).max(0.0);
                d2 = -x0 * x0 + x1 * (x1 + 2.0 * b1) + c;
            }
        } else if x1 <= -ext {
            x0 = (-(-a01 * extent + b0)).max(0.0);
            x1 = if x0 > 0.0 {
                -extent
            } else {
                (-b1).clamp(-extent, extent)
            };
            d2 = -x0 * x0 + x1 * (x1 + 2.0 * b1) + c;
        } else if x1 <= ext {
            x0 = 0.0;
            x1 = (-b1).clamp(-extent, extent);
            d2 = x1 * (x1 + 2.0 * b1) + c;
        } else {
            x0 = (-(a01 * extent + b0)).max(0.0);
            x1 = if x0 > 0.0 {
                extent
            } else {
                (-b1).clamp(-extent, extent)
            };
            d2 = -x0 * x0 + x1 * (x1 + 2.0 * b1) + c;
        }
        s0 = x0;
    } else {
        let x1 = if a01 > 0.0 { -extent } else { extent };
        let x0 = (-(a01 * x1 + b0)).max(0.0);
        d2 = -x0 * x0 + x1 * (x1 + 2.0 * b1) + c;
        s0 = x0;
    }
    (d2, r.at(s0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_culling_follows_three() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let front = Ray {
            origin: [0.2, 0.2, 1.0],
            direction: [0.0, 0.0, -1.0],
        };
        assert_eq!(intersect_triangle(&front, a, b, c, true), Some(1.0));
        let back = Ray {
            origin: [0.2, 0.2, -1.0],
            direction: [0.0, 0.0, 1.0],
        };
        assert_eq!(intersect_triangle(&back, a, b, c, true), None);
        assert_eq!(intersect_triangle(&back, a, b, c, false), Some(1.0));
    }

    #[test]
    fn f32_position_reader_builds_bit_identical_bvh() {
        let positions: Vec<f32> = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 2.0, 2.0],
            [2.0, 2.0, 2.0],
            [2.0, 2.0, 2.0],
            [-0.0, f32::from_bits(0x7fc0_0001), f32::INFINITY],
            [f32::NEG_INFINITY, 3.0, 0.5],
            [4.0, 3.0, 0.5],
        ]
        .into_iter()
        .flatten()
        .collect();
        let f64_positions: Vec<Vec3> = positions
            .chunks_exact(3)
            .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
            .collect();
        let triangles = [
            [0, 1, 2],
            [3, 4, 5],
            [6, 7, 8],
            [0, 3, 6],
            [1, 4, 7],
            [2, 5, 8],
            [0, 4, 8],
        ];
        let from_slice = Bvh::build(&f64_positions, &triangles);
        let from_f32 = Bvh::build_from_positions(&triangles, |i| {
            let at = i * 3;
            [
                f64::from(positions[at]),
                f64::from(positions[at + 1]),
                f64::from(positions[at + 2]),
            ]
        });
        assert_eq!(from_slice.order, from_f32.order);
        assert_eq!(from_slice.nodes.len(), from_f32.nodes.len());
        for (a, b) in from_slice.nodes.iter().zip(&from_f32.nodes) {
            assert_eq!(
                (a.0.map(f64::to_bits), a.1.map(f64::to_bits), a.2, a.3),
                (b.0.map(f64::to_bits), b.1.map(f64::to_bits), b.2, b.3)
            );
        }
    }

    #[test]
    fn static_point_and_loop_readers_preserve_index_order() {
        let points = [-1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 99.0];
        // The trailing scalar is discarded by the former `chunks_exact(3)`.
        let ray = Ray {
            origin: [0.0, 0.0, 1.0],
            direction: [0.0, 0.0, -1.0],
        };
        assert_eq!(
            points_hit(
                StaticPositions(&points),
                Some(&[u32::MAX, 1, 0]),
                &ray,
                2.0,
                &math::IDENTITY,
                &ray,
            )
            .map(|(_, index)| index),
            Some(1),
        );

        let square = [
            -1.0f32, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0,
        ];
        let ray = Ray {
            origin: [-1.0, 0.0, 1.0],
            direction: [0.0, 0.0, -1.0],
        };
        assert_eq!(
            line_hit(
                StaticPositions(&square),
                None,
                super::super::Mode::LineLoop,
                &ray,
                0.01,
                &math::IDENTITY,
                &ray,
            )
            .map(|(_, index)| index),
            Some(3),
        );
    }

    #[test]
    fn bvh_matches_brute_force_and_refits() {
        let n = 30;
        let mut p = Vec::new();
        let mut t = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                p.push([
                    f64::from(i) / f64::from(n),
                    f64::from(j) / f64::from(n),
                    f64::from((i * 7 + j * 3) % 5) * 0.01,
                ]);
            }
        }
        for j in 0..n {
            for i in 0..n {
                let a = j * (n + 1) + i;
                t.push([a, a + 1, a + n + 2]);
                t.push([a, a + n + 2, a + n + 1]);
            }
        }
        let mut bvh = Bvh::build(&p, &t);
        for pass in 0..2 {
            for k in 0..300 {
                let x = f64::from(k % 20) / 19.0;
                let y = f64::from(k / 20) / 15.0;
                let r = Ray {
                    origin: [x * 1.2 - 0.1, y * 1.2 - 0.1, 2.0],
                    direction: normalize([0.05, -0.03, -1.0]),
                };
                let test = |i: u32| {
                    let tri = t[i as usize];
                    intersect_triangle(
                        &r,
                        p[tri[0] as usize],
                        p[tri[1] as usize],
                        p[tri[2] as usize],
                        false,
                    )
                };
                let a = bvh.closest(&r, test).map(|h| h.0);
                let b = (0..t.len() as u32)
                    .filter_map(|i| test(i).map(|d| (d, i)))
                    .min_by(|x, y| x.0.partial_cmp(&y.0).unwrap())
                    .map(|h| h.0);
                assert_eq!(a, b, "pass {pass} ray {k}");
            }
            for v in &mut p {
                v[2] += (v[0] * 3.0).sin() * 0.2;
                v[1] += v[0] * 0.3;
            }
            bvh.refit(&p, &t);
        }
    }

    #[test]
    fn segment_distance_matches_sampling() {
        let r = Ray {
            origin: [1.0, -2.0, 3.0],
            direction: normalize([-0.2, 0.5, -1.0]),
        };
        let (a, b) = ([0.2, -1.0, 0.1], [0.3, 1.0, -0.4]);
        let (d2, _) = distance_sq_to_segment(&r, a, b);
        let mut best = f64::INFINITY;
        for i in 0..=2000 {
            let p = add(a, scale(sub(b, a), f64::from(i) / 2000.0));
            let along = dot(sub(p, r.origin), r.direction).max(0.0);
            best = best.min(distance_sq(r.at(along), p));
        }
        assert!((d2 - best).abs() < 1e-4, "{d2} {best}");
    }
}
