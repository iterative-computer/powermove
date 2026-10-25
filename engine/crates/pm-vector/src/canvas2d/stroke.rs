//! Graphite's hardware stroke tessellation (Skia m153
//! `TessellateStrokesRenderStep::writeVertices`, the stroking `PatchWriter`
//! with `TrackJoinControlPoints` and `ReplicateLineEndPoints`, and the
//! `tessellate_stroked_curve` vertex function), reproduced on the CPU in
//! `f32`: every stroke becomes the triangle strips the GPU rasterizes. The
//! depth test lets each sample be covered once per draw, so a stroke's
//! coverage is the union of its triangles.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::skpath::{Mat, P, SkPath, Verb};
use super::tess::{PRECISION, Vx, conic_p2, cubic_p4};

/// The join encoding of `skgpu::tess::GetJoinType`: negative round, zero
/// bevel, positive the miter limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeParams {
    pub radius: f32,
    pub join: f32,
}

/// `SkPaint::Cap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Butt,
}

/// A stroke patch as written to the instance buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokePatch {
    /// For conics: `p[3] = (w, inf)`.
    pub p: [P; 4],
    pub conic: bool,
    /// The join control point (`prevPoint`).
    pub join: P,
}

const MAX_PARAMETRIC_SEGMENTS_P4: f32 = 32.0 * 32.0 * 32.0 * 32.0;
const MAX_SEGMENTS_PER_CURVE_P4: f32 = 1024.0 * 1024.0 * 1024.0 * 1024.0;

/// The stroking PatchWriter.
struct Writer {
    vx: Vx,
    out: Vec<StrokePatch>,
    join: P,
    // Deferred patch state (PatchStorage).
    first: P,
    last: P,
    curve_type: f32,
    n_p4: f32,
    deferred: Option<StrokePatch>,
    tolerance_n4: f32,
    max_tolerance_n4: f32,
}

fn tangent_point(p0: P, p1: P, p2: P, p3: P) -> P {
    if p0 != p1 {
        p1
    } else if p2 != p1 {
        p2
    } else {
        p3
    }
}

fn mix4(a: [f32; 4], b: [f32; 4], t: [f32; 4]) -> [f32; 4] {
    [
        (b[0] - a[0]) * t[0] + a[0],
        (b[1] - a[1]) * t[1] + a[1],
        (b[2] - a[2]) * t[2] + a[2],
        (b[3] - a[3]) * t[3] + a[3],
    ]
}

impl Writer {
    fn new(vx: Vx) -> Self {
        Self {
            vx,
            out: Vec::new(),
            join: [0.0, 0.0],
            first: [0.0, 0.0],
            last: [0.0, 0.0],
            curve_type: -1.0,
            n_p4: -1.0,
            deferred: None,
            tolerance_n4: 1.0,
            max_tolerance_n4: 1.0,
        }
    }

    fn must_defer(&self) -> bool {
        self.n_p4 < 0.0
    }

    fn has_verb(&self) -> bool {
        self.curve_type >= 0.0
    }

    fn has_pending(&self) -> bool {
        self.n_p4 > 0.0
    }

    fn reset(&mut self, move_to: P) {
        self.curve_type = -1.0;
        self.n_p4 = -1.0;
        self.first = move_to;
        self.last = move_to;
        self.deferred = None;
    }

    /// `writePatch`.
    fn write_patch(&mut self, p0: P, p1: P, p2: P, p3: P, conic: bool) {
        self.curve_type = if conic { 1.0 } else { 0.0 };
        let last = if conic { p2 } else { p3 };
        if p0 == p1 && p1 == p2 && p2 == last {
            return;
        }
        let patch = StrokePatch {
            p: [p0, p1, p2, p3],
            conic,
            join: self.join,
        };
        if self.must_defer() {
            self.n_p4 = self.tolerance_n4;
            self.deferred = Some(patch);
        } else {
            self.max_tolerance_n4 = self.max_tolerance_n4.max(self.tolerance_n4);
            self.out.push(patch);
        }
        self.last = last;
        self.join = tangent_point(last, p2, p1, p0);
    }

    fn write_cubic_patch(&mut self, p: [P; 4]) {
        self.write_patch(p[0], p[1], p[2], p[3], false);
    }

    fn write_conic_patch(&mut self, p0: P, p1: P, p2: P, w: f32) {
        self.write_patch(p0, p1, p2, [w, f32::INFINITY], true);
    }

    /// `accountForCurve`.
    fn account(&mut self, n4: f32) -> i32 {
        if n4 <= MAX_PARAMETRIC_SEGMENTS_P4 {
            self.tolerance_n4 = n4.max(1.0);
            0
        } else {
            self.tolerance_n4 = MAX_PARAMETRIC_SEGMENTS_P4;
            libm::ceilf(
                (n4.min(MAX_SEGMENTS_PER_CURVE_P4) / MAX_PARAMETRIC_SEGMENTS_P4)
                    .sqrt()
                    .sqrt(),
            ) as i32
        }
    }

