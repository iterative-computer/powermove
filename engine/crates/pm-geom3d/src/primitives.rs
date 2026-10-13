//! three.js r180 primitive geometry (`BoxGeometry`, `SphereGeometry`,
//! `PlaneGeometry`, `CylinderGeometry`, `ConeGeometry`, `TorusGeometry`), as
//! `primitiveGeometry` in `core/scene3d/runtime.ts` builds them.
//!
//! Every value is computed in f64 in three.js's operation order and stored as
//! f32, as `Float32BufferAttribute` stores JavaScript numbers, so the buffers
//! equal three.js's bit for bit (checked against three.js itself by
//! `tests/three_primitives.rs`). Transcendentals use `libm` (plan section 7,
//! "Determinism"); V8's fdlibm differs from it by at most an ulp of f64, far
//! below f32's resolution.

use std::f64::consts::PI;

/// Indexed triangle geometry with three.js's standard attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    /// xyz per vertex.
    pub positions: Vec<f32>,
    /// xyz per vertex.
    pub normals: Vec<f32>,
    /// uv per vertex.
    pub uvs: Vec<f32>,
    /// Triangle list.
    pub indices: Vec<u32>,
}

impl Geometry {
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Axis-aligned bounds of the positions (`computeBoundingBox`).
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut it = self.positions.chunks_exact(3);
        let first = it.next()?;
        let mut lo = [first[0], first[1], first[2]];
        let mut hi = lo;
        for p in it {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        Some((lo, hi))
    }
}

/// A primitive as the scene schema stores it, with JavaScript's defaults
/// already applied (`p.width || 1` and so on; see [`Primitive::from_parameters`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Primitive {
    Box { width: f64, height: f64, depth: f64 },
    Sphere { radius: f64, width_segments: f64, height_segments: f64 },
    Plane { width: f64, height: f64 },
    Cylinder { radius_top: f64, radius_bottom: f64, height: f64, radial_segments: f64 },
    Cone { radius: f64, height: f64, radial_segments: f64 },
    Torus { radius: f64, tube: f64, radial_segments: f64, tubular_segments: f64 },
}

/// The primitive's parameters (`source.parameters`), each `None` when absent.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Parameters {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub depth: Option<f64>,
    pub radius: Option<f64>,
    pub radius_top: Option<f64>,
    pub radius_bottom: Option<f64>,
    pub tube: Option<f64>,
    pub segments: Option<f64>,
    pub detail: Option<f64>,
}

/// JavaScript `v || fallback` for a number (`0`, `NaN`, and absent fall back).
fn or(v: Option<f64>, fallback: f64) -> f64 {
    match v {
        Some(x) if x != 0.0 && !x.is_nan() => x,
        _ => fallback,
    }
}

/// JavaScript `a ?? b` for numbers (`null` and `undefined` are `None`).
fn nullish(a: Option<f64>, b: f64) -> f64 {
    a.unwrap_or(b)
}

impl Primitive {
    /// `primitiveGeometry(source)` for `{ primitive, parameters }`. `None`
    /// for primitives this module does not build (`capsule`,
    /// `icosahedron`) and unknown names.
    pub fn from_parameters(name: &str, p: &Parameters) -> Option<Primitive> {
        let s = or(p.segments, 48.0);
        Some(match name {
            "box" => Primitive::Box {
                width: or(p.width, 1.0),
                height: or(p.height, 1.0),
                depth: or(p.depth, 1.0),
            },
            "sphere" => Primitive::Sphere {
                radius: or(p.radius, 0.5),
                width_segments: s,
                height_segments: 3f64.max((s / 2.0).floor()),
            },
            "plane" => Primitive::Plane {
                width: or(p.width, 4.0),
                height: or(p.height, 4.0),
            },
            "cone" => Primitive::Cone {
                radius: or(p.radius, 0.5),
                height: or(p.height, 1.0),
                radial_segments: s,
            },
            "cylinder" => Primitive::Cylinder {
                radius_top: nullish(p.radius_top, nullish(p.radius, 0.5)),
                radius_bottom: nullish(p.radius_bottom, nullish(p.radius, 0.5)),
                height: or(p.height, 1.0),
                radial_segments: s,
            },
            "torus" => Primitive::Torus {
                radius: or(p.radius, 0.6),
                tube: or(p.tube, 0.18),
                radial_segments: 8f64.max((s / 2.0).floor()),
                tubular_segments: s,
            },
            _ => return None,
        })
    }

