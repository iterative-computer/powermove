//! Original procedural product fixtures: no downloaded model or app asset.
use pm_studio::{
    environment::World,
    math::{Vec3, v3},
    mesh::{self, Mesh},
    scene::{Camera, Material, Object, Scene, Softbox},
};
fn lathe(profile: &[[f32; 2]], segments: usize) -> Mesh {
    let mut mesh = Mesh::default();
    for (row, p) in profile.iter().enumerate() {
        let a = profile[row.saturating_sub(1)];
        let b = profile[(row + 1).min(profile.len() - 1)];
        for i in 0..=segments {
            let angle = std::f32::consts::TAU * i as f32 / segments as f32;
            mesh.positions
                .push(v3(p[0] * angle.cos(), p[1], p[0] * angle.sin()));
            mesh.normals.push(
                v3(
                    (b[1] - a[1]) * angle.cos(),
                    a[0] - b[0],
                    (b[1] - a[1]) * angle.sin(),
                )
                .normalize(),
            );
            mesh.uvs.push([
                i as f32 / segments as f32,
                row as f32 / profile.len() as f32,
            ]);
        }
    }
    for row in 0..profile.len() - 1 {
        for i in 0..segments {
            let a = (row * (segments + 1) + i) as u32;
            let b = a + segments as u32 + 1;
            mesh.indices.extend([[a, b, a + 1], [a + 1, b, b + 1]]);
        }
    }
    mesh
}
fn tube(curve: impl Fn(f32) -> Vec3, radius: impl Fn(f32) -> f32) -> Mesh {
    let mut mesh = Mesh::default();
    let (rings, sides) = (48, 16);
    for row in 0..=rings {
        let t = row as f32 / rings as f32;
        let center = curve(t);
        let tangent = (curve((t + 0.001).min(1.)) - curve((t - 0.001).max(0.))).normalize();
        let n = v3(-tangent.y, tangent.x, 0.);
        for i in 0..=sides {
            let angle = std::f32::consts::TAU * i as f32 / sides as f32;
            let normal = n * angle.cos() + v3(0., 0., angle.sin());
            mesh.positions.push(center + normal * radius(t));
            mesh.normals.push(normal);
            mesh.uvs.push([t, i as f32 / sides as f32]);
        }
    }
    for row in 0..rings {
        for i in 0..sides {
            let a = (row * (sides + 1) + i) as u32;
            let b = a + sides as u32 + 1;
            mesh.indices.extend([[a, a + 1, b], [a + 1, b + 1, b]]);
        }
    }
    mesh
}
pub fn product(name: &str) -> Scene {
    let mut scene = Scene {
        meters_per_unit: 1.,
        world: World::default(),
        objects: vec![],
        materials: vec![],
        lights: vec![
            Softbox::aimed(Vec3::ZERO, -40., 50., 5., 3., 3., Vec3::splat(4.)),
            Softbox::aimed(Vec3::ZERO, 70., 30., 5., 2., 3., Vec3::splat(2.)),
        ],
        camera: Camera::lens(v3(4., 2.3, 7.), Vec3::ZERO, 50., 0.),
        backdrop: Vec3::ZERO,
        environment: (Vec3::splat(0.08), Vec3::splat(0.08)),
        exposure: 1.,
        shadow_catcher: false,
        extra_lights: vec![],
        layers: None,
        instances: vec![],
        scatters: vec![],
        terrains: vec![],
        displacements: vec![],
    };
    let meshes = match name {
        "teapot" => {
            // Matte enamel: authored perceptual roughness, no mirror clearcoat.
            scene
                .materials
                .push(Material::dielectric(v3(0.32, 0.055, 0.018), 0.7));
            vec![
                lathe(
                    &[
                        [0., -0.85],
                        [0.4, -0.85],
                        [0.75, -0.65],
                        [0.95, -0.25],
                        [1., 0.1],
                        [0.85, 0.5],
                        [0.5, 0.7],
                        [0., 0.7],
                    ],
                    96,
                ),
                mesh::sphere(v3(0., 0.76, 0.), 0.16, 16, 32),
                tube(
                    |t| v3(0.75 + 0.9 * t, 0.1 + 0.55 * t * t, 0.),
                    |t| 0.22 - 0.1 * t,
                ),
                tube(
                    |t| {
                        let a = (50. + 260. * t).to_radians();
                        v3(-0.9 + 0.7 * a.cos(), 0.65 * a.sin(), 0.)
                    },
                    |_| 0.11,
                ),
            ]
        }
        "glass" => {
            scene
                .materials
                .push(Material::glass(Vec3::splat(0.99), 0.02));
            vec![
                lathe(
                    &[
                        [0., -0.85],
                        [0.64, -0.85],
                        [0.76, -0.7],
                        [0.8, 0.75],
                        [0.78, 0.82],
                        [0.71, 0.82],
                        [0.7, 0.72],
                        [0.66, -0.63],
                        [0.55, -0.72],
                        [0., -0.72],
                    ],
                    96,
                ),
                tube(
                    |t| {
                        let a = (-120. + 240. * t).to_radians();
                        v3(0.7 + 0.48 * a.cos(), 0.52 * a.sin(), 0.)
                    },
                    |_| 0.075,
                ),
            ]
        }
        "chrome" => {
            scene
                .materials
                .push(Material::metal(Vec3::splat(0.95), 0.06));
            vec![mesh::sphere(Vec3::ZERO, 0.85, 48, 96)]
        }
        _ => panic!("unknown noise fixture"),
    };
    for mesh in meshes {
        scene.objects.push(Object { mesh, material: 0 });
    }
    scene
        .materials
        .push(Material::dielectric(Vec3::splat(0.18), 0.9));
    scene.objects.push(Object {
        material: 1,
        mesh: Mesh {
            positions: vec![
                v3(-6., -0.86, -6.),
                v3(6., -0.86, -6.),
                v3(6., -0.86, 6.),
                v3(-6., -0.86, 6.),
            ],
            normals: vec![Vec3::Y; 4],
            uvs: vec![[0., 0.]; 4],
            indices: vec![[0, 2, 1], [0, 3, 2]],
            ..Default::default()
        },
    });
    scene
}
