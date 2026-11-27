//! A port of the earcut library three.js r180 vendors
//! (`extras/lib/earcut.js`, ISC, Mapbox): the same linked-list ear
//! clipping, z-order hashing above 80 points, hole bridging, and pass
//! structure, so triangulations equal three.js's index for index.

#[derive(Clone, Copy)]
struct Node {
    i: usize,
    x: f64,
    y: f64,
    prev: usize,
    next: usize,
    z: i32,
    prev_z: Option<usize>,
    next_z: Option<usize>,
    steiner: bool,
}

struct Ring {
    nodes: Vec<Node>,
}

impl Ring {
    fn create(&mut self, i: usize, x: f64, y: f64) -> usize {
        let n = self.nodes.len();
        self.nodes.push(Node {
            i,
            x,
            y,
            prev: n,
            next: n,
            z: 0,
            prev_z: None,
            next_z: None,
            steiner: false,
        });
        n
    }

    fn insert(&mut self, i: usize, x: f64, y: f64, last: Option<usize>) -> usize {
        let p = self.create(i, x, y);
        if let Some(last) = last {
            let ln = self.nodes[last].next;
            self.nodes[p].next = ln;
            self.nodes[p].prev = last;
            self.nodes[ln].prev = p;
            self.nodes[last].next = p;
        }
        p
    }

    fn remove(&mut self, p: usize) {
        let (prev, next) = (self.nodes[p].prev, self.nodes[p].next);
        self.nodes[next].prev = prev;
        self.nodes[prev].next = next;
        if let Some(pz) = self.nodes[p].prev_z {
            self.nodes[pz].next_z = self.nodes[p].next_z;
        }
        if let Some(nz) = self.nodes[p].next_z {
            self.nodes[nz].prev_z = self.nodes[p].prev_z;
        }
    }

    fn n(&self, p: usize) -> &Node {
        &self.nodes[p]
    }

