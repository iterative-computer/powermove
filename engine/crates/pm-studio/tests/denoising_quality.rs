//! Small real-GPU image regressions. No saved renders or external project needed.
#![cfg(feature = "oidn")]
use pm_studio::{
    environment::{AdaptiveSampling, EnvironmentMap, World},
    gpu::Renderer,
    math::{Vec3, v3},
    mesh::{self, Mesh},
    oidn::Denoiser,
    scene::{Camera, Material, Object, Scene},
};
use std::cell::RefCell;

fn glass_detail() -> Scene {
    let mut wall = Mesh::default();
    // A sharp colored join behind the glass is real refracted detail.
    for (left, right) in [(-5., 0.), (0., 5.)] {
        let offset = wall.positions.len() as u32;
        wall.positions.extend([
            v3(left, -5., -2.),
            v3(right, -5., -2.),
            v3(right, 5., -2.),
            v3(left, 5., -2.),
        ]);
        wall.normals.extend([v3(0., 0., 1.); 4]);
        wall.uvs.extend([[0., 0.], [1., 0.], [1., 1.], [0., 1.]]);
        wall.indices.extend([
            [offset, offset + 1, offset + 2],
            [offset, offset + 2, offset + 3],
        ]);
    }
    wall.triangle_materials = vec![1, 1, 2, 2];
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
        objects: vec![
            Object {
                mesh: mesh::sphere(Vec3::ZERO, 1., 24, 48),
                material: 0,
            },
            Object {
                mesh: wall,
                material: 1,
            },
        ],
        materials: vec![
            Material::glass(Vec3::splat(1.), 0.025),
            Material::dielectric(v3(0.8, 0.025, 0.015), 1.),
            Material::dielectric(v3(0.015, 0.04, 0.8), 1.),
        ],
        camera: Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 50., 0.),
        lights: vec![],
        extra_lights: vec![],
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

fn tile_ssim(a: &[u8], b: &[u8], width: usize, height: usize) -> f64 {
    let luma =
        |p: &[u8]| (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64) / 255.;
    let mut total = 0.;
    let mut count = 0.;
    for y in (0..height - 7).step_by(8) {
        for x in (0..width - 7).step_by(8) {
            let mut aa = Vec::new();
            let mut bb = Vec::new();
            for dy in 0..8 {
                for dx in 0..8 {
                    let i = ((y + dy) * width + x + dx) * 4;
                    aa.push(luma(&a[i..i + 4]));
                    bb.push(luma(&b[i..i + 4]));
                }
            }
            let ma = aa.iter().sum::<f64>() / 64.;
            let mb = bb.iter().sum::<f64>() / 64.;
            let va = aa.iter().map(|v| (v - ma).powi(2)).sum::<f64>() / 64.;
            let vb = bb.iter().map(|v| (v - mb).powi(2)).sum::<f64>() / 64.;
            let covariance = aa
                .iter()
                .zip(&bb)
                .map(|(a, b)| (a - ma) * (b - mb))
                .sum::<f64>()
                / 64.;
            total += (2. * ma * mb + 0.0001) * (2. * covariance + 0.0009)
                / ((ma * ma + mb * mb + 0.0001) * (va + vb + 0.0009));
            count += 1.;
        }
    }
    total / count
}

#[test]
fn guides_describe_the_wall_through_glass_in_both_traversal_paths() {
    let scene = glass_detail();
    let mut renderer = Renderer::new(48, 32).expect("GPU");
    for hardware in [false, true] {
        renderer.hardware_rt = hardware;
        renderer.set_scene(&scene, &scene.prepare());
        renderer.trace(4, 8, true, 0);
        renderer.finish_with(&|_, albedo, normal| {
            let i = (16 * 48 + 20) * 4;
            assert!(
                albedo[i] > 0.5 && albedo[i + 1] < 0.15,
                "glass must guide the red wall, not white front glass: {:?}",
                &albedo[i..i + 3]
            );
            assert!(
                normal[i + 2] > 0.8,
                "normal must follow refraction to the wall"
            );
            Ok(())
        });
    }
}

#[test]
fn denoised_glass_preserves_refracted_detail_against_2048_spp() {
    let scene = glass_detail();
    let (w, h) = (96, 64);
    let mut r = Renderer::new(w, h).expect("GPU");
    r.set_scene(&scene, &scene.prepare());
    r.trace(2048, 8, true, 0);
    let reference = r.finish(0).0;
    r.trace(256, 8, true, 64);
    let d = Denoiser::new().unwrap();
    let actual = r
        .finish_with_layers(&|color, shade, a, n| {
            d.run_layers_interruptible(&mut [color, shade], a, n, 4, w as usize, h as usize, None)
        })
        .0;
    let score = tile_ssim(&actual, &reference, w as usize, h as usize);
    let mae = actual
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
        .flat_map(|(a, b)| (0..3).map(move |i| (a[i] as f64 - b[i] as f64).abs() / 255.))
        .sum::<f64>()
        / (w * h * 3) as f64;
    assert!(
        score > 0.94 && mae < 0.025,
        "refracted detail regression: tile SSIM={score}, mean error={mae}"
    );
}

