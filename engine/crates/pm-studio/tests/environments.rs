//! Radiometric regression tests, including the portable traversal fallback.
use pm_studio::{
    environment::{EnvironmentMap, Volume, VolumeShape, World},
    gpu::Renderer,
    math::{Vec3, v3},
    mesh,
    scene::{Camera, Material, Object, Scene},
};
fn world_scene() -> Scene {
    Scene {
        meters_per_unit: 1.,
        world: World {
            hdri: Some(EnvironmentMap {
                width: 8,
                height: 4,
                pixels: vec![Vec3::splat(1.); 32],
                ..Default::default()
            }),
            ..Default::default()
        },
        objects: vec![],
        materials: vec![Material {
            specular: 0.,
            ..Material::dielectric(Vec3::splat(0.7), 1.)
        }],
        lights: vec![],
        extra_lights: vec![],
        camera: Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.),
        backdrop: Vec3::ZERO,
        environment: (Vec3::ZERO, Vec3::ZERO),
        exposure: 1.,
        shadow_catcher: false,
        layers: None,
        instances: vec![],
        scatters: vec![],
        terrains: vec![],
        displacements: vec![],
    }
}
fn center_mean(r: &mut Renderer) -> f32 {
    let p = r.radiance();
    let mut sum = 0.;
    let mut count = 0;
    for y in 12..20 {
        for x in 12..20 {
            sum += p[(y * 32 + x) * 4];
            count += 1;
        }
    }
    sum / count as f32
}
#[test]
fn white_furnace_and_absorbing_fog_match_energy() {
    let mut r = Renderer::new(32, 32).expect("GPU required for environment transport validation");
    r.hardware_rt = false;
    let mut s = world_scene();
    s.objects.push(Object {
        mesh: mesh::sphere(Vec3::ZERO, 1., 24, 48),
        material: 0,
    });
    r.set_scene(&s, &s.prepare());
    r.trace(512, 4, true, 1000000);
    let mean = center_mean(&mut r);
    assert!(
        (mean - 0.7).abs() < 0.035,
        "diffuse furnace expected .7, got {mean}"
    );
    s.objects.clear();
    s.world.atmosphere.density = 0.1;
    s.world.atmosphere.distance = 10.;
    s.world.atmosphere.color = Vec3::ZERO;
    r.set_scene(&s, &s.prepare());
    r.trace(1024, 4, true, 1000000);
    let mean = center_mean(&mut r);
    assert!(
        (mean - (-1f32).exp()).abs() < 0.025,
        "Beer global fog expected exp(-1), got {mean}"
    );
    s.world.atmosphere.density = 0.;
    s.world.volumes = vec![Volume {
        shape: VolumeShape::Box,
        center: Vec3::ZERO,
        size: Vec3::splat(1.),
        density: 0.2,
        color: Vec3::ZERO,
        anisotropy: 0.,
    }];
    r.set_scene(&s, &s.prepare());
    r.trace(1024, 4, true, 1000000);
    let mean = center_mean(&mut r);
    assert!(
        (mean - (-0.4f32).exp()).abs() < 0.03,
        "Beer local box expected exp(-.4), got {mean}"
    );
    s.world.volumes[0].shape = VolumeShape::Sphere;
    r.set_scene(&s, &s.prepare());
    r.trace(1024, 4, true, 1000000);
    let mean = center_mean(&mut r);
    assert!(
        (mean - (-0.4f32).exp()).abs() < 0.03,
        "Beer local sphere expected exp(-.4), got {mean}"
    );
}
#[test]
fn adaptive_stops_constant_pixels_and_resets() {
    let mut r = Renderer::new(32, 32).expect("GPU");
    let mut s = world_scene();
    s.world.adaptive.enabled = true;
    s.world.adaptive.min_samples = 8;
    r.set_scene(&s, &s.prepare());
    r.trace(32, 2, true, 1000000);
    let p = r.radiance();
    assert!(p.chunks_exact(4).all(|p| (p[0] - 1.).abs() < 1e-5));
    let counts = r.sample_counts();
    assert!(counts.iter().all(|n| *n == 8.));
    r.trace(4, 2, true, 1000000);
    assert!(r.sample_counts().iter().all(|n| *n == 4.));
}
#[test]
fn shared_blas_and_displacement_render_in_both_traversal_paths() {
    let mut r = Renderer::new(32, 32).expect("GPU");
    let mut s = world_scene();
    s.objects.push(Object {
        mesh: mesh::sphere(Vec3::ZERO, 0.5, 16, 32),
        material: 0,
    });
    s.displacements = vec![pm_studio::displacement::Displacement {
        object: 0,
        width: 2,
        height: 2,
        values: vec![0., 1., 1., 0.],
        strength: 0.15,
        mid_level: 0.5,
        subdivision: 1,
    }];
    s.instances = vec![
        pm_studio::scatter::Instance {
            position: v3(-0.65, 0., 0.),
            ..Default::default()
        },
        pm_studio::scatter::Instance {
            position: v3(0.65, 0., 0.),
            scale: v3(1., 1.5, 1.),
            ..Default::default()
        },
    ];
    r.set_scene(&s, &s.prepare());
    r.hardware_rt = true;
    r.trace(64, 3, true, 1000000);
    let native = r.radiance();
    r.hardware_rt = false;
    r.trace(64, 3, true, 1000000);
    let portable = r.radiance();
    let max = native
        .iter()
        .zip(portable)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(max < 0.01, "instance traversal disagreement {max}");
}

