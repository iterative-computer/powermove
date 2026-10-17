//! Graphite's `AnalyticRRectRenderStep` (Skia m153) for the shapes Canvas2D
//! sends it from the legacy rasterizer: filled rectangles (as all-AA
//! quadrilaterals), stroked rectangles, and stroked lines. The 36-vertex
//! instance mesh is built by a port of `analytic_rrect_vertex_fn`, its
//! triangle strip is rasterized in primitive order at 4× MSAA (the depth test
//! lets the first triangle covering a sample win), and each fragment's
//! coverage is `analytic_rrect_coverage_fn` evaluated on the varyings
//! interpolated at the pixel center.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::msaa::{SUBPIXEL_BITS, snap};
use super::skpath::Mat;

const K_MITER: f32 = 1.0;
const K_BEVEL: f32 = 0.0;
const K_ROUND: f32 = 0.414_213_57;
const K_EPSILON: f32 = 0.00024;

const SOLID_INTERIOR: f32 = 1.0;
const STROKE_INTERIOR: f32 = 0.0;
const FILLED_STROKE_INTERIOR: f32 = -1.0;
const COMPLEX_AA_INSETS: f32 = -1.0;

/// One instance (the appended attributes).
#[derive(Debug, Clone, Copy)]
pub struct Instance {
    pub x_radii_or_flags: [f32; 4],
    pub radii_or_quad_xs: [f32; 4],
    pub ltrb_or_quad_ys: [f32; 4],
    pub center: [f32; 4],
    /// The local-to-device 3×3 (`mat0`, `mat1`, `mat2` columns).
    pub m: [[f32; 3]; 3],
}

/// The geometry handed to the render step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Geometry {
    /// A filled rectangle `[l, t, r, b]`.
    FillRect([f32; 4]),
    /// A stroked rectangle with the stroke radius and join (`StrokeStyle`
    /// convention: negative round, zero bevel, positive miter limit).
    StrokeRect { rect: [f32; 4], radius: f32, join: f32 },
    /// A stroked line `(x0, y0, x1, y1)` with a cap.
    StrokeLine { line: [f32; 4], radius: f32, cap: super::stroke::Cap },
}

fn svd_min(m: &Mat) -> f32 {
    if m.kx == 0.0 && m.ky == 0.0 {
        return m.sx.abs().min(m.sy.abs());
    }
    if m.sx == 0.0 && m.sy == 0.0 {
        return m.kx.abs().min(m.ky.abs());
    }
    let (m00, m01, m10, m11) = (m.sx, m.kx, m.ky, m.sy);
    let s1 = m00 * m00 + m01 * m01 + m10 * m10 + m11 * m11;
    let e = m00 * m00 + m01 * m01 - m10 * m10 - m11 * m11;
    let f = m00 * m10 + m01 * m11;
    let s2 = (e * e + 4.0 * f * f).sqrt();
    (0.5 * (s1 - s2)).sqrt()
}

/// `Transform::localAARadius` for an affine transform.
fn local_aa_radius(m: &Mat) -> f32 {
    let r = 1.0 / svd_min(m);
    if r.is_finite() { r } else { f32::INFINITY }
}

