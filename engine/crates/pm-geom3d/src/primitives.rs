//! three.js r180 primitive geometry (`BoxGeometry`, `SphereGeometry`,
//! `PlaneGeometry`, `CylinderGeometry`, `ConeGeometry`, `TorusGeometry`,
//! `CapsuleGeometry`, `IcosahedronGeometry`), as `primitiveGeometry` in
//! `core/scene3d/runtime.ts` builds them.
//!
//! Every value is computed in f64 in three.js's operation order and stored as
//! f32, as `Float32BufferAttribute` stores JavaScript numbers, so the buffers
//! equal three.js's bit for bit (checked against three.js itself by
//! `tests/three_primitives.rs`). Transcendentals use `libm` (plan section 7,
//! "Determinism"); V8's fdlibm differs from it by at most an ulp of f64, far
//! below f32's resolution.

use std::f64::consts::PI;

/// A `BufferGeometry` group: a range of the draw (indices of an indexed
/// geometry, vertices otherwise) drawn with one material of an array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub start: u32,
    pub count: u32,
    pub material_index: u32,
}

/// Triangle geometry with three.js's standard attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    /// xyz per vertex.
    pub positions: Vec<f32>,
    /// xyz per vertex.
    pub normals: Vec<f32>,
    /// uv per vertex.
    pub uvs: Vec<f32>,
    /// Triangle list. A geometry without an index (`indexed` false) lists
    /// its vertices in order, which draws exactly as `drawArrays`.
    pub indices: Vec<u32>,
    /// Whether three.js's geometry has an index (`geometry.index !== null`).
    pub indexed: bool,
    /// `geometry.groups` in insertion order.
    pub groups: Vec<Group>,
}

impl Geometry {
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// The edge list `WebGLGeometries.getWireframeAttribute` draws for
    /// `material.wireframe`: `a, b, b, c, c, a` per triangle, in order.
    pub fn wireframe_indices(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.indices.len() * 2);
        for t in self.indices.chunks_exact(3) {
            out.extend_from_slice(&[t[0], t[1], t[1], t[2], t[2], t[0]]);
        }
        out
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
    Box {
        width: f64,
        height: f64,
        depth: f64,
    },
    Sphere {
        radius: f64,
        width_segments: f64,
        height_segments: f64,
    },
    Plane {
        width: f64,
        height: f64,
    },
    Cylinder {
        radius_top: f64,
        radius_bottom: f64,
        height: f64,
        radial_segments: f64,
    },
    Cone {
        radius: f64,
        height: f64,
        radial_segments: f64,
    },
    Torus {
        radius: f64,
        tube: f64,
        radial_segments: f64,
        tubular_segments: f64,
    },
    /// `new CapsuleGeometry(radius, height, 8, radialSegments)`.
    Capsule {
        radius: f64,
        height: f64,
        cap_segments: f64,
        radial_segments: f64,
    },
    /// `new IcosahedronGeometry(radius, detail)`.
    Icosahedron {
        radius: f64,
        detail: f64,
    },
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
    /// for unknown names.
    /// Whether `primitiveGeometry` builds anything for `name` (the schema's
    /// primitives); for any other name it returns nothing.
    pub fn known(name: &str) -> bool {
        matches!(
            name,
            "box" | "sphere" | "plane" | "cone" | "cylinder" | "torus" | "capsule" | "icosahedron"
        )
    }

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
            "capsule" => Primitive::Capsule {
                radius: or(p.radius, 0.3),
                height: or(p.height, 0.7),
                cap_segments: 8.0,
                radial_segments: s,
            },
            "icosahedron" => Primitive::Icosahedron {
                radius: or(p.radius, 0.5),
                detail: nullish(p.detail, 2.0),
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
            Primitive::Capsule {
                radius,
                height,
                cap_segments,
                radial_segments,
            } => capsule_geometry(radius, height, cap_segments, radial_segments),
            Primitive::Icosahedron { radius, detail } => icosahedron_geometry(radius, detail),
        }
    }
}

#[derive(Default)]
pub(crate) struct Buffers {
    pub vertices: Vec<f64>,
    pub normals: Vec<f64>,
    pub uvs: Vec<f64>,
    pub indices: Vec<u32>,
    pub groups: Vec<Group>,
}

impl Buffers {
    /// `addGroup(start, count, materialIndex)`.
    pub fn group(&mut self, start: u32, count: u32, material_index: u32) {
        self.groups.push(Group {
            start,
            count,
            material_index,
        });
    }