    fn area(&self, p: usize, q: usize, r: usize) -> f64 {
        let (p, q, r) = (self.n(p), self.n(q), self.n(r));
        (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
    }

    fn equals(&self, a: usize, b: usize) -> bool {
        self.n(a).x == self.n(b).x && self.n(a).y == self.n(b).y
    }

    fn linked_list(
        &mut self,
        data: &[f64],
        start: usize,
        end: usize,
        clockwise: bool,
    ) -> Option<usize> {
        let mut last = None;
        if clockwise == (signed_area(data, start, end) > 0.0) {
            let mut i = start;
            while i < end {
                last = Some(self.insert(i / 2, data[i], data[i + 1], last));
                i += 2;
            }
        } else {
            let mut i = end as isize - 2;
            while i >= start as isize {
                let u = i as usize;
                last = Some(self.insert(u / 2, data[u], data[u + 1], last));
                i -= 2;
            }
        }
        if let Some(l) = last
            && self.equals(l, self.n(l).next)
        {
            self.remove(l);
            last = Some(self.n(l).next);
        }
        last
    }

    fn filter_points(&mut self, start: Option<usize>, end: Option<usize>) -> Option<usize> {
        let start = start?;
        let mut end = end.unwrap_or(start);
        let mut p = start;
        loop {
            let mut again = false;
            let (prev, next) = (self.n(p).prev, self.n(p).next);
            if !self.n(p).steiner && (self.equals(p, next) || self.area(prev, p, next) == 0.0) {
                self.remove(p);
                p = self.n(p).prev;
                end = p;
                if p == self.n(p).next {
                    break;
                }
                again = true;
            } else {
                p = next;
            }
            if !(again || p != end) {
                break;
            }
        }
        Some(end)
    }

    #[allow(clippy::too_many_arguments)]
    fn earcut_linked(
        &mut self,
        ear: Option<usize>,
        triangles: &mut Vec<usize>,
        min_x: f64,
        min_y: f64,
        inv_size: f64,
        pass: u8,
    ) {
        let Some(mut ear) = ear else {
            return;
        };
        if pass == 0 && inv_size != 0.0 {
            self.index_curve(ear, min_x, min_y, inv_size);
        }
        let mut stop = ear;
        while self.n(ear).prev != self.n(ear).next {
            let prev = self.n(ear).prev;
            let next = self.n(ear).next;
            let is_ear = if inv_size != 0.0 {
                self.is_ear_hashed(ear, min_x, min_y, inv_size)
            } else {
                self.is_ear(ear)
            };
            if is_ear {
                triangles.extend_from_slice(&[self.n(prev).i, self.n(ear).i, self.n(next).i]);
                self.remove(ear);
                ear = self.n(next).next;
                stop = self.n(next).next;
                continue;
            }
            ear = next;
            if ear == stop {
                match pass {
                    0 => {
                        let f = self.filter_points(Some(ear), None);
                        self.earcut_linked(f, triangles, min_x, min_y, inv_size, 1);
                    }
                    1 => {
                        let f = self.filter_points(Some(ear), None);
                        let e = self.cure_local_intersections(f, triangles);
                        self.earcut_linked(e, triangles, min_x, min_y, inv_size, 2);
                    }
                    _ => self.split_earcut(ear, triangles, min_x, min_y, inv_size),
                }
                break;
            }
        }
    }

    fn bbox(&self, a: usize, b: usize, c: usize) -> (f64, f64, f64, f64) {
        let (a, b, c) = (self.n(a), self.n(b), self.n(c));
        (
            a.x.min(b.x).min(c.x),
            a.y.min(b.y).min(c.y),
            a.x.max(b.x).max(c.x),
            a.y.max(b.y).max(c.y),
        )
    }

    fn inside(&self, a: usize, b: usize, c: usize, p: usize, bbox: (f64, f64, f64, f64)) -> bool {
        let (x0, y0, x1, y1) = bbox;
        let q = self.n(p);
        let (na, nb, nc) = (self.n(a), self.n(b), self.n(c));
        q.x >= x0
            && q.x <= x1
            && q.y >= y0
            && q.y <= y1
            && point_in_triangle_except_first(na.x, na.y, nb.x, nb.y, nc.x, nc.y, q.x, q.y)
            && self.area(q.prev, p, q.next) >= 0.0
    }

    fn is_ear(&self, ear: usize) -> bool {
        let (a, b, c) = (self.n(ear).prev, ear, self.n(ear).next);
        if self.area(a, b, c) >= 0.0 {
            return false;
        }
        let bbox = self.bbox(a, b, c);
        let mut p = self.n(c).next;
        while p != a {
            if self.inside(a, b, c, p, bbox) {
                return false;
            }
            p = self.n(p).next;
        }
        true
    }

    fn is_ear_hashed(&self, ear: usize, min_x: f64, min_y: f64, inv_size: f64) -> bool {
        let (a, b, c) = (self.n(ear).prev, ear, self.n(ear).next);
        if self.area(a, b, c) >= 0.0 {
            return false;
        }
        let bbox = self.bbox(a, b, c);
        let min_z = z_order(bbox.0, bbox.1, min_x, min_y, inv_size);
        let max_z = z_order(bbox.2, bbox.3, min_x, min_y, inv_size);
        let mut p = self.n(ear).prev_z;
        let mut n = self.n(ear).next_z;
        let check = |q: usize| q != a && q != c && self.inside(a, b, c, q, bbox);
        while let (Some(pp), Some(nn)) = (p, n) {
            if !(self.n(pp).z >= min_z && self.n(nn).z <= max_z) {
                break;
            }
            if check(pp) {
                return false;
            }
            p = self.n(pp).prev_z;
            if check(nn) {
                return false;
            }
            n = self.n(nn).next_z;
        }
        while let Some(pp) = p {
            if self.n(pp).z < min_z {
                break;
            }
            if check(pp) {
                return false;
            }
            p = self.n(pp).prev_z;
        }
        while let Some(nn) = n {
            if self.n(nn).z > max_z {
                break;
            }
            if check(nn) {
                return false;
            }
            n = self.n(nn).next_z;
        }
        true
    }

    fn cure_local_intersections(
        &mut self,
        start: Option<usize>,
        triangles: &mut Vec<usize>,
    ) -> Option<usize> {
        let mut start = start?;
        let mut p = start;
        loop {
            let a = self.n(p).prev;
            let b = self.n(self.n(p).next).next;
            let pn = self.n(p).next;
            if !self.equals(a, b)
                && self.intersects(a, p, pn, b)
                && self.locally_inside(a, b)
                && self.locally_inside(b, a)
            {
                triangles.extend_from_slice(&[self.n(a).i, self.n(p).i, self.n(b).i]);
                self.remove(p);
                let pn = self.n(p).next;
                self.remove(pn);
                p = b;
                start = b;
            }
            p = self.n(p).next;
            if p == start {
                break;
            }
        }
        self.filter_points(Some(p), None)
    }

    fn split_earcut(
        &mut self,
        start: usize,
        triangles: &mut Vec<usize>,
        min_x: f64,
        min_y: f64,
        inv_size: f64,
    ) {
        let mut a = start;
        loop {
            let mut b = self.n(self.n(a).next).next;
            while b != self.n(a).prev {
                if self.n(a).i != self.n(b).i && self.is_valid_diagonal(a, b) {
                    let c = self.split_polygon(a, b);
                    let an = self.n(a).next;
                    let a2 = self.filter_points(Some(a), Some(an));
                    let cn = self.n(c).next;
                    let c2 = self.filter_points(Some(c), Some(cn));
                    self.earcut_linked(a2, triangles, min_x, min_y, inv_size, 0);
                    self.earcut_linked(c2, triangles, min_x, min_y, inv_size, 0);
                    return;
                }
                b = self.n(b).next;
            }
            a = self.n(a).next;
            if a == start {
                break;
            }
        }
    }

    fn eliminate_holes(&mut self, data: &[f64], holes: &[usize], mut outer: usize) -> usize {
        let mut queue = Vec::new();
        for (i, &h) in holes.iter().enumerate() {
            let start = h * 2;
            let end = if i + 1 < holes.len() {
                holes[i + 1] * 2
            } else {
                data.len()
            };
            let Some(list) = self.linked_list(data, start, end, false) else {
                continue;
            };
            if list == self.n(list).next {
                self.nodes[list].steiner = true;
            }
            queue.push(self.leftmost(list));
        }
        // `Array.prototype.sort` with `compareXYSlope` (stable).
        queue.sort_by(|&a, &b| {
            let r = self.compare_xy_slope(a, b);
            r.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
        });
        for h in queue {
            outer = self.eliminate_hole(h, outer);
        }
        outer
    }

    fn compare_xy_slope(&self, a: usize, b: usize) -> f64 {
        let (na, nb) = (self.n(a), self.n(b));
        let mut result = na.x - nb.x;
        if result == 0.0 {
            result = na.y - nb.y;
            if result == 0.0 {
                let an = self.n(na.next);
                let bn = self.n(nb.next);
                let a_slope = (an.y - na.y) / (an.x - na.x);
                let b_slope = (bn.y - nb.y) / (bn.x - nb.x);
                result = a_slope - b_slope;
            }
        }
        result
    }

    fn eliminate_hole(&mut self, hole: usize, outer: usize) -> usize {
        let Some(bridge) = self.find_hole_bridge(hole, outer) else {
            return outer;
        };
        let reverse = self.split_polygon(bridge, hole);
        let rn = self.n(reverse).next;
        self.filter_points(Some(reverse), Some(rn));
        let bn = self.n(bridge).next;
        self.filter_points(Some(bridge), Some(bn)).unwrap_or(bridge)
    }

    fn find_hole_bridge(&self, hole: usize, outer: usize) -> Option<usize> {
        let mut p = outer;
        let (hx, hy) = (self.n(hole).x, self.n(hole).y);
        let mut qx = f64::NEG_INFINITY;
        let mut m: Option<usize> = None;
        if self.equals(hole, p) {
            return Some(p);
        }
        loop {
            let pn = self.n(p).next;
            if self.equals(hole, pn) {
                return Some(pn);
            } else if hy <= self.n(p).y && hy >= self.n(pn).y && self.n(pn).y != self.n(p).y {
                let (np, nn) = (self.n(p), self.n(pn));
                let x = np.x + (hy - np.y) * (nn.x - np.x) / (nn.y - np.y);
                if x <= hx && x > qx {
                    qx = x;
                    let mm = if np.x < nn.x { p } else { pn };
                    m = Some(mm);
                    if x == hx {
                        return Some(mm);
                    }
                }
            }
            p = pn;
            if p == outer {
                break;
            }
        }
        let mut m = m?;
        let stop = m;
        let (mx, my) = (self.n(m).x, self.n(m).y);
        let mut tan_min = f64::INFINITY;
        p = m;
        loop {
            let np = *self.n(p);
            if hx >= np.x
                && np.x >= mx
                && hx != np.x
                && point_in_triangle(
                    if hy < my { hx } else { qx },
                    hy,
                    mx,
                    my,
                    if hy < my { qx } else { hx },
                    hy,
                    np.x,
                    np.y,
                )
            {
                let tan = (hy - np.y).abs() / (hx - np.x);
                if self.locally_inside(p, hole)
                    && (tan < tan_min
                        || (tan == tan_min
                            && (np.x > self.n(m).x
                                || (np.x == self.n(m).x && self.sector_contains_sector(m, p)))))
                {
                    m = p;
                    tan_min = tan;
                }
            }
            p = np.next;
            if p == stop {
                break;
            }
        }
        Some(m)
    }

    fn sector_contains_sector(&self, m: usize, p: usize) -> bool {
        self.area(self.n(m).prev, m, self.n(p).prev) < 0.0
            && self.area(self.n(p).next, m, self.n(m).next) < 0.0
    }

    fn index_curve(&mut self, start: usize, min_x: f64, min_y: f64, inv_size: f64) {
        let mut p = start;
        loop {
            if self.nodes[p].z == 0 {
                self.nodes[p].z = z_order(self.nodes[p].x, self.nodes[p].y, min_x, min_y, inv_size);
            }
            self.nodes[p].prev_z = Some(self.nodes[p].prev);
            self.nodes[p].next_z = Some(self.nodes[p].next);
            p = self.nodes[p].next;
            if p == start {
                break;
            }
        }
        let pz = self.nodes[p].prev_z.expect("linked");
        self.nodes[pz].next_z = None;
        self.nodes[p].prev_z = None;
        self.sort_linked(p);
    }

    fn sort_linked(&mut self, list: usize) {
        let mut list = Some(list);
        let mut in_size = 1usize;
        loop {
            let mut p = list;
            list = None;
            let mut tail: Option<usize> = None;
            let mut merges = 0;
            while let Some(pp) = p {
                merges += 1;
                let mut q = Some(pp);
                let mut p_size = 0;
                for _ in 0..in_size {
                    p_size += 1;
                    q = q.and_then(|x| self.nodes[x].next_z);
                    if q.is_none() {
                        break;
                    }
                }
                let mut q_size = in_size;
                let mut p_cur = Some(pp);
                while p_size > 0 || (q_size > 0 && q.is_some()) {
                    let take_p = p_size != 0
                        && (q_size == 0
                            || q.is_none()
                            || self.nodes[p_cur.unwrap()].z <= self.nodes[q.unwrap()].z);
                    let e = if take_p {
                        let e = p_cur.unwrap();
                        p_cur = self.nodes[e].next_z;
                        p_size -= 1;
                        e
                    } else {
                        let e = q.unwrap();
                        q = self.nodes[e].next_z;
                        q_size -= 1;
                        e
                    };
                    match tail {
                        Some(t) => self.nodes[t].next_z = Some(e),
                        None => list = Some(e),
                    }
                    self.nodes[e].prev_z = tail;
                    tail = Some(e);
                }
                p = q;
            }
            if let Some(t) = tail {
                self.nodes[t].next_z = None;
            }
            in_size *= 2;
            if merges <= 1 {
                break;
            }
        }
    }

    fn leftmost(&self, start: usize) -> usize {
        let mut p = start;
        let mut left = start;
        loop {
            let (np, nl) = (self.n(p), self.n(left));
            if np.x < nl.x || (np.x == nl.x && np.y < nl.y) {
                left = p;
            }
            p = np.next;
            if p == start {
                break;
            }
        }
        left
    }

    fn is_valid_diagonal(&self, a: usize, b: usize) -> bool {
        let (na, nb) = (self.n(a), self.n(b));
        self.n(na.next).i != nb.i
            && self.n(na.prev).i != nb.i
            && !self.intersects_polygon(a, b)
            && ((self.locally_inside(a, b)
                && self.locally_inside(b, a)
                && self.middle_inside(a, b)
                && (truthy(self.area(na.prev, a, nb.prev)) || truthy(self.area(a, nb.prev, b))))
                || (self.equals(a, b)
                    && self.area(na.prev, a, na.next) > 0.0
                    && self.area(nb.prev, b, nb.next) > 0.0))
    }

    fn intersects(&self, p1: usize, q1: usize, p2: usize, q2: usize) -> bool {
        let o1 = sign(self.area(p1, q1, p2));
        let o2 = sign(self.area(p1, q1, q2));
        let o3 = sign(self.area(p2, q2, p1));
        let o4 = sign(self.area(p2, q2, q1));
        if o1 != o2 && o3 != o4 {
            return true;
        }
        (o1 == 0 && self.on_segment(p1, p2, q1))
            || (o2 == 0 && self.on_segment(p1, q2, q1))
            || (o3 == 0 && self.on_segment(p2, p1, q2))
            || (o4 == 0 && self.on_segment(p2, q1, q2))
    }

    fn on_segment(&self, p: usize, q: usize, r: usize) -> bool {
        let (p, q, r) = (self.n(p), self.n(q), self.n(r));
        q.x <= p.x.max(r.x) && q.x >= p.x.min(r.x) && q.y <= p.y.max(r.y) && q.y >= p.y.min(r.y)
    }

    fn intersects_polygon(&self, a: usize, b: usize) -> bool {
        let (ai, bi) = (self.n(a).i, self.n(b).i);
        let mut p = a;
        loop {
            let np = self.n(p);
            if np.i != ai
                && self.n(np.next).i != ai
                && np.i != bi
                && self.n(np.next).i != bi
                && self.intersects(p, np.next, a, b)
            {
                return true;
            }
            p = np.next;
            if p == a {
                break;
            }
        }
        false
    }

    fn locally_inside(&self, a: usize, b: usize) -> bool {
        let na = self.n(a);
        if self.area(na.prev, a, na.next) < 0.0 {
            self.area(a, b, na.next) >= 0.0 && self.area(a, na.prev, b) >= 0.0
        } else {
            self.area(a, b, na.prev) < 0.0 || self.area(a, na.next, b) < 0.0
        }
    }

    fn middle_inside(&self, a: usize, b: usize) -> bool {
        let mut p = a;
        let mut inside = false;
        let px = (self.n(a).x + self.n(b).x) / 2.0;
        let py = (self.n(a).y + self.n(b).y) / 2.0;
        loop {
            let np = self.n(p);
            let nn = self.n(np.next);
            if ((np.y > py) != (nn.y > py))
                && nn.y != np.y
                && (px < (nn.x - np.x) * (py - np.y) / (nn.y - np.y) + np.x)
            {
                inside = !inside;
            }
            p = np.next;
            if p == a {
                break;
            }
        }
        inside
    }

    fn split_polygon(&mut self, a: usize, b: usize) -> usize {
        let (ai, ax, ay) = (self.n(a).i, self.n(a).x, self.n(a).y);
        let (bi, bx, by) = (self.n(b).i, self.n(b).x, self.n(b).y);
        let a2 = self.create(ai, ax, ay);
        let b2 = self.create(bi, bx, by);
        let an = self.n(a).next;
        let bp = self.n(b).prev;
        self.nodes[a].next = b;
        self.nodes[b].prev = a;
        self.nodes[a2].next = an;
        self.nodes[an].prev = a2;
        self.nodes[b2].next = a2;
        self.nodes[a2].prev = b2;
        self.nodes[bp].next = b2;
        self.nodes[b2].prev = bp;
        b2
    }
}

/// `x | 0` (ToInt32) of a finite or non-finite number.
fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let t = x.trunc();
    let m = t.rem_euclid(4_294_967_296.0);
    (m as u64 as u32) as i32
}

