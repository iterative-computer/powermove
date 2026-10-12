//! Model layers posed at a time, and picks with three.js r180's raycast semantics
//! (`Raycaster.setFromCamera`, `Mesh`/`SkinnedMesh`/`InstancedMesh`/`Points`/`Line`
//! raycasts, closest hit first) against the *deformed* geometry of the current frame.
//!
//! Rigid triangle meshes share one BVH per glTF primitive in local space. Deformed
//! meshes (skinned or morphed) keep model-space positions and their own BVH, re-skinned
//! and refit lazily by the first pick after a time change; layer transforms never force
//! a re-skin because rays are moved into model space instead.

use crate::anim::{self, Pose};
use crate::bvh::Bvh;
use crate::lazy::{LazySkin, SkinStatic};
use crate::model::{Model, Side, Topology, compose};
use glam::{DMat4, DVec3, DVec4};
use std::sync::Arc;

/// Per-primitive data shared by every layer that uses a model.
pub struct Prepared {
    pub model: Model,
    /// Triangles and the rest-pose BVH per (mesh, primitive); `None` for points/lines.
    pub tris: Vec<Vec<Option<(Vec<[u32; 3]>, Bvh)>>>,
    /// Joint masks per BVH node for skinned triangle primitives (lazy picks).
    pub skin: Vec<Vec<Option<SkinStatic>>>,
}