/// `AnalyticRRectRenderStep::writeVertices`.
pub fn instance(g: &Geometry, m: &Mat) -> Instance {
    let mut aa_radius = local_aa_radius(m);
    let mut stroke_inset = 0.0f32;
    let mut center_weight = SOLID_INTERIOR;
    let (a, b, c, center_xy, intersects);
    match *g {
        Geometry::FillRect(r) => {
            a = [-1.0; 4];
            b = [r[0], r[2], r[2], r[0]];
            c = [r[1], r[1], r[3], r[3]];
            center_xy = [(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5];
            // opposite_insets_intersect for an all-AA rect quad.
            intersects = (r[2] - r[0]) <= 2.0 * aa_radius || (r[3] - r[1]) <= 2.0 * aa_radius;
        }
        Geometry::StrokeRect { rect: r, radius, join } => {
            let size = [r[2] - r[0], r[3] - r[1]];
            let gap = [size[0] - 2.0 * radius, size[1] - 2.0 * radius];
            if (gap[0] <= 0.0 || gap[1] <= 0.0) && radius > 0.0 {
                stroke_inset = -radius;
            } else {
                center_weight = STROKE_INTERIOR;
                stroke_inset = radius;
            }
            let empty = [size[0] == 0.0, size[1] == 0.0];
            let mut join_style = join;
            if empty[0] && empty[1] {
                // A point would produce caps; Canvas2D rectangles never reach here.
            } else if join > 0.0 {
                join_style = if join < core::f32::consts::SQRT_2 || empty[0] || empty[1] { 0.0 } else { 1.0 };
            }
            a = [-2.0, 0.0, radius, join_style];
            b = [0.0; 4];
            c = r;
            center_xy = [(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5];
            intersects = size[0] <= 2.0 * (stroke_inset + aa_radius) || size[1] <= 2.0 * (stroke_inset + aa_radius);
        }
        Geometry::StrokeLine { line, radius, cap } => {
            let len = ((line[2] - line[0]) * (line[2] - line[0]) + (line[3] - line[1]) * (line[3] - line[1])).sqrt();
            let size = [len, 0.0];
            let gap = [size[0] - 2.0 * radius, size[1] - 2.0 * radius];
            if (gap[0] <= 0.0 || gap[1] <= 0.0) && radius > 0.0 {
                stroke_inset = -radius;
            } else {
                center_weight = STROKE_INTERIOR;
                stroke_inset = radius;
            }
            let join_style = match cap {
                super::stroke::Cap::Round => -1.0,
                super::stroke::Cap::Butt => 0.0,
                super::stroke::Cap::Square => 1.0,
            };
            a = [-2.0, 1.0, radius, join_style];
            b = [0.0; 4];
            c = line;
            // bounds of a line shape: the bounding box of its points.
            let bl = [line[0].min(line[2]), line[1].min(line[3]), line[0].max(line[2]), line[1].max(line[3])];
            center_xy = [(bl[0] + bl[2]) * 0.5, (bl[1] + bl[3]) * 0.5];
            intersects = stroke_inset <= aa_radius;
        }
    }
    if intersects {
        aa_radius = COMPLEX_AA_INSETS;
        if center_weight == STROKE_INTERIOR {
            center_weight = FILLED_STROKE_INTERIOR;
        }
    }
    let _ = stroke_inset;
    Instance {
        x_radii_or_flags: a,
        radii_or_quad_xs: b,
        ltrb_or_quad_ys: c,
        center: [center_xy[0], center_xy[1], center_weight, aa_radius],
        // mat0 = (rc00, rc10, rc30), mat1 = (rc01, rc11, rc31), mat2 = (rc03, rc13, rc33).
        m: [[m.sx, m.ky, 0.0], [m.kx, m.sy, 0.0], [m.tx, m.ty, 1.0]],
    }
}

/// One mesh vertex: device position and varyings.
#[derive(Debug, Clone, Copy, Default)]
pub struct Varyings {
    pub pos: [f32; 2],
    pub jacobian: [f32; 4],
    pub edge: [f32; 4],
    pub x_radii: [f32; 4],
    pub y_radii: [f32; 4],
    pub stroke: [f32; 2],
    pub control: [f32; 2],
}

/// The static per-corner vertex template: position, normal, normal scale,
/// center weight.
const TEMPLATE: [([f32; 2], [f32; 2], f32, f32); 9] = {
    const HR2: f32 = 0.5 * core::f32::consts::SQRT_2;
    [
        ([1.0, 0.0], [1.0, 0.0], 1.0, 0.0),
        ([1.0, 0.0], [HR2, HR2], 1.0, 0.0),
        ([0.0, 1.0], [HR2, HR2], 1.0, 0.0),
        ([0.0, 1.0], [0.0, 1.0], 1.0, 0.0),
        ([1.0, 0.0], [HR2, HR2], 0.0, 0.0),
        ([0.0, 1.0], [HR2, HR2], 0.0, 0.0),
        ([1.0, 0.0], [1.0, 0.0], -1.0, 0.0),
        ([0.0, 1.0], [0.0, 1.0], -1.0, 0.0),
        ([1.0, 0.0], [1.0, 0.0], -1.0, 1.0),
    ]
};

/// The index buffer (triangle strip with degenerate joins).
const INDICES: [u16; 69] = {
    const TL: u16 = 0;
    const TR: u16 = 9;
    const BR: u16 = 18;
    const BL: u16 = 27;
    [
        TL, TL + 4, TL + 1, TL + 5, TL + 2, TL + 3, TL + 5,
        TR, TR + 4, TR + 1, TR + 5, TR + 2, TR + 3, TR + 5,
        BR, BR + 4, BR + 1, BR + 5, BR + 2, BR + 3, BR + 5,
        BL, BL + 4, BL + 1, BL + 5, BL + 2, BL + 3, BL + 5,
        TL, TL + 4,
        TL + 4, TL + 6, TL + 5, TL + 7,
        TR + 4, TR + 6, TR + 5, TR + 7,
        BR + 4, BR + 6, BR + 5, BR + 7,
        BL + 4, BL + 6, BL + 5, BL + 7,
        TL + 4, TL + 6,
        TL + 6, TL + 8, TL + 7, TL + 7, TR + 8,
        TR + 6, TR + 8, TR + 7, TR + 7, BR + 8,
        BR + 6, BR + 8, BR + 7, BR + 7, BL + 8,
        BL + 6, BL + 8, BL + 7, BL + 7, TL + 8,
        TL + 6,
    ]
};

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

fn inverse3(m: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    // Column-major m[col][row].
    let (a00, a01, a02) = (m[0][0], m[0][1], m[0][2]);
    let (a10, a11, a12) = (m[1][0], m[1][1], m[1][2]);
    let (a20, a21, a22) = (m[2][0], m[2][1], m[2][2]);
    let b01 = a22 * a11 - a12 * a21;
    let b11 = -a22 * a10 + a12 * a20;
    let b21 = a21 * a10 - a11 * a20;
    let det = a00 * b01 + a01 * b11 + a02 * b21;
    let inv = 1.0 / det;
    [
        [b01 * inv, (-a22 * a01 + a02 * a21) * inv, (a12 * a01 - a02 * a11) * inv],
        [b11 * inv, (a22 * a00 - a02 * a20) * inv, (-a12 * a00 + a02 * a10) * inv],
        [b21 * inv, (-a21 * a00 + a01 * a20) * inv, (a11 * a00 - a01 * a10) * inv],
    ]
}

/// `analytic_rrect_vertex_fn` for vertex `vid` (0..36) of an instance.
pub fn vertex(inst: &Instance, vid: usize) -> Varyings {
    let corner = vid / 9;
    let (position, normal, normal_scale, center_weight) = TEMPLATE[vid % 9];
    let next = (corner + 1) % 4;
    let xr = inst.x_radii_or_flags;
    let center = inst.center;
    let mut join_scale = K_MITER;
    let bidirectional = center[2] <= 0.0;
    let mut device_space = false;
    let (xs, ys);
    let mut edge_aa = [1.0f32; 4];
    let mut stroked_line = false;
    let (x_radii, y_radii);
    let mut stroke_params;
    let l = inst.ltrb_or_quad_ys;
    if xr[0] < -1.0 {
        stroked_line = xr[1] > 0.0;
        xs = if stroked_line { [l[0], l[0], l[2], l[2]] } else { [l[0], l[2], l[2], l[0]] };
        ys = [l[1], l[1], l[3], l[3]];
        if xr[1] < 0.0 {
            x_radii = xr.map(|v| -v - 2.0);
            y_radii = inst.radii_or_quad_xs;
            stroke_params = [0.0, 1.0];
        } else {
            x_radii = inst.radii_or_quad_xs;
            y_radii = x_radii;
            stroke_params = [xr[2], xr[3]];
            join_scale = if stroke_params[1] < 0.0 { K_ROUND } else { sign(stroke_params[1]) };
        }
    } else if xr.iter().any(|&v| v > 0.0) {
        xs = [l[0], l[2], l[2], l[0]];
        ys = [l[1], l[1], l[3], l[3]];
        x_radii = xr;
        y_radii = inst.radii_or_quad_xs;
        stroke_params = [0.0, -1.0];
    } else {
        xs = inst.radii_or_quad_xs;
        ys = l;
        edge_aa = xr.map(|v| -v);
        x_radii = [0.0; 4];
        y_radii = [0.0; 4];
        stroke_params = [0.0, 1.0];
        device_space = true;
    }
    let mut corner_radii = [x_radii[corner], y_radii[corner]];
    if !corner.is_multiple_of(2) {
        corner_radii = [corner_radii[1], corner_radii[0]];
    }
    let mut corner_aspect = [1.0f32, 1.0];
    if corner_radii[0] > 0.0 && corner_radii[1] > 0.0 {
        join_scale = K_ROUND;
        corner_aspect = [corner_radii[1], corner_radii[0]];
    }
    let wxyz = |v: [f32; 4]| [v[3], v[0], v[1], v[2]];
    let yzwx = |v: [f32; 4]| [v[1], v[2], v[3], v[0]];
    let mut dx = [0.0f32; 4];
    let mut dy = [0.0f32; 4];
    {
        let xw = wxyz(xs);
        let yw = wxyz(ys);
        for i in 0..4 {
            dx[i] = xs[i] - xw[i];
            dy[i] = ys[i] - yw[i];
        }
    }
    for i in 0..4 {
        let inv = 1.0 / dx[i].abs().max(dy[i].abs().max(1.0));
        dx[i] *= inv;
        dy[i] *= inv;
    }
    let mut edge_sq = [0.0f32; 4];
    for i in 0..4 {
        edge_sq[i] = dx[i] * dx[i] + dy[i] * dy[i];
    }
    let edge_mask = edge_sq.map(sign);
    let mut edge_bias = [0.0f32; 4];
    let mut stroke_radius = [stroke_params[0]; 2];
    if edge_mask.contains(&0.0) {
        if edge_mask.iter().all(|&v| v == 0.0) {
            dx = [0.0, 1.0, 0.0, -1.0];
            dy = [-1.0, 0.0, 1.0, 0.0];
            edge_sq = [1.0; 4];
        } else {
            let triangle = (edge_mask[0] + edge_mask[1] + edge_mask[2] + edge_mask[3]) > 2.5;
            let (edge_x, edge_y) = if triangle {
                (yzwx(dx), yzwx(dy))
            } else {
                (yzwx(dy), yzwx(dx).map(|v| -v))
            };
            let mixv = |a: [f32; 4], b: [f32; 4], t: [f32; 4]| {
                [(b[0] - a[0]).mul_add(t[0], a[0]), (b[1] - a[1]).mul_add(t[1], a[1]), (b[2] - a[2]).mul_add(t[2], a[2]), (b[3] - a[3]).mul_add(t[3], a[3])]
            };
            dx = mixv(edge_x, dx, edge_mask);
            dy = mixv(edge_y, dy, edge_mask);
            edge_sq = mixv(yzwx(edge_sq), edge_sq, edge_mask);
            edge_aa = mixv(yzwx(edge_aa), edge_aa, edge_mask);
            if !triangle && join_scale == K_BEVEL {
                stroke_radius = [stroke_radius[0] * edge_mask[corner], stroke_radius[1] * edge_mask[next]];
                for i in 0..4 {
                    edge_bias[i] = (edge_mask[i] - 1.0) * stroke_params[0];
                }
                stroke_params[1] = 1.0;
                join_scale = K_MITER;
            }
        }
    }
    for i in 0..4 {
        let inv = 1.0 / edge_sq[i].sqrt();
        dx[i] *= inv;
        dy[i] *= inv;
    }
    let x_axis = [-dx[next], -dy[next]];
    let y_axis = [dx[corner], dy[corner]];
    let mut local = [0.0f32; 2];
    let mut snap_center = false;
    if normal_scale < 0.0 {
        if center[3] < 0.0 || center_weight * center[2] != 0.0 {
            snap_center = true;
        } else {
            let aa = center[3];
            let inset = if bidirectional {
                [corner_radii[0] - stroke_radius[0], corner_radii[1] - stroke_radius[1]]
            } else {
                [corner_radii[0] + stroke_radius[0], corner_radii[1] + stroke_radius[1]]
            };
            if join_scale == K_MITER || inset[0] <= aa || inset[1] <= aa {
                local = [inset[0] - aa, inset[1] - aa];
            } else {
                local = [inset[0] * position[0] - aa * normal[0], inset[1] * position[1] - aa * normal[1]];
            }
        }
    } else {
        let s = [corner_radii[0] + stroke_radius[0], corner_radii[1] + stroke_radius[1]];
        local = [s[0] * (position[0] + join_scale * position[1]), s[1] * (position[1] + join_scale * position[0])];
    }
    if snap_center {
        local = [center[0], center[1]];
    } else {
        local = [local[0] - corner_radii[0], local[1] - corner_radii[1]];
        local = [
            xs[corner] + x_axis[0] * local[0] + y_axis[0] * local[1],
            ys[corner] + x_axis[1] * local[0] + y_axis[1] * local[1],
        ];
    }
    let mut edge = [0.0f32; 4];
    for i in 0..4 {
        edge[i] = dy[i] * (xs[i] - local[0]) - dx[i] * (ys[i] - local[1]) + edge_bias[i];
    }
    let m = &inst.m;
    let inv = inverse3(m);
    let mut dev = [
        m[0][0] * local[0] + m[1][0] * local[1] + m[2][0],
        m[0][1] * local[0] + m[1][1] * local[1] + m[2][1],
        m[0][2] * local[0] + m[1][2] * local[1] + m[2][2],
    ];
    let mut jac = [
        inv[0][0] - inv[0][2] * local[0],
        inv[0][1] - inv[0][2] * local[1],
        inv[1][0] - inv[1][2] * local[0],
        inv[1][1] - inv[1][2] * local[1],
    ];
    let mut control = [0.0f32; 2];
    if device_space {
        let mut gx = [0.0f32; 4];
        let mut gy = [0.0f32; 4];
        for i in 0..4 {
            gx[i] = -dy[i] * (inv[0][0] - inv[0][2] * xs[i]) + dx[i] * (inv[0][1] - inv[0][2] * ys[i]);
            gy[i] = -dy[i] * (inv[1][0] - inv[1][2] * xs[i]) + dx[i] * (inv[1][1] - inv[1][2] * ys[i]);
        }
        for i in 0..4 {
            edge[i] *= 1.0 / (gx[i] * gx[i] + gy[i] * gy[i]).sqrt();
            edge[i] += (1.0 - edge_aa[i]) * dev[2].abs();
        }
        let dxs = yzwx(dx);
        let dys = yzwx(dy);
        let ortho: f32 = (0..4).map(|i| (dx[i] * dxs[i] + dy[i] * dys[i]).abs()).sum();
        let subpixel = edge_aa == [1.0; 4] && ortho < K_EPSILON;
        if subpixel {
            let dim = [edge[0] + edge[2], edge[1] + edge[3]];
            control[1] = 1.0 + dim[0].min(dim[1]).min(dev[2].abs());
        } else {
            control[1] = 1.0 + dev[2].abs();
        }
    }
    if normal_scale > 0.0 && dev[2] > 0.0 {
        // float2x2(jacobian): columns (jac.xy), (jac.zw); v * J = (dot(v, col0), dot(v, col1)).
        let vj = |v: [f32; 2]| [v[0] * jac[0] + v[1] * jac[1], v[0] * jac[2] + v[1] * jac[3]];
        let ean = [edge_aa[corner] * normal[0], edge_aa[next] * normal[1]];
        let perp = |v: [f32; 2]| [-v[1], v[0]];
        let px = vj(perp([-y_axis[0], -y_axis[1]]));
        let py = vj(perp(x_axis));
        let mut nx = [corner_aspect[0] * ean[0] * px[0], corner_aspect[0] * ean[0] * px[1]];
        let mut ny = [corner_aspect[1] * ean[1] * py[0], corner_aspect[1] * ean[1] * py[1]];
        let is_mid = ean[0] != 0.0 && ean[1] != 0.0;
        if join_scale == K_MITER && is_mid {
            let norm = |v: [f32; 2]| {
                let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
                [v[0] / l, v[1] / l]
            };
            nx = norm(nx);
            ny = norm(ny);
            if nx[0] * ny[0] + nx[1] * ny[1] < -0.8 {
                let s = sign(nx[0] * ny[1] - nx[1] * ny[0]);
                let pnx = perp(nx);
                let pny = perp(ny);
                nx = [s * pnx[0], s * pnx[1]];
                ny = [-s * pny[0], -s * pny[1]];
            }
        }
        let sum = [nx[0] + ny[0], nx[1] + ny[1]];
        let l = (sum[0] * sum[0] + sum[1] * sum[1]).sqrt();
        dev[0] += dev[2] * (sum[0] / l);
        dev[1] += dev[2] * (sum[1] / l);
        if device_space {
            for e in &mut edge {
                *e -= dev[2];
            }
        } else {
            control[1] = -dev[2];
        }
    } else if !device_space {
        control[1] = 0.0;
    }
    control[0] = if center_weight != 0.0 {
        1.0
    } else if bidirectional {
        -1.0
    } else {
        0.0
    };
    if stroked_line {
        // jacobian = float2x2(dy[0], -dy[1], -dx[0], dx[1]) * float2x2(jacobian).
        let a = [[dy[0], -dy[1]], [-dx[0], dx[1]]];
        let jm = [[jac[0], jac[1]], [jac[2], jac[3]]];
        let mut r = [[0.0f32; 2]; 2];
        for (col, rc) in r.iter_mut().enumerate() {
            for (row, v) in rc.iter_mut().enumerate() {
                *v = a[0][row] * jm[col][0] + a[1][row] * jm[col][1];
            }
        }
        jac = [r[0][0], r[0][1], r[1][0], r[1][1]];
    }
    Varyings {
        pos: [dev[0] / dev[2], dev[1] / dev[2]],
        jacobian: jac,
        edge,
        x_radii,
        y_radii,
        stroke: stroke_params,
        control,
    }
}

#[inline]
fn coverage_bias(scale: f32) -> f32 {
    1.0 - 0.5 * scale
}

#[inline]
fn inverse_grad_len(g: [f32; 2], j: &[f32; 4]) -> f32 {
    let d = [g[0] * j[0] + g[1] * j[1], g[0] * j[2] + g[1] * j[3]];
    1.0 / (d[0] * d[0] + d[1] * d[1]).sqrt()
}

fn elliptical_distance(uv: [f32; 2], radii: [f32; 2], stroke_radius: f32, j: &[f32; 4]) -> [f32; 2] {
    let inv_r2 = [1.0 / (radii[0] * radii[0] + stroke_radius * stroke_radius), 1.0 / (radii[1] * radii[1] + stroke_radius * stroke_radius)];
    let norm_uv = [inv_r2[0] * uv[0], inv_r2[1] * uv[1]];
    let igl = inverse_grad_len(norm_uv, j);
    let f = 0.5 * igl * ((uv[0] * norm_uv[0] + uv[1] * norm_uv[1]) - 1.0);
    let width = radii[0] * stroke_radius * inv_r2[0] * igl;
    [width - f, width + f]
}

fn corner_distance(dist: &mut [f32; 2], j: &[f32; 4], stroke: [f32; 2], edge: [f32; 2], flip: [f32; 2], radii: [f32; 2]) {
    let uv = [radii[0] - edge[0], radii[1] - edge[1]];
    if uv[0] > 0.0 && uv[1] > 0.0 {
        if (radii[0] > 0.0 && radii[1] > 0.0) || (stroke[0] > 0.0 && stroke[1] < 0.0) {
            let mut d = elliptical_distance([uv[0] * flip[0], uv[1] * flip[1]], radii, stroke[0], j);
            d[1] = if radii[0] - stroke[0] <= 0.0 { 1.0 } else { -d[1] };
            dist[0] = dist[0].min(d[0]);
            dist[1] = dist[1].min(d[1]);
        } else if stroke[1] == 0.0 {
            let bevel = (stroke[0] - uv[0] - uv[1]) * inverse_grad_len(flip, j);
            dist[0] = dist[0].min(bevel);
        }
    }
}

/// `analytic_rrect_coverage_fn` (with `coords.w` = 1).
pub fn coverage(v: &Varyings) -> f32 {
    let e = v.edge;
    if v.control[0] > 0.0 {
        return 1.0;
    }
    if v.control[1] > 1.0 {
        let outer = [e[0].min(e[2]), e[1].min(e[3])];
        let c = outer[0].min(outer[1]);
        let scale = v.control[1] - 1.0;
        let bias = coverage_bias(scale);
        return (scale * (c + bias)).clamp(0.0, 1.0);
    }
    let j = v.jacobian;
    let s = v.stroke;
    let igl = [inverse_grad_len([1.0, 0.0], &j), inverse_grad_len([0.0, 1.0], &j)];
    let outer = [igl[0] * (s[0] + e[0].min(e[2])), igl[1] * (s[0] + e[1].min(e[3]))];
    let mut d = [outer[0].min(outer[1]), -1.0f32];
    let (scale, bias);
    if v.control[0] > -0.95 {
        let dim = [igl[0] * (e[0] + e[2] + 2.0 * s[0]), igl[1] * (e[1] + e[3] + 2.0 * s[0])];
        scale = dim[0].min(dim[1]).min(1.0);
        bias = coverage_bias(scale);
    } else {
        let sw = [2.0 * s[0] * igl[0], 2.0 * s[0] * igl[1]];
        let inner = [sw[0] - outer[0], sw[1] - outer[1]];
        d[1] = -inner[0].max(inner[1]);
        if s[0] > 0.0 {
            let narrow = sw[0].min(sw[1]);
            let dim = [if inner[0] >= -0.5 { sw[0] } else { narrow }, if inner[1] >= -0.5 { sw[1] } else { narrow }];
            scale = dim[0].max(dim[1]).clamp(0.0, 1.0);
            bias = coverage_bias(scale);
        } else {
            scale = 1.0;
            bias = 1.0;
        }
    }
    corner_distance(&mut d, &j, s, [e[0], e[1]], [-1.0, -1.0], [v.x_radii[0], v.y_radii[0]]);
    corner_distance(&mut d, &j, s, [e[2], e[1]], [1.0, -1.0], [v.x_radii[1], v.y_radii[1]]);
    corner_distance(&mut d, &j, s, [e[2], e[3]], [1.0, 1.0], [v.x_radii[2], v.y_radii[2]]);
    corner_distance(&mut d, &j, s, [e[0], e[3]], [-1.0, 1.0], [v.x_radii[3], v.y_radii[3]]);
    let outset = v.control[1].min(0.0);
    let fc = scale * ((d[0] + outset).min(-d[1]) + bias);
    fc.clamp(0.0, 1.0)
}

/// Per-sample coverage of one instance: for each covered sample, the
/// fragment coverage of the first triangle (in primitive order) covering it.
pub struct Samples {
    pub width: u32,
    pub height: u32,
    /// `width × height × samples per pixel`; `None` where no triangle covers
    /// the sample.
    pub coverage: Vec<Option<f32>>,
}

/// Rasterizes an instance's mesh.
pub fn rasterize(inst: &Instance, width: u32, height: u32, pattern: &[[f64; 2]]) -> Samples {
    let verts: Vec<Varyings> = (0..36).map(|i| vertex(inst, i)).collect();
    let n = pattern.len();
    let mut out = Samples { width, height, coverage: vec![None; (width * height) as usize * n] };
    let one = 1i64 << SUBPIXEL_BITS;
    for k in 2..INDICES.len() {
        let tri = [verts[INDICES[k - 2] as usize], verts[INDICES[k - 1] as usize], verts[INDICES[k] as usize]];
        let p: Vec<(i64, i64)> = tri.iter().map(|v| (snap(f64::from(v.pos[0])), snap(f64::from(v.pos[1])))).collect();
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
        if area == 0 {
            continue;
        }
        let minx = p.iter().map(|q| q.0).min().unwrap().div_euclid(one).max(0);
        let maxx = p.iter().map(|q| q.0).max().unwrap().div_euclid(one).min(i64::from(width) - 1);
        let miny = p.iter().map(|q| q.1).min().unwrap().div_euclid(one).max(0);
        let maxy = p.iter().map(|q| q.1).max().unwrap().div_euclid(one).min(i64::from(height) - 1);
        if maxx < minx || maxy < miny {
            continue;
        }
        // Barycentric weights from the snapped positions (the rasterizer's
        // plane equations), evaluated at pixel centers.
        let (x0, y0) = (p[0].0 as f64, p[0].1 as f64);
        let (x1, y1) = (p[1].0 as f64, p[1].1 as f64);
        let (x2, y2) = (p[2].0 as f64, p[2].1 as f64);
        let det = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
        for py in miny..=maxy {
            for px in minx..=maxx {
                let mut covered = [false; 4];
                let mut any = false;
                for (si, s) in pattern.iter().enumerate() {
                    let idx = (py as u32 * width + px as u32) as usize * n + si;
                    if out.coverage[idx].is_some() {
                        continue;
                    }
                    let sx = px * one + (s[0] * one as f64) as i64;
                    let sy = py * one + (s[1] * one as f64) as i64;
                    if inside_triangle(&p, sx, sy) {
                        covered[si] = true;
                        any = true;
                    }
                }
                if !any {
                    continue;
                }
                // Varyings interpolated at the pixel center.
                let cx = (px as f64 + 0.5) * one as f64;
                let cy = (py as f64 + 0.5) * one as f64;
                let b1 = ((cx - x0) * (y2 - y0) - (x2 - x0) * (cy - y0)) / det;
                let b2 = ((x1 - x0) * (cy - y0) - (cx - x0) * (y1 - y0)) / det;
                let b0 = 1.0 - b1 - b2;
                let lerp = |a: f32, b: f32, c: f32| (b0 * f64::from(a) + b1 * f64::from(b) + b2 * f64::from(c)) as f32;
                let mut v = tri[0];
                for i in 0..4 {
                    v.jacobian[i] = lerp(tri[0].jacobian[i], tri[1].jacobian[i], tri[2].jacobian[i]);
                    v.edge[i] = lerp(tri[0].edge[i], tri[1].edge[i], tri[2].edge[i]);
                }
                for i in 0..2 {
                    v.control[i] = lerp(tri[0].control[i], tri[1].control[i], tri[2].control[i]);
                }
                let c = coverage(&v);
                for (si, &cov) in covered.iter().enumerate().take(n) {
                    if cov {
                        out.coverage[(py as u32 * width + px as u32) as usize * n + si] = Some(c);
                    }
                }
            }
        }
    }
    out
}

/// The top-left sample test for one triangle in grid units.
fn inside_triangle(p: &[(i64, i64)], sx: i64, sy: i64) -> bool {
    let mut w = 0i32;
    for i in 0..3 {
        let (a, b) = (p[i], p[(i + 1) % 3]);
        if a.1 == b.1 {
            continue;
        }
        let (x0, y0, x1, y1, wd) = if a.1 < b.1 { (a.0, a.1, b.0, b.1, 1) } else { (b.0, b.1, a.0, a.1, -1) };
        if sy < y0 || sy >= y1 {
            continue;
        }
        let den = y1 - y0;
        let num = x0 * den + (x1 - x0) * (sy - y0);
        if num <= sx * den {
            w += wd;
        }
    }
    w != 0
}