    /// The geometry three.js builds.
    pub fn geometry(&self) -> Geometry {
        match *self {
            Primitive::Box {
                width,
                height,
                depth,
            } => box_geometry(width, height, depth),
            Primitive::Sphere {
                radius,
                width_segments,
                height_segments,
            } => sphere_geometry(radius, width_segments, height_segments),
            Primitive::Plane { width, height } => plane_geometry(width, height),
            Primitive::Cylinder {
                radius_top,
                radius_bottom,
                height,
                radial_segments,
            } => cylinder_geometry(radius_top, radius_bottom, height, radial_segments),
            Primitive::Cone {
                radius,
                height,
                radial_segments,
            } => cylinder_geometry(0.0, radius, height, radial_segments),
            Primitive::Torus {
                radius,
                tube,
                radial_segments,
                tubular_segments,
            } => torus_geometry(radius, tube, radial_segments, tubular_segments),
        }
    }
}

#[derive(Default)]
struct Buffers {
    vertices: Vec<f64>,
    normals: Vec<f64>,
    uvs: Vec<f64>,
    indices: Vec<u32>,
}

impl Buffers {
    fn finish(self) -> Geometry {
        let f = |v: Vec<f64>| v.into_iter().map(|x| x as f32).collect();
        Geometry {
            positions: f(self.vertices),
            normals: f(self.normals),
            uvs: f(self.uvs),
            indices: self.indices,
        }
    }
}

/// `Vector3.normalize()`: `divideScalar(length() || 1)`, which multiplies by
/// the reciprocal.
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let len = if len == 0.0 || len.is_nan() { 1.0 } else { len };
    let s = 1.0 / len;
    [v[0] * s, v[1] * s, v[2] * s]
}

/// `new BoxGeometry(width, height, depth)` (one segment per side).
pub fn box_geometry(width: f64, height: f64, depth: f64) -> Geometry {
    let mut b = Buffers::default();
    let mut vertices_so_far = 0u32;
    // (u, v, w) axes, udir, vdir, width, height, depth.
    let planes: [(usize, usize, usize, f64, f64, f64, f64, f64); 6] = [
        (2, 1, 0, -1.0, -1.0, depth, height, width),
        (2, 1, 0, 1.0, -1.0, depth, height, -width),
        (0, 2, 1, 1.0, 1.0, width, depth, height),
        (0, 2, 1, 1.0, -1.0, width, depth, -height),
        (0, 1, 2, 1.0, -1.0, width, height, depth),
        (0, 1, 2, -1.0, -1.0, width, height, -depth),
    ];
    for (u, v, w, udir, vdir, pw, ph, pd) in planes {
        let (grid_x, grid_y) = (1.0f64, 1.0f64);
        let segment_width = pw / grid_x;
        let segment_height = ph / grid_y;
        let width_half = pw / 2.0;
        let height_half = ph / 2.0;
        let depth_half = pd / 2.0;
        let mut count = 0;
        for iy in 0..2 {
            let y = f64::from(iy) * segment_height - height_half;
            for ix in 0..2 {
                let x = f64::from(ix) * segment_width - width_half;
                let mut vector = [0.0f64; 3];
                vector[u] = x * udir;
                vector[v] = y * vdir;
                vector[w] = depth_half;
                b.vertices.extend_from_slice(&vector);
                vector[u] = 0.0;
                vector[v] = 0.0;
                vector[w] = if pd > 0.0 { 1.0 } else { -1.0 };
                b.normals.extend_from_slice(&vector);
                b.uvs.push(f64::from(ix) / grid_x);
                b.uvs.push(1.0 - f64::from(iy) / grid_y);
                count += 1;
            }
        }
        let gx1 = 2u32;
        let (ix, iy) = (0u32, 0u32);
        let a = vertices_so_far + ix + gx1 * iy;
        let bb = vertices_so_far + ix + gx1 * (iy + 1);
        let c = vertices_so_far + (ix + 1) + gx1 * (iy + 1);
        let d = vertices_so_far + (ix + 1) + gx1 * iy;
        b.indices.extend_from_slice(&[a, bb, d, bb, c, d]);
        vertices_so_far += count;
    }
    b.finish()
}

