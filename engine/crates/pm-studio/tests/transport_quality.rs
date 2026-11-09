//! Focused radiometric reproductions found through the offline Cycles yardstick.
use pm_studio::{
    environment::{Volume, VolumeShape, World},
    gpu::Renderer,
    math::{Vec3, v3},
    mesh::Mesh,
    scene::{Camera, GpuLight, Material, Object, Scene},
};
fn receiver() -> Scene {
    let mesh = Mesh {
        positions: vec![
            v3(-4., 0., -4.),
            v3(4., 0., -4.),
            v3(4., 0., 4.),
            v3(-4., 0., 4.),
        ],
        normals: vec![Vec3::Y; 4],
        uvs: vec![[0., 0.]; 4],
        indices: vec![[0, 2, 1], [0, 3, 2]],
            ..Default::default()
    };
    Scene {
        meters_per_unit: 1.,
        world: World::default(),
        objects: vec![Object { mesh, material: 0 }],
        materials: vec![Material {
            specular: 0.,
            ..Material::dielectric(Vec3::splat(0.7), 1.)
        }],
        lights: vec![],
        extra_lights: vec![GpuLight::sun(
            Vec3::Y,
            0.00465,
            Vec3::splat(1. / (std::f32::consts::PI * 0.00465f32.sin().powi(2))),
        )],
        camera: Camera::lens(v3(0., 0.5, 3.), Vec3::ZERO, 70., 0.),
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
#[test]
fn analytic_fog_shadows_do_not_add_null_collision_noise() {
    let mut scene = receiver();
    scene.world.volumes.push(Volume {
        shape: VolumeShape::Box,
        center: v3(0., 1.5, 0.),
        size: v3(20., 0.5, 20.),
        density: 0.7,
        color: Vec3::ZERO,
        anisotropy: 0.,
    });
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(16, 2, true, 1000000);
    let p = r.linear_beauty();
    let expected = 0.7 / std::f32::consts::PI * (-0.7f32).exp();
    let values: Vec<_> = (8..16)
        .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4))
        .map(|i| p[i])
        .collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let rms =
        (values.iter().map(|v| (v - expected).powi(2)).sum::<f32>() / values.len() as f32).sqrt();
    eprintln!("absorbing slab: mean={mean}, expected={expected}, rms={rms}");
    assert!(
        (mean - expected).abs() < 0.005,
        "slab energy {mean} != {expected}"
    );
    assert!(
        rms < 0.003,
        "analytic shadow transmittance should be nearly noise-free, rms={rms}"
    );
}

#[test]
fn opaque_dielectric_reflections_follow_the_material_ior() {
    let mut scene = receiver();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    scene.extra_lights.clear();
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    for ior in [1., 1.5, 2.4] {
        scene.materials[0] = Material {
            ior,
            ..Material::dielectric(Vec3::ZERO, 0.03)
        };
        r.set_scene(&scene, &scene.prepare());
        r.trace(256, 2, true, 1000000);
        let p = r.linear_beauty();
        let actual = (10..14)
            .flat_map(|y| (10..14).map(move |x| (y * 24 + x) * 4))
            .map(|i| p[i])
            .sum::<f32>()
            / 16.;
        let expected = ((ior - 1.) / (ior + 1.)).powi(2);
        eprintln!("dielectric IOR {ior}: {actual}, F0={expected}");
        assert!(
            (actual - expected).abs() < 0.003,
            "IOR {ior}: {actual} != Fresnel {expected}"
        );
    }
}