fn z_order(x: f64, y: f64, min_x: f64, min_y: f64, inv_size: f64) -> i32 {
    let mut x = to_int32((x - min_x) * inv_size);
    let mut y = to_int32((y - min_y) * inv_size);
    x = (x | (x << 8)) & 0x00FF_00FF;
    x = (x | (x << 4)) & 0x0F0F_0F0F;
    x = (x | (x << 2)) & 0x3333_3333;
    x = (x | (x << 1)) & 0x5555_5555;
    y = (y | (y << 8)) & 0x00FF_00FF;
    y = (y | (y << 4)) & 0x0F0F_0F0F;
    y = (y | (y << 2)) & 0x3333_3333;
    y = (y | (y << 1)) & 0x5555_5555;
    x | (y << 1)
}

#[allow(clippy::too_many_arguments)]
fn point_in_triangle(
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    px: f64,
    py: f64,
) -> bool {
    (cx - px) * (ay - py) >= (ax - px) * (cy - py)
        && (ax - px) * (by - py) >= (bx - px) * (ay - py)
        && (bx - px) * (cy - py) >= (cx - px) * (by - py)
}

#[allow(clippy::too_many_arguments)]
fn point_in_triangle_except_first(
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    px: f64,
    py: f64,
) -> bool {
    !(ax == px && ay == py) && point_in_triangle(ax, ay, bx, by, cx, cy, px, py)
}

