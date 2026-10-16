//! three.js r180 `LatheGeometry` and `ExtrudeGeometry` (with
//! `Shape.extractPoints`, `ShapeUtils.triangulateShape`, and the
//! `WorldUVGenerator`), as `primitiveGeometry` builds them for
//! `{ lathe, segments }` and `{ extrude, depth, bevel }` sources:
//! `new LatheGeometry(points, segments || 48)` and
//! `new ExtrudeGeometry(new Shape(points), { depth, steps: 1,
//! bevelEnabled: !!bevel, bevelSegments: 3, bevelThickness: bevel || 0,
//! bevelSize: bevel || 0 })`.

use std::f64::consts::PI;

use crate::earcut::earcut;
use crate::primitives::{Buffers, Geometry, Group, normalize, sequential, to_f32};

type V2 = [f64; 2];

/// `new LatheGeometry(points, segments)` (full turn).
pub fn lathe_geometry(points: &[V2], segments: f64) -> Geometry {
    let segments = segments.floor();
    let phi_length = PI * 2.0;
    let inverse_segments = 1.0 / segments;
    let n = points.len();
    let mut b = Buffers::default();
    let mut init_normals: Vec<f64> = Vec::with_capacity(n * 3);
    let mut prev = [0.0f64; 3];
    for j in 0..n {
        if j == 0 {
            let dx = points[1][0] - points[0][0];
            let dy = points[1][1] - points[0][1];
            let normal = [dy * 1.0, -dx, dy * 0.0];
            prev = normal;
            init_normals.extend_from_slice(&normalize(normal));
        } else if j == n - 1 {
            init_normals.extend_from_slice(&prev);
        } else {
            let dx = points[j + 1][0] - points[j][0];
            let dy = points[j + 1][1] - points[j][1];
            let cur = [dy * 1.0, -dx, dy * 0.0];
            let normal = [cur[0] + prev[0], cur[1] + prev[1], cur[2] + prev[2]];
            init_normals.extend_from_slice(&normalize(normal));
            prev = cur;
        }
    }
    let sn = segments as u32;
    for i in 0..=sn {
        let phi = f64::from(i) * inverse_segments * phi_length;
        let sin = libm::sin(phi);
        let cos = libm::cos(phi);
        for (j, p) in points.iter().enumerate() {
            b.vertices.extend_from_slice(&[p[0] * sin, p[1], p[0] * cos]);
            b.uvs.push(f64::from(i) / segments);
            b.uvs.push(j as f64 / (n as f64 - 1.0));
            b.normals.extend_from_slice(&[
                init_normals[3 * j] * sin,
                init_normals[3 * j + 1],
                init_normals[3 * j] * cos,
            ]);
        }
    }
    let n32 = n as u32;
    for i in 0..sn {
        for j in 0..n32.saturating_sub(1) {
            let base = j + i * n32;
            let (a, bb, c, d) = (base, base + n32, base + n32 + 1, base + 1);
            b.indices.extend_from_slice(&[a, bb, d, c, d, bb]);
        }
    }
    b.finish()
}

/// `CurvePath.getPoints` for a `Shape` made of line segments: the points
/// with consecutive duplicates removed.
fn shape_points(points: &[V2]) -> Vec<V2> {
    let mut out: Vec<V2> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_some_and(|l| l[0] == p[0] && l[1] == p[1]) {
            continue;
        }
        out.push(*p);
    }
    out
}

/// `ShapeUtils.area`.
fn area(contour: &[V2]) -> f64 {
    let n = contour.len();
    let mut a = 0.0;
    let mut p = n.wrapping_sub(1);
    for q in 0..n {
        a += contour[p][0] * contour[q][1] - contour[q][0] * contour[p][1];
        p = q;
    }
    a * 0.5
}

/// `mergeOverlappingPoints`.
fn merge_overlapping(points: &mut Vec<V2>) {
    if points.is_empty() {
        return;
    }
    const THRESHOLD_SQ: f64 = 1e-10 * 1e-10;
    let mut prev = points[0];
    let mut i = 1;
    while i <= points.len() {
        let ci = i % points.len();
        let cur = points[ci];
        let dx = cur[0] - prev[0];
        let dy = cur[1] - prev[1];
        let dist_sq = dx * dx + dy * dy;
        let scale = cur[0]
            .abs()
            .max(cur[1].abs())
            .max(prev[0].abs())
            .max(prev[1].abs());
        if dist_sq <= THRESHOLD_SQ * scale * scale {
            points.remove(ci);
            continue;
        }
        prev = cur;
        i += 1;
    }
}

