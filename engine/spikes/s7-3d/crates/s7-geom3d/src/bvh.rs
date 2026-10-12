//! Triangle BVH: binned-SAH build once over the rest pose, bottom-up refit when the
//! geometry deforms (skinning, morphs), and closest-hit traversal with three.js's
//! `Ray.intersectTriangle` (f64, optional back-face culling) at the leaves.
//!
//! Nodes are 32 bytes; children of a node are allocated together after it, so a reverse
//! sweep over the node array refits children before parents.

use glam::DVec3;

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Node {
    pub min: [f32; 3],
    /// Leaf: first index into `order`; interior: index of the left child (right = +1).
    pub first: u32,
    pub max: [f32; 3],
    /// Leaf: triangle count (> 0); interior: 0.
    pub count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Bvh {
    pub nodes: Vec<Node>,
    /// Triangle indices in leaf order.
    pub order: Vec<u32>,
}

pub const LEAF_SIZE: usize = 4;
const BINS: usize = 12;

#[inline]
fn tri_bounds(p: &[[f32; 3]], t: &[u32; 3]) -> ([f32; 3], [f32; 3]) {
    let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
    (
        [a[0].min(b[0]).min(c[0]), a[1].min(b[1]).min(c[1]), a[2].min(b[2]).min(c[2])],
        [a[0].max(b[0]).max(c[0]), a[1].max(b[1]).max(c[1]), a[2].max(b[2]).max(c[2])],
    )
}

fn area(min: [f32; 3], max: [f32; 3]) -> f32 {
    let e = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    if e[0] < 0.0 {
        return 0.0;
    }
    e[0] * e[1] + e[1] * e[2] + e[2] * e[0]
}

impl Bvh {
    /// Builds over `tris` with `positions` (any pose; refit adapts it to others).
    pub fn build(positions: &[[f32; 3]], tris: &[[u32; 3]]) -> Bvh {
        let n = tris.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        if n == 0 {
            return Bvh { nodes: vec![Node { min: [0.0; 3], first: 0, max: [-1.0; 3], count: 0 }], order };
        }
        let bounds: Vec<([f32; 3], [f32; 3])> = tris.iter().map(|t| tri_bounds(positions, t)).collect();
        let centroids: Vec<[f32; 3]> = bounds.iter().map(|(a, b)| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5]).collect();
        let mut nodes = Vec::with_capacity(2 * n / LEAF_SIZE + 1);
        nodes.push(Node { first: 0, count: n as u32, ..Default::default() });
        let mut stack = vec![0usize];
        while let Some(ni) = stack.pop() {
            let (first, count) = (nodes[ni].first as usize, nodes[ni].count as usize);
            let slice = &mut order[first..first + count];
            let (mut bmin, mut bmax) = ([f32::MAX; 3], [f32::MIN; 3]);
            let (mut cmin, mut cmax) = ([f32::MAX; 3], [f32::MIN; 3]);
            for &t in slice.iter() {
                let (a, b) = bounds[t as usize];
                let c = centroids[t as usize];
                for k in 0..3 {
                    bmin[k] = bmin[k].min(a[k]);
                    bmax[k] = bmax[k].max(b[k]);
                    cmin[k] = cmin[k].min(c[k]);
                    cmax[k] = cmax[k].max(c[k]);
                }
            }
            nodes[ni].min = bmin;
            nodes[ni].max = bmax;
            if count <= LEAF_SIZE {
                continue;
            }
            // Binned SAH over the centroid bounds.
            let mut best = (f32::MAX, 0usize, 0usize);
            for axis in 0..3 {
                let extent = cmax[axis] - cmin[axis];
                if extent <= 0.0 {
                    continue;
                }
                let mut bin_min = [[f32::MAX; 3]; BINS];
                let mut bin_max = [[f32::MIN; 3]; BINS];
                let mut bin_n = [0usize; BINS];
                let scale = BINS as f32 / extent;
                for &t in slice.iter() {
                    let b = (((centroids[t as usize][axis] - cmin[axis]) * scale) as usize).min(BINS - 1);
                    let (a, c) = bounds[t as usize];
                    bin_n[b] += 1;
                    for k in 0..3 {
                        bin_min[b][k] = bin_min[b][k].min(a[k]);
                        bin_max[b][k] = bin_max[b][k].max(c[k]);
                    }
                }
                let mut left_area = [0f32; BINS];
                let mut left_n = [0usize; BINS];
                let (mut mn, mut mx, mut cnt) = ([f32::MAX; 3], [f32::MIN; 3], 0);
                for b in 0..BINS - 1 {
                    cnt += bin_n[b];
                    for k in 0..3 {
                        mn[k] = mn[k].min(bin_min[b][k]);
                        mx[k] = mx[k].max(bin_max[b][k]);
                    }
                    left_area[b] = area(mn, mx);
                    left_n[b] = cnt;
                }
                let (mut mn, mut mx, mut cnt) = ([f32::MAX; 3], [f32::MIN; 3], 0);
                for b in (1..BINS).rev() {
                    cnt += bin_n[b];
                    for k in 0..3 {
                        mn[k] = mn[k].min(bin_min[b][k]);
                        mx[k] = mx[k].max(bin_max[b][k]);
                    }
                    let cost = left_area[b - 1] * left_n[b - 1] as f32 + area(mn, mx) * cnt as f32;
                    if left_n[b - 1] > 0 && cnt > 0 && cost < best.0 {
                        best = (cost, axis, b);
                    }
                }
            }
            let mid = if best.0 < f32::MAX {
                let (axis, split) = (best.1, best.2);
                let scale = BINS as f32 / (cmax[axis] - cmin[axis]);
                let mut i = 0;
                let mut j = slice.len();
                while i < j {
                    let b = (((centroids[slice[i] as usize][axis] - cmin[axis]) * scale) as usize).min(BINS - 1);
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
            let mid = if mid == 0 || mid == count { count / 2 } else { mid };
            let left = nodes.len();
            nodes.push(Node { first: first as u32, count: mid as u32, ..Default::default() });
            nodes.push(Node { first: (first + mid) as u32, count: (count - mid) as u32, ..Default::default() });
            nodes[ni].first = left as u32;
            nodes[ni].count = 0;
            stack.push(left);
            stack.push(left + 1);
        }
        Bvh { nodes, order }
    }

    /// Recomputes every box for new vertex positions (same topology).
    pub fn refit(&mut self, positions: &[[f32; 3]], tris: &[[u32; 3]]) {
        for i in (0..self.nodes.len()).rev() {
            let node = self.nodes[i];
            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
            if node.count > 0 {
                for &t in &self.order[node.first as usize..(node.first + node.count) as usize] {
                    let (a, b) = tri_bounds(positions, &tris[t as usize]);
                    for k in 0..3 {
                        mn[k] = mn[k].min(a[k]);
                        mx[k] = mx[k].max(b[k]);
                    }
                }
            } else {
                let (l, r) = (self.nodes[node.first as usize], self.nodes[node.first as usize + 1]);
                for k in 0..3 {
                    mn[k] = l.min[k].min(r.min[k]);
                    mx[k] = l.max[k].max(r.max[k]);
                }
            }
            self.nodes[i].min = mn;
            self.nodes[i].max = mx;
        }
    }

    /// Closest hit along the ray (`dir` normalized). Returns (t, triangle index).
    pub fn closest(&self, positions: &[[f32; 3]], tris: &[[u32; 3]], origin: DVec3, dir: DVec3, cull_back: bool, mut best: f64) -> Option<(f64, u32)> {
        let inv = DVec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut hit = None;
        let mut stack = [0u32; 64];
        let mut sp = 0usize;
        let root = &self.nodes[0];
        if slab(root, origin, inv).is_none_or(|t| t > best) {
            return None;
        }
        stack[sp] = 0;
        sp += 1;
        while sp > 0 {
            sp -= 1;
            let node = &self.nodes[stack[sp] as usize];
            if node.count > 0 {
                for &t in &self.order[node.first as usize..(node.first + node.count) as usize] {
                    let tri = &tris[t as usize];
                    let v = |i: usize| -> DVec3 {
                        let p = positions[tri[i] as usize];
                        DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
                    };
                    if let Some(d) = intersect_triangle(origin, dir, v(0), v(1), v(2), cull_back) {
                        if d < best {
                            best = d;
                            hit = Some((d, t));
                        }
                    }
                }
                continue;
            }
            if sp + 2 > stack.len() {
                // Pathological depth: finish exhaustively.
                return self.brute(positions, tris, origin, dir, cull_back, best).or(hit);
            }
            let (l, r) = (node.first, node.first + 1);
            let tl = slab(&self.nodes[l as usize], origin, inv).filter(|t| *t <= best);
            let tr = slab(&self.nodes[r as usize], origin, inv).filter(|t| *t <= best);
            match (tl, tr) {
                (Some(a), Some(b)) => {
                    // Push the far child first so the near one is visited next.
                    let (near, far) = if a <= b { (l, r) } else { (r, l) };
                    stack[sp] = far;
                    stack[sp + 1] = near;
                    sp += 2;
                }
                (Some(_), None) => {
                    stack[sp] = l;
                    sp += 1;
                }
                (None, Some(_)) => {
                    stack[sp] = r;
                    sp += 1;
                }
                (None, None) => {}
            }
        }
        hit
    }

    fn brute(&self, positions: &[[f32; 3]], tris: &[[u32; 3]], origin: DVec3, dir: DVec3, cull_back: bool, mut best: f64) -> Option<(f64, u32)> {
        let mut hit = None;
        for (i, tri) in tris.iter().enumerate() {
            let v = |k: usize| -> DVec3 {
                let p = positions[tri[k] as usize];
                DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
            };
            if let Some(d) = intersect_triangle(origin, dir, v(0), v(1), v(2), cull_back) {
                if d < best {
                    best = d;
                    hit = Some((d, i as u32));
                }
            }
        }
        hit
    }
}

/// Entry distance of the ray into the box (f64 slab test; 0 when the origin is inside).
#[inline]
fn slab(n: &Node, o: DVec3, inv: DVec3) -> Option<f64> {
    if n.max[0] < n.min[0] {
        return None;
    }
    let mut tmin = 0.0f64;
    let mut tmax = f64::INFINITY;
    for k in 0..3 {
        let (lo, hi) = (n.min[k] as f64, n.max[k] as f64);
        let t1 = (lo - o[k]) * inv[k];
        let t2 = (hi - o[k]) * inv[k];
        // NaN (0 · ∞ for an axis-parallel ray on a slab plane) keeps the interval.
        let (a, b) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
        if a > tmin {
            tmin = a;
        }
        if b < tmax {
            tmax = b;
        }
    }
    if tmin <= tmax { Some(tmin) } else { None }
}

/// three.js r180 `Ray.intersectTriangle` (returns the ray parameter).
#[inline]
pub fn intersect_triangle(origin: DVec3, dir: DVec3, a: DVec3, b: DVec3, c: DVec3, backface_culling: bool) -> Option<f64> {
    let edge1 = b - a;
    let edge2 = c - a;
    let normal = edge1.cross(edge2);
    let mut ddn = dir.dot(normal);
    let sign;
    if ddn > 0.0 {
        if backface_culling {
            return None;
        }
        sign = 1.0;
    } else if ddn < 0.0 {
        sign = -1.0;
        ddn = -ddn;
    } else {
        return None;
    }
    let diff = origin - a;
    let ddqxe2 = sign * dir.dot(diff.cross(edge2));
    if ddqxe2 < 0.0 {
        return None;
    }
    let dde1xq = sign * dir.dot(edge1.cross(diff));
    if dde1xq < 0.0 {
        return None;
    }
    if ddqxe2 + dde1xq > ddn {
        return None;
    }
    let qdn = -sign * diff.dot(normal);
    if qdn < 0.0 {
        return None;
    }
    Some(qdn / ddn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let mut p = Vec::new();
        let mut t = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                p.push([i as f32 / n as f32, j as f32 / n as f32, ((i * 7 + j * 3) % 5) as f32 * 0.01]);
            }
        }
        for j in 0..n {
            for i in 0..n {
                let a = (j * (n + 1) + i) as u32;
                t.push([a, a + 1, a + n as u32 + 2]);
                t.push([a, a + n as u32 + 2, a + n as u32 + 1]);
            }
        }
        (p, t)
    }

    #[test]
    fn bvh_matches_brute_force_before_and_after_refit() {
        let (mut p, t) = grid(40);
        let mut bvh = Bvh::build(&p, &t);
        for pass in 0..2 {
            for k in 0..400 {
                let x = (k % 20) as f64 / 19.0;
                let y = (k / 20) as f64 / 19.0;
                let o = DVec3::new(x * 1.2 - 0.1, y * 1.2 - 0.1, 2.0);
                let d = DVec3::new(0.05, -0.03, -1.0).normalize();
                let a = bvh.closest(&p, &t, o, d, k % 2 == 0, f64::INFINITY).map(|h| h.0);
                let b = bvh.brute(&p, &t, o, d, k % 2 == 0, f64::INFINITY).map(|h| h.0);
                assert_eq!(a, b, "pass {pass} ray {k}");
            }
            // Deform and refit.
            for v in p.iter_mut() {
                v[2] += (v[0] * 3.0).sin() * 0.2;
                v[1] += v[0] * 0.3;
            }
            bvh.refit(&p, &t);
        }
    }

    #[test]
    fn triangle_matches_three_conventions() {
        let (a, b, c) = (DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
        let o = DVec3::new(0.2, 0.2, 1.0);
        // Front face (CCW seen from +z): hit at t = 1 with or without culling.
        assert_eq!(intersect_triangle(o, DVec3::NEG_Z, a, b, c, true), Some(1.0));
        // Seen from behind: culled when single-sided.
        let o2 = DVec3::new(0.2, 0.2, -1.0);
        assert_eq!(intersect_triangle(o2, DVec3::Z, a, b, c, true), None);
        assert_eq!(intersect_triangle(o2, DVec3::Z, a, b, c, false), Some(1.0));
    }
}
