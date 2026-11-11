//! Validate untrusted scene data before geometry expansion or GPU allocation.
use crate::{
    math::{Vec3, v3},
    scene::{Camera, GpuLight, GpuMaterial, Material, Scene, Softbox},
};

pub const SCENE_BUDGET_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_INSTANCES: usize = 1_000_000;
pub const SESSION_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn target_bytes(width: u32, height: u32, max_binding: u64) -> Result<u64, String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > 4096 || height > 4096 || pixels > 8_847_360 {
        return Err("Studio output exceeds the GPU memory budget; reduce the output size".into());
    }
    let bytes = pixels * 320 + 16 * 1024 * 1024;
    if pixels * 32 > max_binding || bytes > SESSION_BUDGET_BYTES {
        return Err("Studio output exceeds the GPU memory budget; reduce the output size".into());
    }
    Ok(bytes)
}

fn finite(v: Vec3) -> bool {
    v.to_array().iter().all(|v| v.is_finite() && v.abs() <= 1e8)
}
fn color(v: Vec3) -> bool {
    finite(v) && v.to_array().iter().all(|v| *v >= 0.)
}

impl Camera {
    pub fn validate(&self) -> Result<(), String> {
        if !finite(self.position)
            || !finite(self.target)
            || self.focus.is_some_and(|v| !finite(v))
            || !self.fov.is_finite()
            || !(0.1..179.).contains(&self.fov)
            || !self.f_stop.is_finite()
            || !(0. ..=1e6).contains(&self.f_stop)
            || !self.focal_mm.is_finite()
            || !(0.0001..=1e6).contains(&self.focal_mm)
            || (self.target - self.position).length() < 1e-6
        {
            return Err("invalid camera: use finite position, target and lens values".into());
        }
        Ok(())
    }
}
impl Material {
    pub fn validate(&self) -> Result<(), String> {
        if !color(self.base)
            || !color(self.emission)
            || [
                self.metallic,
                self.roughness,
                self.coat,
                self.coat_roughness,
                self.transmission,
                self.specular,
                self.sheen,
                self.dispersion,
                self.droplets,
            ]
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            || !self.ior.is_finite()
            || !(1. ..=10.).contains(&self.ior)
            || !self.anisotropy.is_finite()
            || !(-1. ..=1.).contains(&self.anisotropy)
            || !self.film_nm.is_finite()
            || !(0. ..=100_000.).contains(&self.film_nm)
            || !self.subsurface.is_finite()
            || !(0. ..=1e8).contains(&self.subsurface)
            || self.procedural > 6
        {
            return Err("invalid material: use finite colors and supported physical ranges".into());
        }
        Ok(())
    }
}
impl Softbox {
    pub fn validate(&self) -> Result<(), String> {
        if !finite(self.center)
            || !finite(self.u)
            || !finite(self.v)
            || !color(self.radiance)
            || self.u.cross(self.v).length() < 1e-12
        {
            return Err("invalid softbox: use finite radiance and a nonzero emitting area".into());
        }
        Ok(())
    }
}
impl GpuMaterial {
    pub fn validate(&self) -> Result<(), String> {
        let unit = |v: f32| (0. ..=1.).contains(&v);
        let map = |v: f32| (-1. ..=255.).contains(&v) && v.fract() == 0.;
        if bytemuck::cast_slice::<_, f32>(std::slice::from_ref(self))
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1e8)
            || self.base[..3]
                .iter()
                .chain(self.emit[..3].iter())
                .any(|v| *v < 0. || *v > 1e8)
            || !unit(self.base[3])
            || self.spec.iter().any(|&v| !unit(v))
            || self.emit[3] < 0.
            || !(1. ..=10.).contains(&self.extra[0])
            || !unit(self.extra[1])
            || !unit(self.extra[2])
            || !(-1. ..=1.).contains(&self.extra[3])
            || self
                .maps
                .iter()
                .chain(self.maps2[..3].iter())
                .any(|&v| !map(v))
            || !unit(self.layers[0])
            || !(0. ..=100_000.).contains(&self.layers[1])
            || !unit(self.layers[2])
            || !unit(self.layers[3])
            || self.policy[0] < 0.
            || !(0. ..=63.).contains(&self.policy[1])
            || self.policy[1].fract() != 0.
            || !unit(self.policy[2])
            || !(0. ..=6.).contains(&self.policy[3])
            || self.policy[3].fract() != 0.
            || self.physical_maps.iter().any(|m| {
                !map(m.info[0])
                    || ![0., 1.].contains(&m.info[1])
                    || ![0., 1., 2.].contains(&m.info[2])
                    || ![0., 1., 2.].contains(&m.info[3])
            })
        {
            return Err(
                "invalid prepared material: unsupported physical range or texture index".into(),
            );
        }
        if self.imported[5][3] > 0.5
            && (!(1. ..=10.).contains(&self.imported[3][2])
                || self.imported[3][0] < 0.
                || self.imported[2][3] < 0.
                || !unit(self.imported[0][3])
                || !unit(self.imported[1][3]))
        {
            return Err("invalid imported physical material".into());
        }
        Ok(())
    }
}
impl GpuLight {
    pub fn validate(&self) -> Result<(), String> {
        if bytemuck::cast_slice::<_, f32>(std::slice::from_ref(self))
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1e8)
            || !color(v3(self.radiance[0], self.radiance[1], self.radiance[2]))
        {
            return Err("invalid analytic light data".into());
        }
        let direction = |p: [f32; 4]| v3(p[0], p[1], p[2]).length() > 1e-6;
        let valid = match self.pos[3] {
            0. => {
                v3(self.u[0], self.u[1], self.u[2])
                    .cross(v3(self.v[0], self.v[1], self.v[2]))
                    .length()
                    > 1e-12
            }
            1. => self.u[0] > 0. && self.u[1] >= 0. && (0. ..=4.).contains(&self.u[2]),
            2. => {
                self.u[0] > 0.
                    && self.u[1] >= 0.
                    && (0. ..=4.).contains(&self.u[2])
                    && direction(self.v)
                    && self.v[3] >= -1.
                    && self.radiance[3] > self.v[3]
                    && self.radiance[3] <= 1.0001
            }
            3. => direction(self.pos) && (-1. ..1.).contains(&self.u[0]),
            _ => false,
        };
        if !valid {
            return Err("invalid analytic light direction, radius or cone".into());
        }
        Ok(())
    }
}

