//! Seeded surface scatter: produces transforms, never copies source vertices.
use crate::{
    math::{Vec3, v3},
    scene::Scene,
    terrain::{Terrain, hash},
};
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Instance {
    pub object: usize,
    pub position: Vec3,
    /// Euler rotations in degrees, applied X then Y then Z.
    pub rotation: Vec3,
    pub scale: Vec3,
    /// Optional layer override; omitted uses the prototype object's layer.
    pub layer: Option<u32>,
}
impl Default for Instance {
    fn default() -> Self {
        Self {
            object: 0,
            position: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::splat(1.),
            layer: None,
        }
    }
}
impl Instance {
    pub fn matrix(&self) -> [f32; 12] {
        let (sx, cx) = self.rotation.x.to_radians().sin_cos();
        let (sy, cy) = self.rotation.y.to_radians().sin_cos();
        let (sz, cz) = self.rotation.z.to_radians().sin_cos();
        [
            cz * cy * self.scale.x,
            (cz * sy * sx - sz * cx) * self.scale.y,
            (cz * sy * cx + sz * sx) * self.scale.z,
            self.position.x,
            sz * cy * self.scale.x,
            (sz * sy * sx + cz * cx) * self.scale.y,
            (sz * sy * cx - cz * sx) * self.scale.z,
            self.position.y,
            -sy * self.scale.x,
            cy * sx * self.scale.y,
            cy * cx * self.scale.z,
            self.position.z,
        ]
    }
    pub fn validate(&self, objects: usize) -> Result<(), String> {
        if self.object >= objects
            || self
                .position
                .to_array()
                .iter()
                .chain(self.rotation.to_array().iter())
                .any(|x| !x.is_finite() || x.abs()>1e8)
            || self
                .scale
                .to_array()
                .iter()
                .any(|x| !x.is_finite() || *x < 1e-4 || *x>10000.)
            || self.layer.is_some_and(|l| l >= 8_388_607)
        {
            Err("invalid instance transform, source object or layer".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Scatter {
    pub objects: Vec<usize>,
    /// Optional [near, medium, far] source indices; the first is in objects.
    pub lods: Vec<[usize; 3]>,
    pub seed: u32,
    /// Upper bound on accepted instances. Density thins candidates deterministically.
    pub count: u32,
    pub density: f32,
    pub terrain: Option<usize>,
    /// If not terrain, sample this object's triangles by surface area.
    pub surface: Option<usize>,
    /// Flat-plane region when both terrain and surface are omitted.
    pub center: Vec3,
    pub size: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: [f32; 2],
    pub min_height: f32,
    pub max_height: f32,
    pub min_slope: f32,
    pub max_slope: f32,
    pub exclusions: Vec<Exclusion>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Exclusion {
    pub center: Vec3,
    pub radius: f32,
}
impl Default for Scatter {
    fn default() -> Self {
        Self {
            objects: vec![0],
            seed: 1,
            lods: vec![],
            count: 100,
            density: 1.,
            terrain: None,
            surface: None,
            center: Vec3::ZERO,
            size: [100., 100.],
            scale: [0.8, 1.2],
            rotation: [0., 360.],
            min_height: -1e9,
            max_height: 1e9,
            min_slope: 0.,
            max_slope: 60.,
            exclusions: vec![],
        }
    }
}
impl Scatter {
    pub fn validate(&self, s: &Scene) -> Result<(), String> {
        if self.objects.is_empty()
            || self.objects.iter().any(|&i| i >= s.objects.len())
            || self.lods.iter().flatten().any(|&i| i >= s.objects.len())
            || self.count > 1_000_000
            || self.terrain.is_some_and(|i| i >= s.terrains.len())
            || self.surface.is_some_and(|i| i >= s.objects.len())
            || self.terrain.is_some() && self.surface.is_some()
            || !(0. ..=1.).contains(&self.density)
            || self.size.iter().any(|x| !x.is_finite() || *x <= 0.)
            || self.scale[0] <= 0.
            || self.scale[1] < self.scale[0]
            || self.max_height < self.min_height
            || !(0. ..=90.).contains(&self.max_slope)
            || !(0. ..=self.max_slope).contains(&self.min_slope)
            || self.exclusions.iter().any(|e| {
                !e.radius.is_finite()
                    || e.radius < 0.
                    || e.center.to_array().iter().any(|x| !x.is_finite())
            })
            || [
                self.scale[0],
                self.scale[1],
                self.rotation[0],
                self.rotation[1],
                self.min_height,
                self.max_height,
            ]
            .iter()
            .any(|x| !x.is_finite())
        {
            return Err("invalid scatter settings".into());
        }
        Ok(())
    }
    pub fn generate(&self, s: &Scene) -> Vec<Instance> {
        let mut state = self.seed;
        let mut rnd = || {
            state = hash(state);
            (state >> 8) as f32 * (1. / 16777216.)
        };
        let baked_terrain = self
            .terrain
            .filter(|&i| s.terrains[i].erosion > 0)
            .map(|i| s.terrains[i].baked(s.terrains[i].resolution));
        let terrain: Option<&Terrain> = baked_terrain
            .as_ref()
            .or_else(|| self.terrain.map(|i| &s.terrains[i]));
        let lods: Vec<_> = self
            .lods
            .iter()
            .map(|chain| {
                let (lo, hi) = s.objects[chain[0]].mesh.bounds();
                let d = hi - lo;
                (*chain, d.x.max(d.y).max(d.z))
            })
            .collect();
        let surface = self.surface.map(|i| &s.objects[i].mesh);
        let mut cdf = Vec::new();
        let mut total = 0.;
        if let Some(m) = surface {
            for t in &m.indices {
                let [a, b, c] = t.map(|i| m.positions[i as usize]);
                total += (b - a).cross(c - a).length() * 0.5;
                cdf.push(total);
            }
        }
        let mut out = Vec::with_capacity(self.count as usize);
        // Attempts are exactly count: changing limits or density does not reroll
        // later placements. This makes art direction stable as settings change.
        for _ in 0..self.count {
            let r = [rnd(), rnd(), rnd(), rnd(), rnd(), rnd()];
            if r[0] >= self.density {
                continue;
            }
            let (p, n) = if let Some(m) = surface {
                if total <= 0. {
                    continue;
                }
                let i = cdf
                    .partition_point(|&x| x < r[1] * total)
                    .min(m.indices.len() - 1);
                let [a, b, c] = m.indices[i].map(|i| m.positions[i as usize]);
                let u = r[2].sqrt();
                let v = r[3];
                (
                    a * (1. - u) + b * (u * (1. - v)) + c * (u * v),
                    (b - a).cross(c - a).normalize(),
                )
            } else {
                let x = self.center.x + (r[1] - 0.5) * self.size[0];
                let z = self.center.z + (r[2] - 0.5) * self.size[1];
                if terrain.is_some_and(|t| {
                    (x - t.center.x).abs() > t.size[0] * 0.5
                        || (z - t.center.z).abs() > t.size[1] * 0.5
                }) {
                    continue;
                }
                let (y, n) = terrain.map_or((self.center.y, Vec3::Y), |t| t.sample(x, z));
                (v3(x, y, z), n)
            };
            if p.y < self.min_height
                || p.y > self.max_height
                || n.y.clamp(-1., 1.).acos().to_degrees() > self.max_slope
                || n.y.clamp(-1., 1.).acos().to_degrees() < self.min_slope
                || self.exclusions.iter().any(|e| {
                    let d = p - e.center;
                    d.x * d.x + d.z * d.z < e.radius * e.radius
                })
            {
                continue;
            }
            let scale = self.scale[0] + (self.scale[1] - self.scale[0]) * r[4];
            let source =
                self.objects[(r[5] * self.objects.len() as f32) as usize % self.objects.len()];
            let object = lods.iter().find(|(chain, _)| chain[0] == source).map_or(
                source,
                |(chain, diameter)| {
                    chain[crate::nature::select_lod(
                        diameter * scale,
                        (p - s.camera.position).length(),
                    )]
                },
            );
            out.push(Instance {
                object,
                position: p,
                rotation: v3(
                    0.,
                    self.rotation[0] + (self.rotation[1] - self.rotation[0]) * r[3],
                    0.,
                ),
                scale: Vec3::splat(scale),
                layer: None,
            });
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seed_repeats_and_exclusions_are_respected() {
        let s = crate::scene::preset("hero", "blue", 0.).unwrap();
        let a = Scatter {
            count: 10000,
            exclusions: vec![Exclusion {
                center: Vec3::ZERO,
                radius: 10.,
            }],
            ..Default::default()
        };
        let first = a.generate(&s);
        assert_eq!(first, a.generate(&s));
        assert!(first.len() > 9000);
        assert!(
            first
                .iter()
                .all(|p| p.position.x.powi(2) + p.position.z.powi(2) >= 100.)
        );
        let b = Scatter { seed: 2, ..a };
        assert_ne!(first, b.generate(&s));
    }
    #[test]
    fn lod_changes_geometry_without_rerolling_the_forest() {
        let mut s = crate::scene::preset("hero", "blue", 0.).unwrap();
        let n = crate::nature::Nature {
            species: crate::nature::Species::Conifer,
            seed: 41,
        };
        s.objects = (0..3)
            .map(|lod| crate::scene::Object {
                mesh: n.mesh(lod),
                material: 0,
            })
            .collect();
        s.materials = n.materials().to_vec();
        s.camera.position = v3(0., 1., 0.);
        let scatter = Scatter {
            objects: vec![0],
            lods: vec![[0, 1, 2]],
            count: 3000,
            size: [2000., 2000.],
            scale: [1., 1.],
            ..Default::default()
        };
        scatter.validate(&s).unwrap();
        let a = scatter.generate(&s);
        let b = Scatter {
            lods: vec![],
            ..scatter.clone()
        }
        .generate(&s);
        assert!(a.iter().any(|i| i.object == 0));
        assert!(a.iter().any(|i| i.object == 1));
        assert!(a.iter().any(|i| i.object == 2));
        assert!(a.iter().zip(&b).all(|(a, b)| a.position == b.position
            && a.scale == b.scale
            && a.rotation == b.rotation));
    }
    #[test]
    fn terrain_height_and_slope_limits() {
        let mut s = crate::scene::preset("hero", "blue", 0.).unwrap();
        s.terrains.push(Terrain {
            amplitude: 0.,
            ..Default::default()
        });
        let a = Scatter {
            terrain: Some(0),
            count: 1000,
            min_height: 1.,
            ..Default::default()
        };
        assert!(a.generate(&s).is_empty());
    }
}
