//! A bounding volume hierarchy built with the binned surface area heuristic.
//!
//! Nodes are 32 bytes, laid out as the shader reads them: a leaf has
//! `count > 0` and its triangles start at `left_first`; an interior node's
//! children are `left_first` and `left_first + 1`.

use crate::math::Vec3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Node {
    pub min: [f32; 3],
    pub left_first: u32,
    pub max: [f32; 3],
    pub count: u32,
}

#[derive(Clone, Copy)]
struct Aabb {
    lo: Vec3,
    hi: Vec3,
}

impl Aabb {
    const EMPTY: Aabb = Aabb { lo: Vec3 { x: f32::MAX, y: f32::MAX, z: f32::MAX }, hi: Vec3 { x: f32::MIN, y: f32::MIN, z: f32::MIN } };
    fn grow(&mut self, p: Vec3) {
        self.lo = self.lo.min(p);
        self.hi = self.hi.max(p);
    }
    fn merge(&mut self, o: &Aabb) {
        self.lo = self.lo.min(o.lo);
        self.hi = self.hi.max(o.hi);
    }
    fn area(&self) -> f32 {
        let e = self.hi - self.lo;
        if e.x < 0.0 { 0.0 } else { e.x * e.y + e.y * e.z + e.z * e.x }
    }
}

/// Builds the tree over `tris` (three corners each). Returns the nodes and the
/// triangle order the leaves refer to.
#[allow(clippy::needless_range_loop)] // axis indexes Vec3 components
pub fn build(tris: &[[Vec3; 3]]) -> (Vec<Node>, Vec<u32>) {
    let bounds: Vec<Aabb> = tris
        .iter()
        .map(|t| {
            let mut b = Aabb::EMPTY;
            t.iter().for_each(|&p| b.grow(p));
            b
        })
        .collect();
    let centroids: Vec<Vec3> = bounds.iter().map(|b| (b.lo + b.hi) * 0.5).collect();
    let mut order: Vec<u32> = (0..tris.len() as u32).collect();
    let mut nodes = vec![Node::default(); 1];
    nodes.reserve(tris.len() * 2);
    let mut stack = vec![(0usize, 0usize, tris.len())];
    // Reserve slot 1 so children always come in pairs at even offsets.
    nodes.push(Node::default());
    while let Some((index, first, count)) = stack.pop() {
        let mut b = Aabb::EMPTY;
        let mut cb = Aabb::EMPTY;
        for &t in &order[first..first + count] {
            b.merge(&bounds[t as usize]);
            cb.grow(centroids[t as usize]);
        }
        let leaf = |nodes: &mut Vec<Node>| {
            nodes[index] = Node { min: [b.lo.x, b.lo.y, b.lo.z], left_first: first as u32, max: [b.hi.x, b.hi.y, b.hi.z], count: count as u32 };
        };
        if count <= 2 {
            leaf(&mut nodes);
            continue;
        }
        // Binned SAH over the centroid bounds, 16 bins per axis.
        const BINS: usize = 16;
        let mut best = (f32::MAX, 0usize, 0usize);
        for axis in 0..3 {
            let (lo, hi) = (cb.lo[axis], cb.hi[axis]);
            if hi - lo < 1e-6 {
                continue;
            }
            let scale = BINS as f32 / (hi - lo);
            let mut bins = [(Aabb::EMPTY, 0usize); BINS];
            for &t in &order[first..first + count] {
                let i = (((centroids[t as usize][axis] - lo) * scale) as usize).min(BINS - 1);
                bins[i].0.merge(&bounds[t as usize]);
                bins[i].1 += 1;
            }
            let (mut left_area, mut left_count) = ([0.0f32; BINS], [0usize; BINS]);
            let (mut acc, mut n) = (Aabb::EMPTY, 0);
            for i in 0..BINS - 1 {
                acc.merge(&bins[i].0);
                n += bins[i].1;
                left_area[i] = acc.area();
                left_count[i] = n;
            }
            let (mut acc, mut n) = (Aabb::EMPTY, 0);
            for i in (1..BINS).rev() {
                acc.merge(&bins[i].0);
                n += bins[i].1;
                let cost = left_area[i - 1] * left_count[i - 1] as f32 + acc.area() * n as f32;
                if left_count[i - 1] > 0 && n > 0 && cost < best.0 {
                    best = (cost, axis, i);
                }
            }
        }
        let leaf_cost = b.area() * count as f32;
        if best.0 >= leaf_cost && count <= 8 {
            leaf(&mut nodes);
            continue;
        }
        let (axis, split) = (best.1, best.2);
        let (lo, hi) = (cb.lo[axis], cb.hi[axis]);
        let mut mid = if best.0 < f32::MAX {
            let scale = 16.0 / (hi - lo);
            let slice = &mut order[first..first + count];
            let mut i = 0;
            for j in 0..slice.len() {
                let bin = (((centroids[slice[j] as usize][axis] - lo) * scale) as usize).min(15);
                if bin < split {
                    slice.swap(i, j);
                    i += 1;
                }
            }
            first + i
        } else {
            first + count / 2
        };
        if mid == first || mid == first + count {
            mid = first + count / 2;
        }
        let left = nodes.len();
        nodes.push(Node::default());
        nodes.push(Node::default());
        nodes[index] = Node { min: [b.lo.x, b.lo.y, b.lo.z], left_first: left as u32, max: [b.hi.x, b.hi.y, b.hi.z], count: 0 };
        stack.push((left, first, mid - first));
        stack.push((left + 1, mid, first + count - mid));
    }
    (nodes, order)
}