    fn write_line(&mut self, a: P, b: P) {
        self.tolerance_n4 = 1.0;
        self.write_cubic_patch([a, a, b, b]);
    }

    fn write_circle(&mut self, p: P) {
        self.tolerance_n4 = 1.0;
        self.out.push(StrokePatch {
            p: [p; 4],
            conic: false,
            join: p,
        });
    }

    fn write_cubic(&mut self, p: [P; 4]) {
        let n = self.account(cubic_p4(&p, &self.vx));
        if n == 0 {
            self.write_cubic_patch(p);
            return;
        }
        let mut p = p;
        let mut n = n;
        while n >= 3 {
            let t = [
                1.0 / n as f32,
                1.0 / n as f32,
                2.0 / n as f32,
                2.0 / n as f32,
            ];
            let xy = |q: P| [q[0], q[1], q[0], q[1]];
            let ab = mix4(xy(p[0]), xy(p[1]), t);
            let bc = mix4(xy(p[1]), xy(p[2]), t);
            let cd = mix4(xy(p[2]), xy(p[3]), t);
            let abc = mix4(ab, bc, t);
            let bcd = mix4(bc, cd, t);
            let abcd = mix4(abc, bcd, t);
            let middle = mix4(abc, bcd, [t[2], t[3], t[0], t[1]]);
            let lo = |v: [f32; 4]| [v[0], v[1]];
            let hi = |v: [f32; 4]| [v[2], v[3]];
            self.write_cubic_patch([p[0], lo(ab), lo(abc), lo(abcd)]);
            self.write_cubic_patch([lo(abcd), lo(middle), hi(middle), hi(abcd)]);
            p = [hi(abcd), hi(bcd), hi(cd), p[3]];
            n -= 2;
        }
        if n == 2 {
            let h = |a: P, b: P| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let ab = h(p[0], p[1]);
            let bc = h(p[1], p[2]);
            let cd = h(p[2], p[3]);
            let abc = h(ab, bc);
            let bcd = h(bc, cd);
            let abcd = h(abc, bcd);
            self.write_cubic_patch([p[0], ab, abc, abcd]);
            self.write_cubic_patch([abcd, bcd, cd, p[3]]);
        } else {
            self.write_cubic_patch(p);
        }
    }

    fn write_quad(&mut self, p0: P, p1: P, p2: P) {
        let v = self.vx_apply([-2.0 * p1[0] + p0[0] + p2[0], -2.0 * p1[1] + p0[1] + p2[1]]);
        let n4 = (v[0] * v[0] + v[1] * v[1]) * ((4.0 / 64.0) * (PRECISION * PRECISION));
        let n = self.account(n4);
        let as_cubic = |p0: P, p1: P, p2: P| -> [P; 4] {
            let c1 = [
                (p1[0] - p0[0]) * (2.0 / 3.0) + p0[0],
                (p1[1] - p0[1]) * (2.0 / 3.0) + p0[1],
            ];
            let c2 = [
                (p1[0] - p2[0]) * (2.0 / 3.0) + p2[0],
                (p1[1] - p2[1]) * (2.0 / 3.0) + p2[1],
            ];
            [p0, c1, c2, p2]
        };
        if n == 0 {
            self.write_cubic_patch(as_cubic(p0, p1, p2));
            return;
        }
        let (mut p0, mut p1) = (p0, p1);
        let mut n = n;
        while n >= 3 {
            let t = [
                1.0 / n as f32,
                1.0 / n as f32,
                2.0 / n as f32,
                2.0 / n as f32,
            ];
            let xy = |q: P| [q[0], q[1], q[0], q[1]];
            let ab = mix4(xy(p0), xy(p1), t);
            let bc = mix4(xy(p1), xy(p2), t);
            let abc = mix4(ab, bc, t);
            let tm = mix4(t, [t[2], t[3], t[0], t[1]], [2.0 / 3.0; 4]);
            let middle = mix4(ab, bc, tm);
            let lo = |v: [f32; 4]| [v[0], v[1]];
            let hi = |v: [f32; 4]| [v[2], v[3]];
            self.write_cubic_patch(as_cubic(p0, lo(ab), lo(abc)));
            self.write_cubic_patch([lo(abc), lo(middle), hi(middle), hi(abc)]);
            p0 = hi(abc);
            p1 = hi(bc);
            n -= 2;
        }
        if n == 2 {
            let h = |a: P, b: P| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let ab = h(p0, p1);
            let bc = h(p1, p2);
            let abc = h(ab, bc);
            self.write_cubic_patch(as_cubic(p0, ab, abc));
            self.write_cubic_patch(as_cubic(abc, bc, p2));
        } else {
            self.write_cubic_patch(as_cubic(p0, p1, p2));
        }
    }

    fn vx_apply(&self, v: P) -> P {
        // VectorXform: c0 * x + c1 * y.
        let m = &self.vx;
        let (c0, c1) = m.columns();
        [c0[0] * v[0] + c1[0] * v[1], c0[1] * v[0] + c1[1] * v[1]]
    }

