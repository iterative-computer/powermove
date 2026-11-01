//! Triangle meshes and the procedural shapes Phase 0 renders.
//!
//! The tablet is an original, generic design: a rounded slab with softened
//! edges, an inset display and a camera bump, proportioned like an 8.3-inch
//! tablet (195 × 135 × 6.3 mm). Units are millimetres.

use crate::math::{Vec3, v3};

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<[u32; 3]>,
}

impl Mesh {
    fn vertex(&mut self, p: Vec3, n: Vec3, uv: [f32; 2]) -> u32 {
        self.positions.push(p);
        self.normals.push(n.normalize());
        self.uvs.push(uv);
        (self.positions.len() - 1) as u32
    }

    /// Apply `f` to every position and `g` to every normal (rigid placement).
    pub fn transformed(&self, f: impl Fn(Vec3) -> Vec3, g: impl Fn(Vec3) -> Vec3) -> Mesh {
        Mesh {
            positions: self.positions.iter().map(|&p| f(p)).collect(),
            normals: self.normals.iter().map(|&n| g(n).normalize()).collect(),
            uvs: self.uvs.clone(),
            indices: self.indices.clone(),
        }
    }

    /// Rotate about x by `tilt`, then about y by `turn`, then move by `offset`.
    pub fn placed(&self, tilt: f32, turn: f32, offset: Vec3) -> Mesh {
        self.transformed(move |p| p.rotate_x(tilt).rotate_y(turn) + offset, move |n| n.rotate_x(tilt).rotate_y(turn))
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &p| (lo.min(p), hi.max(p)))
    }
}

/// A rounded rectangle outline in the xz plane: points and outward normals,
/// counter-clockwise seen from +y.
fn rounded_rect(half_w: f32, half_d: f32, radius: f32, segments: usize) -> Vec<(Vec3, Vec3)> {
    let corners = [
        (v3(half_w - radius, 0.0, half_d - radius), 0.0f32),
        (v3(-(half_w - radius), 0.0, half_d - radius), 90.0),
        (v3(-(half_w - radius), 0.0, -(half_d - radius)), 180.0),
        (v3(half_w - radius, 0.0, -(half_d - radius)), 270.0),
    ];
    let mut out = Vec::new();
    for (center, start) in corners {
        for s in 0..=segments {
            let a = (start + 90.0 * s as f32 / segments as f32).to_radians();
            // +x → +z is counter-clockwise when seen from +y with z toward the viewer.
            let n = v3(a.cos(), 0.0, a.sin());
            out.push((center + n * radius, n));
        }
    }
    out
}

/// A flat, fan-triangulated cap over a convex outline at height `y`.
fn cap(mesh: &mut Mesh, outline: &[(Vec3, Vec3)], inset: f32, y: f32, up: bool, uv_extent: (f32, f32)) {
    let n = if up { Vec3::Y } else { -Vec3::Y };
    let uv = |p: Vec3| [0.5 + p.x / (2.0 * uv_extent.0), 0.5 - p.z / (2.0 * uv_extent.1)];
    let center = mesh.vertex(v3(0.0, y, 0.0), n, [0.5, 0.5]);
    let ring: Vec<u32> = outline
        .iter()
        .map(|&(p, o)| {
            let q = p - o * inset + v3(0.0, y, 0.0);
            mesh.vertex(q, n, uv(q))
        })
        .collect();
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        mesh.indices.push(if up { [center, b, a] } else { [center, a, b] });
    }
}

/// The tablet body: sides swept with a rounded edge profile, plus top and
/// bottom caps. Returns (body, top glass, display) meshes; the display sits
/// a hair above the glass and carries UVs for the screen composition.
pub struct Tablet {
    pub body: Mesh,
    pub glass: Mesh,
    pub display: Mesh,
    pub camera_ring: Mesh,
    pub camera_lens: Mesh,
}