/// `new SphereGeometry(radius, widthSegments, heightSegments)` (full sphere).
pub fn sphere_geometry(radius: f64, width_segments: f64, height_segments: f64) -> Geometry {
    let ws = 3f64.max(width_segments.floor());
    let hs = 2f64.max(height_segments.floor());
    let (phi_start, phi_length) = (0.0f64, PI * 2.0);
    let (theta_start, theta_length) = (0.0f64, PI);
    let theta_end = (theta_start + theta_length).min(PI);
    let (wn, hn) = (ws as u32, hs as u32);
    let mut b = Buffers::default();
    for iy in 0..=hn {
        let v = f64::from(iy) / hs;
        let mut u_offset = 0.0;
        if iy == 0 && theta_start == 0.0 {
            u_offset = 0.5 / ws;
        } else if iy == hn && theta_end == PI {
            u_offset = -0.5 / ws;
        }
        for ix in 0..=wn {
            let u = f64::from(ix) / ws;
            let x = -radius
                * libm::cos(phi_start + u * phi_length)
                * libm::sin(theta_start + v * theta_length);
            let y = radius * libm::cos(theta_start + v * theta_length);
            let z = radius
                * libm::sin(phi_start + u * phi_length)
                * libm::sin(theta_start + v * theta_length);
            b.vertices.extend_from_slice(&[x, y, z]);
            b.normals.extend_from_slice(&normalize([x, y, z]));
            b.uvs.push(u + u_offset);
            b.uvs.push(1.0 - v);
        }
    }
    let row = wn + 1;
    let grid = |iy: u32, ix: u32| iy * row + ix;
    for iy in 0..hn {
        for ix in 0..wn {
            let a = grid(iy, ix + 1);
            let bb = grid(iy, ix);
            let c = grid(iy + 1, ix);
            let d = grid(iy + 1, ix + 1);
            if iy != 0 || theta_start > 0.0 {
                b.indices.extend_from_slice(&[a, bb, d]);
            }
            if iy != hn - 1 || theta_end < PI {
                b.indices.extend_from_slice(&[bb, c, d]);
            }
        }
    }
    b.finish()
}

/// `new PlaneGeometry(width, height)` (one segment).
pub fn plane_geometry(width: f64, height: f64) -> Geometry {
    let width_half = width / 2.0;
    let height_half = height / 2.0;
    let (grid_x, grid_y) = (1.0f64, 1.0f64);
    let segment_width = width / grid_x;
    let segment_height = height / grid_y;
    let mut b = Buffers::default();
    for iy in 0..2 {
        let y = f64::from(iy) * segment_height - height_half;
        for ix in 0..2 {
            let x = f64::from(ix) * segment_width - width_half;
            b.vertices.extend_from_slice(&[x, -y, 0.0]);
            b.normals.extend_from_slice(&[0.0, 0.0, 1.0]);
            b.uvs.push(f64::from(ix) / grid_x);
            b.uvs.push(1.0 - f64::from(iy) / grid_y);
        }
    }
    b.indices.extend_from_slice(&[0, 2, 1, 2, 3, 1]);
    b.finish()
}

