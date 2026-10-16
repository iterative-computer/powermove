//! Graphite's fixed-count path tessellation for fills (Skia m153
//! `PatchWriter`, `TessellateWedgesRenderStep`, and the
//! `tessellate_filled_curve` vertex function), reproduced on the CPU in
//! `f32`: every curve becomes the polyline the GPU rasterizes.
//!
//! Filled wedges, stencilled wedges, and stencilled curves with middle-out
//! triangles all cover exactly the samples inside the polygonized path under
//! the fill rule, so a fill reduces to that polygon.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::skpath::{Mat, P, SkPath, Verb};

/// `skgpu::tess::kPrecision`.
pub const PRECISION: f32 = 4.0;
/// `kMaxResolveLevel`.
pub const MAX_RESOLVE_LEVEL: i32 = 5;
const MAX_PARAMETRIC_SEGMENTS_P4: f32 = 32.0 * 32.0 * 32.0 * 32.0;
const MAX_SEGMENTS_PER_CURVE_P4: f32 = 1024.0 * 1024.0 * 1024.0 * 1024.0;

/// The 2×2 part of the local-to-device transform (`VectorXform`).
#[derive(Debug, Clone, Copy)]
pub struct Vx {
    c0: [f32; 2],
    c1: [f32; 2],
}

impl Vx {
    /// The two columns.
    pub fn columns(&self) -> ([f32; 2], [f32; 2]) {
        (self.c0, self.c1)
    }

    pub fn of(m: &Mat) -> Self {
        Self {
            c0: [m.sx, m.ky],
            c1: [m.kx, m.sy],
        }
    }

    #[inline]
    fn apply(&self, v: [f32; 2]) -> [f32; 2] {
        [self.c0[0] * v[0] + self.c1[0] * v[1], self.c0[1] * v[0] + self.c1[1] * v[1]]
    }
}

#[inline]
fn root4(x: f32) -> f32 {
    x.sqrt().sqrt()
}

/// `wangs_formula::cubic_p4` (CPU).
pub fn cubic_p4(p: &[P; 4], vx: &Vx) -> f32 {
    let d0 = [-2.0 * p[1][0] + p[0][0] + p[2][0], -2.0 * p[1][1] + p[0][1] + p[2][1]];
    let d1 = [-2.0 * p[2][0] + p[1][0] + p[3][0], -2.0 * p[2][1] + p[1][1] + p[3][1]];
    let (d0, d1) = (vx.apply(d0), vx.apply(d1));
    let k = (9.0 * 4.0 / 64.0) * (PRECISION * PRECISION);
    (d0[0] * d0[0] + d0[1] * d0[1]).max(d1[0] * d1[0] + d1[1] * d1[1]) * k
}

/// `wangs_formula::conic_p2` (CPU).
pub fn conic_p2(p: &[P; 3], w: f32, vx: &Vx) -> f32 {
    let (mut p0, mut p1, mut p2) = (vx.apply(p[0]), vx.apply(p[1]), vx.apply(p[2]));
    let c = [
        0.5 * (p0[0].min(p1[0]).min(p2[0]) + p0[0].max(p1[0]).max(p2[0])),
        0.5 * (p0[1].min(p1[1]).min(p2[1]) + p0[1].max(p1[1]).max(p2[1])),
    ];
    for q in [&mut p0, &mut p1, &mut p2] {
        q[0] -= c[0];
        q[1] -= c[1];
    }
    let dot = |a: [f32; 2]| a[0] * a[0] + a[1] * a[1];
    let max_len = dot(p0).max(dot(p1).max(dot(p2))).sqrt();
    let dp = [-2.0 * w * p1[0] + p0[0] + p2[0], -2.0 * w * p1[1] + p0[1] + p2[1]];
    let dw = (-2.0 * w + 2.0).abs();
    let rp_minus_1 = (max_len * PRECISION - 1.0).max(0.0);
    let numer = dot(dp).sqrt() * PRECISION + rp_minus_1 * dw;
    let denom = 4.0 * w.min(1.0);
    numer / denom
}

/// The vertex shader's `wangs_formula_cubic_log2` (`f32`, transformed
/// forward differences).
fn gpu_cubic_log2(p: &[P; 4], vx: &Vx) -> f32 {
    let d0 = vx.apply([(-2.0f32).mul_add(p[1][0], p[2][0]) + p[0][0], (-2.0f32).mul_add(p[1][1], p[2][1]) + p[0][1]]);
    let d1 = vx.apply([(-2.0f32).mul_add(p[2][0], p[3][0]) + p[1][0], (-2.0f32).mul_add(p[2][1], p[3][1]) + p[1][1]]);
    let m = (d0[0] * d0[0] + d0[1] * d0[1]).max(d1[0] * d1[0] + d1[1] * d1[1]);
    let k = (9.0 * 4.0 / 64.0) * (PRECISION * PRECISION);
    libm::ceilf(libm::log2f((k * m).max(1.0)) * 0.25)
}

