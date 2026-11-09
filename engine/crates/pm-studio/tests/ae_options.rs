//! AE lighting and visibility policies must agree in portable and hardware traversal.
use pm_studio::{
    environment::{EnvironmentMap, World},
    gpu::Renderer,
    math::{Vec3, v3},
    mesh::{self, Mesh},
    scene::{Camera, Material, Object, Scene},
};
fn scene() -> Scene {
    Scene {
        meters_per_unit: 1.,
        world: World {
            hdri: Some(EnvironmentMap {
                width: 1,
                height: 1,
                pixels: vec![Vec3::ZERO],
                ..Default::default()
            }),
            ..Default::default()
        },
        objects: vec![],
        materials: vec![],
        lights: vec![],
        extra_lights: vec![],
        camera: Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 50., 0.),
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
fn plane(z: f32, facing: f32) -> Mesh {
    Mesh {
        positions: vec![
            v3(-5., -5., z),
            v3(5., -5., z),
            v3(5., 5., z),
            v3(-5., 5., z),
        ],
        normals: vec![v3(0., 0., facing); 4],
        uvs: vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
        indices: if facing > 0. {
            vec![[0, 1, 2], [0, 2, 3]]
        } else {
            vec![[0, 2, 1], [0, 3, 2]]
        },
        ..Default::default()
    }
}
fn center(r: &mut Renderer) -> f32 {
    let pixels = r.radiance();
    let mut value = 0.;
    for y in 14..18 {
        for x in 14..18 {
            value += pixels[(y * 32 + x) * 4];
        }
    }
    value / 16.
}
#[test]
fn ambient_layers_light_a_diffuse_surface_without_a_world_map_or_shadows() {
    let mut s = scene();
    s.materials.push(Material {
        specular: 0.,
        ..Material::dielectric(Vec3::splat(0.7), 1.)
    });
    s.objects.push(Object {
        mesh: mesh::sphere(Vec3::ZERO, 1., 24, 48),
        material: 0,
    });
    let mut r = Renderer::new(32, 32).expect("GPU");
    for hardware in [false, true] {
        r.hardware_rt = hardware;
        r.set_scene(&s, &s.prepare());
        r.set_ambient([1., 0.5, 0.2]);
        r.trace(32, 4, true, 1000000);
        let lit = center(&mut r);
        assert!(
            (lit - 0.7).abs() < 0.01,
            "ambient diffuse expected .7, got {lit}"
        );
        r.set_ambient([0.; 3]);
        r.trace(16, 4, true, 1000000);
        assert!(center(&mut r) < 0.001);
    }
}
#[test]
fn reflection_visibility_excludes_secondary_hits_and_preserves_primary_hits() {
    let mut s = scene();
    s.materials = vec![
        Material::metal(Vec3::splat(1.), 0.002),
        Material {
            emission: v3(3., 0., 0.),
            ..Material::dielectric(Vec3::ZERO, 1.)
        },
    ];
    s.objects = vec![
        Object {
            mesh: plane(0., 1.),
            material: 0,
        },
        Object {
            mesh: plane(8., -1.),
            material: 1,
        },
    ];
    let mut r = Renderer::new(32, 32).expect("GPU");
    for hardware in [false, true] {
        r.hardware_rt = hardware;
        let mut prepared = s.prepare();
        r.set_scene(&s, &prepared);
        r.trace(32, 4, true, 1000000);
        let visible = center(&mut r);
        assert!(visible > 2., "mirror must see the red panel: {visible}");
        prepared.materials[1].policy[1] = (prepared.materials[1].policy[1] as u32 | 32) as f32;
        r.set_scene(&s, &prepared);
        r.trace(32, 4, true, 1000000);
        let hidden = center(&mut r);
        assert!(hidden < 0.01, "reflection-hidden panel leaked {hidden}");
        let mut direct = s.clone();
        direct.camera.target = v3(0., 0., 8.);
        r.set_scene(&direct, &prepared);
        r.trace(16, 4, true, 1000000);
        let (pixels, _) = r.finish(0);
        assert!(
            pixels[(16 * 32 + 16) * 4] > 200,
            "primary camera must still see reflection-hidden objects"
        );
    }
}