    fn write_conic(&mut self, p0: P, p1: P, p2: P, w: f32) {
        let n2 = conic_p2(&[p0, p1, p2], w, &self.vx);
        let n = self.account(n2 * n2);
        if n == 0 {
            self.write_conic_patch(p0, p1, p2, w);
            return;
        }
        let mut h0 = [p0[0], p0[1], 1.0, 1.0];
        let mut h1 = [p1[0] * w, p1[1] * w, w, w];
        let h2 = [p2[0], p2[1], 1.0, 1.0];
        let mut n = n;
        while n >= 2 {
            let t = [1.0 / n as f32; 4];
            let ab = mix4(h0, h1, t);
            let bc = mix4(h1, h2, t);
            let abc = mix4(ab, bc, t);
            let mid = [abc[0] / abc[3], abc[1] / abc[3]];
            self.write_conic_patch(
                [h0[0] / h0[3], h0[1] / h0[3]],
                [ab[0] / ab[3], ab[1] / ab[3]],
                mid,
                ab[3] / (h0[3] * abc[3]).sqrt(),
            );
            h0 = abc;
            h1 = bc;
            n -= 1;
        }
        self.write_conic_patch(
            [h0[0] / h0[3], h0[1] / h0[3]],
            [h1[0] / h1[3], h1[1] / h1[3]],
            [h2[0], h2[1]],
            h1[3] / h0[3].sqrt(),
        );
    }

    /// `writeDeferredStrokePatch(moveTo, cap)`.
    fn write_deferred(&mut self, move_to: P, cap: Option<Cap>) {
        if self.has_pending() {
            let mut patch = self.deferred.take().expect("pending patch");
            let join = match cap {
                None => self.join,
                Some(cap) => {
                    let first = self.first;
                    let Cap::Butt = cap;
                    first
                }
            };
            patch.join = join;
            self.max_tolerance_n4 = self.max_tolerance_n4.max(self.n_p4);
            self.out.push(patch);
        } else if let (Some(cap), true) = (cap, self.has_verb()) {
            let Cap::Butt = cap;
        }
        self.reset(move_to);
    }

    /// `closeDeferredStrokePatch(cap)`.
    fn close_deferred(&mut self, cap: Cap) {
        self.write_line(self.last, self.first);
        let first = self.first;
        if self.has_pending() {
            self.write_deferred(first, None);
        } else {
            self.write_deferred(first, Some(cap));
        }
    }
}

/// `ConicHasCusp`.
fn conic_has_cusp(p: &[P]) -> bool {
    let a = [p[1][0] - p[0][0], p[1][1] - p[0][1]];
    let b = [p[2][0] - p[1][0], p[2][1] - p[1][1]];
    a[0] * b[1] - a[1] * b[0] == 0.0 && a[0] * b[0] + a[1] * b[1] < 0.0
}

/// `FindCubicConvex180Chops`.
pub fn find_cubic_convex_180_chops(p: &[P]) -> (Vec<f32>, bool) {
    const EPS: f32 = 1.0 / 2048.0;
    let one_minus_2eps: u32 = (127 << 23) - 2 * (1 << (24 - 11));
    let inside = |root: f32| (root - EPS).to_bits() < one_minus_2eps;
    let sub = |a: P, b: P| [a[0] - b[0], a[1] - b[1]];
    let cross = |a: P, b: P| a[0] * b[1] - a[1] * b[0];
    let c = sub(p[1], p[0]);
    let d = sub(p[2], p[1]);
    let e = sub(p[3], p[0]);
    let b = sub(d, c);
    let a = [-3.0 * d[0] + e[0], -3.0 * d[1] + e[1]];
    let mut qa = cross(a, b);
    let qb = cross(a, c);
    let mut qc = cross(b, c);
    let mut b_over_minus_2 = -0.5 * qb;
    let mut discr = b_over_minus_2 * b_over_minus_2 - qa * qc;
    let mut cusp_threshold = qa * (EPS / 2.0);
    cusp_threshold *= cusp_threshold;
    if discr < -cusp_threshold {
        let root = qc / b_over_minus_2;
        return if inside(root) {
            (vec![root], false)
        } else {
            (vec![], false)
        };
    }
    let are_cusps = discr <= cusp_threshold;
    if are_cusps {
        if qa != 0.0 || b_over_minus_2 != 0.0 || qc != 0.0 {
            let root = b_over_minus_2 / qa;
            return if inside(root) {
                (vec![root], true)
            } else {
                (vec![], true)
            };
        }
        let tan0 = [
            if c[0] != 0.0 { c[0] } else { p[2][0] - p[0][0] },
            if c[1] != 0.0 { c[1] } else { p[2][1] - p[0][1] },
        ];
        let dot = |u: P, v: P| u[0] * v[0] + u[1] * v[1];
        qa = dot(tan0, a);
        b_over_minus_2 = -dot(tan0, b);
        qc = dot(tan0, c);
        discr = b_over_minus_2 * b_over_minus_2 - qa * qc;
        if discr < -cusp_threshold {
            return (vec![], are_cusps);
        }
        discr = discr.max(0.0);
    }
    let mut q = discr.sqrt();
    q = q.copysign(b_over_minus_2);
    q += b_over_minus_2;
    let roots = [q / qa, qc / q];
    let ins = |r: f32| r > EPS && r < 1.0 - EPS;
    if ins(roots[0]) {
        if ins(roots[1]) && roots[0] != roots[1] {
            let (lo, hi) = if roots[0] > roots[1] {
                (roots[1], roots[0])
            } else {
                (roots[0], roots[1])
            };
            return (vec![lo, hi], are_cusps);
        }
        return (vec![roots[0]], are_cusps);
    }
    if ins(roots[1]) {
        return (vec![roots[1]], are_cusps);
    }
    (vec![], are_cusps)
}