/// A node that stores both children's bounds, so traversal reads one node per
/// visit and tests two boxes. A child with `count > 0` is a leaf whose
/// triangles start at its index; otherwise its index is another wide node.
/// An absent child has `count == u32::MAX` and an empty box.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WideNode {
    pub lmin: [f32; 3],
    pub left: u32,
    pub lmax: [f32; 3],
    pub lcount: u32,
    pub rmin: [f32; 3],
    pub right: u32,
    pub rmax: [f32; 3],
    pub rcount: u32,
}

pub fn widen(nodes: &[Node]) -> Vec<WideNode> {
    let root = nodes[0];
    let empty = ([f32::MAX; 3], [f32::MIN; 3]);
    if root.count > 0 {
        return vec![WideNode { lmin: root.min, left: root.left_first, lmax: root.max, lcount: root.count, rmin: empty.0, right: 0, rmax: empty.1, rcount: u32::MAX }];
    }
    let mut out: Vec<WideNode> = Vec::with_capacity(nodes.len() / 2);
    // Map interior node index → wide index, breadth-first so parents come first.
    let mut queue = std::collections::VecDeque::from([0usize]);
    let mut index = std::collections::HashMap::new();
    index.insert(0usize, 0u32);
    out.push(WideNode::default());
    while let Some(i) = queue.pop_front() {
        let n = nodes[i];
        let child = |c: usize, out: &mut Vec<WideNode>, queue: &mut std::collections::VecDeque<usize>, index: &mut std::collections::HashMap<usize, u32>| -> (u32, u32) {
            let cn = nodes[c];
            if cn.count > 0 {
                (cn.left_first, cn.count)
            } else {
                let w = out.len() as u32;
                out.push(WideNode::default());
                index.insert(c, w);
                queue.push_back(c);
                (w, 0)
            }
        };
        let (l, r) = (n.left_first as usize, n.left_first as usize + 1);
        let (li, lc) = child(l, &mut out, &mut queue, &mut index);
        let (ri, rc) = child(r, &mut out, &mut queue, &mut index);
        let me = index[&i] as usize;
        out[me] = WideNode { lmin: nodes[l].min, left: li, lmax: nodes[l].max, lcount: lc, rmin: nodes[r].min, right: ri, rmax: nodes[r].max, rcount: rc };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    fn soup(n: usize) -> Vec<[Vec3; 3]> {
        // A deterministic scatter of small triangles.
        let mut s = 0x9e37_79b9u32;
        let mut r = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s as f32 / u32::MAX as f32
        };
        (0..n)
            .map(|_| {
                let c = v3(r() * 200.0, r() * 50.0, r() * 120.0);
                [c, c + v3(r() * 4.0, r(), 0.5), c + v3(0.3, r() * 4.0, r() * 3.0)]
            })
            .collect()
    }

    fn contains(lo: [f32; 3], hi: [f32; 3], p: Vec3) -> bool {
        (0..3).all(|i| p[i] >= lo[i] - 1e-4 && p[i] <= hi[i] + 1e-4)
    }

    #[test]
    fn every_triangle_is_in_exactly_one_leaf_inside_its_bounds() {
        let tris = soup(5000);
        let (nodes, order) = build(&tris);
        let mut seen = vec![0u32; tris.len()];
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let n = nodes[i];
            if n.count > 0 {
                for k in n.left_first..n.left_first + n.count {
                    let t = order[k as usize] as usize;
                    seen[t] += 1;
                    assert!(tris[t].iter().all(|&p| contains(n.min, n.max, p)));
                }
            } else {
                for c in [n.left_first as usize, n.left_first as usize + 1] {
                    let ch = nodes[c];
                    assert!(contains(n.min, n.max, v3(ch.min[0], ch.min[1], ch.min[2])));
                    assert!(contains(n.min, n.max, v3(ch.max[0], ch.max[1], ch.max[2])));
                    stack.push(c);
                }
            }
        }
        assert!(seen.iter().all(|&c| c == 1));
    }

    #[test]
    fn wide_nodes_cover_the_same_leaves() {
        let tris = soup(3000);
        let (nodes, _) = build(&tris);
        let wide = widen(&nodes);
        let mut leaf_tris = 0u32;
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let n = wide[i];
            for (child, count) in [(n.left, n.lcount), (n.right, n.rcount)] {
                match count {
                    u32::MAX => {}
                    0 => stack.push(child as usize),
                    c => leaf_tris += c,
                }
            }
        }
        assert_eq!(leaf_tris as usize, tris.len());
    }

    #[test]
    fn a_single_leaf_scene_still_traverses() {
        let tris = soup(2);
        let (nodes, _) = build(&tris);
        let wide = widen(&nodes);
        assert_eq!(wide.len(), 1);
        assert_eq!(wide[0].lcount, 2);
        assert_eq!(wide[0].rcount, u32::MAX);
    }
}
