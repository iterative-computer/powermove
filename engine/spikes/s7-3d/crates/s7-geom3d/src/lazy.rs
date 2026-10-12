//! Lazy skinned picks: after a time change nothing is re-skinned up front. A BVH node's
//! box for the current pose is bounded on demand by the union, over the joints that
//! influence the node's vertices, of its rest-pose box moved by each joint matrix (with
//! non-negative weights summing to 1, a skinned vertex lies in the convex hull of its
//! per-joint images). Leaves the ray reaches skin only their own vertices, with exactly
//! the arithmetic of the full path, so hits are identical. Both caches are stamped with
//! a pose epoch, so later picks at the same frame reuse them.
//!
//! The first pick after a time change then costs O(visited nodes) instead of O(vertices).
//! Primitives with active morph targets or unnormalized weights use the full path.

use crate::bvh::{Bvh, intersect_triangle};
use glam::DVec3;

/// Per skinned primitive, computed once.
pub struct SkinStatic {
    pub words: usize,
    /// Joint bitmask per BVH node (`words` u64 each).
    pub node_joints: Vec<u64>,
    /// Every vertex has non-negative weights summing to 1 (within f32 rounding).
    pub normalized: bool,
}

impl SkinStatic {
    pub fn new(bvh: &Bvh, tris: &[[u32; 3]], joints: &[[u16; 4]], weights: &[[f32; 4]], joint_count: usize) -> SkinStatic {
        let words = joint_count.div_ceil(64).max(1);
        let mut node_joints = vec![0u64; bvh.nodes.len() * words];
        for i in (0..bvh.nodes.len()).rev() {
            let n = bvh.nodes[i];
            if n.count > 0 {
                for &t in &bvh.order[n.first as usize..(n.first + n.count) as usize] {
                    for &v in &tris[t as usize] {
                        for k in 0..4 {
                            let (j, w) = (joints[v as usize][k] as usize, weights[v as usize][k]);
                            if w != 0.0 && j < joint_count {
                                node_joints[i * words + j / 64] |= 1 << (j % 64);
                            }
                        }
                    }
                }
            } else {
                for k in 0..words {
                    node_joints[i * words + k] = node_joints[n.first as usize * words + k] | node_joints[(n.first as usize + 1) * words + k];
                }
            }
        }
        let normalized = weights.iter().all(|w| w.iter().all(|x| *x >= 0.0) && ((w[0] + w[1] + w[2] + w[3]) - 1.0).abs() <= 1e-5);
        SkinStatic { words, node_joints, normalized }
    }
}

/// Skins one vertex (no morphs): Σ wᵢ · Mᵢ · p, the same operations as the full path.
#[inline]
pub fn skin_one(p: [f32; 3], j: [u16; 4], w: [f32; 4], mats: &[[f32; 12]]) -> [f32; 3] {
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
    acc
}

#[derive(Default)]
pub struct LazySkin {
    epoch: u32,
    node_epoch: Vec<u32>,
    node_bounds: Vec<[f64; 6]>,
    vert_epoch: Vec<u32>,
    verts: Vec<[f32; 3]>,
    pub mats: Vec<[f32; 12]>,
    /// Work done since the last pose change (diagnostics).
    pub nodes_bounded: usize,
    pub verts_skinned: usize,
}

impl LazySkin {
    pub fn new(nodes: usize, verts: usize) -> LazySkin {
        LazySkin { epoch: 1, node_epoch: vec![0; nodes], node_bounds: vec![[0.0; 6]; nodes], vert_epoch: vec![0; verts], verts: vec![[0.0; 3]; verts], ..Default::default() }
    }

    /// New joint matrices: invalidates both caches in O(1).
    pub fn set_pose(&mut self, mats: Vec<[f32; 12]>) {
        self.mats = mats;
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.nodes_bounded = 0;
        self.verts_skinned = 0;
    }