/// The vertex shader's `wangs_formula_conic_log2` (on transformed points).
fn gpu_conic_log2(p: &[P; 3], w: f32, vx: &Vx) -> f32 {
    let (mut p0, mut p1, mut p2) = (vx.apply(p[0]), vx.apply(p[1]), vx.apply(p[2]));
    let c = [
        (p0[0].min(p1[0]).min(p2[0]) + p0[0].max(p1[0]).max(p2[0])) * 0.5,
        (p0[1].min(p1[1]).min(p2[1]) + p0[1].max(p1[1]).max(p2[1])) * 0.5,
    ];
    for q in [&mut p0, &mut p1, &mut p2] {
        q[0] -= c[0];
        q[1] -= c[1];
    }
    let dot = |a: [f32; 2]| a[0] * a[0] + a[1] * a[1];
    let m = dot(p0).max(dot(p1)).max(dot(p2)).sqrt();
    let dp = [(-2.0 * w).mul_add(p1[0], p0[0]) + p2[0], (-2.0 * w).mul_add(p1[1], p0[1]) + p2[1]];
    let dw = (-2.0f32).mul_add(w, 2.0).abs();
    let rp_minus_1 = m.mul_add(PRECISION, -1.0).max(0.0);
    let numer = dot(dp).sqrt() * PRECISION + rp_minus_1 * dw;
    let denom = 4.0 * w.min(1.0);
    let n2 = numer / denom;
    libm::ceilf(libm::log2f(n2.max(1.0)) * 0.5)
}

/// Metal's `mix` as the fast-math compiler emits it: `fma(b - a, t, a)`
/// (measured: the other expansions lose bit-exact ellipse fills).
#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    (b - a).mul_add(t, a)
}

#[inline]
fn mix2(a: P, b: P, t: f32) -> P {
    [mix(a[0], b[0], t), mix(a[1], b[1], t)]
}

/// One patch as PatchWriter writes it: a cubic, or a conic (`w` ≥ 0) with
/// its three points (`p[3]` unused).
#[derive(Debug, Clone, Copy)]
pub struct Patch {
    pub p: [P; 4],
    /// Conic weight, or `None` for an integral cubic.
    pub w: Option<f32>,
}

/// Points of a patch as the vertex shader places them for its resolve
/// level (`tessellate_filled_curve`), excluding the first point.
pub fn patch_points(patch: &Patch, vx: &Vx, out: &mut Vec<P>) {
    let [p0, mut p1, p2, mut p3] = patch.p;
    let w = patch.w.unwrap_or(-1.0);
    let max_level = match patch.w {
        Some(w) => gpu_conic_log2(&[p0, p1, p2], w, vx),
        None => gpu_cubic_log2(&patch.p, vx),
    };
    if patch.w.is_some() {
        p1 = [p1[0] * w, p1[1] * w];
        p3 = p2;
    }
    let level = max_level.min(MAX_RESOLVE_LEVEL as f32).max(0.0) as i32;
    let n = 1 << level;
    for i in 1..=n {
        let fixed = libm::floorf(0.5 + libm::ldexpf(i as f32, 5 - level));
        if 0.0 < fixed && fixed < 32.0 {
            let t = fixed * (1.0 / 32.0);
            let ab = mix2(p0, p1, t);
            let bc = mix2(p1, p2, t);
            let cd = mix2(p2, p3, t);
            let abc = mix2(ab, bc, t);
            let bcd = mix2(bc, cd, t);
            let abcd = mix2(abc, bcd, t);
            let u = mix(1.0, w, t);
            let v = w + 1.0 - u;
            let uv = mix(u, v, t);
            out.push(if w < 0.0 { abcd } else { [abc[0] / uv, abc[1] / uv] });
        } else {
            out.push(if fixed == 0.0 { p0 } else { p3 });
        }
    }
}

/// `PatchWriter::accountForCurve`: how many patches a curve is chopped into
/// (0: written as it is).
fn chops(n4: f32) -> i32 {
    if n4 <= MAX_PARAMETRIC_SEGMENTS_P4 {
        0
    } else {
        libm::ceilf(root4(n4.min(MAX_SEGMENTS_PER_CURVE_P4) / MAX_PARAMETRIC_SEGMENTS_P4)) as i32
    }
}