/// JavaScript truthiness of a number.
fn truthy(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

fn sign(v: f64) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

fn signed_area(data: &[f64], start: usize, end: usize) -> f64 {
    let mut sum = 0.0;
    if end < start + 2 {
        return sum;
    }
    let mut j = end - 2;
    let mut i = start;
    while i < end {
        sum += (data[j] - data[i]) * (data[i + 1] + data[j + 1]);
        j = i;
        i += 2;
    }
    sum
}

/// `earcut(data, holeIndices, 2)`: triangle vertex indices.
pub fn earcut(data: &[f64], holes: &[usize]) -> Vec<usize> {
    let has_holes = !holes.is_empty();
    let outer_len = if has_holes { holes[0] * 2 } else { data.len() };
    let mut ring = Ring { nodes: Vec::new() };
    let mut triangles = Vec::new();
    if outer_len < 2 {
        return triangles;
    }
    let Some(mut outer) = ring.linked_list(data, 0, outer_len, true) else {
        return triangles;
    };
    if ring.n(outer).next == ring.n(outer).prev {
        return triangles;
    }
    if has_holes {
        outer = ring.eliminate_holes(data, holes, outer);
    }
    let (mut min_x, mut min_y, mut inv_size) = (0.0, 0.0, 0.0);
    if data.len() > 80 * 2 {
        min_x = f64::INFINITY;
        min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut i = 2;
        while i < outer_len {
            let (x, y) = (data[i], data[i + 1]);
            if x < min_x {
                min_x = x;
            }
            if y < min_y {
                min_y = y;
            }
            if x > max_x {
                max_x = x;
            }
            if y > max_y {
                max_y = y;
            }
            i += 2;
        }
        inv_size = (max_x - min_x).max(max_y - min_y);
        inv_size = if inv_size != 0.0 {
            32767.0 / inv_size
        } else {
            0.0
        };
    }
    ring.earcut_linked(Some(outer), &mut triangles, min_x, min_y, inv_size, 0);
    triangles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_and_hole() {
        let sq = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        let t = earcut(&sq, &[]);
        assert_eq!(t.len(), 6);
        let with_hole = [
            0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0, 3.0, 3.0, 3.0, 7.0, 7.0, 7.0, 7.0, 3.0,
        ];
        let t = earcut(&with_hole, &[4]);
        assert_eq!(t.len(), 8 * 3);
    }

    #[test]
    fn int32_wraps_like_javascript() {
        assert_eq!(to_int32(3.9), 3);
        assert_eq!(to_int32(-3.9), -3);
        assert_eq!(to_int32(4_294_967_297.0), 1);
        assert_eq!(to_int32(f64::NAN), 0);
    }
}
