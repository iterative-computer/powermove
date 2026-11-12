//! Small, deterministic, texture-free nature prototypes. Metres, root at y=0.
//! Three shared meshes per species; instance count never multiplies vertices.
use crate::{
    math::{Vec3, v3},
    mesh::{self, Mesh},
    scene::Material,
    terrain::{hash, noise},
};
use std::f32::consts::{PI, TAU};
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Species {
    Conifer,
    Broadleaf,
    Birch,
    Bush,
    Grass,
    Rock,
}
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Nature {
    pub species: Species,
    pub seed: u32,
}
/// Angular size thresholds are independent of output resolution and renderer.
/// Full leaves above ~5 degrees, clusters above ~1.1 degrees, sparse cards beyond.
pub fn select_lod(diameter: f32, distance: f32) -> usize {
    let angular = diameter / distance.max(0.001);
    if angular >= 0.09 {
        0
    } else if angular >= 0.02 {
        1
    } else {
        2
    }
}
struct Builder {
    mesh: Mesh,
    state: u32,
}
impl Builder {
    fn rnd(&mut self) -> f32 {
        self.state = hash(self.state);
        (self.state >> 8) as f32 / 16777216.
    }
    fn vertex(&mut self, p: Vec3, n: Vec3, uv: [f32; 2], tint: f32) -> u32 {
        let i = self.mesh.positions.len() as u32;
        self.mesh.positions.push(p);
        self.mesh.normals.push(n.normalize());
        self.mesh.uvs.push(uv);
        self.mesh.colors.push(Vec3::splat(tint));
        i
    }
    fn tri(&mut self, a: u32, b: u32, c: u32, material: usize) {
        self.mesh.indices.push([a, b, c]);
        self.mesh.triangle_materials.push(material);
    }
    fn branch(&mut self, a: Vec3, b: Vec3, radius: f32, end: f32, segments: usize) {
        let axis = (b - a).normalize();
        let side = if axis.y.abs() < 0.9 {
            axis.cross(Vec3::Y).normalize()
        } else {
            axis.cross(v3(1., 0., 0.)).normalize()
        };
        let up = axis.cross(side).normalize();
        let start = self.mesh.positions.len() as u32;
        for j in 0..=1 {
            for k in 0..segments {
                let angle = TAU * k as f32 / segments as f32;
                let n = side * angle.cos() + up * angle.sin();
                let r = if j == 0 { radius } else { end };
                self.vertex(
                    if j == 0 { a } else { b } + n * r,
                    n,
                    [k as f32 / segments as f32, j as f32],
                    0.8 + 0.2 * (angle * 3.).cos().abs(),
                );
            }
        }
        for k in 0..segments as u32 {
            let q = (k + 1) % segments as u32;
            let s = segments as u32;
            self.tri(start + k, start + q, start + k + s, 0);
            self.tri(start + q, start + q + s, start + k + s, 0);
        }
    }
    /// A bent diamond with a raised midrib, true silhouette and thin-sheet BSDF.
    fn leaf(&mut self, p: Vec3, length: f32, width: f32, yaw: f32, pitch: f32) {
        let p = v3(p.x, p.y.max(-(length * pitch.sin()).min(0.) + 0.02), p.z);
        let axis = v3(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        );
        let side = v3(-yaw.sin(), 0., yaw.cos());
        let n = side.cross(axis).normalize();
        let tint = 0.68 + self.rnd() * 0.32;
        let start = self.mesh.positions.len() as u32;
        for (u, v, fold) in [
            (0.5, 0., 0.),
            (0., 0.48, 0.),
            (0.5, 0.55, 0.12),
            (1., 0.48, 0.),
            (0.5, 1., 0.),
        ] {
            let normal = (n + side * ((u - 0.5) * 0.35)).normalize();
            self.vertex(
                p + axis * (v * length) + side * ((u - 0.5) * width) + n * (length * fold),
                normal,
                [u, v],
                tint,
            );
        }
        for [a, b, c] in [[0, 1, 2], [1, 4, 2], [4, 3, 2], [3, 0, 2]] {
            self.tri(start + a, start + b, start + c, 1);
        }
    }
    fn spray(&mut self, p: Vec3, radius: f32, count: usize, leaf_size: f32) {
        for _ in 0..count {
            let yaw = self.rnd() * TAU;
            let z = self.rnd() * 2. - 1.;
            let ring = (1. - z * z).sqrt();
            let spread = radius * self.rnd().cbrt();
            let q = p + v3(ring * yaw.cos(), z, ring * yaw.sin()) * spread;
            let pitch = (self.rnd() - 0.5) * 1.8;
            let length = leaf_size * (0.65 + self.rnd() * 0.7);
            self.leaf(q, length, length * 0.48, yaw, pitch);
        }
    }
}
impl Nature {
    pub fn materials(&self) -> [Material; 2] {
        let bark = if self.species == Species::Birch {
            v3(0.66, 0.64, 0.55)
        } else {
            v3(0.19, 0.105, 0.046)
        };
        let leaf = match self.species {
            Species::Conifer => v3(0.085, 0.18, 0.066),
            Species::Birch => v3(0.23, 0.34, 0.055),
            Species::Grass => v3(0.23, 0.30, 0.065),
            _ => v3(0.15, 0.26, 0.046),
        };
        [
            Material {
                procedural: if self.species == Species::Birch {
                    6
                } else if self.species == Species::Rock {
                    2
                } else {
                    3
                },
                ..Material::dielectric(
                    if self.species == Species::Rock {
                        v3(0.34, 0.31, 0.26)
                    } else {
                        bark
                    },
                    0.87,
                )
            },
            Material {
                procedural: if self.species == Species::Grass { 5 } else { 4 },
                specular: 0.25,
                ..Material::dielectric(leaf, 0.72)
            },
        ]
    }
    /// Triangle materials are local [bark/stone, leaf]; remap when installing.
    pub fn mesh(&self, lod: usize) -> Mesh {
        let lod = lod.min(2);
        let mut b = Builder {
            mesh: Mesh::default(),
            state: self.seed,
        };
        let segments = [7, 5, 4][lod];
        if self.species == Species::Rock {
            let rings = [10, 6, 4][lod];
            let segments = [16, 10, 6][lod];
            let stride = segments + 1;
            let mut m = mesh::sphere(Vec3::ZERO, 1., rings, segments);
            let stretch = v3(
                0.8 + b.rnd() * 0.8,
                0.45 + b.rnd() * 0.45,
                0.7 + b.rnd() * 0.5,
            );
            for (i, p) in m.positions.iter_mut().enumerate() {
                if i / stride == 0 || i / stride == rings {
                    p.x = 0.;
                    p.z = 0.;
                }
                let d = 1.
                    + noise(p.x * 2.7 + p.y * 0.6, p.z * 2.7, self.seed) * 0.22
                    + noise(p.y * 6., p.x * 5. + p.z * 3., self.seed ^ 73) * 0.08;
                *p = v3(
                    p.x * stretch.x * d,
                    (p.y * d + 1.).max(0.) * stretch.y,
                    p.z * stretch.z * d,
                );
            }
            for row in 0..=rings {
                m.positions[row * stride + segments] = m.positions[row * stride];
            }
            let floor = m.positions.iter().map(|p| p.y).fold(f32::MAX, f32::min);
            for p in &mut m.positions {
                p.y -= floor;
            }
            m.normals.fill(Vec3::ZERO);
            for t in &m.indices {
                let [a, c, d] = t.map(|i| m.positions[i as usize]);
                let n = (c - a).cross(d - a);
                for &i in t {
                    m.normals[i as usize] = m.normals[i as usize] + n;
                }
            }
            // Weld the UV seam's shading normals, and use exact pole normals
            // where zero-area triangles otherwise depend on float sine rounding.
            for row in 1..rings {
                let a = row * stride;
                let b = a + segments;
                let n = m.normals[a] + m.normals[b];
                m.normals[a] = n;
                m.normals[b] = n;
            }
            for col in 0..=segments {
                m.normals[col] = Vec3::Y;
                m.normals[rings * stride + col] = -Vec3::Y;
            }
            for i in 0..m.normals.len() {
                let n = m.normals[i];
                m.normals[i] = if n.length() > 1e-8 {
                    n.normalize()
                } else {
                    Vec3::Y
                };
                m.colors.push(Vec3::splat(
                    0.72 + 0.23
                        * noise(m.positions[i].x * 3., m.positions[i].z * 3., self.seed).abs(),
                ));
            }
            return m;
        }
        if self.species == Species::Grass {
            for _ in 0..[28, 12, 5][lod] {
                let yaw = b.rnd() * TAU;
                let root = v3((b.rnd() - 0.5) * 0.3, 0., (b.rnd() - 0.5) * 0.3);
                let height = 0.22 + b.rnd() * 0.4;
                let bend = 0.1 + b.rnd() * 0.16;
                let side = v3(-yaw.sin(), 0., yaw.cos());
                let forward = v3(yaw.cos(), 0., yaw.sin());
                let n = forward;
                let start = b.mesh.positions.len() as u32;
                for j in 0..=3 {
                    let t = j as f32 / 3.;
                    let center = root + Vec3::Y * (height * t) + forward * (bend * t * t);
                    let width = 0.014 * (1. - t) + 0.0005;
                    for (u, sign) in [(0., -1.), (1., 1.)] {
                        b.vertex(center + side * (width * sign), n, [u, t], 0.7 + t * 0.25);
                    }
                }
                for j in 0..3 {
                    let a = start + j * 2;
                    b.tri(a, a + 1, a + 2, 1);
                    b.tri(a + 1, a + 3, a + 2, 1);
                }
            }
            return b.mesh;
        }
        if self.species == Species::Conifer {
            let height = 7. + b.rnd() * 1.6;
            let lean = v3((b.rnd() - 0.5) * 0.35, 0., (b.rnd() - 0.5) * 0.35);
            for j in 0..[9, 6, 3][lod] {
                let n = [9, 6, 3][lod];
                let t = j as f32 / n as f32;
                let u = (j + 1) as f32 / n as f32;
                b.branch(
                    Vec3::Y * (t * height) + lean * t * t,
                    Vec3::Y * (u * height) + lean * u * u,
                    0.17 * (1. - t) + 0.012,
                    0.17 * (1. - u) + 0.012,
                    segments,
                );
            }
            let tiers = [15, 10, 6][lod];
            let arms = [7, 5, 3][lod];
            for tier in 0..tiers {
                let t = tier as f32 / tiers as f32;
                let y = height * (0.12 + 0.82 * t) + (b.rnd() - 0.5) * height / tiers as f32 * 0.55;
                let radius = 1.75 * (1. - t).powf(0.8) + 0.10;
                for arm in 0..arms {
                    let yaw = arm as f32 / arms as f32 * TAU
                        + tier as f32 * 2.399
                        + (b.rnd() - 0.5) * 0.5;
                    let r = radius * (0.7 + b.rnd() * 0.4);
                    let a = v3(lean.x * t, y, lean.z * t);
                    let end = a + v3(yaw.cos() * r, -0.10 * r, yaw.sin() * r);
                    if lod < 2 {
                        b.branch(a, end, 0.035 * (1. - t) + 0.006, 0.004, segments);
                    }
                    let shoots = [5, 3, 1][lod];
                    for j in 0..shoots {
                        let u = (j as f32 + 0.5) / shoots as f32;
                        let p = a * (1. - u) + end * u + Vec3::Y * (0.1 + u * 0.18);
                        b.spray(
                            p,
                            [0.38, 0.44, 0.65][lod] * (1. - t * 0.5),
                            [18, 8, 4][lod],
                            [0.28, 0.46, 0.85][lod] * (1. - t * 0.4),
                        );
                    }
                }
            }
        } else {
            let bush = self.species == Species::Bush;
            let birch = self.species == Species::Birch;
            let height = if bush {
                1.4
            } else if birch {
                8.2
            } else {
                6.4
            };
            let width = if bush {
                1.0
            } else if birch {
                1.6
            } else {
                2.5
            };
            let trunk = height * if bush { 0.3 } else { 0.6 };
            let lean = v3(
                (b.rnd() - 0.5) * height * 0.12,
                trunk,
                (b.rnd() - 0.5) * height * 0.12,
            );
            b.branch(
                Vec3::ZERO,
                lean,
                if bush {
                    0.06
                } else if birch {
                    0.12
                } else {
                    0.25
                },
                0.025,
                segments,
            );
            let arms = [18, 12, 7][lod];
            for i in 0..arms {
                let yaw = i as f32 * 2.399 + b.rnd() * 0.7;
                let t = (i as f32 + 0.5) / arms as f32;
                let y = height * (0.36 + 0.55 * t);
                let r = width * (0.55 + 0.45 * (t * PI).sin()) * (0.75 + b.rnd() * 0.35);
                let a = lean * (0.35 + 0.6 * t);
                let end = v3(yaw.cos() * r + lean.x, y, yaw.sin() * r + lean.z);
                if lod < 2 {
                    b.branch(
                        a,
                        end,
                        if bush { 0.025 } else { 0.055 } * (1. - t * 0.6),
                        0.008,
                        segments,
                    );
                }
                let clusters = [5, 3, 1][lod];
                for j in 0..clusters {
                    let u = 0.45 + j as f32 / clusters as f32 * 0.55;
                    let p = a * (1. - u) + end * u;
                    let rad = if bush { 0.34 } else { 0.58 };
                    let leaf = if bush {
                        0.14
                    } else if birch {
                        0.16
                    } else {
                        0.22
                    };
                    b.spray(
                        p,
                        rad * [1., 1.15, 1.5][lod],
                        [26, 12, 6][lod],
                        leaf * [1., 1.9, 4.5][lod],
                    );
                }
            }
        }
        b.mesh
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeded_assets_are_repeatable_and_not_cones() {
        for species in [
            Species::Conifer,
            Species::Broadleaf,
            Species::Birch,
            Species::Bush,
            Species::Grass,
            Species::Rock,
        ] {
            let a = Nature { species, seed: 41 }.mesh(0);
            let b = Nature { species, seed: 41 }.mesh(0);
            let c = Nature { species, seed: 42 }.mesh(0);
            assert_eq!(a.positions, b.positions);
            assert_ne!(a.positions, c.positions);
            assert_eq!(a.positions.len(), a.colors.len());
            assert!(
                a.normals
                    .iter()
                    .all(|n| n.length().is_finite() && (n.length() - 1.).abs() < 1e-4)
            );
            assert!(a.indices.len() > 100);
            assert!(a.positions.iter().all(|p| p.y >= -0.05));
        }
    }
    #[test]
    fn rocks_touch_the_ground_and_have_no_uv_normal_seam() {
        for lod in 0..3 {
            let m = Nature {
                species: Species::Rock,
                seed: 81,
            }
            .mesh(lod);
            let segments = [16, 10, 6][lod];
            let rings = [10, 6, 4][lod];
            assert!(
                m.positions
                    .iter()
                    .map(|p| p.y)
                    .fold(f32::MAX, f32::min)
                    .abs()
                    < 1e-6
            );
            for row in 0..=rings {
                let a = row * (segments + 1);
                let b = a + segments;
                assert_eq!(m.positions[a], m.positions[b]);
                assert_eq!(m.normals[a], m.normals[b]);
            }
        }
    }
    #[test]
    fn detail_falls_with_distance_without_changing_seed() {
        let n = Nature {
            species: Species::Broadleaf,
            seed: 7,
        };
        assert!(n.mesh(0).indices.len() > n.mesh(1).indices.len() * 2);
        assert!(n.mesh(1).indices.len() > n.mesh(2).indices.len() * 2);
        assert_eq!(select_lod(8., 10.), 0);
        assert_eq!(select_lod(8., 100.), 1);
        assert_eq!(select_lod(8., 1000.), 2);
        assert_eq!(select_lod(8., 0.), 0);
    }
}