/// `new CylinderGeometry(radiusTop, radiusBottom, height, radialSegments)`
/// (one height segment, closed, full turn); `ConeGeometry` is a cylinder
/// with a zero top radius.
pub fn cylinder_geometry(
    radius_top: f64,
    radius_bottom: f64,
    height: f64,
    radial_segments: f64,
) -> Geometry {
    let radial = radial_segments.floor();
    let height_segments = 1.0f64;
    let (theta_start, theta_length) = (0.0f64, PI * 2.0);
    let rn = radial as u32;
    let hn = height_segments as u32;
    let half_height = height / 2.0;
    let mut b = Buffers::default();
    let mut index = 0u32;
    // Torso.
    let slope = (radius_bottom - radius_top) / height;
    let mut index_array: Vec<Vec<u32>> = Vec::new();
    for y in 0..=hn {
        let mut row = Vec::new();
        let v = f64::from(y) / height_segments;
        let radius = v * (radius_bottom - radius_top) + radius_top;
        for x in 0..=rn {
            let u = f64::from(x) / radial;
            let theta = u * theta_length + theta_start;
            let sin_theta = libm::sin(theta);
            let cos_theta = libm::cos(theta);
            b.vertices
                .extend_from_slice(&[radius * sin_theta, -v * height + half_height, radius * cos_theta]);
            b.normals
                .extend_from_slice(&normalize([sin_theta, slope, cos_theta]));
            b.uvs.push(u);
            b.uvs.push(1.0 - v);
            row.push(index);
            index += 1;
        }
        index_array.push(row);
    }
    for x in 0..rn as usize {
        for y in 0..hn as usize {
            let a = index_array[y][x];
            let bb = index_array[y + 1][x];
            let c = index_array[y + 1][x + 1];
            let d = index_array[y][x + 1];
            if radius_top > 0.0 || y != 0 {
                b.indices.extend_from_slice(&[a, bb, d]);
            }
            if radius_bottom > 0.0 || y != hn as usize - 1 {
                b.indices.extend_from_slice(&[bb, c, d]);
            }
        }
    }
    // Caps.
    for top in [true, false] {
        let radius = if top { radius_top } else { radius_bottom };
        if radius <= 0.0 || radius.is_nan() {
            continue;
        }
        let sign = if top { 1.0 } else { -1.0 };
        let center_start = index;
        for _ in 1..=rn {
            b.vertices.extend_from_slice(&[0.0, half_height * sign, 0.0]);
            b.normals.extend_from_slice(&[0.0, sign, 0.0]);
            b.uvs.extend_from_slice(&[0.5, 0.5]);
            index += 1;
        }
        let center_end = index;
        for x in 0..=rn {
            let u = f64::from(x) / radial;
            let theta = u * theta_length + theta_start;
            let cos_theta = libm::cos(theta);
            let sin_theta = libm::sin(theta);
            b.vertices
                .extend_from_slice(&[radius * sin_theta, half_height * sign, radius * cos_theta]);
            b.normals.extend_from_slice(&[0.0, sign, 0.0]);
            b.uvs.push(cos_theta * 0.5 + 0.5);
            b.uvs.push(sin_theta * 0.5 * sign + 0.5);
            index += 1;
        }
        for x in 0..rn {
            let c = center_start + x;
            let i = center_end + x;
            if top {
                b.indices.extend_from_slice(&[i, i + 1, c]);
            } else {
                b.indices.extend_from_slice(&[i + 1, i, c]);
            }
        }
    }
    b.finish()
}