/// `SkChopCubicAt(src, dst, t)` (one chop).
fn chop_cubic_at(p: &[P], t: f32) -> [P; 7] {
    let lerp = |a: P, b: P| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let ab = lerp(p[0], p[1]);
    let bc = lerp(p[1], p[2]);
    let cd = lerp(p[2], p[3]);
    let abc = lerp(ab, bc);
    let bcd = lerp(bc, cd);
    let abcd = lerp(abc, bcd);
    [p[0], ab, abc, abcd, bcd, cd, p[3]]
}

/// `SkChopCubicAt(src, dst, t0, t1)`.
fn chop_cubic_at2(p: &[P], t0: f32, t1: f32) -> [P; 10] {
    let first = chop_cubic_at(p, t0);
    // The second chop is at t1 within the remainder: (t1 - t0) / (1 - t0).
    let t = (t1 - t0) / (1.0 - t0);
    let second = chop_cubic_at(&first[3..7], t);
    [
        first[0], first[1], first[2], first[3], second[1], second[2], second[3], second[4],
        second[5], second[6],
    ]
}

/// The stroke patches `TessellateStrokesRenderStep::writeVertices` writes.
pub fn stroke_patches(path: &SkPath, m: &Mat, cap: Cap) -> Vec<StrokePatch> {
    stroke_patches_with_tolerance(path, m, cap).0
}

/// Retain the allocator's pre-chop tolerance, including deferred patches.
/// Recomputing it from the emitted cubics changes quad/line edge counts.
pub(super) fn stroke_patches_with_tolerance(
    path: &SkPath,
    m: &Mat,
    cap: Cap,
) -> (Vec<StrokePatch>, f32) {
    let mut w = Writer::new(Vx::of(m));
    for (verb, pts) in path.segments() {
        match verb {
            Verb::Move => w.write_deferred(pts[0], Some(cap)),
            Verb::Close => w.close_deferred(cap),
            Verb::Line => w.write_line(pts[0], pts[1]),
            Verb::Quad => {
                if conic_has_cusp(&pts) {
                    // A flat quad with a 180-degree turnaround; not reached by
                    // the legacy geometry (no quads), handled for completeness.
                    let t = quad_mid_tangent(&pts);
                    let cusp = eval_quad(&pts, t);
                    w.write_circle(cusp);
                    w.write_line(pts[0], cusp);
                    w.write_line(cusp, pts[2]);
                } else {
                    w.write_quad(pts[0], pts[1], pts[2]);
                }
            }
            Verb::Conic(weight) => {
                if conic_has_cusp(&pts) {
                    let cusp = eval_conic(&pts, weight, 0.5);
                    w.write_circle(cusp);
                    w.write_line(pts[0], cusp);
                    w.write_line(cusp, pts[2]);
                } else {
                    w.write_conic(pts[0], pts[1], pts[2], weight);
                }
            }
            Verb::Cubic => {
                let (t, cusps) = find_cubic_convex_180_chops(&pts);
                match t.len() {
                    0 => w.write_cubic([pts[0], pts[1], pts[2], pts[3]]),
                    1 => {
                        let mut c = chop_cubic_at(&pts, t[0]);
                        if cusps {
                            w.write_circle(c[3]);
                            c[2] = c[3];
                            c[4] = c[3];
                        }
                        w.write_cubic([c[0], c[1], c[2], c[3]]);
                        w.write_cubic([c[3], c[4], c[5], c[6]]);
                    }
                    _ => {
                        let c = chop_cubic_at2(&pts, t[0], t[1]);
                        if cusps {
                            w.write_circle(c[3]);
                            w.write_circle(c[6]);
                            w.write_line(c[0], c[3]);
                            w.write_line(c[3], c[6]);
                            w.write_line(c[6], c[9]);
                        } else {
                            w.write_cubic([c[0], c[1], c[2], c[3]]);
                            w.write_cubic([c[3], c[4], c[5], c[6]]);
                            w.write_cubic([c[6], c[7], c[8], c[9]]);
                        }
                    }
                }
            }
        }
    }
    w.write_deferred([0.0, 0.0], Some(cap));
    (w.out, w.max_tolerance_n4)
}