impl Prepared {
    pub fn new(model: Model) -> Self {
        let tris = model
            .meshes
            .iter()
            .map(|m| {
                m.primitives
                    .iter()
                    .map(|p| {
                        if p.topology != Topology::Triangles {
                            return None;
                        }
                        let t: Vec<[u32; 3]> = match &p.indices {
                            Some(ix) => ix.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
                            None => (0..p.positions.len() as u32 / 3).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect(),
                        };
                        let bvh = Bvh::build(&p.positions, &t);
                        Some((t, bvh))
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Option<(Vec<[u32; 3]>, Bvh)>>>>();
        let skin = model
            .meshes
            .iter()
            .enumerate()
            .map(|(mi, m)| {
                m.primitives
                    .iter()
                    .enumerate()
                    .map(|(pi, p)| {
                        let (t, bvh) = tris[mi][pi].as_ref()?;
                        let (j, w) = (p.joints.as_ref()?, p.weights.as_ref()?);
                        let count = j.iter().flat_map(|x| x.iter()).copied().max().unwrap_or(0) as usize + 1;
                        Some(SkinStatic::new(bvh, t, j, w, count))
                    })
                    .collect()
            })
            .collect();
        Prepared { model, tris, skin }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub projection: DMat4,
    pub world: DMat4,
    pub orthographic: bool,
    pub near: f64,
    pub far: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: DVec3,
    pub dir: DVec3,
}

/// three.js `Vector3.applyMatrix4` (with the perspective divide).
fn apply4(m: &DMat4, v: DVec3) -> DVec3 {
    let r = *m * DVec4::new(v.x, v.y, v.z, 1.0);
    DVec3::new(r.x / r.w, r.y / r.w, r.z / r.w)
}

/// three.js `Ray.applyMatrix4`: direction is transformed as a point pair, renormalized.
fn ray_apply(m: &DMat4, r: &Ray) -> Ray {
    let origin = apply4(m, r.origin);
    let dir = (apply4(m, r.dir + r.origin) - origin).normalize();
    Ray { origin, dir }
}

impl Camera {
    /// three.js `Raycaster.setFromCamera` for NDC (x, y); `unproject` applies the
    /// inverse projection, then the camera's world matrix.
    pub fn ray(&self, x: f64, y: f64) -> Ray {
        let inv = self.projection.inverse();
        let unproject = |v: DVec3| apply4(&self.world, apply4(&inv, v));
        if self.orthographic {
            let origin = unproject(DVec3::new(x, y, (self.near + self.far) / (self.near - self.far)));
            let dir = (self.world * DVec4::new(0.0, 0.0, -1.0, 0.0)).truncate().normalize();
            Ray { origin, dir }
        } else {
            let origin = self.world.w_axis.truncate();
            let dir = (unproject(DVec3::new(x, y, 0.5)) - origin).normalize();
            Ray { origin, dir }
        }
    }
}

/// The raycast pre-test three.js runs on Points and Line objects: the geometry's bounding
/// sphere (GLTFLoader's accessor-bounds sphere, else `computeBoundingSphere`), moved by
/// `Sphere.applyMatrix4` (radius × the largest axis scale) and grown by the threshold.
fn sphere_test(prim: &crate::model::Primitive, m: &DMat4, ray: &Ray, pad: f64) -> bool {
    let (center, radius) = match prim.accessor_bounds {
        Some((lo, hi)) => {
            let (lo, hi) = (DVec3::from_array(lo), DVec3::from_array(hi));
            ((lo + hi) * 0.5, lo.distance(hi) / 2.0)
        }
        None => {
            let (mut mn, mut mx) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
            for p in &prim.positions {
                let v = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64);
                mn = mn.min(v);
                mx = mx.max(v);
            }
            let center = (mn + mx) * 0.5;
            let r2 = prim.positions.iter().map(|p| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64).distance_squared(center)).fold(0.0, f64::max);
            (center, r2.sqrt())
        }
    };
    let scale = m.x_axis.truncate().length_squared().max(m.y_axis.truncate().length_squared()).max(m.z_axis.truncate().length_squared()).sqrt();
    let c = apply4(m, center);
    let r = radius * scale + pad;
    let along = (c - ray.origin).dot(ray.dir);
    let d2 = if along < 0.0 { ray.origin.distance_squared(c) } else { (ray.origin + ray.dir * along).distance_squared(c) };
    d2 <= r * r
}

struct Drawable {
    mesh: usize,
    prim: usize,
    node: usize,
    instance: Option<u32>,
    /// Model-space matrix (rigid drawables); world = layer.gltf_to_world × this.
    local: DMat4,
    deformed: Option<usize>,
    side: Side,
    hidden: bool,
}

struct Deformed {
    positions: Vec<[f32; 3]>,
    bvh: Bvh,
    /// Lazy skinning cache; `Some` and current when the pose is picked lazily.
    lazy: Option<LazySkin>,
    lazy_current: bool,
}

pub struct Layer {
    pub id: String,
    prepared: Arc<Prepared>,
    pub gltf_to_world: DMat4,
    time: Option<f64>,
    pose: Option<Pose>,
    globals: Vec<DMat4>,
    drawables: Vec<Drawable>,
    deformed: Vec<Deformed>,
    dirty: bool,
    pub visible: bool,
    pub locked: bool,
    /// Last re-skin + refit cost in milliseconds (diagnostics).
    pub last_update_ms: f64,
    /// Skinned primitives without active morphs are picked lazily (see [`crate::lazy`]);
    /// `false` re-skins every vertex and refits the BVH after each time change.
    pub lazy: bool,
}

/// One pick result.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub layer: usize,
    pub node: usize,
    pub kind: Topology,
    pub distance: f64,
    /// Triangle index (meshes) or vertex/segment index (points, lines).
    pub index: u32,
    pub instance: Option<u32>,
    pub point: DVec3,
}

pub type Clock = fn() -> f64;

impl Layer {
    pub fn new(id: &str, prepared: Arc<Prepared>, gltf_to_world: DMat4) -> Layer {
        let model = &prepared.model;
        let mut drawables = Vec::new();
        let mut deformed = Vec::new();
        // Nodes reachable from the scene roots, as GLTFLoader builds them.
        let mut stack: Vec<usize> = model.roots.iter().rev().copied().collect();
        let mut visited = vec![false; model.nodes.len()];
        while let Some(ni) = stack.pop() {
            if ni >= model.nodes.len() || visited[ni] {
                continue;
            }
            visited[ni] = true;
            let node = &model.nodes[ni];
            for &c in node.children.iter().rev() {
                stack.push(c);
            }
            let Some(mi) = node.mesh else { continue };
            for (pi, p) in model.meshes[mi].primitives.iter().enumerate() {
                let material = p.material.and_then(|m| model.materials.get(m));
                let side = material.map(|m| m.side).unwrap_or(Side::Front);
                // three.js: a mesh whose every material has opacity ≤ 0 is never picked.
                let hidden = material.map(|m| m.opacity <= 0.0).unwrap_or(false);
                let skinned = node.skin.is_some() && p.joints.is_some() && p.weights.is_some() && p.topology == Topology::Triangles;
                let morphed = !p.morph_positions.is_empty() && p.topology == Topology::Triangles;
                let instances: Vec<Option<u32>> = match &node.instances {
                    Some(list) if p.topology == Topology::Triangles => (0..list.len() as u32).map(Some).collect(),
                    _ => vec![None],
                };
                for instance in instances {
                    let def = if (skinned || morphed) && instance.is_none() {
                        let (_, bvh) = prepared.tris[mi][pi].as_ref().unwrap();
                        deformed.push(Deformed { positions: p.positions.clone(), bvh: bvh.clone(), lazy: None, lazy_current: false });
                        Some(deformed.len() - 1)
                    } else {
                        None
                    };
                    drawables.push(Drawable { mesh: mi, prim: pi, node: ni, instance, local: DMat4::IDENTITY, deformed: def, side, hidden });
                }
            }
        }
        Layer {
            id: id.to_string(),
            prepared,
            gltf_to_world,
            time: None,
            pose: None,
            globals: Vec::new(),
            drawables,
            deformed,
            dirty: true,
            visible: true,
            locked: false,
            last_update_ms: 0.0,
            lazy: true,
        }
    }

    /// World matrix of a node at the current pose (`set_time` first).
    pub fn node_world(&self, node: usize) -> Option<DMat4> {
        self.globals.get(node).map(|g| self.gltf_to_world * *g)
    }

    /// Current morph weights of a node (animated, node, or mesh defaults).
    pub fn morph_weights(&self, node: usize) -> Vec<f64> {
        let model = &self.prepared.model;
        match self.pose.as_ref().and_then(|p| p.weights.get(node).cloned().flatten()) {
            Some(w) => w,
            None => model.nodes[node].weights.clone().or_else(|| model.nodes[node].mesh.map(|m| model.meshes[m].weights.clone())).unwrap_or_default().iter().map(|v| *v as f64).collect(),
        }
    }

    pub fn model(&self) -> &Model {
        &self.prepared.model
    }

    /// World-space triangle-list positions per triangle drawable at the current pose,
    /// in scene traversal order (the geometry a Blender snapshot carries). Forces the
    /// full skinning path.
    pub fn posed_triangles(&mut self, clock: Clock) -> Vec<(String, Vec<[f64; 3]>)> {
        let lazy = self.lazy;
        self.lazy = false;
        self.dirty = !self.deformed.is_empty();
        self.update(clock);
        self.lazy = lazy;
        self.dirty = !self.deformed.is_empty();
        let model = &self.prepared.model;
        let mut out = Vec::new();
        for d in &self.drawables {
            let p = &model.meshes[d.mesh].primitives[d.prim];
            if p.topology != Topology::Triangles {
                continue;
            }
            let skinned = d.deformed.is_some() && model.nodes[d.node].skin.is_some();
            let to_world = if skinned { self.gltf_to_world } else { self.gltf_to_world * d.local };
            let positions: &[[f32; 3]] = match d.deformed {
                Some(di) => &self.deformed[di].positions,
                None => &p.positions,
            };
            let tris = &self.prepared.tris[d.mesh][d.prim].as_ref().unwrap().0;
            let list = tris.iter().flatten().map(|&v| {
                let q = positions[v as usize];
                apply4(&to_world, DVec3::new(q[0] as f64, q[1] as f64, q[2] as f64)).to_array()
            });
            out.push((model.nodes[d.node].name.clone(), list.collect()));
        }
        out
    }

    pub fn vertex_count(&self) -> usize {
        self.drawables.iter().map(|d| self.prepared.model.meshes[d.mesh].primitives[d.prim].positions.len()).sum()
    }

    /// Poses the layer at `time` (cheap); deformed geometry updates at the next pick.
    pub fn set_time(&mut self, time: f64) {
        if self.time == Some(time) {
            return;
        }
        let model = &self.prepared.model;
        let pose = anim::pose(model, time);
        // Node globals in model (glTF scene) space: parent × compose(T, R, S).
        let n = model.nodes.len();
        let mut globals = vec![DMat4::IDENTITY; n];
        let mut done = vec![false; n];
        let mut order: Vec<usize> = model.roots.clone();
        let mut k = 0;
        while k < order.len() {
            let ni = order[k];
            k += 1;
            if ni >= n || done[ni] {
                continue;
            }
            done[ni] = true;
            let local = compose(pose.translation[ni], pose.rotation[ni], pose.scale[ni]);
            globals[ni] = match model.nodes[ni].parent {
                Some(p) if done[p] => globals[p] * local,
                _ => local,
            };
            order.extend(model.nodes[ni].children.iter().copied());
        }
        for d in self.drawables.iter_mut() {
            d.local = match (d.instance, &model.nodes[d.node].instances) {
                (Some(i), Some(list)) => globals[d.node] * DMat4::from_cols_array(&list[i as usize].map(|v| v as f64)),
                _ => globals[d.node],
            };
        }
        self.globals = globals;
        self.pose = Some(pose);
        self.time = Some(time);
        self.dirty = !self.deformed.is_empty();
    }

    /// Skins and morphs every deformed primitive into model space and refits its BVH.
    pub fn update(&mut self, clock: Clock) {
        if !self.dirty {
            return;
        }
        let t0 = clock();
        let model = &self.prepared.model;
        let pose = self.pose.as_ref().expect("set_time before update");
        for d in &self.drawables {
            let Some(di) = d.deformed else { continue };
            let p = &model.meshes[d.mesh].primitives[d.prim];
            let node = &model.nodes[d.node];
            let Deformed { positions: out, bvh, lazy, lazy_current } = &mut self.deformed[di];
            *lazy_current = false;
            // Morph weights: animated, else node, else mesh defaults (f32 influences).
            let weights: Vec<f32> = match &pose.weights[d.node] {
                Some(w) => w.iter().map(|v| *v as f32).collect(),
                None => node.weights.clone().unwrap_or_else(|| model.meshes[d.mesh].weights.clone()),
            };
            let active: Vec<(usize, f32)> = weights.iter().copied().enumerate().filter(|(i, w)| *w != 0.0 && *i < p.morph_positions.len()).collect();
            match node.skin {
                Some(si) if p.joints.is_some() => {
                    let skin = &model.skins[si];
                    // jointMatrix = global(joint) × inverseBind (the mesh's bind matrix is
                    // identity in GLTFLoader's bind; the page export checks it).
                    let mats: Vec<[f32; 12]> = skin
                        .joints
                        .iter()
                        .zip(&skin.inverse_bind)
                        .map(|(&j, ibm)| {
                            let m = self.globals[j] * *ibm;
                            let c = m.to_cols_array();
                            [c[0] as f32, c[1] as f32, c[2] as f32, c[4] as f32, c[5] as f32, c[6] as f32, c[8] as f32, c[9] as f32, c[10] as f32, c[12] as f32, c[13] as f32, c[14] as f32]
                        })
                        .collect();
                    let joints = p.joints.as_ref().unwrap();
                    let wts = p.weights.as_ref().unwrap();
                    let stat = self.prepared.skin[d.mesh][d.prim].as_ref();
                    if self.lazy && active.is_empty() && stat.is_some_and(|s| s.normalized) {
                        let cache = lazy.get_or_insert_with(|| LazySkin::new(bvh.nodes.len(), p.positions.len()));
                        cache.set_pose(mats);
                        *lazy_current = true;
                        continue;
                    }
                    skin_vertices(&p.positions, &p.morph_positions, &active, joints, wts, &mats, out);
                }
                _ => {
                    // Morph only, in the node's local space (rigid transform applies at pick).
                    for (v, o) in p.positions.iter().zip(out.iter_mut()) {
                        *o = *v;
                    }
                    for &(m, w) in &active {
                        for (o, dlt) in out.iter_mut().zip(&p.morph_positions[m]) {
                            o[0] += w * dlt[0];
                            o[1] += w * dlt[1];
                            o[2] += w * dlt[2];
                        }
                    }
                }
            }
            let tris = &self.prepared.tris[d.mesh][d.prim].as_ref().unwrap().0;
            bvh.refit(out, tris);
        }
        self.dirty = false;
        self.last_update_ms = clock() - t0;
    }
}

/// Linear blend skinning into model space: Σ wᵢ · Mᵢ · (rest + Σ morph).
fn skin_vertices(rest: &[[f32; 3]], morphs: &[Vec<[f32; 3]>], active: &[(usize, f32)], joints: &[[u16; 4]], weights: &[[f32; 4]], mats: &[[f32; 12]], out: &mut [[f32; 3]]) {
    for i in 0..rest.len() {
        let mut p = rest[i];
        for &(m, w) in active {
            let d = morphs[m][i];
            p[0] += w * d[0];
            p[1] += w * d[1];
            p[2] += w * d[2];
        }
        let (j, w) = (joints[i], weights[i]);
        let mut acc = [0f32; 3];
        for k in 0..4 {
            let wk = w[k];
            if wk == 0.0 {
                continue;
            }
            let Some(m) = mats.get(j[k] as usize) else { continue };
            acc[0] += wk * (m[0] * p[0] + m[3] * p[1] + m[6] * p[2] + m[9]);
            acc[1] += wk * (m[1] * p[0] + m[4] * p[1] + m[7] * p[2] + m[10]);
            acc[2] += wk * (m[2] * p[0] + m[5] * p[1] + m[8] * p[2] + m[11]);
        }
        out[i] = acc;
    }
}

pub struct Scene {
    pub layers: Vec<Layer>,
    pub camera: Camera,
    pub clock: Clock,
}

fn no_clock() -> f64 {
    0.0
}

impl Scene {
    pub fn new(camera: Camera) -> Scene {
        Scene { layers: Vec::new(), camera, clock: no_clock }
    }

    /// Closest pickable hit at NDC (x, y) among `eligible` layers (the depth set),
    /// skipping hidden and locked layers. Deformed layers update first if needed.
    pub fn pick(&mut self, x: f64, y: f64, eligible: Option<&[usize]>) -> Option<Hit> {
        let ray = self.camera.ray(x, y);
        let clock = self.clock;
        let mut best: Option<Hit> = None;
        for (li, layer) in self.layers.iter_mut().enumerate() {
            if !layer.visible || layer.locked || eligible.is_some_and(|e| !e.contains(&li)) {
                continue;
            }
            layer.update(clock);
            let prepared = layer.prepared.clone();
            let model = &prepared.model;
            for d in &layer.drawables {
                if d.hidden {
                    continue;
                }
                let p = &model.meshes[d.mesh].primitives[d.prim];
                let to_world = if d.deformed.is_some() && model.nodes[d.node].skin.is_some() { layer.gltf_to_world } else { layer.gltf_to_world * d.local };
                let inv = to_world.inverse();
                let local_ray = ray_apply(&inv, &ray);
                let found: Option<(DVec3, u32)> = match p.topology {
                    Topology::Triangles if d.deformed.is_some_and(|di| layer.deformed[di].lazy_current) => {
                        let di = d.deformed.unwrap();
                        let (tris, bvh) = layer.prepared.tris[d.mesh][d.prim].as_ref().unwrap();
                        let stat = layer.prepared.skin[d.mesh][d.prim].as_ref().unwrap();
                        let cache = layer.deformed[di].lazy.as_mut().unwrap();
                        let cull = d.side == Side::Front;
                        cache
                            .closest(bvh, stat, tris, &p.positions, p.joints.as_ref().unwrap(), p.weights.as_ref().unwrap(), local_ray.origin, local_ray.dir, cull)
                            .map(|(t, tri)| (local_ray.origin + local_ray.dir * t, tri))
                    }
                    Topology::Triangles => {
                        let tris = &layer.prepared.tris[d.mesh][d.prim].as_ref().unwrap().0;
                        let (positions, bvh): (&[[f32; 3]], &Bvh) = match d.deformed {
                            Some(di) => (&layer.deformed[di].positions, &layer.deformed[di].bvh),
                            None => (&p.positions, &layer.prepared.tris[d.mesh][d.prim].as_ref().unwrap().1),
                        };
                        let cull = d.side == Side::Front;
                        bvh.closest(positions, tris, local_ray.origin, local_ray.dir, cull, f64::INFINITY).map(|(t, tri)| (local_ray.origin + local_ray.dir * t, tri))
                    }
                    Topology::Points if !sphere_test(p, &to_world, &ray, 1.0) => None,
                    Topology::Points => {
                        let threshold = 1.0 / local_scale_mean(model, d.node);
                        points_hit(&p.positions, p.indices.as_deref(), &local_ray, threshold * threshold, &to_world, &ray)
                    }
                    _ if !sphere_test(p, &to_world, &ray, 1.0) => None,
                    _ => {
                        let threshold = 1.0 / local_scale_mean(model, d.node);
                        line_hit(&p.positions, p.indices.as_deref(), p.topology, &local_ray, threshold * threshold, &to_world, &ray)
                    }
                };
                if let Some((local_point, index)) = found {
                    let world_point = apply4(&to_world, local_point);
                    let distance = ray.origin.distance(world_point);
                    if best.as_ref().is_none_or(|b| distance < b.distance) {
                        best = Some(Hit { layer: li, node: d.node, kind: p.topology, distance, index, instance: d.instance, point: world_point });
                    }
                }
            }
        }
        best
    }
}

/// three.js thresholds divide by the object's own local scale, `(sx + sy + sz) / 3`.
fn local_scale_mean(model: &Model, node: usize) -> f64 {
    let s = model.nodes[node].scale;
    (s.x + s.y + s.z) / 3.0
}

/// three.js `Points.raycast` (threshold 1): returns the closest point on the ray (local).
fn points_hit(pos: &[[f32; 3]], indices: Option<&[u32]>, r: &Ray, threshold_sq: f64, to_world: &DMat4, world_ray: &Ray) -> Option<(DVec3, u32)> {
    let mut best: Option<(f64, DVec3, u32)> = None;
    let count = indices.map(|i| i.len()).unwrap_or(pos.len());
    for i in 0..count {
        let vi = indices.map(|ix| ix[i] as usize).unwrap_or(i);
        let p = DVec3::new(pos[vi][0] as f64, pos[vi][1] as f64, pos[vi][2] as f64);
        let along = (p - r.origin).dot(r.dir);
        let dist_sq = if along < 0.0 { r.origin.distance_squared(p) } else { (r.origin + r.dir * along).distance_squared(p) };
        if dist_sq < threshold_sq {
            let closest = if along < 0.0 { r.origin } else { r.origin + r.dir * along };
            let d = world_ray.origin.distance(apply4(to_world, closest));
            if best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, closest, vi as u32));
            }
        }
    }
    best.map(|(_, p, i)| (p, i))
}

