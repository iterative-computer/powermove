//! Source Studio swatch recipe (`core/studio/swatch.ts`), without DOM/canvas.
//! These are content renders; no native control layout or styling is defined.
use crate::{
    gpu::Renderer,
    math::{Vec3, v3},
    mesh,
    scene::{Camera, Material, Object, Scene, Softbox},
};

pub const SIZE: u32 = 96;

fn rectangle(center: Vec3, target: Vec3, width: f32, height: f32, intensity: f32) -> Softbox {
    let facing = (target - center).normalize();
    let right = Vec3::Y.cross(facing).normalize();
    let up = facing.cross(right).normalize();
    Softbox {
        center,
        u: right * (width * 0.5),
        v: up * (height * 0.5),
        radiance: Vec3::splat(intensity),
    }
}

pub fn scene(material: Material) -> Scene {
    Scene {
        objects: vec![Object {
            mesh: mesh::sphere(Vec3::ZERO, 1., 32, 64),
            material: 0,
        }],
        materials: vec![material],
        lights: vec![
            rectangle(v3(-2.5, 3., 3.), Vec3::ZERO, 3., 2., 6.),
            rectangle(v3(3., 1., -2.5), Vec3::ZERO, 4., 0.8, 4.),
        ],
        camera: Camera {
            position: v3(0., 0.6, 4.4),
            target: Vec3::ZERO,
            fov: 28.,
            f_stop: 0.,
            focus: None,
            focal_mm: 50.,
        },
        backdrop: v3(0.965, 0.961, 0.973),
        environment: (v3(0.35, 0.35, 0.37), v3(0.22, 0.22, 0.24)),
        exposure: 1.,
        shadow_catcher: false,
        extra_lights: vec![],
        layers: None,
    }
}

/// Call on a worker. Reuse a 96×96 renderer to cache device/pipeline work.
/// Texture-backed screen emission needs a real scene render, not this swatch.
pub fn render(renderer: &mut Renderer, material: Material) -> Result<Vec<u8>, String> {
    if renderer.size() != (SIZE, SIZE) {
        return Err("Studio swatches require the source's 96×96 target".into());
    }
    if material.screen {
        return Err("Studio screen swatches require a resolved composition".into());
    }
    let scene = scene(material);
    renderer.set_scene(&scene, &scene.prepare());
    renderer.set_view(crate::extract::View {
        ground: Some(v3(0.08, 0.08, 0.09)),
        ..Default::default()
    })?;
    renderer.trace_seeded(32, 6, true, 64, 0);
    Ok(renderer.finish(2).0)
}
