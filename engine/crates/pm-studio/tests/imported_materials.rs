use pm_studio::{
    gpu::Renderer,
    math::{Vec3, v3},
    mesh,
    scene::{Camera, GpuLight, GpuMaterial, GpuTextureMap, GpuTri, Material, Object, Scene},
};

fn scene() -> Scene {
    Scene {
        meters_per_unit: 1.,
        world: Default::default(),
        objects: vec![],
        materials: vec![],
        lights: vec![],
        extra_lights: vec![],
        camera: Camera::lens(v3(0., 0., 4.), Vec3::ZERO, 50., 0.),
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
fn plane(z: f32) -> mesh::Mesh {
    mesh::Mesh {
        positions: vec![
            v3(-5., -5., z),
            v3(5., -5., z),
            v3(5., 5., z),
            v3(-5., 5., z),
        ],
        normals: vec![v3(0., 0., 1.); 4],
        uvs: vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
        indices: vec![[0, 1, 2], [0, 2, 3]],
        ..Default::default()
    }
}
fn imported(material: &mut GpuMaterial) {
    material.imported = [
        [1., 1., 1., 1.],
        [1., 1., 1., 0.],
        [1., 1., 1., 0.],
        [0., 0., 1.3, 100.],
        [400., 1., 1., 1.],
        [1., 0., 0., 1.],
    ];
    material.physical_maps = [GpuTextureMap {
        info: [-1., 0., 0., 0.],
        u: [1., 0., 0., 0.],
        v: [0., 1., 0., 0.],
    }; 18];
}
fn center(renderer: &Renderer) -> [f32; 3] {
    let pixels = renderer.linear_beauty();
    let o = (8 * 16 + 8) * 4;
    [pixels[o], pixels[o + 1], pixels[o + 2]]
}

#[test]
fn lit_materials_keep_layer_ids_coverage_and_denoising_guides() {
    let mut s = scene();
    s.materials.push(Material {
        emission: Vec3::splat(1.),
        ..Material::dielectric(Vec3::splat(1.), 1.)
    });
    s.objects.push(Object {
        mesh: plane(0.),
        material: 0,
    });
    s.extra_lights.push(GpuLight {
        pos: [0., 2., 3., 1.],
        u: [0.1, 0., 2., 0.],
        v: [0.; 4],
        radiance: [100.; 4],
    });
    let mut r = Renderer::new(16, 16).expect("GPU");
    for hardware in [false, true] {
        r.hardware_rt = hardware;
        let mut p = s.prepare();
        p.materials[0].policy[1] = 15.;
        r.set_scene(&s, &p);
        r.trace(4, 2, true, 1000000);
        assert!(center(&r).iter().all(|v| *v > 0.95));
        for source in [false, true] {
            let m = &mut p.materials[0];
            m.emit = [0.; 4];
            m.policy[1] = 7.;
            if source {
                imported(m);
                m.layers[0] = 0.3;
                m.imported[0][3] = 0.35;
                m.imported[1] = [1., 0.2, 0.1, 0.];
            }
            r.set_scene(&s, &p);
            r.trace(16, 4, true, 1000000);
            assert_eq!(
                r.first_hit_ids()[8 * 16 + 8],
                1,
                "lit first hit, hardware {hardware}, source {source}"
            );
            let linear = r.linear_beauty();
            let c = &linear[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4];
            assert!(
                c[3] > 0.99 && c[0] > 0.001,
                "lit pixel, hardware {hardware}, source {source}: {c:?}"
            );
            r.finish_with(&|color, albedo, normal| {
                assert!(
                    color
                        .iter()
                        .chain(albedo)
                        .chain(normal)
                        .all(|v| v.is_finite())
                );
                assert!(
                    color[(8 * 16 + 8) * 4] > 0.001
                        && albedo[(8 * 16 + 8) * 4] > 0.9
                        && normal[(8 * 16 + 8) * 4 + 2] > 0.9
                );
                Ok(())
            });
            assert_eq!(
                r.first_hit_ids()[8 * 16 + 8],
                1,
                "guides preserve first hit"
            );
        }
    }
}

#[test]
fn shared_layout_and_transformed_secondary_uv_sampling_match_on_both_traversals() {
    assert_eq!(std::mem::size_of::<GpuTri>(), 36 * 4);
    assert_eq!(std::mem::size_of::<GpuMaterial>(), 272 * 4);
    let mut s = scene();
    s.materials.push(Material {
        emission: Vec3::splat(1.),
        ..Material::dielectric(Vec3::splat(1.), 1.)
    });
    s.objects.push(Object {
        mesh: plane(0.),
        material: 0,
    });
    let mut p = s.prepare();
    let m = &mut p.materials[0];
    imported(m);
    m.policy[1] = 15.;
    let map = GpuTextureMap {
        info: [0., 1., 1., 1.],
        u: [0.5, 0., 0.5, 0.],
        v: [0., 0.5, 0., 0.],
    };
    m.physical_maps[0] = map;
    m.physical_maps[1] = map;
    for tri in &mut p.tris {
        tri.uv01_secondary = [0.5, 0.5, 0.5, 0.5];
        tri.uv2_secondary = [0.5, 0.5, 1., 0.];
    }
    let mut r = Renderer::new(16, 16).expect("GPU");
    for hardware in [false, true] {
        r.hardware_rt = hardware;
        r.set_scene(&s, &p);
        r.set_maps(
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ],
            2,
            2,
            1,
        );
        r.trace(4, 2, true, 1000000);
        let c = center(&r);
        assert!(
            c[1] > 0.95 && c[0] < 0.02 && c[2] < 0.02,
            "secondary UV transform: {c:?}"
        );
    }
}

#[test]
fn imported_volume_obeys_beer_lambert_in_geometry_units() {
    let mut s = scene();
    s.materials.push(Material {
        transmission: 1.,
        ior: 1.,
        specular: 0.,
        ..Material::dielectric(Vec3::splat(1.), 0.002)
    });
    s.materials.push(Material {
        emission: Vec3::splat(1.),
        ..Material::dielectric(Vec3::splat(1.), 1.)
    });
    s.objects.push(Object {
        mesh: mesh::sphere(Vec3::ZERO, 1., 32, 64),
        material: 0,
    });
    s.objects.push(Object {
        mesh: plane(-2.),
        material: 1,
    });
    let mut p = s.prepare();
    imported(&mut p.materials[0]);
    p.materials[0].imported[2] = [0.25, 0.5, 1., 2.];
    p.materials[0].imported[3][0] = 1.;
    p.materials[1].policy[1] = 15.;
    let mut r = Renderer::new(16, 16).expect("GPU");
    for hardware in [false, true] {
        r.hardware_rt = hardware;
        r.set_scene(&s, &p);
        r.caustics = false;
        r.trace(32, 8, true, 1000000);
        let c = center(&r);
        assert!(c[2] > 0.9, "unabsorbed channel: {c:?}");
        assert!(
            (c[0] / c[2] - 0.25).abs() < 0.04 && (c[1] / c[2] - 0.5).abs() < 0.04,
            "Beer-Lambert: {c:?}"
        );
    }
}