/// `getBevelVec(inPt, inPrev, inNext)`.
fn bevel_vec(pt: V2, prev: V2, next: V2) -> V2 {
    let v_prev_x = pt[0] - prev[0];
    let v_prev_y = pt[1] - prev[1];
    let v_next_x = next[0] - pt[0];
    let v_next_y = next[1] - pt[1];
    let v_prev_lensq = v_prev_x * v_prev_x + v_prev_y * v_prev_y;
    let collinear0 = v_prev_x * v_next_y - v_prev_y * v_next_x;
    let (tx, ty, shrink);
    if collinear0.abs() > f64::EPSILON {
        let v_prev_len = v_prev_lensq.sqrt();
        let v_next_len = (v_next_x * v_next_x + v_next_y * v_next_y).sqrt();
        let prev_shift_x = prev[0] - v_prev_y / v_prev_len;
        let prev_shift_y = prev[1] + v_prev_x / v_prev_len;
        let next_shift_x = next[0] - v_next_y / v_next_len;
        let next_shift_y = next[1] + v_next_x / v_next_len;
        let sf = ((next_shift_x - prev_shift_x) * v_next_y
            - (next_shift_y - prev_shift_y) * v_next_x)
            / (v_prev_x * v_next_y - v_prev_y * v_next_x);
        let x = prev_shift_x + v_prev_x * sf - pt[0];
        let y = prev_shift_y + v_prev_y * sf - pt[1];
        let lensq = x * x + y * y;
        if lensq <= 2.0 {
            return [x, y];
        }
        tx = x;
        ty = y;
        shrink = (lensq / 2.0).sqrt();
    } else {
        let direction_eq = if v_prev_x > f64::EPSILON {
            v_next_x > f64::EPSILON
        } else if v_prev_x < -f64::EPSILON {
            v_next_x < -f64::EPSILON
        } else {
            js_sign(v_prev_y) == js_sign(v_next_y)
        };
        if direction_eq {
            tx = -v_prev_y;
            ty = v_prev_x;
            shrink = v_prev_lensq.sqrt();
        } else {
            tx = v_prev_x;
            ty = v_prev_y;
            shrink = (v_prev_lensq / 2.0).sqrt();
        }
    }
    [tx / shrink, ty / shrink]
}

/// `Math.sign` (NaN compares unequal to everything, as `===` does).
fn js_sign(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}

/// `scalePt2(pt, vec, size)`: `pt.clone().addScaledVector(vec, size)`.
fn scale_pt(pt: V2, vec: V2, size: f64) -> V2 {
    [pt[0] + vec[0] * size, pt[1] + vec[1] * size]
}

/// `ShapeUtils.triangulateShape(contour, [])` (which drops a closing
/// duplicate of the first point from `contour`).
fn triangulate(contour: &mut Vec<V2>) -> Vec<[usize; 3]> {
    let l = contour.len();
    if l > 2 && contour[l - 1] == contour[0] {
        contour.pop();
    }
    let data: Vec<f64> = contour.iter().flat_map(|p| [p[0], p[1]]).collect();
    earcut(&data, &[])
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect()
}