#[test]
fn emission_is_attenuated_by_the_coat_it_passes_through() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.camera = Camera::lens(v3(0., 2., 5.), Vec3::ZERO, 120., 0.);
    scene.materials[0] = Material {
        coat: 1.,
        coat_roughness: 0.01,
        emission: Vec3::splat(1.),
        specular: 0.,
        ..Material::dielectric(Vec3::ZERO, 0.5)
    };
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(64, 4, true, 1000000);
    let p = r.linear_beauty();
    let actual = p[(12 * 24 + 12) * 4];
    let cosine = 2. / 29f32.sqrt();
    let transmitted = (1. - (1. - cosine * cosine) / 2.25).sqrt();
    let rs = (cosine - 1.5 * transmitted) / (cosine + 1.5 * transmitted);
    let rp = (1.5 * cosine - transmitted) / (1.5 * cosine + transmitted);
    let expected = 1. - 0.5 * (rs * rs + rp * rp);
    eprintln!("coated emission: {actual}, expected {expected}");
    assert!(
        (actual - expected).abs() < 0.015,
        "emission must lose the coat's reflected energy"
    );
}

#[test]
fn rough_white_metal_preserves_white_furnace_energy() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    scene.materials[0] = Material::metal(Vec3::splat(1.), 1.);
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(1024, 4, true, 1000000);
    let p = r.linear_beauty();
    let actual = (8..16)
        .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4))
        .map(|i| p[i])
        .sum::<f32>()
        / 64.;
    eprintln!("rough metal furnace: {actual}");
    assert!(
        (actual - 1.).abs() < 0.04,
        "lossless rough metal must preserve energy, got {actual}"
    );
}

#[test]
fn rough_white_dielectric_preserves_layer_energy() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    scene.materials[0] = Material::dielectric(Vec3::splat(1.), 1.);
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(1024, 4, true, 1000000);
    let p = r.linear_beauty();
    let actual = (8..16)
        .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4))
        .map(|i| p[i])
        .sum::<f32>()
        / 64.;
    eprintln!("rough dielectric furnace: {actual}");
    assert!(
        (actual - 1.).abs() < 0.035,
        "a white diffuse base must retain energy below its specular layer, got {actual}"
    );
}

#[test]
fn photon_irradiance_is_unchanged_by_an_index_matched_sheet() {
    let mut scene = receiver();
    // The camera stays below the transparent sheet. IOR=1 cannot focus, tint,
    // or reflect light: photon and direct estimators must give the same energy.
    let mut sheet = scene.objects[0].mesh.clone();
    for p in &mut sheet.positions {
        p.y = 1.;
    }
    scene.materials.push(Material {
        ior: 1.,
        ..Material::glass(Vec3::splat(1.), 0.)
    });
    scene.objects.push(Object {
        mesh: sheet,
        material: 1,
    });
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    for (height, slope) in [(0., 0.), (0.013, 0.), (0.032, 0.4), (-0.015, -0.7)] {
        let mut tilted = scene.clone();
        for object in &mut tilted.objects {
            for p in &mut object.mesh.positions {
                p.y += height + slope * p.x;
            }
            for n in &mut object.mesh.normals {
                *n = v3(-slope, 1., 0.).normalize();
            }
        }
        tilted.camera.position.y += height;
        tilted.camera.target.y += height;
        r.set_scene(&tilted, &tilted.prepare());
        r.trace(256, 4, true, 1000000);
        let pixels = r.linear_beauty();
        let actual = (8..16)
            .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4))
            .map(|i| pixels[i])
            .sum::<f32>()
            / 64.;
        let expected = 0.7 / std::f32::consts::PI / (1f32 + slope * slope).sqrt();
        eprintln!(
            "index-matched photon sheet at {height}, slope {slope}: {actual}, target {expected}"
        );
        assert!(
            (actual / expected - 1.).abs() < 0.08,
            "photon surface energy: {actual} != {expected}"
        );
    }
}

