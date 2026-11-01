//! Studio, the path-traced product renderer (`pm-studio`,
//! docs/3d-studio-redesign.md), as the 3D pass's renderer for object layers:
//! one trace of a layer's depth set, each layer presented with only its own
//! pixels, as the app does on WebGPU (`core/studio`). Opt in with
//! [`Scene3dRenderer::set_studio`](super::Scene3dRenderer::set_studio) or
//! `PM_STUDIO=1`.
//!
//! Converted from the evaluated scene: meshes in world space, the
//! `MeshStandardMaterial` base (color, roughness, metalness, emission),
//! directional, point, spot and rect-area lights, the perspective camera,
//! the hemisphere as the environment and the background. Texture maps and
//! glTF material extensions are not converted yet; orthographic cameras
//! stay on the rasterizer.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pm_gpu::{Target, TargetPool};
use pm_studio::gpu::{PresentTarget, Renderer};
use pm_studio::math::{Vec3 as SVec3, v3};
use pm_studio::mesh::Mesh;
use pm_studio::scene::{Camera, GpuLight, Material, Object, Scene, Softbox};

use super::model::{LightKind, Materials, Projection, Scene3d};
use crate::three::math::{self, Mat4};

/// Samples per pixel of a Studio frame (the app's Final "Standard").
pub const STUDIO_SAMPLES: u32 = 64;

/// Studio's state across frames: one renderer per output size, and the
/// scene it holds (geometry is uploaded again only when it changes).
pub struct StudioPass {
    renderer: Option<Renderer>,
    scene_key: u64,
    view_key: u64,
    pub samples: u32,
}

impl Default for StudioPass {
    fn default() -> Self {
        StudioPass { renderer: None, scene_key: 0, view_key: 0, samples: STUDIO_SAMPLES }
    }
}