    /// An indexed geometry (`setIndex` then `Float32BufferAttribute`s).
    pub fn finish(self) -> Geometry {
        Geometry {
            positions: to_f32(&self.vertices),
            normals: to_f32(&self.normals),
            uvs: to_f32(&self.uvs),
            indices: self.indices,
            indexed: true,
            groups: self.groups,
        }
    }
}

/// `Float32BufferAttribute` from JavaScript numbers.
pub(crate) fn to_f32(v: &[f64]) -> Vec<f32> {
    v.iter().map(|x| *x as f32).collect()
}

/// A non-indexed geometry's draw order.
pub(crate) fn sequential(vertices: usize) -> Vec<u32> {
    (0..vertices as u32).collect()
}

/// `Vector3.normalize()`: `divideScalar(length() || 1)`, which multiplies by
/// the reciprocal.
pub(crate) fn normalize(v: [f64; 3]) -> [f64; 3] {
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
    type Side = (usize, usize, usize, f64, f64, f64, f64, f64);
    let planes: [Side; 6] = [
        (2, 1, 0, -1.0, -1.0, depth, height, width),
        (2, 1, 0, 1.0, -1.0, depth, height, -width),
        (0, 2, 1, 1.0, 1.0, width, depth, height),
        (0, 2, 1, 1.0, -1.0, width, depth, -height),
        (0, 1, 2, 1.0, -1.0, width, height, depth),
        (0, 1, 2, -1.0, -1.0, width, height, -depth),
    ];
    let mut group_start = 0u32;
    for (material_index, (u, v, w, udir, vdir, pw, ph, pd)) in planes.into_iter().enumerate() {
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
        b.group(group_start, 6, material_index as u32);
        group_start += 6;
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
            b.vertices.extend_from_slice(&[
                radius * sin_theta,
                -v * height + half_height,
                radius * cos_theta,
            ]);
            b.normals
                .extend_from_slice(&normalize([sin_theta, slope, cos_theta]));
            b.uvs.push(u);
            b.uvs.push(1.0 - v);
            row.push(index);
            index += 1;
        }
        index_array.push(row);
    }
    let mut group_start = 0u32;
    let mut group_count = 0u32;
    for x in 0..rn as usize {
        for y in 0..hn as usize {
            let a = index_array[y][x];
            let bb = index_array[y + 1][x];
            let c = index_array[y + 1][x + 1];
            let d = index_array[y][x + 1];
            if radius_top > 0.0 || y != 0 {
                b.indices.extend_from_slice(&[a, bb, d]);
                group_count += 3;
            }
            if radius_bottom > 0.0 || y != hn as usize - 1 {
                b.indices.extend_from_slice(&[bb, c, d]);
                group_count += 3;
            }
        }
    }
    b.group(group_start, group_count, 0);
    group_start += group_count;
    // Caps.
    for top in [true, false] {
        let radius = if top { radius_top } else { radius_bottom };
        if radius <= 0.0 || radius.is_nan() {
            continue;
        }
        let sign = if top { 1.0 } else { -1.0 };
        let center_start = index;
        for _ in 1..=rn {
            b.vertices
                .extend_from_slice(&[0.0, half_height * sign, 0.0]);
            b.normals.extend_from_slice(&[0.0, sign, 0.0]);
            b.uvs.extend_from_slice(&[0.5, 0.5]);
            index += 1;
        }
        let center_end = index;
        let mut group_count = 0u32;
        for x in 0..=rn {
            let u = f64::from(x) / radial;
            let theta = u * theta_length + theta_start;
            let cos_theta = libm::cos(theta);
            let sin_theta = libm::sin(theta);
            b.vertices.extend_from_slice(&[
                radius * sin_theta,
                half_height * sign,
                radius * cos_theta,
            ]);
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
            group_count += 3;
        }
        b.group(group_start, group_count, if top { 1 } else { 2 });
        group_start += group_count;
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
            b.normals
                .extend_from_slice(&normalize([x - cx, y - cy, z - 0.0]));
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

/// `new CapsuleGeometry(radius, height, capSegments, radialSegments)` (one
/// height segment).
pub fn capsule_geometry(
    radius: f64,
    height: f64,
    cap_segments: f64,
    radial_segments: f64,
) -> Geometry {
    let height = height.max(0.0);
    let cap = 1f64.max(cap_segments.floor());
    let radial = 3f64.max(radial_segments.floor());
    let height_segments = 1.0f64;
    let half_height = height / 2.0;
    let cap_arc_length = (PI / 2.0) * radius;
    let cylinder_part_length = height;
    let total_arc_length = 2.0 * cap_arc_length + cylinder_part_length;
    let vertical = (cap * 2.0 + height_segments) as u32;
    let (cn, rn) = (cap as u32, radial as u32);
    let per_row = rn + 1;
    let mut b = Buffers::default();
    for iy in 0..=vertical {
        let fy = f64::from(iy);
        let (profile_y, profile_radius, normal_y, arc);
        if iy <= cn {
            let progress = fy / cap;
            let angle = (progress * PI) / 2.0;
            profile_y = -half_height - radius * libm::cos(angle);
            profile_radius = radius * libm::sin(angle);
            normal_y = -radius * libm::cos(angle);
            arc = progress * cap_arc_length;
        } else if iy <= cn + 1 {
            let progress = (fy - cap) / height_segments;
            profile_y = -half_height + progress * height;
            profile_radius = radius;
            normal_y = 0.0;
            arc = cap_arc_length + progress * cylinder_part_length;
        } else {
            let progress = (fy - cap - height_segments) / cap;
            let angle = (progress * PI) / 2.0;
            profile_y = half_height + radius * libm::sin(angle);
            profile_radius = radius * libm::cos(angle);
            normal_y = radius * libm::sin(angle);
            arc = cap_arc_length + cylinder_part_length + progress * cap_arc_length;
        }
        let v = 0f64.max(1f64.min(arc / total_arc_length));
        let u_offset = if iy == 0 {
            0.5 / radial
        } else if iy == vertical {
            -0.5 / radial
        } else {
            0.0
        };
        for ix in 0..=rn {
            let u = f64::from(ix) / radial;
            let theta = u * PI * 2.0;
            let sin_theta = libm::sin(theta);
            let cos_theta = libm::cos(theta);
            b.vertices.extend_from_slice(&[
                -profile_radius * cos_theta,
                profile_y,
                profile_radius * sin_theta,
            ]);
            b.normals.extend_from_slice(&normalize([
                -profile_radius * cos_theta,
                normal_y,
                profile_radius * sin_theta,
            ]));
            b.uvs.push(u + u_offset);
            b.uvs.push(v);
        }
        if iy > 0 {
            let prev = (iy - 1) * per_row;
            for ix in 0..rn {
                let i1 = prev + ix;
                let i2 = prev + ix + 1;
                let i3 = iy * per_row + ix;
                let i4 = iy * per_row + ix + 1;
                b.indices.extend_from_slice(&[i1, i2, i3, i2, i4, i3]);
            }
        }
    }
    b.finish()
}

/// `Vector3.lerp(v, alpha)` on a copy of `a`.
fn lerp(a: [f64; 3], v: [f64; 3], alpha: f64) -> [f64; 3] {
    [
        a[0] + (v[0] - a[0]) * alpha,
        a[1] + (v[1] - a[1]) * alpha,
        a[2] + (v[2] - a[2]) * alpha,
    ]
}

/// `new IcosahedronGeometry(radius, detail)`.
pub fn icosahedron_geometry(radius: f64, detail: f64) -> Geometry {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let vertices: [f64; 36] = [
        -1.0, t, 0.0, 1.0, t, 0.0, -1.0, -t, 0.0, 1.0, -t, 0.0, 0.0, -1.0, t, 0.0, 1.0, t, 0.0,
        -1.0, -t, 0.0, 1.0, -t, t, 0.0, -1.0, t, 0.0, 1.0, -t, 0.0, -1.0, -t, 0.0, 1.0,
    ];
    let indices: [usize; 60] = [
        0, 11, 5, 0, 5, 1, 0, 1, 7, 0, 7, 10, 0, 10, 11, 1, 5, 9, 5, 11, 4, 11, 10, 2, 10, 7, 6, 7,
        1, 8, 3, 9, 4, 3, 4, 2, 3, 2, 6, 3, 6, 8, 3, 8, 9, 4, 9, 5, 2, 4, 11, 6, 2, 10, 8, 6, 7, 9,
        8, 1,
    ];
    polyhedron_geometry(&vertices, &indices, radius, detail)
}

/// `new PolyhedronGeometry(vertices, indices, radius, detail)`: subdivided,
/// projected onto the sphere, spherical UVs with the seam corrections, flat
/// normals at detail 0 and smooth ones otherwise. Not indexed.
pub fn polyhedron_geometry(
    vertices: &[f64],
    indices: &[usize],
    radius: f64,
    detail: f64,
) -> Geometry {
    let mut buffer: Vec<f64> = Vec::new();
    let vertex = |i: usize| [vertices[i * 3], vertices[i * 3 + 1], vertices[i * 3 + 2]];
    for f in indices.chunks_exact(3) {
        let (a, b, c) = (vertex(f[0]), vertex(f[1]), vertex(f[2]));
        // `subdivideFace(a, b, c, detail)`.
        let cols = detail + 1.0;
        let n = cols as usize;
        let mut v: Vec<Vec<[f64; 3]>> = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let fi = i as f64;
            let aj = lerp(a, c, fi / cols);
            let bj = lerp(b, c, fi / cols);
            let rows = cols - fi;
            let mut row = Vec::new();
            for j in 0..=(rows as usize) {
                if j == 0 && i == n {
                    row.push(aj);
                } else {
                    row.push(lerp(aj, bj, j as f64 / rows));
                }
            }
            v.push(row);
        }
        for i in 0..n {
            for j in 0..(2 * (n - i) - 1) {
                let k = j / 2;
                let tri = if j % 2 == 0 {
                    [v[i][k + 1], v[i + 1][k], v[i][k]]
                } else {
                    [v[i][k + 1], v[i + 1][k + 1], v[i + 1][k]]
                };
                for p in tri {
                    buffer.extend_from_slice(&p);
                }
            }
        }
    }
    // `applyRadius`.
    for p in buffer.chunks_exact_mut(3) {
        let n = normalize([p[0], p[1], p[2]]);
        p[0] = n[0] * radius;
        p[1] = n[1] * radius;
        p[2] = n[2] * radius;
    }
    // `generateUVs`.
    let azimuth = |v: [f64; 3]| libm::atan2(v[2], -v[0]);
    let inclination = |v: [f64; 3]| libm::atan2(-v[1], (v[0] * v[0] + v[2] * v[2]).sqrt());
    let mut uvs: Vec<f64> = Vec::with_capacity(buffer.len() / 3 * 2);
    for p in buffer.chunks_exact(3) {
        let v = [p[0], p[1], p[2]];
        let u = azimuth(v) / 2.0 / PI + 0.5;
        let w = inclination(v) / PI + 0.5;
        uvs.push(u);
        uvs.push(1.0 - w);
    }
    // `correctUVs`.
    let mut i = 0;
    let mut j = 0;
    while i + 8 < buffer.len() {
        let a = [buffer[i], buffer[i + 1], buffer[i + 2]];
        let b = [buffer[i + 3], buffer[i + 4], buffer[i + 5]];
        let c = [buffer[i + 6], buffer[i + 7], buffer[i + 8]];
        let uv = [uvs[j], uvs[j + 2], uvs[j + 4]];
        let third = 1.0 / 3.0;
        let centroid = [
            (a[0] + b[0] + c[0]) * third,
            (a[1] + b[1] + c[1]) * third,
            (a[2] + b[2] + c[2]) * third,
        ];
        let azi = azimuth(centroid);
        for (k, p) in [a, b, c].into_iter().enumerate() {
            let stride = j + 2 * k;
            if azi < 0.0 && uv[k] == 1.0 {
                uvs[stride] = uv[k] - 1.0;
            }
            if p[0] == 0.0 && p[2] == 0.0 {
                uvs[stride] = azi / 2.0 / PI + 0.5;
            }
        }
        i += 9;
        j += 6;
    }
    // `correctSeam`.
    let mut i = 0;
    while i + 4 < uvs.len() {
        let (x0, x1, x2) = (uvs[i], uvs[i + 2], uvs[i + 4]);
        let max = x0.max(x1).max(x2);
        let min = x0.min(x1).min(x2);
        if max > 0.9 && min < 0.1 {
            for k in [0, 2, 4] {
                if uvs[i + k] < 0.2 {
                    uvs[i + k] += 1.0;
                }
            }
        }
        i += 6;
    }
    let positions = to_f32(&buffer);
    let count = positions.len() / 3;
    let normals = if detail == 0.0 {
        crate::normals::compute_vertex_normals(&positions, None)
    } else {
        let mut n = positions.clone();
        crate::normals::normalize_normals(&mut n);
        n
    };
    Geometry {
        positions,
        normals,
        uvs: to_f32(&uvs),
        indices: sequential(count),
        indexed: false,
        groups: Vec::new(),
    }
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
        assert_eq!(
            Primitive::from_parameters("icosahedron", &none),
            Some(Primitive::Icosahedron {
                radius: 0.5,
                detail: 2.0
            })
        );
        assert_eq!(Primitive::from_parameters("knot", &none), None);
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