/// three.js `Line.raycast` (threshold 1) for segments, loops, and strips.
fn line_hit(pos: &[[f32; 3]], indices: Option<&[u32]>, topology: Topology, r: &Ray, threshold_sq: f64, to_world: &DMat4, world_ray: &Ray) -> Option<(DVec3, u32)> {
    let v = |i: usize| -> DVec3 {
        let vi = indices.map(|ix| ix[i] as usize).unwrap_or(i);
        DVec3::new(pos[vi][0] as f64, pos[vi][1] as f64, pos[vi][2] as f64)
    };
    let count = indices.map(|i| i.len()).unwrap_or(pos.len());
    if count < 2 {
        return None;
    }
    let step = if topology == Topology::LineSegments { 2 } else { 1 };
    let mut segments: Vec<(usize, usize)> = (0..count - 1).step_by(step).map(|i| (i, i + 1)).collect();
    if topology == Topology::LineLoop {
        segments.push((count - 1, 0));
    }
    let mut best: Option<(f64, DVec3, u32)> = None;
    for (a, b) in segments {
        let (dist_sq, on_ray) = distance_sq_to_segment(r, v(a), v(b));
        if dist_sq > threshold_sq {
            continue;
        }
        let d = world_ray.origin.distance(apply4(to_world, on_ray));
        if best.as_ref().is_none_or(|x| d < x.0) {
            best = Some((d, on_ray, a as u32));
        }
    }
    best.map(|(_, p, i)| (p, i))
}