fn point(m: &Mat4, p: [f32; 3]) -> SVec3 {
    let (x, y, z) = (f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
    v3(
        (m[0] * x + m[4] * y + m[8] * z + m[12]) as f32,
        (m[1] * x + m[5] * y + m[9] * z + m[13]) as f32,
        (m[2] * x + m[6] * y + m[10] * z + m[14]) as f32,
    )
}

fn direction(n: &[f64; 9], d: [f32; 3]) -> SVec3 {
    let (x, y, z) = (f64::from(d[0]), f64::from(d[1]), f64::from(d[2]));
    v3((n[0] * x + n[3] * y + n[6] * z) as f32, (n[1] * x + n[4] * y + n[7] * z) as f32, (n[2] * x + n[5] * y + n[8] * z) as f32).normalize()
}

fn v(c: [f64; 3]) -> SVec3 {
    v3(c[0] as f32, c[1] as f32, c[2] as f32)
}

/// The depth set as Studio's scene: one Studio object per (mesh, material
/// slot), each tagged with its layer's object index.
pub fn studio_scene(scene: &Scene3d, depth_set: &[usize]) -> Option<Scene> {
    let Projection::Perspective { fov, zoom, .. } = scene.camera.projection else { return None };
    let mut objects = Vec::new();
    let mut materials: Vec<Material> = Vec::new();
    let mut layers = Vec::new();
    let mut size = 0.0f32;
    for &index in depth_set {
        let Some(object) = scene.objects.get(index) else { continue };
        if !object.visible || object.missing {
            continue;
        }
        for mesh in &object.meshes {
            let g = &mesh.geometry.geometry;
            let (pos, nrm, uv) = (g.positions(), g.normals(), g.uvs());
            let count = pos.len() / 3;
            if count < 3 {
                continue;
            }
            let normal_matrix = math::normal_matrix(&mesh.world);
            let at = |a: &[f32], i: usize, k: usize| -> [f32; 3] { if a.len() >= (i + 1) * k { [a[i * k], a[i * k + 1], if k == 3 { a[i * k + 2] } else { 0.0 }] } else { [0.0; 3] } };
            let positions: Vec<SVec3> = (0..count).map(|i| point(&mesh.world, at(pos, i, 3))).collect();
            let normals: Vec<SVec3> = (0..count).map(|i| if nrm.is_empty() { SVec3::Y } else { direction(&normal_matrix, at(nrm, i, 3)) }).collect();
            let uvs: Vec<[f32; 2]> = (0..count).map(|i| { let t = at(uv, i, 2); [t[0], t[1]] }).collect();
            let indices: Vec<u32> = if g.indexed() { g.indices().to_vec() } else { (0..count as u32).collect() };
            for p in &positions {
                size = size.max(p.length());
            }
            // Material slots follow the geometry's groups; one material covers all.
            let slots: Vec<(u32, u32, usize)> = match (&mesh.materials, g.groups()) {
                (Materials::Slots(_), groups) if !groups.is_empty() => groups.iter().map(|gr| (gr.start, gr.count, gr.material_index as usize)).collect(),
                _ => vec![(0, indices.len() as u32, 0)],
            };
            for (start, n, slot) in slots {
                let Some(m) = mesh.materials.iter().nth(slot) else { continue };
                if m.opacity <= 0.0 {
                    continue;
                }
                let end = (start + n).min(indices.len() as u32) as usize;
                let tris: Vec<[u32; 3]> = indices[start as usize..end].chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
                if tris.is_empty() {
                    continue;
                }
                let mut material = Material::dielectric(v(m.color), m.roughness.clamp(0.02, 1.0) as f32);
                material.metallic = m.metalness as f32;
                material.emission = v(m.emissive) * m.emissive_intensity as f32;
                materials.push(material);
                objects.push(Object { mesh: Mesh { positions: positions.clone(), normals: normals.clone(), uvs: uvs.clone(), indices: tris }, material: materials.len() - 1 });
                layers.push(index as u32);
            }
        }
    }
    let size = size.max(1e-3);
    let mut softboxes = Vec::new();
    let mut extra = Vec::new();
    for l in scene.lights.iter().filter(|l| l.visible && l.intensity > 0.0) {
        let c = v(l.color) * l.intensity as f32;
        let at = v([l.world[12], l.world[13], l.world[14]]);
        match l.kind {
            LightKind::Directional => {
                // three's directional intensity is irradiance; a 1.5° disc carries it.
                let toward = at - v(l.target);
                let angle = 1.5f32.to_radians();
                let solid = 2.0 * std::f32::consts::PI * (1.0 - angle.cos());
                extra.push(GpuLight::sun(toward, angle, c * (1.0 / solid)));
            }
            LightKind::Point | LightKind::Spot => {
                // A small sphere with the light's intensity (candela).
                let r = (size * 0.004).max(1e-3);
                let radiance = c * (1.0 / (std::f32::consts::PI * r * r));
                if l.kind == LightKind::Spot {
                    let outer = l.angle.cos() as f32;
                    let inner = (l.angle * (1.0 - l.penumbra)).cos() as f32;
                    extra.push(GpuLight::spot(at, r, v(l.target) - at, outer, inner, radiance));
                } else {
                    extra.push(GpuLight::sphere(at, r, radiance));
                }
            }
            LightKind::RectArea => {
                // Local x by width, y by height; three's rect lights emit along -z, so u × v = -z.
                let x = v([l.world[0], l.world[1], l.world[2]]).normalize() * (l.width as f32 * 0.5);
                let y = v([l.world[4], l.world[5], l.world[6]]).normalize() * (l.height as f32 * 0.5);
                softboxes.push(Softbox { center: at, u: y, v: x, radiance: c });
            }
        }
    }
    let w = &scene.camera.world;
    let position = v([w[12], w[13], w[14]]);
    let forward = v([-w[8], -w[9], -w[10]]).normalize();
    let fov_y = 2.0 * ((fov.to_radians() / 2.0).tan() / zoom.max(1e-3)).atan();
    let (sky, ground, hemi) = scene.hemisphere;
    Some(Scene {
        objects,
        materials,
        lights: softboxes,
        camera: Camera { position, target: position + forward, fov: fov_y.to_degrees() as f32, f_stop: 0.0, focus: None, focal_mm: (12.0 / (fov_y / 2.0).tan()) as f32 },
        backdrop: SVec3::ZERO,
        environment: (v(sky) * hemi as f32, (v(sky) + v(ground)) * (0.5 * hemi as f32)),
        exposure: scene.exposure as f32,
        shadow_catcher: false,
        extra_lights: extra,
        layers: Some(layers),
    })
}

impl StudioPass {
    pub fn with_samples(samples: u32) -> StudioPass {
        StudioPass { samples, ..Default::default() }
    }

    /// One layer of a depth set, traced (once per scene and view) and
    /// presented into a pooled RGBA8 target in GL row order.
    #[allow(clippy::too_many_arguments)]
    pub fn render_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &mut TargetPool,
        scene: &Scene3d,
        object: usize,
        depth_set: &[usize],
        background: bool,
    ) -> Option<Target> {
        let studio = studio_scene(scene, depth_set)?;
        let (w, h) = (scene.width.max(1), scene.height.max(1));
        if self.renderer.as_ref().is_none_or(|r| r.size() != (w, h)) {
            self.renderer = Renderer::with_device(device.clone(), queue.clone(), "native".into(), false, w, h).ok();
            self.scene_key = 0;
        }
        let r = self.renderer.as_mut()?;
        // Geometry, materials and lights (by content); the camera and void separately.
        let mut hs = DefaultHasher::new();
        for &i in depth_set {
            if let Some(o) = scene.objects.get(i) {
                format!("{:?}{:?}", o.meshes.iter().map(|m| (m.geometry.key, m.world.map(f64::to_bits))).collect::<Vec<_>>(), o.meshes.iter().map(|m| format!("{:?}", m.materials)).collect::<Vec<_>>()).hash(&mut hs);
                o.visible.hash(&mut hs);
            }
        }
        format!("{:?}{:?}", scene.lights, scene.hemisphere).hash(&mut hs);
        let scene_key = hs.finish();
        let mut hv = DefaultHasher::new();
        (scene_key, scene.camera.world.map(f64::to_bits), format!("{:?}", scene.camera.projection), scene.exposure.to_bits(), background, format!("{:?}", scene.background), self.samples).hash(&mut hv);
        let view_key = hv.finish();
        if scene_key != self.scene_key {
            r.set_scene(&studio, &studio.prepare());
            self.scene_key = scene_key;
            self.view_key = 0;
        }
        if view_key != self.view_key {
            r.set_camera(&studio);
            let void = match (background, scene.background) {
                (true, Some(c)) => Some([math::linear_to_srgb(c[0]) as f32, math::linear_to_srgb(c[1]) as f32, math::linear_to_srgb(c[2]) as f32]),
                _ => None,
            };
            r.set_void(void);
            r.trace(self.samples, 8, true, 64);
            self.view_key = view_key;
        }
        let target = pool.take(device, w, h, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        r.present_to(0, PresentTarget { view: &target.view, format: wgpu::TextureFormat::Rgba8Unorm, layer: Some(object as u32), include_void: background, flip_y: true });
        Some(target)
    }
}
