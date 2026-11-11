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
    /// Additional fine height noise as a fraction of amplitude (0 preserves old scenes).
    pub detail: f32,
    /// Mass-conserving thermal erosion iterations, 0..64.
    pub erosion: u32,
    /// Camera-dependent grid density, bounded to 256 quads per side.
    pub adaptive: bool,
    /// Smooth region overlap, as a fraction of amplitude / 45 slope degrees.
    pub blend: f32,
    /// Procedural grass, soil, rock, beach and snow beneath authored regions.
    pub natural: bool,
    pub water_level: f32,
    pub snow_level: f32,
    /// Optional row-major signed normalized heights (-1..1), (resolution+1)².
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
            detail: 0.,
            erosion: 0,
            adaptive: false,
            blend: 0.,
            natural: false,
            water_level: -1e9,
            snow_level: 1e9,
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
pub(crate) fn noise(x: f32, z: f32, seed: u32) -> f32 {
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
    /// Decode and resize normalized grayscale image data for this heightfield.
    pub fn load_height_image(&mut self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        if !(2..=512).contains(&self.resolution) {
            return Err("terrain resolution must be 2..512".into());
        }
        let image = image::ImageReader::open(path)
            .map_err(|e| e.to_string())?
            .with_guessed_format()
            .map_err(|e| e.to_string())?
            .decode()
            .map_err(|e| e.to_string())?
            .into_rgb32f();
        // Match the scene-contract image sampler: encoded integer RGB is height
        // data, not radiance. Endpoint-aligned nearest sampling is intentional.
        let n = self.resolution;
        self.heights.clear();
        for y in 0..=n {
            for x in 0..=n {
                let sx = (x as f64 / n as f64 * (image.width() - 1) as f64).round() as u32;
                let sy = (y as f64 / n as f64 * (image.height() - 1) as f64).round() as u32;
                let p = image.get_pixel(sx, sy);
                self.heights.push((0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]).clamp(0., 1.));
            }
        }
        Ok(())
    }
    pub fn validate(&self, materials: usize) -> Result<(), String> {
        if !(2..=512).contains(&self.resolution)
            || self.octaves > 12
            || self.octaves == 0
            || self.erosion > 64
            || !self.detail.is_finite()
            || !(0. ..=0.25).contains(&self.detail)
            || !self.blend.is_finite()
            || !(0. ..=1.).contains(&self.blend)
            || !self.water_level.is_finite()
            || !self.snow_level.is_finite()
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
                    .any(|v| !v.is_finite() || !(-1. ..=1.).contains(v)))
        {
            return Err(
                "terrain heights must be (resolution+1)² values in -1..1".into(),
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
        self.center.y
            + (value / norm
                + self.detail
                    * noise(
                        px * self.frequency * 24.,
                        pz * self.frequency * 24.,
                        self.seed ^ 0xaced,
                    ))
                * self.amplitude
    }
    /// Uniform grid LOD chosen from projected cell size. It changes only on a
    /// scene preparation, never re-tessellates while a resident camera is dragged.
    pub fn resolution_for_camera(&self, camera: &crate::scene::Camera) -> u32 {
        if !self.adaptive {
            return self.resolution;
        }
        let dx = ((camera.position.x - self.center.x).abs() - self.size[0] * 0.5).max(0.);
        let dz = ((camera.position.z - self.center.z).abs() - self.size[1] * 0.5).max(0.);
        let distance = (dx * dx + dz * dz + (camera.position.y - self.center.y).powi(2))
            .sqrt()
            .max(1.);
        let cell = distance * 2. * (camera.fov.to_radians() * 0.5).tan() * 4. / 1080.;
        let requested = (self.size[0].max(self.size[1]) / cell)
            .ceil()
            .clamp(2., 256.) as u32;
        requested
            .next_power_of_two()
            .clamp(self.resolution.min(256), 256)
    }
    /// Bake detail and erosion once. Scatter samples this same final grid.
    pub fn baked(&self, resolution: u32) -> Self {
        let mut out = self.clone();
        out.resolution = resolution;
        let n = resolution as usize;
        let mut heights = Vec::with_capacity((n + 1) * (n + 1));
        for z in 0..=n {
            for x in 0..=n {
                let h = if self.heights.is_empty() {
                    out.grid_height(x as u32, z as u32)
                } else if resolution == self.resolution {
                    self.grid_height(x as u32, z as u32)
                } else {
                    self.sample_grid(
                        self.center.x + (x as f32 / resolution as f32 - 0.5) * self.size[0],
                        self.center.z + (z as f32 / resolution as f32 - 0.5) * self.size[1],
                    )
                    .0
                };
                heights.push(h);
            }
        }
        // Four-neighbour Jacobi flux: transfer a quarter of the excess over a
        // 34-degree talus. Equal/opposite transfers conserve sediment and remain stable.
        let talus = (34f32.to_radians().tan()) * self.size[0].min(self.size[1]) / resolution as f32;
        let mut delta = vec![0.; heights.len()];
        for _ in 0..self.erosion {
            delta.fill(0.);
            for z in 0..=n {
                for x in 0..=n {
                    let i = z * (n + 1) + x;
                    for j in [
                        if x < n { Some(i + 1) } else { None },
                        if z < n { Some(i + n + 1) } else { None },
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let diff = heights[i] - heights[j];
                        let flux = diff.signum() * (diff.abs() - talus).max(0.) * 0.125;
                        delta[i] -= flux;
                        delta[j] += flux;
                    }
                }
            }
            for (h, d) in heights.iter_mut().zip(&delta) {
                *h += d;
            }
        }
        // Stored fields are normalized like image heights. Generated noise can
        // be negative, so bake uses an affine range without clamping the shape.
        let lo = heights.iter().copied().fold(f32::MAX, f32::min);
        let hi = heights.iter().copied().fold(f32::MIN, f32::max);
        out.center.y = lo;
        out.amplitude = (hi - lo).max(1e-6);
        out.heights = heights
            .into_iter()
            .map(|h| (h - lo) / out.amplitude)
            .collect();
        out.erosion = 0;
        out.detail = 0.;
        out.adaptive = false;
        out
    }
    pub fn sample(&self, x: f32, z: f32) -> (f32, Vec3) {
        if self.erosion > 0 {
            self.baked(self.resolution).sample_grid(x, z)
        } else {
            self.sample_grid(x, z)
        }
    }
    /// Bilinear interpolation of the actual mesh grid (scatter never floats over a
    /// differently evaluated procedural surface). Returns height and slope normal.
    fn sample_grid(&self, x: f32, z: f32) -> (f32, Vec3) {
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
    /// One smoothly tinted mesh for natural/blended ground; all indices and
    /// normals are shared across cover boundaries instead of splitting faces.
    pub fn blended_mesh(&self, materials: &[crate::scene::Material]) -> Mesh {
        if self.erosion > 0 {
            return self.baked(self.resolution).blended_mesh(materials);
        }
        let mut plain = self.clone();
        plain.layers.clear();
        let mut mesh = plain.meshes().remove(0).1;
        let smooth = |a: f32, b: f32, x: f32| {
            let t = ((x - a) / (b - a).max(1e-6)).clamp(0., 1.);
            t * t * (3. - 2. * t)
        };
        let hw = (self.amplitude.abs() * self.blend).max(0.01);
        let sw = (45. * self.blend).max(0.1);
        for (i, &p) in mesh.positions.iter().enumerate() {
            let normal = mesh.normals[i];
            let slope = normal.y.clamp(-1., 1.).acos().to_degrees();
            let x = (i % (self.resolution as usize + 1)) as u32;
            let z = (i / (self.resolution as usize + 1)) as u32;
            let h = self.grid_height(x, z);
            let curvature = (self.grid_height(x.saturating_sub(1), z)
                + self.grid_height((x + 1).min(self.resolution), z)
                + self.grid_height(x, z.saturating_sub(1))
                + self.grid_height(x, (z + 1).min(self.resolution))
                - 4. * h)
                * self.resolution as f32
                / self.size[0].max(0.001);
            let variation = noise(p.x * 0.16, p.z * 0.16, self.seed ^ 91) * 0.15;
            let grass = v3(0.115, 0.16, 0.045) * (1. + variation);
            let soil = v3(0.13, 0.085, 0.04);
            let rock = v3(0.25, 0.23, 0.19);
            let mut color = materials[self.material].base;
            let mut roughness = materials[self.material].roughness;
            let mut metallic = materials[self.material].metallic;
            if self.natural {
                roughness = 0.88;
                metallic = 0.;
                let earth = smooth(0.05, 0.5, curvature.abs() + variation * 0.5) * 0.65;
                color = grass * (1. - earth) + soil * earth;
                let steep = smooth(25., 48., slope + variation * 20.);
                color = color * (1. - steep) + rock * steep;
                let beach = 1. - smooth(self.water_level + 0.15, self.water_level + 1.3, p.y);
                color = color * (1. - beach) + v3(0.48, 0.38, 0.22) * beach;
                let snow = smooth(self.snow_level - hw, self.snow_level + hw, p.y)
                    * (1. - smooth(40., 65., slope));
                color = color * (1. - snow) + v3(0.82, 0.86, 0.87) * snow;
            }
            let mut remaining = 1.;
            let mut authored = Vec3::ZERO;
            let mut authored_roughness = 0.;
            let mut authored_metallic = 0.;
            for l in &self.layers {
                let weight = smooth(l.min_height - hw, l.min_height + hw, p.y)
                    * (1. - smooth(l.max_height - hw, l.max_height + hw, p.y))
                    * (if l.min_slope <= 0. {
                        1.
                    } else {
                        smooth(l.min_slope - sw, l.min_slope + sw, slope)
                    })
                    * (if l.max_slope >= 90. {
                        1.
                    } else {
                        1. - smooth(l.max_slope - sw, l.max_slope + sw, slope)
                    });
                authored = authored + materials[l.material].base * (remaining * weight);
                authored_roughness += materials[l.material].roughness * remaining * weight;
                authored_metallic += materials[l.material].metallic * remaining * weight;
                remaining *= 1. - weight;
            }
            // Natural ground uses object-space procedural mapping. Its UV pair
            // carries the blended roughness/metallic fields without new GPU buffers.
            mesh.uvs[i] = [
                authored_roughness + roughness * remaining,
                authored_metallic + metallic * remaining,
            ];
            mesh.colors
                .push((authored + color * remaining) * (1. + variation * 0.35));
        }
        mesh
    }
    /// One mesh per material region, avoiding duplicated vertex arrays per face.
    pub fn meshes(&self) -> Vec<(usize, Mesh)> {
        if self.erosion > 0 {
            return self.baked(self.resolution).meshes();
        }
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
    fn signed_document_heights_preserve_negative_elevation() {
        let t = Terrain { resolution: 2, amplitude: 4., heights: vec![-0.5; 9], ..Default::default() };
        t.validate(1).unwrap();
        assert_eq!(t.sample(0., 0.).0, -2.);
    }
    #[test]
    fn retained_height_image_uses_encoded_luminance_and_endpoint_sampling() {
        let path = std::env::temp_dir().join(format!("pm-height-{}.bin", std::process::id()));
        let image = image::RgbImage::from_fn(2, 2, |x, y| image::Rgb(match (x, y) {
            (0, 0) => [128, 128, 128], (1, 0) => [255, 0, 0], (0, 1) => [0, 255, 0], _ => [0, 0, 255],
        }));
        image.save_with_format(&path, image::ImageFormat::Png).unwrap();
        let mut t = Terrain { resolution: 2, ..Default::default() };
        let result = t.load_height_image(&path);
        std::fs::remove_file(path).unwrap();
        result.unwrap();
        assert!((t.heights[0] - 128. / 255.).abs() < 1e-6);
        assert!((t.heights[1] - 0.2126).abs() < 1e-6);
        assert!((t.heights[6] - 0.7152).abs() < 1e-6);
        assert!((t.heights[8] - 0.0722).abs() < 1e-6);
    }
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
    fn erosion_conserves_height_and_scatter_uses_the_eroded_grid() {
        let t = Terrain {
            resolution: 16,
            size: [8., 8.],
            amplitude: 5.,
            frequency: 0.8,
            erosion: 24,
            detail: 0.02,
            ..Default::default()
        };
        let baked = t.baked(t.resolution);
        let mut before = 0.;
        let mut after = 0.;
        for z in 0..=t.resolution {
            for x in 0..=t.resolution {
                before += t.grid_height(x, z);
                after += baked.grid_height(x, z);
            }
        }
        assert!((before - after).abs() < 0.001);
        let m = t.meshes().remove(0).1;
        for p in &m.positions {
            assert!((t.sample(p.x, p.z).0 - p.y).abs() < 0.0001);
        }
        assert_ne!(t.grid_height(8, 8), baked.grid_height(8, 8));
    }
    #[test]
    fn camera_resolution_is_bounded_and_blending_keeps_shared_smooth_normals() {
        let t = Terrain {
            resolution: 32,
            adaptive: true,
            natural: true,
            blend: 0.2,
            ..Default::default()
        };
        let mut camera = crate::scene::Camera::lens(v3(0., 5., 20.), Vec3::ZERO, 35., 0.);
        let near = t.resolution_for_camera(&camera);
        camera.position = v3(0., 5000., 5000.);
        let far = t.resolution_for_camera(&camera);
        assert!(near > far && near <= 256 && far >= 32);
        let m = t.blended_mesh(&[crate::scene::Material::dielectric(Vec3::splat(0.3), 0.9)]);
        assert_eq!(m.positions.len(), 33 * 33);
        assert_eq!(m.colors.len(), m.positions.len());
        assert!(
            m.normals
                .iter()
                .all(|n| n.y > 0. && (n.length() - 1.).abs() < 0.00001)
        );
        assert!(m.normals.windows(2).any(|n| n[0] != n[1]));
    }
    #[test]
    fn authored_roughness_and_metallic_survive_blended_ground() {
        let t = Terrain {
            resolution: 2,
            amplitude: 0.,
            blend: 0.2,
            natural: true,
            layers: vec![TerrainLayer {
                material: 1,
                min_height: -1000.,
                max_height: 1000.,
                min_slope: 0.,
                max_slope: 90.,
            }],
            ..Default::default()
        };
        let materials = [
            crate::scene::Material::dielectric(Vec3::splat(0.1), 0.9),
            crate::scene::Material {
                metallic: 0.6,
                ..crate::scene::Material::dielectric(Vec3::splat(0.4), 0.23)
            },
        ];
        let mesh = t.blended_mesh(&materials);
        assert!(
            mesh.uvs
                .iter()
                .all(|uv| (uv[0] - 0.23).abs() < 1e-5 && (uv[1] - 0.6).abs() < 1e-5)
        );
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