/// The `{ extrude, depth, bevel }` source: an `ExtrudeGeometry` of one
/// shape without holes, extruded along +z, one step, three bevel segments
/// when `bevel > 0`. Not indexed; groups: lids (0), side walls (1).
pub fn extrude_geometry(points: &[V2], depth: f64, bevel: f64) -> Geometry {
    let bevel_enabled = bevel != 0.0 && !bevel.is_nan();
    let steps = 1.0f64;
    let steps_n = 1usize;
    let (bevel_segments, bevel_thickness, bevel_size, bevel_offset) = if bevel_enabled {
        (3usize, bevel, bevel, 0.0)
    } else {
        (0usize, 0.0, 0.0, 0.0)
    };
    let mut vertices = shape_points(points);
    let a = area(&vertices);
    if a.is_nan() || a >= 0.0 {
        vertices.reverse();
    }
    merge_overlapping(&mut vertices);
    // `contour` and `vertices` are one array (the shape has no holes).
    let mut contour = vertices;
    let vlen = contour.len();
    let n = contour.len();
    let mut movements: Vec<V2> = Vec::with_capacity(n);
    for i in 0..n {
        let j = if i == 0 { n - 1 } else { i - 1 };
        let k = if i + 1 == n { 0 } else { i + 1 };
        movements.push(bevel_vec(contour[i], contour[j], contour[k]));
    }
    let mut placeholder: Vec<f64> = Vec::new();
    let faces = if bevel_segments == 0 {
        triangulate(&mut contour)
    } else {
        let mut contracted: Vec<V2> = Vec::new();
        for b in 0..bevel_segments {
            let t = b as f64 / bevel_segments as f64;
            let z = bevel_thickness * libm::cos(t * PI / 2.0);
            let bs = bevel_size * libm::sin(t * PI / 2.0) + bevel_offset;
            for i in 0..contour.len() {
                let vert = scale_pt(contour[i], movements[i], bs);
                placeholder.extend_from_slice(&[vert[0], vert[1], -z]);
                if t == 0.0 {
                    contracted.push(vert);
                }
            }
        }
        triangulate(&mut contracted)
    };
    let bs = bevel_size + bevel_offset;
    let at = |vertices: &[V2], i: usize| {
        if bevel_enabled {
            scale_pt(vertices[i], movements[i], bs)
        } else {
            vertices[i]
        }
    };
    for i in 0..vlen {
        let v = at(&contour, i);
        placeholder.extend_from_slice(&[v[0], v[1], 0.0]);
    }
    for s in 1..=steps_n {
        for i in 0..vlen {
            let v = at(&contour, i);
            placeholder.extend_from_slice(&[v[0], v[1], depth / steps * s as f64]);
        }
    }
    for b in (0..bevel_segments).rev() {
        let t = b as f64 / bevel_segments as f64;
        let z = bevel_thickness * libm::cos(t * PI / 2.0);
        let bs = bevel_size * libm::sin(t * PI / 2.0) + bevel_offset;
        for i in 0..contour.len() {
            let vert = scale_pt(contour[i], movements[i], bs);
            placeholder.extend_from_slice(&[vert[0], vert[1], depth + z]);
        }
    }
    let mut out: Vec<f64> = Vec::new();
    let mut uvs: Vec<f64> = Vec::new();
    let add = |out: &mut Vec<f64>, index: usize| {
        out.extend_from_slice(&placeholder[index * 3..index * 3 + 3]);
    };
    let f3 = |out: &mut Vec<f64>, uvs: &mut Vec<f64>, a: usize, b: usize, c: usize| {
        for i in [a, b, c] {
            add(out, i);
        }
        let next = out.len() / 3;
        for i in next - 3..next {
            uvs.push(out[i * 3]);
            uvs.push(out[i * 3 + 1]);
        }
    };
    let f4 = |out: &mut Vec<f64>, uvs: &mut Vec<f64>, a: usize, b: usize, c: usize, d: usize| {
        for i in [a, b, d, b, c, d] {
            add(out, i);
        }
        let next = out.len() / 3;
        let p = |i: usize| [out[i * 3], out[i * 3 + 1], out[i * 3 + 2]];
        let (pa, pb, pc, pd) = (p(next - 6), p(next - 3), p(next - 2), p(next - 1));
        let wall: [V2; 4] = if (pa[1] - pb[1]).abs() < (pa[0] - pb[0]).abs() {
            [
                [pa[0], 1.0 - pa[2]],
                [pb[0], 1.0 - pb[2]],
                [pc[0], 1.0 - pc[2]],
                [pd[0], 1.0 - pd[2]],
            ]
        } else {
            [
                [pa[1], 1.0 - pa[2]],
                [pb[1], 1.0 - pb[2]],
                [pc[1], 1.0 - pc[2]],
                [pd[1], 1.0 - pd[2]],
            ]
        };
        for k in [0, 1, 3, 1, 2, 3] {
            uvs.extend_from_slice(&wall[k]);
        }
    };
    let mut groups = Vec::new();
    // Lids.
    let start = out.len() / 3;
    if bevel_enabled {
        for f in &faces {
            f3(&mut out, &mut uvs, f[2], f[1], f[0]);
        }
        let offset = vlen * (steps_n + bevel_segments * 2);
        for f in &faces {
            f3(&mut out, &mut uvs, f[0] + offset, f[1] + offset, f[2] + offset);
        }
    } else {
        for f in &faces {
            f3(&mut out, &mut uvs, f[2], f[1], f[0]);
        }
        let offset = vlen * steps_n;
        for f in &faces {
            f3(&mut out, &mut uvs, f[0] + offset, f[1] + offset, f[2] + offset);
        }
    }
    groups.push(Group {
        start: start as u32,
        count: (out.len() / 3 - start) as u32,
        material_index: 0,
    });
    // Side walls.
    let start = out.len() / 3;
    let layers = steps_n + bevel_segments * 2;
    let cl = contour.len();
    for i in (0..cl).rev() {
        let j = i;
        let k = if i == 0 { cl - 1 } else { i - 1 };
        for s in 0..layers {
            let slen1 = vlen * s;
            let slen2 = vlen * (s + 1);
            f4(
                &mut out,
                &mut uvs,
                j + slen1,
                k + slen1,
                k + slen2,
                j + slen2,
            );
        }
    }
    groups.push(Group {
        start: start as u32,
        count: (out.len() / 3 - start) as u32,
        material_index: 1,
    });
    let positions = to_f32(&out);
    let count = positions.len() / 3;
    let normals = crate::normals::compute_vertex_normals(&positions, None);
    Geometry {
        positions,
        normals,
        uvs: to_f32(&uvs),
        indices: sequential(count),
        indexed: false,
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lathe_counts() {
        let g = lathe_geometry(&[[0.0, -0.5], [0.5, 0.0], [0.0, 0.5]], 12.0);
        assert_eq!(g.vertex_count(), 13 * 3);
        assert_eq!(g.indices.len(), 12 * 2 * 6);
    }

    #[test]
    fn extruded_square_has_lids_and_walls() {
        let sq = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let g = extrude_geometry(&sq, 0.5, 0.0);
        // Two lids of two triangles, four walls of two triangles.
        assert_eq!(g.vertex_count(), (2 * 2 + 4 * 2) * 3);
        assert_eq!(g.groups.len(), 2);
        assert_eq!(g.groups[0].count, 12);
        let b = extrude_geometry(&sq, 0.5, 0.1);
        // Lids, then (1 + 2·3) wall layers.
        assert_eq!(b.vertex_count(), (2 * 2 + 4 * 7 * 2) * 3);
    }
}