/// `new TorusGeometry(radius, tube, radialSegments, tubularSegments)` (full
/// turn).
pub fn torus_geometry(
    radius: f64,
    tube: f64,
    radial_segments: f64,
    tubular_segments: f64,
) -> Geometry {
    let radial = radial_segments.floor();
    let tubular = tubular_segments.floor();
    let arc = PI * 2.0;
    let (rn, tn) = (radial as u32, tubular as u32);
    let mut b = Buffers::default();
    for j in 0..=rn {
        for i in 0..=tn {
            let u = f64::from(i) / tubular * arc;
            let v = f64::from(j) / radial * PI * 2.0;
            let x = (radius + tube * libm::cos(v)) * libm::cos(u);
            let y = (radius + tube * libm::cos(v)) * libm::sin(u);
            let z = tube * libm::sin(v);
            b.vertices.extend_from_slice(&[x, y, z]);
            let cx = radius * libm::cos(u);
            let cy = radius * libm::sin(u);
            b.normals.extend_from_slice(&normalize([x - cx, y - cy, z - 0.0]));
            b.uvs.push(f64::from(i) / tubular);
            b.uvs.push(f64::from(j) / radial);
        }
    }
    for j in 1..=rn {
        for i in 1..=tn {
            let a = (tn + 1) * j + i - 1;
            let bb = (tn + 1) * (j - 1) + i - 1;
            let c = (tn + 1) * (j - 1) + i;
            let d = (tn + 1) * j + i;
            b.indices.extend_from_slice(&[a, bb, d, bb, c, d]);
        }
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_primitive_geometry() {
        let none = Parameters::default();
        assert_eq!(
            Primitive::from_parameters("sphere", &none),
            Some(Primitive::Sphere {
                radius: 0.5,
                width_segments: 48.0,
                height_segments: 24.0
            })
        );
        let odd = Parameters {
            segments: Some(5.0),
            radius: Some(0.0),
            ..Parameters::default()
        };
        assert_eq!(
            Primitive::from_parameters("sphere", &odd),
            Some(Primitive::Sphere {
                radius: 0.5,
                width_segments: 5.0,
                height_segments: 3.0
            })
        );
        // `radiusTop ?? radius ?? .5`: a zero top radius is kept (a cone).
        let p = Parameters {
            radius_top: Some(0.0),
            radius: Some(2.0),
            ..Parameters::default()
        };
        assert_eq!(
            Primitive::from_parameters("cylinder", &p),
            Some(Primitive::Cylinder {
                radius_top: 0.0,
                radius_bottom: 2.0,
                height: 1.0,
                radial_segments: 48.0
            })
        );
        assert_eq!(Primitive::from_parameters("capsule", &none), None);
        assert_eq!(Primitive::from_parameters("icosahedron", &none), None);
    }

    #[test]
    fn counts_match_three() {
        let g = box_geometry(1.0, 1.0, 1.0);
        assert_eq!((g.vertex_count(), g.indices.len()), (24, 36));
        let g = sphere_geometry(1.0, 48.0, 24.0);
        assert_eq!(g.vertex_count(), 49 * 25);
        // Poles drop one triangle per segment.
        assert_eq!(g.indices.len(), (48 * 24 * 2 - 2 * 48) * 3);
        let g = plane_geometry(2.0, 1.0);
        assert_eq!((g.vertex_count(), g.indices.len()), (4, 6));
        let g = cylinder_geometry(0.0, 0.5, 1.0, 8.0);
        // Torso 2 × 9, bottom cap 8 centers + 9 ring.
        assert_eq!(g.vertex_count(), 18 + 17);
        assert_eq!(g.indices.len(), (8 + 8) * 3);
        let g = torus_geometry(0.6, 0.18, 8.0, 16.0);
        assert_eq!(g.vertex_count(), 9 * 17);
        assert_eq!(g.indices.len(), 8 * 16 * 6);
    }

    #[test]
    fn sphere_normals_are_unit() {
        let g = sphere_geometry(2.0, 12.0, 6.0);
        for n in g.normals.chunks_exact(3) {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-6);
        }
        let (lo, hi) = g.bounds().unwrap();
        assert!((hi[1] - 2.0).abs() < 1e-6 && (lo[1] + 2.0).abs() < 1e-6);
    }
}
