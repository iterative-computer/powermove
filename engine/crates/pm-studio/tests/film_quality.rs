//! Linear HDR footprint and selective convergence on real GPU paths.
use pm_studio::{
    environment::{AdaptiveSampling, World},
    film::PixelFilter,
    gpu::Renderer,
    math::{Vec3, v3},
    mesh::Mesh,
    scene::{Camera, Material, Object, Scene},
};
fn edge() -> Scene {
    let mesh = Mesh {
        positions: vec![
            v3(-10., -10., 0.),
            v3(0.025, -10., 0.),
            v3(0.025, 10., 0.),
            v3(-10., 10., 0.),
        ],
        normals: vec![v3(0., 0., 1.); 4],
        uvs: vec![[0., 0.]; 4],
        indices: vec![[0, 1, 2], [0, 2, 3]],
        ..Default::default()
    };
    Scene {
        meters_per_unit: 1.,
        world: World::default(),
        objects: vec![Object { mesh, material: 0 }],
        materials: vec![Material {
            emission: Vec3::splat(4.),
            specular: 0.,
            ..Material::dielectric(Vec3::ZERO, 0.7)
        }],
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
#[test]
fn blackman_harris_reconstructs_hdr_coverage_before_tone_mapping() {
    let scene = edge();
    let (w, h) = (64, 32);
    let mut r = Renderer::new(w, h).unwrap();
    for hardware in [false, true] {
        r.hardware_rt = hardware;
        r.set_scene(&scene, &scene.prepare());
        r.set_void(None);
        let mut spread = vec![];
        for filter in [PixelFilter::Box, PixelFilter::default()] {
            r.set_pixel_filter(filter).unwrap();
            r.trace(256, 1, true, 0);
            let pixels = r.linear_beauty();
            assert!(pixels.iter().all(|p| p.is_finite()));
            assert!(pixels.chunks_exact(4).any(|p| p[0] > 3.99));
            assert!(
                pixels
                    .chunks_exact(4)
                    .all(|p| (p[0] - 4. * p[3]).abs() < 0.0001),
                "HDR/coverage must share the linear estimator"
            );
            let columns: Vec<_> = (0..w as usize)
                .map(|x| {
                    (0..h as usize)
                        .map(|y| pixels[(y * w as usize + x) * 4 + 3])
                        .sum::<f32>()
                        / h as f32
                })
                .collect();
            spread.push(
                columns
                    .iter()
                    .filter(|a| **a > 0.001 && **a < 0.999)
                    .count(),
            );
        }
        assert!(
            spread[1] > spread[0],
            "Blackman-Harris footprint, hardware={hardware}: {spread:?}"
        );
    }
}
#[test]
fn adaptive_edges_receive_extra_samples_while_flat_pixels_converge() {
    let scene = edge();
    let (w, h) = (64, 32);
    let mut r = Renderer::new(w, h).unwrap();
    r.set_scene(&scene, &scene.prepare());
    r.set_void(None);
    r.set_adaptive_sampling_with_floor(
        AdaptiveSampling {
            enabled: true,
            min_samples: 16,
            relative_error: 0.1,
            absolute_error: 0.001,
        },
        true,
    );
    r.trace(256, 1, true, 0);
    let counts = r.sample_counts();
    let flat: Vec<_> = (4..h - 4).map(|y| counts[(y * w + 8) as usize]).collect();
    let edges: Vec<_> = (4..h - 4)
        .flat_map(|y| (30..35).map(move |x| (y * w + x) as usize))
        .map(|i| counts[i])
        .collect();
    assert!(
        flat.iter().all(|n| *n == 16.),
        "flat emitter should stop at its floor: {flat:?}"
    );
    assert!(
        edges.iter().any(|n| *n >= 64.),
        "coverage/detail edges need at least four times the floor: {edges:?}"
    );
    assert!(counts.iter().all(|n| *n <= 256.));
    // Changing the reconstruction kernel also invalidates an accumulation even
    // if the caller continues refinement without explicitly requesting reset.
    r.set_pixel_filter(PixelFilter::Box).unwrap();
    r.trace(4, 1, false, 0);
    assert!(r.sample_counts().iter().all(|n| *n == 4.));
}