#[test]
fn temporal_export_never_reuses_front_surface_motion_for_glass() {
    let scene = glass_detail();
    let mut r = Renderer::new(48, 32).expect("GPU");
    r.set_scene(&scene, &scene.prepare());
    r.quality_temporal = true;
    let color = RefCell::new([0.2, 0.1, 0.1]);
    let filter = |c: &mut [f32], _: &mut [f32], _: &[f32], _: &[f32]| {
        for pixel in c.chunks_exact_mut(4) {
            pixel[..3].copy_from_slice(&*color.borrow());
        }
        Ok(())
    };
    r.trace(8, 8, true, 64);
    r.finish_with_layers(&filter);
    *color.borrow_mut() = [0.1, 0.1, 0.2];
    r.trace(8, 8, true, 64);
    let temporal = r.finish_with_layers(&filter).0;
    r.quality_temporal = false;
    let fresh = r.finish_with_layers(&filter).0;
    let i = (16 * 48 + 24) * 4;
    assert_eq!(
        &temporal[i..i + 4],
        &fresh[i..i + 4],
        "glass must reject temporal history even when its front geometry agrees"
    );
}

#[test]
fn temporal_export_stabilizes_shadow_catcher_caustics_too() {
    let mut scene = glass_detail();
    scene.objects.clear();
    scene.shadow_catcher = true;
    scene.backdrop = Vec3::splat(0.8);
    scene.camera = Camera::lens(v3(0., 2., 5.), Vec3::ZERO, 50., 0.);
    let mut r = Renderer::new(48, 32).expect("GPU");
    r.set_scene(&scene, &scene.prepare());
    r.quality_temporal = true;
    let phase = RefCell::new(false);
    let filter = |_: &mut [f32], shade: &mut [f32], _: &[f32], _: &[f32]| {
        for (i, pixel) in shade.chunks_exact_mut(4).enumerate() {
            pixel[..3].fill(if (i % 2 == 0) ^ *phase.borrow() {
                0.4
            } else {
                0.6
            });
        }
        Ok(())
    };
    r.trace(4, 8, true, 64);
    r.finish_with_layers(&filter);
    *phase.borrow_mut() = true;
    r.trace(4, 8, true, 64);
    let temporal = r.finish_with_layers(&filter).0;
    r.quality_temporal = false;
    let fresh = r.finish_with_layers(&filter).0;
    let spread = |pixels: &[u8]| {
        let row = &pixels[(24 * 48 * 4)..(25 * 48 * 4)];
        let values: Vec<f64> = row.chunks_exact(4).map(|p| p[0] as f64).collect();
        let mean = values.iter().sum::<f64>() / 48.;
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 48.
    };
    assert!(
        spread(&temporal) < spread(&fresh) * 0.1,
        "analytic catcher history must filter shading, not empty RGB"
    );
}

#[test]
fn oidn_guides_include_the_diffuse_shadow_catcher() {
    let mut scene = glass_detail();
    scene.objects.clear();
    scene.shadow_catcher = true;
    scene.backdrop = Vec3::splat(0.8);
    scene.camera = Camera::lens(v3(0., 2., 5.), Vec3::ZERO, 50., 0.);
    let mut r = Renderer::new(48, 32).expect("GPU");
    r.set_scene(&scene, &scene.prepare());
    r.trace(4, 8, true, 64);
    r.finish_with(&|_, albedo, normal| {
        let i = (24 * 48 + 24) * 4;
        assert!(
            albedo[i] > 0.8 && normal[i + 1] > 0.9,
            "OIDN must see the catcher's diffuse albedo and up normal"
        );
        Ok(())
    });
}

#[test]
fn adaptive_caustics_preserve_the_fixed_budget_image() {
    // A partially filled photon map can look locally quiet while its estimate
    // is still changing. Early per-pixel stopping used to freeze bright floor
    // patches here, even though the frame continued to its sample ceiling.
    let scene = pm_studio::scene::preset("chrome", "starlight", 0.).unwrap();
    let (w, h) = (320, 180);
    let mut r = Renderer::new(w, h).expect("GPU");
    r.set_scene(&scene, &scene.prepare());
    r.trace(512, 8, true, 64);
    let fixed = r.finish(0).0;
    r.set_adaptive_sampling(AdaptiveSampling {
        enabled: true,
        min_samples: 64,
        relative_error: 0.015,
        absolute_error: 0.0005,
    });
    r.trace(512, 8, true, 64);
    let adaptive = r.finish(0).0;
    let score = tile_ssim(&adaptive, &fixed, w as usize, h as usize);
    assert!(
        score > 0.98,
        "adaptive caustics changed the converged image: SSIM={score}"
    );
}