fn mix4(a: [f32; 4], b: [f32; 4], t: [f32; 4]) -> [f32; 4] {
    [(b[0] - a[0]) * t[0] + a[0], (b[1] - a[1]) * t[1] + a[1], (b[2] - a[2]) * t[2] + a[2], (b[3] - a[3]) * t[3] + a[3]]
}

/// Splits a cubic into PatchWriter's chopped patches.
fn chop_cubic(mut p: [P; 4], mut n: i32, out: &mut Vec<Patch>) {
    while n >= 3 {
        let t = [1.0 / n as f32, 1.0 / n as f32, 2.0 / n as f32, 2.0 / n as f32];
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
        out.push(Patch { p: [p[0], lo(ab), lo(abc), lo(abcd)], w: None });
        out.push(Patch { p: [lo(abcd), lo(middle), hi(middle), hi(abcd)], w: None });
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
        out.push(Patch { p: [p[0], ab, abc, abcd], w: None });
        out.push(Patch { p: [abcd, bcd, cd, p[3]], w: None });
    } else {
        out.push(Patch { p, w: None });
    }
}

/// Splits a conic into PatchWriter's chopped patches.
fn chop_conic(p: [P; 3], w: f32, mut n: i32, out: &mut Vec<Patch>) {
    let mut h0 = [p[0][0], p[0][1], 1.0, 1.0];
    let mut h1 = [p[1][0] * w, p[1][1] * w, w, w];
    let h2 = [p[2][0], p[2][1], 1.0, 1.0];
    while n >= 2 {
        let t = 1.0 / n as f32;
        let tt = [t; 4];
        let ab = mix4(h0, h1, tt);
        let bc = mix4(h1, h2, tt);
        let abc = mix4(ab, bc, tt);
        let mid = [abc[0] / abc[3], abc[1] / abc[3]];
        out.push(Patch {
            p: [[h0[0] / h0[3], h0[1] / h0[3]], [ab[0] / ab[3], ab[1] / ab[3]], mid, [0.0; 2]],
            w: Some(ab[3] / (h0[3] * abc[3]).sqrt()),
        });
        h0 = abc;
        h1 = bc;
        n -= 1;
    }
    out.push(Patch {
        p: [[h0[0] / h0[3], h0[1] / h0[3]], [h1[0] / h1[3], h1[1] / h1[3]], [h2[0], h2[1]], [0.0; 2]],
        w: Some(h1[3] / h0[3].sqrt()),
    });
}

/// The patches PatchWriter writes for a curve verb (lines are written as
/// cubics with control points at one and two thirds).
pub fn curve_patches(verb: Verb, pts: &[P], vx: &Vx, out: &mut Vec<Patch>) {
    match verb {
        Verb::Line => {
            let (a, b) = (pts[0], pts[1]);
            let third = |i: f32| [(b[0] - a[0]) * (1.0 / 3.0) * i + a[0], (b[1] - a[1]) * (1.0 / 3.0) * i + a[1]];
            // `(p0p1.zwxy() - p0p1) * (1/3.f) + p0p1`: p1 = a + (b - a)/3,
            // p2 = b + (a - b)/3.
            let p1 = third(1.0);
            let p2 = [(a[0] - b[0]) * (1.0 / 3.0) + b[0], (a[1] - b[1]) * (1.0 / 3.0) + b[1]];
            let _ = third;
            out.push(Patch { p: [a, p1, p2, b], w: None });
        }
        Verb::Quad => {
            let (p0, p1, p2) = (pts[0], pts[1], pts[2]);
            let n4 = {
                let v = vx.apply([-2.0 * p1[0] + p0[0] + p2[0], -2.0 * p1[1] + p0[1] + p2[1]]);
                (v[0] * v[0] + v[1] * v[1]) * ((4.0 / 64.0) * (PRECISION * PRECISION))
            };
            let n = chops(n4);
            let as_cubic = |p0: P, p1: P, p2: P| {
                // `mix(float4(p0, p2), p1.xyxy(), 2/3.f)`.
                let c1 = [(p1[0] - p0[0]) * (2.0 / 3.0) + p0[0], (p1[1] - p0[1]) * (2.0 / 3.0) + p0[1]];
                let c2 = [(p1[0] - p2[0]) * (2.0 / 3.0) + p2[0], (p1[1] - p2[1]) * (2.0 / 3.0) + p2[1]];
                Patch { p: [p0, c1, c2, p2], w: None }
            };
            if n == 0 {
                out.push(as_cubic(p0, p1, p2));
            } else {
                chop_quad(p0, p1, p2, n, out, &as_cubic);
            }
        }
        Verb::Conic(w) => {
            let p = [pts[0], pts[1], pts[2]];
            let n2 = conic_p2(&p, w, vx);
            let n = chops(n2 * n2);
            if n == 0 {
                out.push(Patch { p: [p[0], p[1], p[2], [0.0; 2]], w: Some(w) });
            } else {
                chop_conic(p, w, n, out);
            }
        }
        Verb::Cubic => {
            let p = [pts[0], pts[1], pts[2], pts[3]];
            let n = chops(cubic_p4(&p, vx));
            if n == 0 {
                out.push(Patch { p, w: None });
            } else {
                chop_cubic(p, n, out);
            }
        }
        Verb::Move | Verb::Close => {}
    }
}