#[test]
fn small_colored_light_rigs_keep_low_sample_direct_light_stable() {
    let mut scene = receiver();
    let unit = std::f32::consts::PI * 0.00465f32.sin().powi(2);
    scene.extra_lights = [v3(1., 0., 0.), v3(0., 2., 0.), v3(0., 0., 4.)]
        .into_iter()
        .map(|color| GpuLight::sun(Vec3::Y, 0.00465, color / unit))
        .collect();
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(4, 2, true, 1000000);
    let pixels = r.linear_beauty();
    for (channel, irradiance) in [1., 2., 4.].into_iter().enumerate() {
        let expected = 0.7 * irradiance / std::f32::consts::PI;
        let rms = ((8..16)
            .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4 + channel))
            .map(|i| (pixels[i] - expected).powi(2))
            .sum::<f32>()
            / 64.)
            .sqrt();
        eprintln!("colored rig channel {channel}: rms {rms}");
        assert!(
            rms < 0.003,
            "each emitter must be sampled without light-selection noise: {rms}"
        );
    }
}

#[test]
fn rgb_dispersion_does_not_add_channel_count_noise_in_a_white_furnace() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    scene.materials[0] = Material {
        dispersion: 1.,
        ..Material::glass(Vec3::splat(1.), 0.)
    };
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(12, 8, true, 1000000);
    let pixels = r.linear_beauty();
    let rms = ((8..16)
        .flat_map(|y| (8..16).flat_map(move |x| (0..3).map(move |c| (y * 24 + x) * 4 + c)))
        .map(|i| (pixels[i] - 1.).powi(2))
        .sum::<f32>()
        / (64. * 3.))
        .sqrt();
    eprintln!("dispersive white furnace RGB rms: {rms}");
    assert!(
        rms < 0.04,
        "wavelength strata should preserve neutral light: {rms}"
    );
}

#[test]
fn a_lossless_filmed_metal_cannot_absorb_a_white_furnace() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    scene.materials[0] = Material {
        film_nm: 520.,
        ..Material::metal(Vec3::splat(1.), 0.08)
    };
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(256, 4, true, 1000000);
    let pixels = r.linear_beauty();
    for channel in 0..3 {
        let actual = (8..16)
            .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4 + channel))
            .map(|i| pixels[i])
            .sum::<f32>()
            / 64.;
        eprintln!("lossless thin-film channel {channel}: {actual}");
        assert!(
            (actual - 1.).abs() < 0.025,
            "a lossless film on a perfect metal must return white: {actual}"
        );
    }
}

#[test]
fn finite_global_fog_has_the_same_boundary_for_camera_and_shadow_rays() {
    let mut scene = receiver();
    scene.camera = Camera::lens(v3(0., 0.5, 3.), Vec3::ZERO, 5000., 0.);
    scene.world.atmosphere.density = 0.2;
    scene.world.atmosphere.color = Vec3::ZERO;
    scene.world.atmosphere.distance = 3.1;
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.live = true; // Analytic absorption isolates the boundary from collision noise.
    r.set_scene(&scene, &scene.prepare());
    r.trace(4, 2, true, 1000000);
    let pixels = r.linear_beauty();
    let distance = 9.25f32.sqrt();
    let shadow_distance = 0.5 + (3.1f32 * 3.1 - 9.).sqrt();
    let expected = 0.7 / std::f32::consts::PI * (-0.2 * (distance + shadow_distance)).exp();
    let actual = pixels[(12 * 24 + 12) * 4];
    eprintln!("finite global fog: {actual}, expected {expected}");
    assert!(
        (actual / expected - 1.).abs() < 0.015,
        "all segments must use the same physical fog boundary"
    );
}

#[test]
fn local_volumes_keep_their_bounds_outside_the_global_fog_radius() {
    let mut scene = receiver();
    scene.objects.clear();
    scene.extra_lights.clear();
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 5000., 0.);
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    scene.world.atmosphere.distance = 1.;
    scene.world.volumes.push(Volume {
        shape: VolumeShape::Box,
        center: Vec3::ZERO,
        size: Vec3::splat(1.),
        density: 0.2,
        color: Vec3::ZERO,
        anisotropy: 0.,
    });
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.live = true;
    r.set_scene(&scene, &scene.prepare());
    r.trace(4, 2, true, 1000000);
    let actual = r.linear_beauty()[(12 * 24 + 12) * 4];
    let expected = (-0.4f32).exp();
    assert!(
        (actual - expected).abs() < 0.002,
        "local volume was clipped by the global radius: {actual} != {expected}"
    );
}