    fn node_box(&mut self, i: usize, bvh: &Bvh, stat: &SkinStatic) -> [f64; 6] {
        if self.node_epoch[i] == self.epoch {
            return self.node_bounds[i];
        }
        let n = &bvh.nodes[i];
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        if n.max[0] >= n.min[0] {
            let c = DVec3::new((n.min[0] as f64 + n.max[0] as f64) * 0.5, (n.min[1] as f64 + n.max[1] as f64) * 0.5, (n.min[2] as f64 + n.max[2] as f64) * 0.5);
            let e = DVec3::new((n.max[0] as f64 - n.min[0] as f64) * 0.5, (n.max[1] as f64 - n.min[1] as f64) * 0.5, (n.max[2] as f64 - n.min[2] as f64) * 0.5);
            for word in 0..stat.words {
                let mut bits = stat.node_joints[i * stat.words + word];
                while bits != 0 {
                    let j = word * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let Some(m) = self.mats.get(j) else { continue };
                    let m: [f64; 12] = std::array::from_fn(|k| m[k] as f64);
                    let cc = DVec3::new(m[0] * c.x + m[3] * c.y + m[6] * c.z + m[9], m[1] * c.x + m[4] * c.y + m[7] * c.z + m[10], m[2] * c.x + m[5] * c.y + m[8] * c.z + m[11]);
                    let ee = DVec3::new(
                        m[0].abs() * e.x + m[3].abs() * e.y + m[6].abs() * e.z,
                        m[1].abs() * e.x + m[4].abs() * e.y + m[7].abs() * e.z,
                        m[2].abs() * e.x + m[5].abs() * e.y + m[8].abs() * e.z,
                    );
                    // f32 skinning rounds; pad by a few ulps of the magnitudes involved.
                    let pad = 1e-5 * (1.0 + cc.abs().max_element() + ee.max_element());
                    lo = lo.min(cc - ee - DVec3::splat(pad));
                    hi = hi.max(cc + ee + DVec3::splat(pad));
                }
            }
        }
        let b = [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z];
        self.node_bounds[i] = b;
        self.node_epoch[i] = self.epoch;
        self.nodes_bounded += 1;
        b
    }

    fn vert(&mut self, v: usize, rest: &[[f32; 3]], joints: &[[u16; 4]], weights: &[[f32; 4]]) -> DVec3 {
        if self.vert_epoch[v] != self.epoch {
            self.verts[v] = skin_one(rest[v], joints[v], weights[v], &self.mats);
            self.vert_epoch[v] = self.epoch;
            self.verts_skinned += 1;
        }
        let p = self.verts[v];
        DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
    }

    /// Closest hit in model space, as [`Bvh::closest`] would find on the skinned mesh.
    #[allow(clippy::too_many_arguments)]
    pub fn closest(&mut self, bvh: &Bvh, stat: &SkinStatic, tris: &[[u32; 3]], rest: &[[f32; 3]], joints: &[[u16; 4]], weights: &[[f32; 4]], origin: DVec3, dir: DVec3, cull_back: bool) -> Option<(f64, u32)> {
        let inv = DVec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut best = f64::INFINITY;
        let mut hit = None;
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        let root = self.node_box(0, bvh, stat);
        if slab(&root, origin, inv).is_none() {
            return None;
        }
        stack.push(0);
        while let Some(ni) = stack.pop() {
            let node = bvh.nodes[ni as usize];
            if node.count > 0 {
                for &t in &bvh.order[node.first as usize..(node.first + node.count) as usize] {
                    let tri = tris[t as usize];
                    let a = self.vert(tri[0] as usize, rest, joints, weights);
                    let b = self.vert(tri[1] as usize, rest, joints, weights);
                    let c = self.vert(tri[2] as usize, rest, joints, weights);
                    if let Some(d) = intersect_triangle(origin, dir, a, b, c, cull_back) {
                        if d < best {
                            best = d;
                            hit = Some((d, t));
                        }
                    }
                }
                continue;
            }
            let (l, r) = (node.first, node.first + 1);
            let bl = self.node_box(l as usize, bvh, stat);
            let br = self.node_box(r as usize, bvh, stat);
            let tl = slab(&bl, origin, inv).filter(|t| *t <= best);
            let tr = slab(&br, origin, inv).filter(|t| *t <= best);
            match (tl, tr) {
                (Some(a), Some(b)) => {
                    let (near, far) = if a <= b { (l, r) } else { (r, l) };
                    stack.push(far);
                    stack.push(near);
                }
                (Some(_), None) => stack.push(l),
                (None, Some(_)) => stack.push(r),
                (None, None) => {}
            }
        }
        hit
    }
}

#[inline]
fn slab(b: &[f64; 6], o: DVec3, inv: DVec3) -> Option<f64> {
    if b[3] < b[0] {
        return None;
    }
    let mut tmin = 0.0f64;
    let mut tmax = f64::INFINITY;
    for k in 0..3 {
        let t1 = (b[k] - o[k]) * inv[k];
        let t2 = (b[k + 3] - o[k]) * inv[k];
        let (a, c) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
        if a > tmin {
            tmin = a;
        }
        if c < tmax {
            tmax = c;
        }
    }
    if tmin <= tmax { Some(tmin) } else { None }
}