fn chop_quad(mut p0: P, mut p1: P, p2: P, mut n: i32, out: &mut Vec<Patch>, as_cubic: &dyn Fn(P, P, P) -> Patch) {
    while n >= 3 {
        let t = [1.0 / n as f32, 1.0 / n as f32, 2.0 / n as f32, 2.0 / n as f32];
        let xy = |q: P| [q[0], q[1], q[0], q[1]];
        let ab = mix4(xy(p0), xy(p1), t);
        let bc = mix4(xy(p1), xy(p2), t);
        let abc = mix4(ab, bc, t);
        let tm = mix4(t, [t[2], t[3], t[0], t[1]], [2.0 / 3.0; 4]);
        let middle = mix4(ab, bc, tm);
        let lo = |v: [f32; 4]| [v[0], v[1]];
        let hi = |v: [f32; 4]| [v[2], v[3]];
        out.push(as_cubic(p0, lo(ab), lo(abc)));
        out.push(Patch { p: [lo(abc), lo(middle), hi(middle), hi(abc)], w: None });
        p0 = hi(abc);
        p1 = hi(bc);
        n -= 2;
    }
    if n == 2 {
        let h = |a: P, b: P| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
        let ab = h(p0, p1);
        let bc = h(p1, p2);
        let abc = h(ab, bc);
        out.push(as_cubic(p0, ab, abc));
        out.push(as_cubic(abc, bc, p2));
    } else {
        out.push(as_cubic(p0, p1, p2));
    }
}

/// The polygons (local coordinates) a filled path rasterizes as: one closed
/// polyline per contour, curves tessellated as the GPU tessellates them.
pub fn fill_polygons_local(path: &SkPath, m: &Mat) -> Vec<Vec<P>> {
    let vx = Vx::of(m);
    let mut contours: Vec<Vec<P>> = Vec::new();
    let mut cur: Vec<P> = Vec::new();
    let mut patches = Vec::new();
    for (verb, pts) in path.segments() {
        match verb {
            Verb::Move => {
                if cur.len() > 1 {
                    contours.push(std::mem::take(&mut cur));
                }
                cur.clear();
                cur.push(pts[0]);
            }
            Verb::Close => {}
            _ => {
                patches.clear();
                curve_patches(verb, &pts, &vx, &mut patches);
                for p in &patches {
                    patch_points(p, &vx, &mut cur);
                }
            }
        }
    }
    if cur.len() > 1 {
        contours.push(cur);
    }
    contours
}

/// Maps local points to window coordinates as the vertex shader does.
pub fn to_device(contours: &[Vec<P>], m: &Mat, width: u32, height: u32) -> Vec<Vec<[f64; 2]>> {
    contours
        .iter()
        .map(|c| c.iter().map(|p| window(*p, m, width, height)).collect())
        .collect()
}

/// A device position (already transformed) through the NDC conversion and
/// viewport transform.
pub fn window_device(d: P, _width: u32, _height: u32) -> [f64; 2] {
    [f64::from(d[0]), f64::from(d[1])]
}

/// One vertex through the vertex shader: `localToDevice * float4(p, 0, 1)`
/// as a fused multiply-add chain in `f32`. (The NDC round trip and viewport
/// transform measured no effect on any reference.)
pub fn window(p: P, m: &Mat, _width: u32, _height: u32) -> [f64; 2] {
    let x = m.sx.mul_add(p[0], m.kx.mul_add(p[1], m.tx));
    let y = m.ky.mul_add(p[0], m.sy.mul_add(p[1], m.ty));
    [f64::from(x), f64::from(y)]
}
