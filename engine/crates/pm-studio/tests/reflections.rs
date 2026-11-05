mod common;
use pm_studio::{
    extract::{Extractor, MeshInstance, SceneInput, Transform},
    gpu::{PresentTarget, Renderer},
    math::{Vec3, v3},
    mesh::{self, Mesh},
    scene::{Camera, Material, preset},
};
use std::sync::Arc;

#[test]
fn an_off_camera_object_is_visible_in_another_layers_reflection() {
    let mut scene = preset("hero", "space-gray", 0.).unwrap();
    scene.objects.clear();
    scene.lights.clear();
    scene.extra_lights.clear();
    scene.environment = (Vec3::ZERO, Vec3::ZERO);
    scene.shadow_catcher = false;
    scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 32., 0.);
    let mut emitter = Material::dielectric(Vec3::ZERO, 1.);
    // Stay below Neutral's 0.76 shoulder: pure red remains pure red.
    emitter.emission = v3(0.5, 0., 0.);
    scene.materials = vec![Material::metal(Vec3::splat(0.9), 0.02), emitter];
    let mirror = Arc::new(mesh::sphere(Vec3::ZERO, 1., 24, 48));
    // Entirely behind the camera; primary rays cannot see this red card.
    let card = Arc::new(Mesh {
        positions: vec![
            v3(-6., -6., 6.),
            v3(6., -6., 6.),
            v3(6., 6., 6.),
            v3(-6., 6., 6.),
        ],
        normals: vec![v3(0., 0., -1.); 4],
        uvs: vec![[0.; 2]; 4],
        indices: vec![[0, 2, 1], [0, 3, 2]],
    });
    let mut input = SceneInput {
        scene,
        instances: vec![MeshInstance {
            layer: "mirror".into(),
            mesh: mirror,
            material: 0,
            world: Transform::default(),
        }],
        materials: None,
        maps: None,
        view: Default::default(),
    };
    let mut extractor = Extractor::default();
    let mut renderer = Renderer::new(64, 64).expect("Metal or Vulkan required");
    let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut draw = |input: &SceneInput, layer: u32| {
        let extracted = extractor.extract(input).unwrap();
        renderer.set_extracted(&input.scene, &extracted);
        renderer.set_void(None);
        renderer.trace_seeded(4, 8, true, 64, 10);
        renderer.present_to(
            0,
            PresentTarget {
                view: &view,
                format: wgpu::TextureFormat::Rgba8Unorm,
                layer: Some(layer),
                include_void: false,
                flip_y: false,
            },
        );
        common::read(renderer.device(), renderer.queue(), &texture)
    };
    let dark = draw(&input, 0);
    input.instances.push(MeshInstance {
        layer: "red-card".into(),
        mesh: card,
        material: 1,
        world: Transform::default(),
    });
    let reflected = draw(&input, 0);
    let off_camera = draw(&input, 1);
    assert!(
        off_camera.chunks_exact(4).all(|p| p[3] == 0),
        "the red card is outside the primary view"
    );
    assert!(
        dark.chunks_exact(4)
            .all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0),
        "unlit metal starts black"
    );
    assert!(
        reflected
            .chunks_exact(4)
            .filter(|p| p[0] > 20 && p[1] == 0 && p[2] == 0)
            .count()
            > 100,
        "the mirror layer must contain the off-camera red card's reflection"
    );
}