/// Conservative scene working-set allowance, including BLAS build scratch,
/// TLAS/instance copies, material data and staging. Targets/textures have their
/// own allowance. Check counts before cloning or building a BVH.
pub fn validate_scene_budget(
    triangles: usize,
    instances: usize,
    materials: usize,
    world_bytes: usize,
) -> Result<usize, String> {
    let bytes = triangles
        .checked_mul(512)
        .and_then(|v| instances.checked_mul(512).and_then(|i| v.checked_add(i)))
        .and_then(|v| {
            materials
                .checked_mul(std::mem::size_of::<crate::scene::GpuMaterial>())
                .and_then(|m| v.checked_add(m))
        })
        .and_then(|v| v.checked_add(world_bytes));
    match bytes {
        Some(bytes) if bytes <= SCENE_BUDGET_BYTES && instances <= MAX_INSTANCES => Ok(bytes),
        _ => Err(
            "Studio scene exceeds the 512 MiB geometry budget; reduce mesh detail or scatter count"
                .into(),
        ),
    }
}

impl Scene {
    pub fn validate(&self) -> Result<(), String> {
        self.world.validate()?;
        self.camera.validate()?;
        if !self.meters_per_unit.is_finite()
            || !(1e-6..=1000.).contains(&self.meters_per_unit)
            || !self.exposure.is_finite()
            || !(0. ..=1e6).contains(&self.exposure)
            || !color(self.backdrop)
            || !color(self.environment.0)
            || !color(self.environment.1)
        {
            return Err("invalid scene scale, exposure or environment color".into());
        }
        if self.objects.len() > 65535
            || self.materials.is_empty()
            || self.materials.len() > 65535
            || self.lights.len() + self.extra_lights.len() > 65535
        {
            return Err("scene exceeds the object/material/light count limit".into());
        }
        let triangles = self
            .objects
            .iter()
            .try_fold(0usize, |n, o| n.checked_add(o.mesh.indices.len()))
            .ok_or("scene triangle count overflow")?;
        let candidates = self
            .scatters
            .iter()
            .try_fold(self.instances.len().max(self.objects.len()), |n, s| {
                n.checked_add(s.count as usize)
            })
            .ok_or("scene instance count overflow")?;
        // A terrain can contribute resolution² × 2 triangles after expansion.
        let triangles = self
            .terrains
            .iter()
            .try_fold(triangles, |n, t| {
                let r = if t.adaptive {
                    t.resolution.max(256)
                } else {
                    t.resolution
                } as usize;
                r.checked_mul(r)
                    .and_then(|v| v.checked_mul(2))
                    .and_then(|v| n.checked_add(v))
            })
            .ok_or("scene terrain count overflow")?;
        let world_bytes = self
            .world
            .hdri
            .as_ref()
            .map_or(0, |h| h.pixels.len().saturating_mul(32));
        validate_scene_budget(triangles, candidates, self.materials.len(), world_bytes)?;
        for m in &self.materials {
            m.validate()?;
        }
        for l in &self.lights {
            l.validate()?;
        }
        for l in &self.extra_lights {
            l.validate()?;
        }
        for i in &self.instances {
            i.validate(self.objects.len())?;
        }
        for o in &self.objects {
            let m = &o.mesh;
            if o.material >= self.materials.len()
                || m.positions.is_empty()
                || m.indices.is_empty()
                || m.normals.len() != m.positions.len()
                || m.uvs.len() != m.positions.len()
                || !m.colors.is_empty() && m.colors.len() != m.positions.len()
                || !m.triangle_materials.is_empty()
                    && (m.triangle_materials.len() != m.indices.len()
                        || m.triangle_materials
                            .iter()
                            .any(|&i| i >= self.materials.len()))
            {
                return Err("invalid mesh attributes or material index".into());
            }
            if m.positions
                .iter()
                .chain(m.normals.iter())
                .any(|&p| !finite(p))
                || m.colors.iter().any(|&c| !color(c))
                || m.uvs
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || v.abs() > 1e8)
                || m.indices
                    .iter()
                    .flatten()
                    .any(|&i| i as usize >= m.positions.len())
            {
                return Err("invalid mesh coordinates, texture coordinates or indices".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_scenes_are_rejected_before_allocating_geometry() {
        assert!(validate_scene_budget(20_000, 100_000, 20, 0).is_ok());
        assert!(
            validate_scene_budget(2_000_000, 0, 1, 0)
                .unwrap_err()
                .contains("reduce mesh detail")
        );
        assert!(validate_scene_budget(0, 1_000_001, 1, 0).is_err());
        assert!(validate_scene_budget(usize::MAX, usize::MAX, 1, 0).is_err());
    }
    #[test]
    fn oversized_targets_fail_before_creating_a_device() {
        assert!(target_bytes(1920, 1080, u64::MAX).is_ok());
        assert!(target_bytes(0, 1080, u64::MAX).is_err());
        assert!(target_bytes(u32::MAX, u32::MAX, u64::MAX).is_err());
        assert!(target_bytes(4096, 2160, u64::MAX).is_err());
        assert!(target_bytes(1920, 1080, 32 * 1024 * 1024).is_err());
    }
    #[test]
    fn packed_materials_and_lights_reject_invalid_physical_parameters() {
        let original = crate::scene::preset("hero", "blue", 0.)
            .unwrap()
            .prepare()
            .materials[0];
        original.validate().unwrap();
        for field in 0..6 {
            let mut m = original;
            match field {
                0 => m.extra[0] = 0.,
                1 => m.base[3] = 2.,
                2 => m.spec[0] = -0.1,
                3 => m.maps[0] = 256.,
                4 => m.policy[1] = 1.5,
                _ => m.physical_maps[0].info[1] = 2.,
            }
            assert!(m.validate().is_err(), "material field {field}");
        }
        assert!(
            GpuLight::sun(Vec3::ZERO, 0.01, Vec3::splat(1.))
                .validate()
                .is_err()
        );
        assert!(
            GpuLight::sphere(Vec3::ZERO, 0., Vec3::splat(1.))
                .validate()
                .is_err()
        );
        assert!(
            GpuLight::sun(Vec3::Y, 0., Vec3::splat(1.))
                .validate()
                .is_err()
        );
        assert!(
            GpuLight::sun(Vec3::Y, 0.01, Vec3::splat(1.))
                .validate()
                .is_ok()
        );
        let mut huge = original;
        huge.emit[3] = f32::MAX;
        assert!(
            huge.validate().is_err(),
            "finite subsurface overflow must fail"
        );
        let huge = GpuLight::sphere(Vec3::splat(f32::MAX), 1., Vec3::splat(1.));
        assert!(
            huge.validate().is_err(),
            "finite light coordinate overflow must fail"
        );
    }
}