fn eval_quad(p: &[P], t: f32) -> P {
    let lerp = |a: P, b: P| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    lerp(lerp(p[0], p[1]), lerp(p[1], p[2]))
}

fn quad_mid_tangent(p: &[P]) -> f32 {
    // SkFindQuadMidTangent for the flat cusp case: the turnaround is where
    // the tangent reverses, at the parameter of the extreme point.
    let a = [
        p[0][0] - 2.0 * p[1][0] + p[2][0],
        p[0][1] - 2.0 * p[1][1] + p[2][1],
    ];
    let b = [p[1][0] - p[0][0], p[1][1] - p[0][1]];
    let den = a[0] * a[0] + a[1] * a[1];
    if den == 0.0 {
        0.5
    } else {
        (-(a[0] * b[0] + a[1] * b[1]) / den).clamp(0.0, 1.0)
    }
}

fn eval_conic(p: &[P], w: f32, t: f32) -> P {
    let num = |i: usize| {
        let a = p[0][i];
        let b = p[1][i] * w;
        let c = p[2][i];
        (1.0 - t) * (1.0 - t) * a + 2.0 * t * (1.0 - t) * b + t * t * c
    };
    let den = (1.0 - t) * (1.0 - t) + 2.0 * t * (1.0 - t) * w + t * t;
    [num(0) / den, num(1) / den]
}

#[inline]
fn robust_normalize_diff(a: P, b: P) -> P {
    let diff = [a[0] - b[0], a[1] - b[1]];
    if diff == [0.0, 0.0] {
        return [0.0, 0.0];
    }
    let inv = 1.0 / diff[0].abs().max(diff[1].abs());
    let v = [inv * diff[0], inv * diff[1]];
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    [v[0] / len, v[1] / len]
}

#[inline]
fn cosine_between(a: P, b: P) -> f32 {
    (a[0] * b[0] + a[1] * b[1]).clamp(-1.0, 1.0)
}

#[inline]
fn miter_extent(cos_theta: f32, miter_limit: f32) -> f32 {
    let x = cos_theta.mul_add(0.5, 0.5);
    if x * miter_limit * miter_limit >= 1.0 {
        1.0 / x.sqrt()
    } else {
        x.sqrt()
    }
}

#[inline]
fn num_radial_segments_per_radian(radius: f32) -> f32 {
    0.5 / libm::acosf((1.0 - (1.0 / PRECISION) / radius).max(-1.0))
}

#[inline]
fn cross(a: P, b: P) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

#[inline]
fn umix(a: f32, b: f32, t: f32) -> f32 {
    (b - a).mul_add(t, a)
}

#[inline]
fn umix2(a: P, b: P, t: f32) -> P {
    [umix(a[0], b[0], t), umix(a[1], b[1], t)]
}

/// The GPU's Wang's formula for a cubic (`wangs_formula_cubic`, not log2).
fn gpu_wang_cubic(p: &[P; 4], m: &[f32; 4]) -> f32 {
    let mv = |v: P| [m[0] * v[0] + m[2] * v[1], m[1] * v[0] + m[3] * v[1]];
    let d0 = mv([
        (-2.0f32).mul_add(p[1][0], p[2][0]) + p[0][0],
        (-2.0f32).mul_add(p[1][1], p[2][1]) + p[0][1],
    ]);
    let d1 = mv([
        (-2.0f32).mul_add(p[2][0], p[3][0]) + p[1][0],
        (-2.0f32).mul_add(p[2][1], p[3][1]) + p[1][1],
    ]);
    let mm = (d0[0] * d0[0] + d0[1] * d0[1]).max(d1[0] * d1[0] + d1[1] * d1[1]);
    let k = (3.0 * 2.0 / 8.0) * PRECISION;
    libm::ceilf((k * mm.sqrt()).sqrt()).max(1.0)
}

/// The GPU's Wang's formula for a conic (`wangs_formula_conic`).
fn gpu_wang_conic(p0: P, p1: P, p2: P, w: f32) -> f32 {
    let mut q = [p0, p1, p2];
    let c = [
        (q[0][0].min(q[1][0]).min(q[2][0]) + q[0][0].max(q[1][0]).max(q[2][0])) * 0.5,
        (q[0][1].min(q[1][1]).min(q[2][1]) + q[0][1].max(q[1][1]).max(q[2][1])) * 0.5,
    ];
    for v in &mut q {
        v[0] -= c[0];
        v[1] -= c[1];
    }
    let dot = |a: P| a[0] * a[0] + a[1] * a[1];
    let m = dot(q[0]).max(dot(q[1])).max(dot(q[2])).sqrt();
    let dp = [
        (-2.0 * w).mul_add(q[1][0], q[0][0]) + q[2][0],
        (-2.0 * w).mul_add(q[1][1], q[0][1]) + q[2][1],
    ];
    let dw = (-2.0f32).mul_add(w, 2.0).abs();
    let rp_minus_1 = m.mul_add(PRECISION, -1.0).max(0.0);
    let numer = dot(dp).sqrt() * PRECISION + rp_minus_1 * dw;
    let denom = 4.0 * w.min(1.0);
    libm::ceilf((numer / denom).sqrt()).max(1.0)
}