#[test]
fn spatial_light_tree_preserves_the_flat_estimator_energy() {
    let mut r = Renderer::new(32, 32).expect("GPU");
    r.hardware_rt = false;
    let mut s = world_scene();
    s.objects.push(Object {
        mesh: mesh::sphere(Vec3::ZERO, 1., 16, 32),
        material: 0,
    });
    s.world.hdri.as_mut().unwrap().pixels.fill(Vec3::ZERO);
    s.extra_lights = (0..50)
        .map(|i| {
            let angle = i as f32 / 50. * std::f32::consts::TAU;
            pm_studio::scene::GpuLight::sphere(
                v3(angle.sin() * 3., 1.8, angle.cos() * 3.),
                0.12,
                Vec3::splat(if i == 0 { 200. } else { 20. }),
            )
        })
        .collect();
    let mut p = s.prepare();
    r.set_scene(&s, &p);
    r.trace(1024, 4, true, 1000000);
    let tree = center_mean(&mut r);
    let world = s.extra_lights.len() * 4 + 9;
    let tree_offset = p.sdata[world + 4][3] as usize;
    p.sdata.truncate(world + tree_offset);
    p.sdata[world + 4][3] = 0.;
    r.set_scene(&s, &p);
    r.trace(1024, 4, true, 1000000);
    let flat = center_mean(&mut r);
    assert!(
        (tree - flat).abs() / flat < 0.03,
        "spatial light tree {tree}, flat CDF {flat}"
    );
}

#[test]
fn live_fog_retains_scattering_when_the_environment_is_hidden() {
    let mut r = Renderer::new(32, 32).expect("GPU");
    let mut s = world_scene();
    s.world.hdri.as_mut().unwrap().visible = false;
    s.world.atmosphere.density = 0.1;
    s.world.atmosphere.distance = 10.;
    s.world.atmosphere.color = Vec3::splat(1.);
    r.set_scene(&s, &s.prepare());
    r.live = true;
    r.trace(4, 3, true, 1000000);
    let mean = center_mean(&mut r);
    assert!(
        mean > 0.5,
        "hidden dome must retain visible fog scattering, got {mean}"
    );
}

#[test]
fn live_height_fog_matches_closed_form_optical_depth() {
    let mut r = Renderer::new(32, 32).expect("GPU");
    let mut s = world_scene();
    s.camera.position = Vec3::ZERO;
    s.camera.target = v3(0., 1., -1.);
    s.camera.fov = 1.;
    s.world.atmosphere.density = 0.1;
    s.world.atmosphere.height_falloff = 0.5;
    s.world.atmosphere.distance = 10.;
    s.world.atmosphere.color = Vec3::ZERO;
    r.set_scene(&s, &s.prepare());
    r.live = true;
    r.trace(4, 3, true, 1000000);
    let rate = 0.5 / std::f32::consts::SQRT_2;
    let expected = (-0.1 * (1. - (-rate * 10.).exp()) / rate).exp();
    let got = center_mean(&mut r);
    assert!(
        (got - expected).abs() < 0.01,
        "height fog expected {expected}, got {got}"
    );
}

#[test]
fn metre_scenes_keep_millimetre_surface_hits() {
    let mut r = Renderer::new(32, 32).expect("GPU");
    let mut s = world_scene();
    s.camera.position = v3(0., 0., 0.001);
    s.camera.target = Vec3::ZERO;
    s.objects.push(Object {
        mesh: mesh::Mesh {
            positions: vec![v3(-1., -1., 0.), v3(1., -1., 0.), v3(0., 1., 0.)],
            normals: vec![v3(0., 0., 1.); 3],
            uvs: vec![[0., 0.]; 3],
            indices: vec![[0, 1, 2]],
            ..Default::default()
        },
        material: 0,
    });
    r.set_scene(&s, &s.prepare());
    let hit = r.probe(16., 16.);
    assert!(
        hit[1] >= 0. && hit[5] >= 0.,
        "both traversal paths must retain the thin nearby surface: {hit:?}"
    );
    assert!((hit[0] - 0.001).abs() < 0.0001 && (hit[4] - 0.001).abs() < 0.0001);
}

#[test]
fn instance_layer_override_does_not_hide_the_geometry() {
    let mut r = Renderer::new(32, 32).expect("GPU");
    let mut s = world_scene();
    s.objects.push(Object {
        mesh: mesh::sphere(Vec3::ZERO, 1., 16, 32),
        material: 0,
    });
    s.instances = vec![pm_studio::scatter::Instance {
        layer: Some(7),
        ..Default::default()
    }];
    r.set_scene(&s, &s.prepare());
    for rt in [true, false] {
        r.hardware_rt = rt;
        let pick = r
            .pick_surface(0.5, 0.5)
            .expect("layer override must remain visible");
        assert_eq!(pick.layer, 7);
    }
}
