//! Procedural environment corpus: no external or licensed binary assets.
use crate::{
    environment::{Sky, World},
    math::{Vec3, v3},
    mesh::{self, Mesh},
    nature::{Nature, Species},
    scatter::{Instance, Scatter},
    scene::{Camera, GpuLight, Material, Object, Scene},
    terrain::Terrain,
};
fn cube(size: Vec3) -> Mesh {
    let mut m = Mesh::default();
    let h = size * 0.5;
    for (n, u, v) in [
        (v3(1., 0., 0.), v3(0., 0., -1.), Vec3::Y),
        (v3(-1., 0., 0.), v3(0., 0., 1.), Vec3::Y),
        (Vec3::Y, v3(1., 0., 0.), v3(0., 0., -1.)),
        (-Vec3::Y, v3(1., 0., 0.), v3(0., 0., 1.)),
        (v3(0., 0., 1.), v3(1., 0., 0.), Vec3::Y),
        (v3(0., 0., -1.), v3(-1., 0., 0.), Vec3::Y),
    ] {
        let at = m.positions.len() as u32;
        for (a, b) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
            let p = n + u * a + v * b;
            m.positions.push(v3(p.x * h.x, p.y * h.y, p.z * h.z));
            m.normals.push(n);
            m.uvs.push([(a + 1.) / 2., (b + 1.) / 2.]);
        }
        m.indices
            .extend([[at, at + 1, at + 2], [at, at + 2, at + 3]]);
    }
    m
}
fn add_box(s: &mut Scene, at: Vec3, size: Vec3, material: usize) {
    s.objects.push(Object {
        mesh: cube(size).placed(0., 0., at),
        material,
    });
}
fn base(sky: &str) -> Scene {
    Scene {
        meters_per_unit: 1.,
        world: World {
            sky: Sky::preset(sky),
            ..Default::default()
        },
        objects: vec![],
        materials: vec![],
        lights: vec![],
        camera: Camera::lens(v3(10., 5., 20.), Vec3::ZERO, 35., 0.),
        backdrop: Vec3::ZERO,
        environment: (Vec3::ZERO, Vec3::ZERO),
        exposure: 1.,
        shadow_catcher: false,
        extra_lights: vec![],
        layers: None,
        instances: vec![],
        scatters: vec![],
        terrains: vec![],
        displacements: vec![],
    }
}
pub fn scene(name: &str) -> Option<Scene> {
    Some(match name {
        "env-room" => room(),
        "env-street" => street(),
        "env-landscape" => landscape(50_000),
        "env-landscape-100k" => landscape(100_000),
        "env-default-landscape" => default_landscape(),
        "env-nature-closeup" => nature_closeup(),
        _ => return None,
    })
}
fn room() -> Scene {
    let mut s = base("golden-hour");
    s.world.sky.as_mut().unwrap().elevation = 24.;
    s.world.sky.as_mut().unwrap().azimuth = -110.;
    s.world.atmosphere.density = 0.022;
    s.world.atmosphere.anisotropy = 0.55;
    s.world.atmosphere.distance = 35.;
    s.world.volumes.push(crate::environment::Volume {
        shape: crate::environment::VolumeShape::Box,
        center: v3(-2.9, 2.4, -1.5),
        size: v3(2., 2., 2.5),
        density: 0.035,
        color: Vec3::splat(0.9),
        anisotropy: 0.65,
    });
    s.materials = vec![
        Material::dielectric(v3(0.62, 0.57, 0.47), 0.9),
        Material::dielectric(v3(0.23, 0.105, 0.045), 0.48),
        Material::dielectric(v3(0.12, 0.18, 0.18), 0.9),
        Material::metal(v3(0.6, 0.4, 0.18), 0.3),
        Material::dielectric(v3(0.07, 0.075, 0.08), 0.9),
        Material::dielectric(v3(0.55, 0.16, 0.06), 0.6),
    ];
    add_box(&mut s, v3(0., -0.12, 0.), v3(12., 0.24, 18.), 1);
    // Back and right wall; the left wall surrounds a real geometric opening.
    add_box(&mut s, v3(0., 3., -6.), v3(10.4, 6., 0.25), 0);
    add_box(&mut s, v3(5.1, 3., 0.), v3(0.25, 6., 12.), 0);
    add_box(&mut s, v3(-5.1, 0.55, 0.), v3(0.25, 1.1, 12.), 0);
    add_box(&mut s, v3(-5.1, 5., 0.), v3(0.25, 2., 12.), 0);
    add_box(&mut s, v3(-5.1, 2.55, -5.), v3(0.25, 2.9, 2.), 0);
    add_box(&mut s, v3(-5.1, 2.55, 3.5), v3(0.25, 2.9, 5.), 0);
    add_box(&mut s, v3(0., 6.1, 0.), v3(10.4, 0.25, 12.), 0);
    for z in [-3.8, -1.5, 0.8] {
        add_box(&mut s, v3(-5., 2.55, z), v3(0.35, 3., 0.06), 1);
    }
    add_box(&mut s, v3(-5., 2.5, -1.5), v3(0.35, 0.06, 4.7), 1);
    // Floorboards and rug create readable scale without texture assets.
    for i in 0..20 {
        add_box(
            &mut s,
            v3(-4.8 + i as f32 * 0.5, 0.008, 0.),
            v3(0.012, 0.012, 12.),
            4,
        );
    }
    add_box(&mut s, v3(0.4, 0.025, 0.), v3(5.8, 0.025, 4.8), 2);
    add_box(&mut s, v3(1.7, 0.55, -3.8), v3(4.4, 0.6, 1.8), 2);
    add_box(&mut s, v3(1.7, 1.2, -4.55), v3(4.4, 1.2, 0.3), 2);
    for x in [-0.5, 3.9] {
        add_box(&mut s, v3(x, 0.9, -3.8), v3(0.3, 0.8, 1.8), 2);
    }
    add_box(&mut s, v3(0.5, 0.85, 0.), v3(3.8, 0.16, 1.8), 1);
    for x in [-1., 2.] {
        for z in [-0.6, 0.6] {
            add_box(&mut s, v3(x, 0.4, z), v3(0.12, 0.8, 0.12), 3);
        }
    }
    s.objects.push(Object {
        mesh: mesh::sphere(v3(-0.2, 1.12, 0.), 0.2, 20, 32),
        material: 5,
    });
    for (i, p) in [v3(3.8, 2.5, -3.6), v3(-2., 4.8, -1.)]
        .into_iter()
        .enumerate()
    {
        s.extra_lights
            .push(GpuLight::sphere(p, 0.12, v3(140., 70., 25.)));
        add_box(
            &mut s,
            p - v3(0., 0.22, 0.),
            v3(if i == 0 { 0.7 } else { 1. }, 0.4, 0.7),
            3,
        );
        add_box(
            &mut s,
            if i == 0 {
                p - v3(0., 1.5, 0.)
            } else {
                p + v3(0., 0.6, 0.)
            },
            v3(0.04, if i == 0 { 2.6 } else { 1.2 }, 0.04),
            3,
        );
    }
    s.camera = Camera::lens(v3(4.3, 2.7, 9.), v3(-0.6, 1.8, -1.9), 27., 0.);
    s.exposure = 0.9;
    s
}
fn street() -> Scene {
    let mut s = base("blue-hour");
    s.world.atmosphere.density = 0.004;
    s.world.atmosphere.height_falloff = 0.06;
    s.world.atmosphere.anisotropy = 0.3;
    s.world.atmosphere.distance = 260.;
    s.materials = vec![
        Material {
            coat: 0.25,
            coat_roughness: 0.08,
            ..Material::dielectric(v3(0.045, 0.05, 0.06), 0.55)
        },
        Material::dielectric(v3(0.3, 0.28, 0.25), 0.85),
        Material::dielectric(v3(0.12, 0.16, 0.19), 0.8),
        Material::metal(v3(0.2, 0.22, 0.24), 0.4),
        Material::dielectric(v3(0.7, 0.62, 0.36), 0.7),
        Material {
            emission: v3(2., 0.8, 0.25),
            ..Material::dielectric(v3(0.2, 0.12, 0.05), 0.6)
        },
    ];
    add_box(&mut s, v3(0., -0.12, -80.), v3(60., 0.24, 270.), 0);
    for side in [-1., 1.] {
        add_box(&mut s, v3(side * 8., 0.12, -80.), v3(4., 0.24, 260.), 1);
        for i in 0..25 {
            let z = 18. - i as f32 * 8.;
            let p = v3(side * 6.4, 5.2, z);
            s.extra_lights
                .push(GpuLight::sphere(p, 0.16, v3(180., 92., 35.)));
            add_box(&mut s, p - v3(side * 0.7, 2.6, 0.), v3(0.1, 5.2, 0.1), 3);
            add_box(&mut s, p - v3(side * 0.35, 0., 0.), v3(0.9, 0.1, 0.3), 3);
            // Thin emissive fixture just below the explicit light, without a
            // duplicate sphere surface at the sampled light's position.
            add_box(&mut s, p - v3(0., 0.08, 0.), v3(0.5, 0.03, 0.22), 5);
            let height = 9. + ((i * 7) % 5) as f32 * 2.;
            let x = side * (15. + ((i * 3) % 4) as f32);
            add_box(
                &mut s,
                v3(x, height * 0.5, z),
                v3(9., height, 7.2),
                if i % 2 == 0 { 1 } else { 2 },
            );
            for y in 0..3 {
                for k in 0..3 {
                    add_box(
                        &mut s,
                        v3(side * 10.45, 2.5 + y as f32 * 2.5, z - 2. + k as f32 * 2.),
                        v3(0.04, 1.3, 1.),
                        if (i + y + k) % 3 == 0 { 5 } else { 3 },
                    );
                }
            }
        }
    }
    for i in 0..36 {
        add_box(
            &mut s,
            v3(0., 0.012, 25. - i as f32 * 6.),
            v3(0.15, 0.015, 2.6),
            4,
        );
    }
    s.camera = Camera::lens(v3(3.7, 2.3, 28.), v3(-0.8, 3.3, -40.), 30., 0.);
    s.exposure = 1.8;
    s
}
/// Install one three-level multi-material prototype, preserving shared geometry.
fn add_nature(s: &mut Scene, species: Species, seed: u32) -> [usize; 3] {
    let nature = Nature { species, seed };
    let material = s.materials.len();
    s.materials.extend(nature.materials());
    std::array::from_fn(|lod| {
        let mut mesh = nature.mesh(lod);
        for m in &mut mesh.triangle_materials {
            *m += material;
        }
        let index = s.objects.len();
        s.objects.push(Object { mesh, material });
        index
    })
}
fn landscape(count: u32) -> Scene {
    let mut s = base("golden-hour");
    s.world.sky.as_mut().unwrap().azimuth = -70.;
    s.world.sky.as_mut().unwrap().elevation = 9.;
    s.world.atmosphere.density = 0.0015;
    s.world.atmosphere.height_falloff = 0.02;
    s.world.atmosphere.anisotropy = 0.55;
    s.world.atmosphere.distance = 1800.;
    s.materials = vec![Material::dielectric(v3(0.14, 0.19, 0.055), 0.88)];
    let pine = add_nature(&mut s, Species::Conifer, 41);
    let oak = add_nature(&mut s, Species::Broadleaf, 73);
    let birch = add_nature(&mut s, Species::Birch, 19);
    let rock = add_nature(&mut s, Species::Rock, 81);
    let grass = add_nature(&mut s, Species::Grass, 66);
    let bush = add_nature(&mut s, Species::Bush, 94);
    let terrain = Terrain {
        size: [1200., 1200.],
        resolution: 128,
        adaptive: true,
        amplitude: 55.,
        frequency: 0.003,
        seed: 712,
        material: 0,
        detail: 0.015,
        erosion: 10,
        natural: true,
        blend: 0.12,
        water_level: -24.,
        snow_level: 46.,
        ..Default::default()
    };
    let y = terrain.sample(0., 240.).0;
    s.terrains.push(terrain);
    // One explicit off-camera placement suppresses the prototypes' implicit
    // identity placements. Every forest placement below is a real tree.
    s.instances = vec![Instance {
        object: pine[0],
        position: v3(1500., 0., 0.),
        ..Default::default()
    }];
    s.scatters = vec![
        Scatter {
            objects: vec![pine[0], pine[0], oak[0], birch[0]],
            lods: vec![pine, oak, birch],
            seed: 517,
            count,
            terrain: Some(0),
            size: [1180., 1180.],
            scale: [0.65, 1.4],
            max_slope: 90.,
            ..Default::default()
        },
        Scatter {
            objects: vec![grass[0]],
            lods: vec![grass],
            seed: 633,
            count: count / 2,
            terrain: Some(0),
            size: [1180., 1180.],
            scale: [0.7, 1.5],
            max_slope: 90.,
            ..Default::default()
        },
        Scatter {
            objects: vec![rock[0], bush[0]],
            lods: vec![rock, bush],
            seed: 483,
            count: count / 25,
            terrain: Some(0),
            size: [1180., 1180.],
            scale: [0.5, 1.7],
            max_slope: 90.,
            ..Default::default()
        },
    ];
    s.camera = Camera::lens(v3(0., y + 25., 240.), v3(0., 8., -120.), 40., 0.);
    s.exposure = 0.85;
    s
}
fn default_landscape() -> Scene {
    let mut s = landscape(520);
    s.terrains[0] = Terrain {
        size: [180., 180.],
        resolution: 128,
        adaptive: true,
        amplitude: 14.,
        frequency: 0.018,
        seed: 41,
        detail: 0.018,
        erosion: 12,
        natural: true,
        blend: 0.16,
        water_level: -9.,
        snow_level: 14.,
        ..Default::default()
    };
    s.scatters[0].size = [174., 174.];
    s.scatters[0].scale = [0.65, 1.05];
    s.scatters[0].max_slope = 40.;
    s.scatters[1].size = [174., 174.];
    s.scatters[1].count = 18000;
    s.scatters[1].scale = [0.7, 1.5];
    s.scatters[1].max_slope = 40.;
    s.scatters[2].size = [174., 174.];
    s.scatters[2].count = 100;
    s.scatters[2].max_slope = 50.;
    s.world.atmosphere.density = 0.003;
    s.world.atmosphere.height_falloff = 0.08;
    s.world.atmosphere.distance = 500.;
    let y = s.terrains[0].sample(10., 62.).0;
    s.camera = Camera::lens(v3(10., y + 12., 62.), v3(-8., 2., -35.), 32., 0.);
    s
}
fn nature_closeup() -> Scene {
    let mut s = default_landscape();
    s.terrains[0].size = [60., 60.];
    s.terrains[0].amplitude = 0.8;
    s.terrains[0].frequency = 0.06;
    s.terrains[0].adaptive = false;
    s.terrains[0].resolution = 128;
    s.scatters.clear();
    s.instances.clear();
    // Keep a tree, sapling, several rocks and grass in a single inspectable shot.
    for (object, x, z, scale) in [
        (0, -3., -3., 0.8),
        (3, 3., -5., 0.85),
        (6, 0.5, -8., 0.85),
        (9, 1., 0., 1.1),
        (9, -1.2, 0.5, 0.65),
        (15, -2.5, -0.8, 0.85),
    ] {
        let y = s.terrains[0].sample(x, z).0;
        s.instances.push(Instance {
            object,
            position: v3(x, y, z),
            scale: Vec3::splat(scale),
            rotation: v3(0., x * 41., 0.),
            ..Default::default()
        });
    }
    s.scatters = vec![Scatter {
        objects: vec![12],
        lods: vec![[12, 13, 14]],
        seed: 87,
        count: 1500,
        terrain: Some(0),
        size: [20., 20.],
        scale: [0.5, 1.2],
        max_slope: 90.,
        ..Default::default()
    }];
    s.camera = Camera::lens(v3(7., 2.8, 11.), v3(-0.5, 2., -3.), 35., 0.);
    s.world.atmosphere.density = 0.001;
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corpus_has_required_counts() {
        let s = street();
        assert_eq!(s.extra_lights.len(), 50);
        let s = landscape(50000).expanded().unwrap();
        assert_eq!(s.instances.len(), 50000 + 25000 + 2000 + 2);
        assert_eq!(s.objects.len(), 19);
        assert!(s.instances.iter().filter(|i| i.object < 9).count() >= 50000);
    }
}