#[test]
fn glossy_coat_samples_converge_in_a_small_bright_environment() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    let mut environment = vec![Vec3::splat(0.05); 32 * 16];
    for y in 6..10 {
        for x in [0, 1, 30, 31] {
            environment[y * 32 + x] = Vec3::splat(12.);
        }
    }
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 32,
        height: 16,
        pixels: environment,
        ..Default::default()
    });
    scene.materials[0] = Material {
        coat: 1.,
        coat_roughness: 0.01,
        ..Material::dielectric(Vec3::splat(0.4), 0.8)
    };
    let mut r = Renderer::new(32, 32).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.set_time_seed(7);
    r.trace(4096, 4, true, 1000000);
    let reference = r.linear_beauty();
    let mut error = 0.;
    let mut mean = 0.;
    let mut expected = 0.;
    let mut count = 0.;
    for seed in 0..4 {
        r.set_time_seed(seed);
        r.trace(64, 4, true, 1000000);
        let pixels = r.linear_beauty();
        for y in 10..22 {
            for x in 10..22 {
                let i = (y * 32 + x) * 4;
                error += (pixels[i] - reference[i]).powi(2);
                mean += pixels[i];
                expected += reference[i];
                count += 1.;
            }
        }
    }
    let rms = (error / count).sqrt();
    let gain = mean / expected;
    eprintln!("glossy coat 64spp RMS {rms}, aggregate gain {gain}");
    assert!(
        (gain - 1.).abs() < 0.03,
        "coat sampling must retain the converged energy: {gain}"
    );
    assert!(rms < 0.10, "coat highlight noise regressed: {rms}");
}

#[test]
fn conservative_subsurface_walk_preserves_a_white_furnace() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    scene.materials[0] = Material {
        subsurface: 0.2,
        specular: 0.,
        ..Material::dielectric(Vec3::splat(1.), 0.45)
    };
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(2048, 4, true, 1000000);
    let pixels = r.linear_beauty();
    let actual = (8..16)
        .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4))
        .map(|i| pixels[i])
        .sum::<f32>()
        / 64.;
    eprintln!("conservative subsurface furnace: {actual}");
    assert!(
        (actual - 1.).abs() < 0.03,
        "a conservative bounded walk must return its white incident energy: {actual}"
    );
}

#[test]
fn advanced_materials_agree_in_native_and_portable_traversal() {
    let scene = pm_studio::scene::preset("materials", "starlight", 0.).unwrap();
    let mut r = Renderer::new(64, 36).expect("GPU");
    if !r.has_hardware_rt() {
        return;
    }
    r.set_scene(&scene, &scene.prepare());
    r.caustics = false;
    r.hardware_rt = true;
    r.trace(256, 8, true, 1000000);
    let native = r.linear_beauty();
    r.hardware_rt = false;
    r.trace(256, 8, true, 1000000);
    let portable = r.linear_beauty();
    let mut error = 0.;
    let mut energy = 0.;
    let mut count = 0.;
    for (a, b) in native.chunks_exact(4).zip(portable.chunks_exact(4)) {
        if a[3] > 0.98 && b[3] > 0.98 {
            for c in 0..3 {
                error += (a[c] - b[c]).abs();
                energy += a[c].abs();
                count += 1.;
            }
        }
    }
    let relative = error / energy;
    eprintln!("advanced-material native/portable relative MAE {relative}, {count} components");
    assert!(count > 100., "comparison must cover visible materials");
    assert!(
        relative < 0.01,
        "native/portable advanced material transport disagreed: {relative}"
    );
}