pub fn tablet() -> Tablet {
    let (w, d, t) = (134.8f32, 195.4f32, 6.3f32);
    let (corner, edge, bezel) = (10.5f32, 1.4f32, 6.8f32);
    let outline = rounded_rect(w / 2.0, d / 2.0, corner, 24);

    // Edge profile: bottom arc, straight side, top arc (offset outward, height, normal).
    let mut profile: Vec<(f32, f32, f32, f32)> = Vec::new();
    for s in 0..=8 {
        let a = (-90.0 + 90.0 * s as f32 / 8.0f32).to_radians();
        profile.push((-edge + edge * a.cos(), edge + edge * a.sin(), a.cos(), a.sin()));
    }
    for s in 0..=8 {
        let a = (90.0 * s as f32 / 8.0f32).to_radians();
        profile.push((-edge + edge * a.cos(), t - edge + edge * a.sin(), a.cos(), a.sin()));
    }

    let mut body = Mesh::default();
    let rows = profile.len() as u32;
    for (p, n) in &outline {
        for &(o, y, no, ny) in &profile {
            let pos = *p + *n * o + v3(0.0, y, 0.0);
            body.vertex(pos, *n * no + v3(0.0, ny, 0.0), [0.0, 0.0]);
        }
    }
    let count = outline.len() as u32;
    for i in 0..count {
        let j = (i + 1) % count;
        for r in 0..rows - 1 {
            let (a, b, c, e) = (i * rows + r, j * rows + r, j * rows + r + 1, i * rows + r + 1);
            body.indices.push([a, e, b]);
            body.indices.push([b, e, c]);
        }
    }
    cap(&mut body, &outline, edge, 0.0, false, (w / 2.0, d / 2.0));

    let mut glass = Mesh::default();
    cap(&mut glass, &outline, edge, t, true, (w / 2.0, d / 2.0));

    let screen_outline = rounded_rect(w / 2.0 - bezel, d / 2.0 - bezel, corner - bezel * 0.75, 16);
    let mut display = Mesh::default();
    cap(&mut display, &screen_outline, 0.0, t + 0.01, true, (w / 2.0 - bezel, d / 2.0 - bezel));

    // Camera bump on the back, near one corner.
    let at = v3(w / 2.0 - 13.0, 0.0, -(d / 2.0 - 13.0));
    let camera_ring = cylinder(at, 5.6, -0.7, 48).0;
    let camera_lens = cylinder(at, 4.4, -0.72, 48).1;
    Tablet { body, glass, display, camera_ring, camera_lens }
}