/// `tessellate_stroked_curve` for one vertex: device coordinates.
#[allow(clippy::too_many_arguments)]
fn stroked_vertex(
    edge_id: f32,
    max_edges: f32,
    affine: &[f32; 4],
    translate: P,
    max_scale: f32,
    patch: &StrokePatch,
    params: StrokeParams,
) -> P {
    let [mut p0, mut p1, mut p2, mut p3] = patch.p;
    let mut w = -1.0f32;
    if patch.conic {
        w = p3[0];
        p3 = p2;
    }
    let num_parametric_segments_init = if w < 0.0 {
        if p0 == p1 && p2 == p3 {
            1.0
        } else {
            gpu_wang_cubic(&[p0, p1, p2, p3], affine)
        }
    } else if p0 == p1 || p1 == p2 {
        1.0
    } else {
        let mv = |v: P| {
            [
                affine[0] * v[0] + affine[2] * v[1],
                affine[1] * v[0] + affine[3] * v[1],
            ]
        };
        gpu_wang_conic(mv(p0), mv(p1), mv(p2), w)
    };
    let mut num_parametric_segments = num_parametric_segments_init;
    let mut stroke_radius = params.radius;
    let mut join_type = params.join;
    let is_hairline = params.radius == 0.0;
    let num_radial_segments_per_radian = if is_hairline {
        stroke_radius = 0.5;
        num_radial_segments_per_radian(0.5)
    } else {
        num_radial_segments_per_radian(max_scale * params.radius)
    };
    let mut last_control = patch.join;
    let mv = |v: P| {
        [
            affine[0] * v[0] + affine[2] * v[1],
            affine[1] * v[0] + affine[3] * v[1],
        ]
    };
    if is_hairline {
        p0 = mv(p0);
        p1 = mv(p1);
        p2 = mv(p2);
        p3 = mv(p3);
        last_control = mv(last_control);
    }
    let mut tan0 = robust_normalize_diff(
        if p0 == p1 {
            if p1 == p2 { p3 } else { p2 }
        } else {
            p1
        },
        p0,
    );
    let mut tan1 = robust_normalize_diff(
        p3,
        if p3 == p2 {
            if p2 == p1 { p0 } else { p1 }
        } else {
            p2
        },
    );
    let mut prev_tan = robust_normalize_diff(p0, last_control);
    if tan0 == [0.0, 0.0] {
        join_type = 0.0;
        if w < 0.0 {
            tan0 = [1.0, 0.0];
            tan1 = [-1.0, 0.0];
        } else {
            w = -1.0;
            tan0 = prev_tan;
            tan1 = prev_tan;
            if prev_tan == [0.0, 0.0] {
                p2 = [p0[0] + stroke_radius, p0[1]];
                p3 = p2;
                p0 = [p0[0] - stroke_radius, p0[1]];
                p1 = p0;
                prev_tan = [1.0, 0.0];
                tan0 = prev_tan;
                tan1 = prev_tan;
            } else {
                p2 = [
                    p0[0] + stroke_radius * prev_tan[0],
                    p0[1] + stroke_radius * prev_tan[1],
                ];
                p3 = p2;
            }
        }
    }
    let num_edges_in_join = if join_type >= 0.0 {
        sign(join_type) + (1.0 + 2.0)
    } else {
        let join_rads = libm::acosf(cosine_between(prev_tan, tan0));
        let n = libm::ceilf(join_rads * num_radial_segments_per_radian).max(1.0);
        (n + 2.0).min(max_edges - 2.0)
    };
    let mut turn = cross(
        [p2[0] - p0[0], p2[1] - p0[1]],
        [p3[0] - p1[0], p3[1] - p1[1]],
    );
    let mut combined_edge_id = edge_id.abs() - num_edges_in_join;
    if combined_edge_id < 0.0 {
        tan1 = tan0;
        if last_control != p0 {
            tan0 = prev_tan;
        }
        turn = cross(tan0, tan1);
    }
    let cos_theta = cosine_between(tan0, tan1);
    let mut rotation = libm::acosf(cos_theta);
    if turn < 0.0 {
        rotation = -rotation;
    }
    let num_radial_segments;
    let mut stroke_outset = sign(edge_id);
    if combined_edge_id < 0.0 {
        num_radial_segments = num_edges_in_join - 2.0;
        num_parametric_segments = 1.0;
        p1 = p0;
        p2 = p0;
        p3 = p0;
        combined_edge_id += num_radial_segments + 1.0;
        if combined_edge_id < 0.0 {
            combined_edge_id = 0.0;
        } else {
            let sin_epsilon = 1e-2f32;
            let nearly_parallel = (turn.abs()
                * (1.0
                    / ((tan0[0] * tan0[0] + tan0[1] * tan0[1])
                        * (tan1[0] * tan1[0] + tan1[1] * tan1[1]))
                        .sqrt()))
                < sin_epsilon;
            if !nearly_parallel || tan0[0] * tan1[0] + tan0[1] * tan1[1] < 0.0 {
                stroke_outset = if turn < 0.0 {
                    stroke_outset.min(0.0)
                } else {
                    stroke_outset.max(0.0)
                };
            }
        }
    } else {
        let max_combined = max_edges - num_edges_in_join - 1.0;
        let n = libm::ceilf(rotation.abs() * num_radial_segments_per_radian).max(1.0);
        num_radial_segments = n.min(max_combined);
        num_parametric_segments =
            num_parametric_segments.min(max_combined - num_radial_segments + 1.0);
    }
    let rads_per_segment = rotation / num_radial_segments;
    let num_combined_segments = num_parametric_segments + num_radial_segments - 1.0;
    let is_final_edge = combined_edge_id >= num_combined_segments;
    if combined_edge_id > num_combined_segments {
        stroke_outset = 0.0;
    }
    if edge_id.abs() == 2.0 && join_type > 0.0 {
        stroke_outset *= miter_extent(cos_theta, join_type);
    }
    let (tangent, mut stroke_coord);
    if combined_edge_id != 0.0 && !is_final_edge {
        let (a, b, c);
        let mut cc = [p1[0] - p0[0], p1[1] - p0[1]];
        let d = [p3[0] - p0[0], p3[1] - p0[1]];
        if w >= 0.0 {
            cc = [cc[0] * w, cc[1] * w];
            b = [0.5 * d[0] - cc[0], 0.5 * d[1] - cc[1]];
            a = [(w - 1.0) * d[0], (w - 1.0) * d[1]];
            p1 = [p1[0] * w, p1[1] * w];
        } else {
            let e = [p2[0] - p1[0], p2[1] - p1[1]];
            b = [e[0] - cc[0], e[1] - cc[1]];
            a = [(-3.0f32).mul_add(e[0], d[0]), (-3.0f32).mul_add(e[1], d[1])];
        }
        c = cc;
        let b_ = [
            b[0] * (num_parametric_segments * 2.0),
            b[1] * (num_parametric_segments * 2.0),
        ];
        let c_ = [
            c[0] * (num_parametric_segments * num_parametric_segments),
            c[1] * (num_parametric_segments * num_parametric_segments),
        ];
        let mut last_parametric = 0.0f32;
        let max_parametric = (num_parametric_segments - 1.0).min(combined_edge_id);
        let neg_abs_rads = -rads_per_segment.abs();
        let max_rotation0 = (1.0 + combined_edge_id) * rads_per_segment.abs();
        let mut exp = 32.0f32;
        while exp >= 1.0 {
            let test = last_parametric + exp;
            if test <= max_parametric {
                let mut tt = [test.mul_add(a[0], b_[0]), test.mul_add(a[1], b_[1])];
                tt = [test.mul_add(tt[0], c_[0]), test.mul_add(tt[1], c_[1])];
                let len = (tt[0] * tt[0] + tt[1] * tt[1]).sqrt();
                let cos_rot = (tt[0] / len) * tan0[0] + (tt[1] / len) * tan0[1];
                let max_rot = test
                    .mul_add(neg_abs_rads, max_rotation0)
                    .min(core::f32::consts::PI);
                if cos_rot >= libm::cosf(max_rot) {
                    last_parametric = test;
                }
            }
            exp *= 0.5;
        }
        let parametric_t = last_parametric / num_parametric_segments;
        let last_radial = combined_edge_id - last_parametric;
        let mut angle0 = libm::acosf(tan0[0].clamp(-1.0, 1.0));
        if tan0[1] < 0.0 {
            angle0 = -angle0;
        }
        let radial_angle = last_radial.mul_add(rads_per_segment, angle0);
        let mut tg = [libm::cosf(radial_angle), libm::sinf(radial_angle)];
        let norm = [-tg[1], tg[0]];
        let qa = norm[0] * a[0] + norm[1] * a[1];
        let b_over_2 = norm[0] * b[0] + norm[1] * b[1];
        let qc = norm[0] * c[0] + norm[1] * c[1];
        let discr = (b_over_2 * b_over_2 - qa * qc).max(0.0);
        let mut q = discr.sqrt();
        if b_over_2 > 0.0 {
            q = -q;
        }
        q -= b_over_2;
        let five_qa = -0.5 * q * qa;
        let root = if q.mul_add(q, five_qa).abs() < qa.mul_add(qc, five_qa).abs() {
            [q, qa]
        } else {
            [qc, q]
        };
        let radial_t = if last_radial != 0.0 && root[1] != 0.0 {
            (root[0] / root[1]).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let t = parametric_t.max(radial_t);
        let ab = umix2(p0, p1, t);
        let bc = umix2(p1, p2, t);
        let cd = umix2(p2, p3, t);
        let abc = umix2(ab, bc, t);
        let bcd = umix2(bc, cd, t);
        let abcd = umix2(abc, bcd, t);
        let u = umix(1.0, w, t);
        let v = w + 1.0 - u;
        let uv = umix(u, v, t);
        if t != radial_t {
            tg = if w >= 0.0 {
                robust_normalize_diff([bc[0] * u, bc[1] * u], [ab[0] * v, ab[1] * v])
            } else {
                robust_normalize_diff(bcd, abc)
            };
        }
        tangent = tg;
        // `abc / uv` compiles to `abc * rcp(uv)` under Metal fast math
        // (see `tess::patch_points`).
        stroke_coord = if w >= 0.0 {
            let r = 1.0 / uv;
            [abc[0] * r, abc[1] * r]
        } else {
            abcd
        };
    } else {
        tangent = if combined_edge_id == 0.0 { tan0 } else { tan1 };
        stroke_coord = if combined_edge_id == 0.0 { p0 } else { p3 };
    }
    let ortho = [tangent[1], -tangent[0]];
    let k = stroke_radius * stroke_outset;
    stroke_coord = [
        stroke_coord[0] + ortho[0] * k,
        stroke_coord[1] + ortho[1] * k,
    ];
    if is_hairline {
        [
            stroke_coord[0] + translate[0],
            stroke_coord[1] + translate[1],
        ]
    } else {
        let d = mv(stroke_coord);
        [d[0] + translate[0], d[1] + translate[1]]
    }
}

/// `sign()` with Metal's signed zeros.
#[inline]
fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

/// Every triangle (device coordinates) the stroke of `path` rasterizes.
pub fn stroke_triangles(path: &SkPath, m: &Mat, params: StrokeParams, cap: Cap) -> Vec<[P; 3]> {
    let patches = stroke_patches(path, m, cap);
    let affine = [m.sx, m.ky, m.kx, m.sy];
    let translate = [m.tx, m.ty];
    let max_scale = svd_max(m.sx, m.kx, m.ky, m.sy, m);
    let max_edges = 16383.0f32;
    let mut tris = Vec::new();
    for patch in &patches {
        // Enough edges for the join and every combined segment; the GPU's
        // further edges collapse onto the end point.
        let needed = edges_needed(patch, &affine, max_scale, params);
        let verts: Vec<P> = (0..2 * needed)
            .map(|v| {
                let e = (v >> 1) as f32;
                let id = if v & 1 == 1 { -e } else { e };
                stroked_vertex(id, max_edges, &affine, translate, max_scale, patch, params)
            })
            .collect();
        for i in 2..verts.len() {
            tris.push([verts[i - 2], verts[i - 1], verts[i]]);
        }
    }
    tris
}

/// The number of strip edges a patch uses (join edges, combined segments,
/// and the final edge), from the same formulas as the vertex function.
fn edges_needed(
    patch: &StrokePatch,
    affine: &[f32; 4],
    max_scale: f32,
    params: StrokeParams,
) -> usize {
    // Generous bound: join edges (at most the radial segments of half a
    // turn plus 2) plus parametric (at most the GPU's Wang count) plus
    // radial (at most half a turn per patch) plus 2.
    let radius = if params.radius == 0.0 {
        0.5
    } else {
        max_scale * params.radius
    };
    let per_rad = num_radial_segments_per_radian(radius);
    let half_turn = libm::ceilf(per_rad * core::f32::consts::PI).max(1.0);
    let [p0, p1, p2, p3] = patch.p;
    let param = if patch.conic {
        let mv = |v: P| {
            [
                affine[0] * v[0] + affine[2] * v[1],
                affine[1] * v[0] + affine[3] * v[1],
            ]
        };
        if p0 == p1 || p1 == p2 {
            1.0
        } else {
            gpu_wang_conic(mv(p0), mv(p1), mv(p2), p3[0])
        }
    } else if p0 == p1 && p2 == p3 {
        1.0
    } else {
        gpu_wang_cubic(&patch.p, affine)
    };
    let param = if param.is_finite() {
        param.min(16383.0)
    } else {
        16383.0
    };
    (half_turn + 2.0 + param + half_turn + 2.0) as usize
}

/// `compute_svd`'s maximum (or the diagonal's for rect-stays-rect).
pub(super) fn svd_max(m00: f32, m01: f32, m10: f32, m11: f32, m: &Mat) -> f32 {
    if m.kx == 0.0 && m.ky == 0.0 {
        return m.sx.abs().max(m.sy.abs());
    }
    if m.sx == 0.0 && m.sy == 0.0 {
        return m.kx.abs().max(m.ky.abs());
    }
    let s1 = m00 * m00 + m01 * m01 + m10 * m10 + m11 * m11;
    let e = m00 * m00 + m01 * m01 - m10 * m10 - m11 * m11;
    let f = m00 * m10 + m01 * m11;
    let s2 = (e * e + 4.0 * f * f).sqrt();
    (0.5 * (s1 + s2)).sqrt()
}