#[test]
fn nearly_nondispersive_glass_keeps_achromatic_lighting_neutral() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    let pixels = (0..16)
        .flat_map(|y| {
            (0..32).map(move |x| {
                let dx = (x as f32 - 16.) / 5.;
                let dy = (y as f32 - 8.) / 3.;
                Vec3::splat(0.05 + 8. * (-0.5 * (dx * dx + dy * dy)).exp())
            })
        })
        .collect();
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 32,
        height: 16,
        pixels,
        ..Default::default()
    });
    scene.materials[0] = Material {
        dispersion: 0.00001,
        ..Material::glass(Vec3::splat(1.), 0.)
    };
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(96, 8, true, 1000000);
    let pixels = r.linear_beauty();
    let error = (8..16)
        .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4))
        .map(|i| (pixels[i] - pixels[i + 1]).powi(2) + (pixels[i + 1] - pixels[i + 2]).powi(2))
        .sum::<f32>();
    let rms = (error / 128.).sqrt();
    // Correlation must change variance without changing the mean. Compare
    // nearly identical IORs with the ordinary RGB path on independent seeds.
    r.set_time_seed(11);
    r.trace(2048, 8, true, 1000000);
    let dispersed = r.linear_beauty();
    scene.materials[0].dispersion = 0.;
    r.set_scene(&scene, &scene.prepare());
    r.set_time_seed(19);
    r.trace(2048, 8, true, 1000000);
    let ordinary = r.linear_beauty();
    let (mut a, mut b) = (0., 0.);
    for y in 8..16 {
        for x in 8..16 {
            for c in 0..3 {
                let i = (y * 24 + x) * 4 + c;
                a += dispersed[i];
                b += ordinary[i];
            }
        }
    }
    let gain = a / b;
    eprintln!("coupled wavelength energy ratio {gain}");
    assert!(
        (gain - 1.).abs() < 0.02,
        "correlation must preserve the achromatic transport mean: {gain}"
    );

    eprintln!("nearly nondispersive achromatic glass channel RMS: {rms}");
    assert!(
        rms < 0.03,
        "wavelength sampling should not tint neutral highlights: {rms}"
    );
}

#[test]
fn sheen_layers_cannot_create_energy_in_a_white_furnace() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.objects[0].mesh = pm_studio::mesh::sphere(Vec3::ZERO, 1., 32, 64);
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 70., 0.);
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    scene.materials[0] = Material {
        sheen: 1.,
        ..Material::dielectric(Vec3::splat(1.), 0.75)
    };
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(2048, 4, true, 1000000);
    let pixels = r.linear_beauty();
    let actual = (8..16)
        .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4))
        .map(|i| pixels[i])
        .sum::<f32>()
        / 64.;
    eprintln!("layered sheen white furnace: {actual}");
    assert!(
        (actual - 1.).abs() < 0.035,
        "white sheen must replace base energy rather than add it: {actual}"
    );
}

#[test]
fn sheen_reserves_the_same_energy_for_reflection_and_emission() {
    let mut scene = receiver();
    scene.extra_lights.clear();
    scene.camera = Camera::lens(v3(0., 0.5, 3.), Vec3::ZERO, 5000., 0.);
    scene.world.hdri = Some(pm_studio::environment::EnvironmentMap {
        width: 8,
        height: 4,
        pixels: vec![Vec3::splat(1.); 32],
        ..Default::default()
    });
    scene.materials[0] = Material {
        sheen: 1.,
        specular: 0.,
        emission: Vec3::splat(1.),
        ..Material::dielectric(Vec3::ZERO, 0.75)
    };
    let mut r = Renderer::new(24, 24).expect("GPU");
    r.hardware_rt = false;
    r.set_scene(&scene, &scene.prepare());
    r.trace(2048, 4, true, 1000000);
    let pixels = r.linear_beauty();
    let actual = (8..16)
        .flat_map(|y| (8..16).map(move |x| (y * 24 + x) * 4))
        .map(|i| pixels[i])
        .sum::<f32>()
        / 64.;
    eprintln!("sheen reflected plus emitted white energy: {actual}");
    assert!(
        (actual - 1.).abs() < 0.035,
        "sheen reflection and underlying emission must sum to one: {actual}"
    );
}
