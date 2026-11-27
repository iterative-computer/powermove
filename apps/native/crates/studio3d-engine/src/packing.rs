//! Pure adapters for the pinned engine's public scene contract.
use pm_studio::{
    math::Vec3,
    scene::{GpuLight, GpuMaterial, Material, Scene},
};

pub trait MaterialExt {
    fn gpu(&self) -> GpuMaterial;
}
impl MaterialExt for Material {
    fn gpu(&self) -> GpuMaterial {
        GpuMaterial {
            base: self.base.to4(self.metallic),
            spec: [
                self.roughness.max(0.002),
                self.coat,
                self.coat_roughness.max(0.002),
                self.transmission,
            ],
            emit: self.emission.to4(self.subsurface),
            extra: [self.ior, self.specular, 1., self.anisotropy],
            maps: [-1., if self.screen { 0. } else { -1. }, -1., -1.],
            layers: [self.sheen, self.film_nm, self.dispersion, self.droplets],
            policy: [1., 7., 0., self.procedural as f32],
            maps2: [-1., -1., -1., 1.],
            imported: [[0.; 4]; 6],
            physical_maps: [Default::default(); 18],
        }
    }
}
pub trait SceneExt {
    fn gpu_materials(&self) -> Vec<GpuMaterial>;
    fn gpu_lights(&self) -> Vec<GpuLight>;
}
impl SceneExt for Scene {
    fn gpu_materials(&self) -> Vec<GpuMaterial> {
        self.materials.iter().map(MaterialExt::gpu).collect()
    }
    fn gpu_lights(&self) -> Vec<GpuLight> {
        self.lights
            .iter()
            .map(|l| GpuLight {
                pos: l.center.to4(0.),
                u: l.u.to4(4. * l.u.cross(l.v).length()),
                v: l.v.to4(0.),
                radiance: l.radiance.to4(0.),
            })
            .chain(self.extra_lights.iter().copied())
            .chain(self.world.sun())
            .collect()
    }
}
pub fn tangent(p: [Vec3; 3], uv: [[f32; 2]; 3]) -> [f32; 4] {
    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
    let (d1, d2) = (
        [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]],
        [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]],
    );
    let det = d1[0] * d2[1] - d2[0] * d1[1];
    if det.abs() < 1e-12 {
        return [0.; 4];
    }
    let t = (e1 * d2[1] - e2 * d1[1]) / det;
    let b = (e2 * d1[0] - e1 * d2[0]) / det;
    let n = e1.cross(e2).normalize();
    t.normalize()
        .to4(if n.cross(t).dot(b) < 0. { -1. } else { 1. })
}