/// A short cylinder from y=0 to y=`height` around `center`: (side + cap, cap only).
fn cylinder(center: Vec3, radius: f32, height: f32, segments: usize) -> (Mesh, Mesh) {
    let mut side = Mesh::default();
    for s in 0..=segments {
        let a = std::f32::consts::TAU * s as f32 / segments as f32;
        let n = v3(a.cos(), 0.0, a.sin());
        side.vertex(center + n * radius, n, [0.0, 0.0]);
        side.vertex(center + n * radius + v3(0.0, height, 0.0), n, [0.0, 0.0]);
    }
    for s in 0..segments as u32 {
        let (a, b, c, d) = (2 * s, 2 * s + 1, 2 * s + 2, 2 * s + 3);
        if height < 0.0 {
            side.indices.push([a, c, b]);
            side.indices.push([b, c, d]);
        } else {
            side.indices.push([a, b, c]);
            side.indices.push([b, d, c]);
        }
    }
    let ring: Vec<(Vec3, Vec3)> = (0..segments)
        .map(|s| {
            let a = std::f32::consts::TAU * s as f32 / segments as f32;
            let n = v3(a.cos(), 0.0, a.sin());
            (center + n * radius - v3(center.x, 0.0, center.z), n)
        })
        .collect();
    let mut lid = Mesh::default();
    cap(&mut lid, &ring, 0.0, height, height > 0.0, (radius, radius));
    let lid = lid.transformed(|p| p + v3(center.x, 0.0, center.z), |n| n);
    let mut both = side.clone();
    let base = both.positions.len() as u32;
    both.positions.extend(&lid.positions);
    both.normals.extend(&lid.normals);
    both.uvs.extend(&lid.uvs);
    both.indices.extend(lid.indices.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
    (both, lid)
}

/// A stylus: a round barrel with a conical tip, along +x, resting at y=radius.
pub fn stylus() -> Mesh {
    let (radius, barrel, tip) = (4.45f32, 148.0f32, 18.0f32);
    let segments = 48usize;
    let mut m = Mesh::default();
    // Rings: back cap edge, barrel start, barrel end, tip end.
    let rings = [(0.0f32, radius), (barrel, radius), (barrel + tip * 0.85, 1.2), (barrel + tip, 0.35)];
    for &(x, r) in &rings {
        for s in 0..=segments {
            let a = std::f32::consts::TAU * s as f32 / segments as f32;
            let (c, sn) = (a.cos(), a.sin());
            let slope = if x > barrel { (radius - 1.2) / (tip * 0.85) } else { 0.0 };
            m.vertex(v3(x, radius + r * c, r * sn), v3(slope, c, sn), [0.0, 0.0]);
        }
    }
    let stride = segments as u32 + 1;
    for ring in 0..rings.len() as u32 - 1 {
        for s in 0..segments as u32 {
            let (a, b) = (ring * stride + s, ring * stride + s + 1);
            let (c, d) = (a + stride, b + stride);
            m.indices.push([a, c, b]);
            m.indices.push([b, c, d]);
        }
    }
    // Flat back cap.
    let center = m.vertex(v3(0.0, radius, 0.0), v3(-1.0, 0.0, 0.0), [0.0, 0.0]);
    for s in 0..segments as u32 {
        m.indices.push([center, s, s + 1]);
    }
    m
}

/// A sphere (test scenes).
pub fn sphere(center: Vec3, radius: f32, rings: usize, segments: usize) -> Mesh {
    let mut m = Mesh::default();
    for r in 0..=rings {
        let phi = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..=segments {
            let th = std::f32::consts::TAU * s as f32 / segments as f32;
            let n = v3(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin());
            m.vertex(center + n * radius, n, [s as f32 / segments as f32, r as f32 / rings as f32]);
        }
    }
    let stride = segments as u32 + 1;
    for r in 0..rings as u32 {
        for s in 0..segments as u32 {
            let (a, b) = (r * stride + s, r * stride + s + 1);
            let (c, d) = (a + stride, b + stride);
            m.indices.push([a, b, c]);
            m.indices.push([b, d, c]);
        }
    }
    m
}

/// A Wavefront OBJ (triangles or polygons; positions, normals, texcoords).
/// A `.usdz` converts to OBJ with macOS ModelIO (`tools/usdz-to-obj.swift`).
pub fn load_obj(text: &str, scale: f32) -> Mesh {
    let (mut p, mut n, mut t) = (Vec::new(), Vec::new(), Vec::new());
    let mut m = Mesh::default();
    let mut cache = std::collections::HashMap::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let f = |it: &mut std::str::SplitWhitespace| -> f32 { it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0) };
        match it.next() {
            Some("v") => p.push(v3(f(&mut it), f(&mut it), f(&mut it)) * scale),
            Some("vn") => n.push(v3(f(&mut it), f(&mut it), f(&mut it))),
            Some("vt") => t.push([f(&mut it), f(&mut it)]),
            Some("f") => {
                let ids: Vec<u32> = it
                    .map(|c| {
                        *cache.entry(c.to_string()).or_insert_with(|| {
                            let mut parts = c.split('/');
                            let idx = |s: Option<&str>, len: usize| -> Option<usize> {
                                let v: i64 = s?.parse().ok()?;
                                Some(if v < 0 { (len as i64 + v) as usize } else { v as usize - 1 })
                            };
                            let pi = idx(parts.next(), p.len()).unwrap_or(0);
                            let ti = idx(parts.next(), t.len());
                            let ni = idx(parts.next(), n.len());
                            m.positions.push(p[pi]);
                            m.uvs.push(ti.map(|i| t[i]).unwrap_or([0.0, 0.0]));
                            m.normals.push(ni.map(|i| n[i]).unwrap_or(Vec3::ZERO));
                            (m.positions.len() - 1) as u32
                        })
                    })
                    .collect();
                for k in 1..ids.len().saturating_sub(1) {
                    m.indices.push([ids[0], ids[k], ids[k + 1]]);
                }
            }
            _ => {}
        }
    }
    // Faceted normals where the file has none.
    for tri in m.indices.clone() {
        let [a, b, c] = tri.map(|i| m.positions[i as usize]);
        let fnorm = (b - a).cross(c - a).normalize();
        for i in tri {
            if m.normals[i as usize] == Vec3::ZERO {
                m.normals[i as usize] = fnorm;
            }
        }
    }
    m
}