/// three.js `Ray.distanceSqToSegment` (Geometric Tools); returns the point on the ray.
fn distance_sq_to_segment(r: &Ray, v0: DVec3, v1: DVec3) -> (f64, DVec3) {
    let seg_center = (v0 + v1) * 0.5;
    let seg_dir = (v1 - v0).normalize_or_zero();
    let diff = r.origin - seg_center;
    let seg_extent = v0.distance(v1) * 0.5;
    let a01 = -r.dir.dot(seg_dir);
    let b0 = diff.dot(r.dir);
    let b1 = -diff.dot(seg_dir);
    let c = diff.length_squared();
    let det = (1.0 - a01 * a01).abs();
    let (s0, sqr_dist);
    if det > 0.0 {
        let mut s0_ = a01 * b1 - b0;
        let mut s1 = a01 * b0 - b1;
        let ext_det = seg_extent * det;
        if s0_ >= 0.0 {
            if s1 >= -ext_det {
                if s1 <= ext_det {
                    let inv_det = 1.0 / det;
                    s0_ *= inv_det;
                    s1 *= inv_det;
                    sqr_dist = s0_ * (s0_ + a01 * s1 + 2.0 * b0) + s1 * (a01 * s0_ + s1 + 2.0 * b1) + c;
                } else {
                    s1 = seg_extent;
                    s0_ = (-(a01 * s1 + b0)).max(0.0);
                    sqr_dist = -s0_ * s0_ + s1 * (s1 + 2.0 * b1) + c;
                }
            } else {
                s1 = -seg_extent;
                s0_ = (-(a01 * s1 + b0)).max(0.0);
                sqr_dist = -s0_ * s0_ + s1 * (s1 + 2.0 * b1) + c;
            }
        } else if s1 <= -ext_det {
            s0_ = (-(-a01 * seg_extent + b0)).max(0.0);
            s1 = if s0_ > 0.0 { -seg_extent } else { (-b1).clamp(-seg_extent, seg_extent) };
            sqr_dist = -s0_ * s0_ + s1 * (s1 + 2.0 * b1) + c;
        } else if s1 <= ext_det {
            s0_ = 0.0;
            s1 = (-b1).clamp(-seg_extent, seg_extent);
            sqr_dist = s1 * (s1 + 2.0 * b1) + c;
        } else {
            s0_ = (-(a01 * seg_extent + b0)).max(0.0);
            s1 = if s0_ > 0.0 { seg_extent } else { (-b1).clamp(-seg_extent, seg_extent) };
            sqr_dist = -s0_ * s0_ + s1 * (s1 + 2.0 * b1) + c;
        }
        s0 = s0_;
        let _ = s1;
    } else {
        let s1 = if a01 > 0.0 { -seg_extent } else { seg_extent };
        let s0_ = (-(a01 * s1 + b0)).max(0.0);
        sqr_dist = -s0_ * s0_ + s1 * (s1 + 2.0 * b1) + c;
        s0 = s0_;
    }
    (sqr_dist, r.origin + r.dir * s0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_distance_matches_sampling() {
        let rays = [
            Ray { origin: DVec3::new(0.0, 0.0, 5.0), dir: DVec3::new(0.01, 0.02, -1.0).normalize() },
            Ray { origin: DVec3::new(1.0, -2.0, 3.0), dir: DVec3::new(-0.2, 0.5, -1.0).normalize() },
            Ray { origin: DVec3::new(0.3, 0.3, -2.0), dir: DVec3::new(0.0, 0.0, 1.0) },
        ];
        let segs = [(DVec3::new(-1.0, 0.5, 0.0), DVec3::new(1.0, 0.7, 0.2)), (DVec3::new(0.2, -1.0, 0.1), DVec3::new(0.3, 1.0, -0.4)), (DVec3::new(0.0, 0.0, 10.0), DVec3::new(0.0, 1.0, 10.0))];
        for r in &rays {
            for (a, b) in &segs {
                let (d2, _) = distance_sq_to_segment(r, *a, *b);
                let mut best = f64::INFINITY;
                for i in 0..=400 {
                    let p = *a + (*b - *a) * (i as f64 / 400.0);
                    let along = (p - r.origin).dot(r.dir).max(0.0);
                    best = best.min((r.origin + r.dir * along).distance_squared(p));
                }
                assert!((d2 - best).abs() < 1e-3, "{d2} vs {best}");
            }
        }
    }
}
