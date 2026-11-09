//! Deterministic heightfields and material regions. Meshes are generated once,
//! then use the same BLAS path as imported geometry.
use crate::{
    math::{Vec3, v3},
    mesh::Mesh,
};
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Terrain {
    pub center: Vec3,
    pub size: [f32; 2],
    pub resolution: u32,
    pub amplitude: f32,
    pub frequency: f32,
    pub octaves: u32,
    pub seed: u32,
    pub ridged: bool,
    /// Optional row-major normalized grayscale heights, (resolution+1)².
    pub heights: Vec<f32>,
    pub material: usize,
    /// First matching region selects that triangle's material.
    pub layers: Vec<TerrainLayer>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TerrainLayer {
    pub material: usize,
    pub min_height: f32,
    pub max_height: f32,
    pub min_slope: f32,
    pub max_slope: f32,
}
impl Default for Terrain {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            size: [100., 100.],
            resolution: 64,
            amplitude: 10.,
            frequency: 0.02,
            octaves: 5,
            seed: 1,
            ridged: false,
            heights: vec![],
            material: 0,
            layers: vec![],
        }
    }
}
pub(crate) fn hash(mut x: u32) -> u32 {
    x = x.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277803737);
    (w >> 22) ^ w
}
fn noise(x: f32, z: f32, seed: u32) -> f32 {
    let a = x.floor() as i32;
    let b = z.floor() as i32;
    let u = x - x.floor();
    let v = z - z.floor();
    let u = u * u * (3. - 2. * u);
    let v = v * v * (3. - 2. * v);
    let at = |i: i32, j: i32| {
        hash((i as u32).wrapping_mul(1597334677) ^ (j as u32).wrapping_mul(3812015801) ^ seed)
            as f32
            / u32::MAX as f32
            * 2.
            - 1.
    };
    let low = at(a, b) * (1. - u) + at(a + 1, b) * u;
    let high = at(a, b + 1) * (1. - u) + at(a + 1, b + 1) * u;
    low * (1. - v) + high * v
}
impl Terrain {
    pub fn validate(&self, materials: usize) -> Result<(), String> {
        if !(2..=512).contains(&self.resolution)
            || self.octaves > 12
            || self.octaves == 0
            || self.material >= materials
            || !self.amplitude.is_finite()
            || !self.frequency.is_finite()
            || self.frequency <= 0.
            || self.size.iter().any(|v| !v.is_finite() || *v <= 0.)
            || self.center.to_array().iter().any(|x| !x.is_finite())
        {
            return Err("invalid terrain settings".into());
        }
        if !self.heights.is_empty()
            && (self.heights.len() != (self.resolution as usize + 1).pow(2)
                || self
                    .heights
                    .iter()
                    .any(|v| !v.is_finite() || !(0. ..=1.).contains(v)))
        {
            return Err(
                "terrain heights must be normalized (resolution+1)² grayscale values".into(),
            );
        }
        for l in &self.layers {
            if l.material >= materials
                || [l.min_height, l.max_height, l.min_slope, l.max_slope]
                    .iter()
                    .any(|v| !v.is_finite())
                || l.min_height > l.max_height
                || l.min_slope > l.max_slope
                || l.min_slope < 0.
                || l.max_slope > 90.
            {
                return Err("invalid terrain material region".into());
            }
        }
        Ok(())
    }
    fn grid_height(&self, x: u32, z: u32) -> f32 {
        if !self.heights.is_empty() {
            return self.center.y
                + self.heights[(z * (self.resolution + 1) + x) as usize] * self.amplitude;
        }
        let px = (x as f32 / self.resolution as f32 - 0.5) * self.size[0] + self.center.x;
        let pz = (z as f32 / self.resolution as f32 - 0.5) * self.size[1] + self.center.z;
        let mut value = 0.;
        let mut norm = 0.;
        let mut amp = 1.;
        let mut freq = self.frequency;
        for o in 0..self.octaves {
            let n = noise(px * freq, pz * freq, self.seed.wrapping_add(o));
            value += amp * if self.ridged { 1. - n.abs() } else { n };
            norm += amp;
            amp *= 0.5;
            freq *= 2.;
        }
        self.center.y + value / norm * self.amplitude
    }
    /// Bilinear interpolation of the actual mesh grid (scatter never floats over a
    /// differently evaluated procedural surface). Returns height and slope normal.
    pub fn sample(&self, x: f32, z: f32) -> (f32, Vec3) {
        let fx = ((x - self.center.x) / self.size[0] + 0.5).clamp(0., 1.) * self.resolution as f32;
        let fz = ((z - self.center.z) / self.size[1] + 0.5).clamp(0., 1.) * self.resolution as f32;
        let ix = (fx.floor() as u32).min(self.resolution - 1);
        let iz = (fz.floor() as u32).min(self.resolution - 1);
        let u = fx - ix as f32;
        let v = fz - iz as f32;
        let a = self.grid_height(ix, iz);
        let b = self.grid_height(ix + 1, iz);
        let c = self.grid_height(ix, iz + 1);
        let d = self.grid_height(ix + 1, iz + 1);
        // Match the two triangles' diagonal exactly, rather than a curved patch.
        let (h, dx, dz) = if v <= u {
            (a + (b - a) * u + (d - b) * v, b - a, d - b)
        } else {
            (a + (d - c) * u + (c - a) * v, d - c, c - a)
        };
        (
            h,
            v3(
                -dx * self.resolution as f32 / self.size[0],
                1.,
                -dz * self.resolution as f32 / self.size[1],
            )
            .normalize(),
        )
    }
    /// One mesh per material region, avoiding duplicated vertex arrays per face.
    pub fn meshes(&self) -> Vec<(usize, Mesh)> {
        let mut base = Mesh::default();
        let n = self.resolution;
        for z in 0..=n {
            for x in 0..=n {
                let px = self.center.x + (x as f32 / n as f32 - 0.5) * self.size[0];
                let pz = self.center.z + (z as f32 / n as f32 - 0.5) * self.size[1];
                base.positions.push(v3(px, self.grid_height(x, z), pz));
                let xm = x.saturating_sub(1);
                let xp = (x + 1).min(n);
                let zm = z.saturating_sub(1);
                let zp = (z + 1).min(n);
                let dx = (self.grid_height(xp, z) - self.grid_height(xm, z))
                    / ((xp - xm) as f32 * self.size[0] / n as f32);
                let dz = (self.grid_height(x, zp) - self.grid_height(x, zm))
                    / ((zp - zm) as f32 * self.size[1] / n as f32);
                base.normals.push(v3(-dx, 1., -dz).normalize());
                base.uvs.push([x as f32 / n as f32, z as f32 / n as f32]);
            }
        }
        let mut groups = std::collections::BTreeMap::<usize, Vec<[u32; 3]>>::new();
        for z in 0..n {
            for x in 0..n {
                let a = z * (n + 1) + x;
                let b = a + 1;
                let c = a + n + 1;
                let d = c + 1;
                for tri in [[a, d, b], [a, c, d]] {
                    let p = tri.map(|i| base.positions[i as usize]);
                    let center = (p[0] + p[1] + p[2]) / 3.;
                    let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
                    let slope = normal.y.clamp(-1., 1.).acos().to_degrees();
                    let m = self
                        .layers
                        .iter()
                        .find(|l| {
                            center.y >= l.min_height
                                && center.y <= l.max_height
                                && slope >= l.min_slope
                                && slope <= l.max_slope
                        })
                        .map_or(self.material, |l| l.material);
                    groups.entry(m).or_default().push(tri);
                }
            }
        }
        groups
            .into_iter()
            .map(|(m, indices)| {
                let mut mesh = base.clone();
                mesh.indices = indices;
                (m, mesh)
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heights_and_normals_are_deterministic_and_match_mesh() {
        let t = Terrain::default();
        let a = t.meshes();
        let b = t.meshes();
        assert_eq!(a[0].1.positions, b[0].1.positions);
        for p in &a[0].1.positions {
            let (h, n) = t.sample(p.x, p.z);
            assert!((h - p.y).abs() < 1e-4);
            assert!((n.length() - 1.).abs() < 1e-5);
        }
    }
    #[test]
    fn image_heightfield_and_layering() {
        let t = Terrain {
            resolution: 2,
            heights: vec![0.5; 9],
            layers: vec![TerrainLayer {
                material: 1,
                min_height: 4.,
                max_height: 6.,
                min_slope: 0.,
                max_slope: 1.,
            }],
            ..Default::default()
        };
        t.validate(2).unwrap();
        let a = t.meshes();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].0, 1);
        assert_eq!(a[0].1.indices.len(), 8);
        assert_eq!(t.sample(0., 0.), (5., Vec3::Y));
    }
}
