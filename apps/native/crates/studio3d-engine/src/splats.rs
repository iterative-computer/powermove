//! Gaussian EWA rasterization from scene3d/splats.ts. Splats composite as
//! ordinary layers; they never enter Studio's surface BVH or reflections.
use crate::{
    extract::Transform,
    math::{Vec3, v3},
    scene::Camera,
};
use std::sync::Arc;
#[derive(Clone)]
pub struct Gaussian {
    pub center: [f32; 3],
    pub covariance: [f32; 6],
    pub rgba: [u8; 4],
}
pub struct Layer {
    pub id: String,
    pub data: Arc<Vec<Gaussian>>,
    pub world: Transform,
    pub camera: Camera,
    pub basis: Option<[Vec3; 3]>,
    pub near: f32,
    pub far: f32,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Projected {
    center: [f32; 2],
    major: [f32; 2],
    minor: [f32; 2],
    color: [f32; 4],
}
fn project(layer: &Layer, width: u32, height: u32) -> Result<Vec<Projected>, String> {
    let camera = layer.camera;
    let forward = (camera.target - camera.position).normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let [forward, right, up] = layer
        .basis
        .unwrap_or([forward, right, right.cross(forward)]);
    let fx = height as f32 / (2. * (camera.fov.to_radians() / 2.).tan());
    let fy = fx;
    let m = layer.world.0;
    if !m.iter().all(|v| v.is_finite()) {
        return Err("Invalid splat transform".into());
    }
    let direction = |v: Vec3| {
        [
            v.dot(v3(m[0], m[1], m[2])),
            v.dot(v3(m[4], m[5], m[6])),
            v.dot(v3(m[8], m[9], m[10])),
        ]
    };
    let (rx, ry, rz) = (direction(right), direction(up), direction(-forward));
    let mut sorted = layer
        .data
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let p = layer.world.point(v3(g.center[0], g.center[1], g.center[2])) - camera.position;
            (i, p, forward.dot(p))
        })
        .collect::<Vec<_>>();
    if sorted
        .iter()
        .any(|(_, p, d)| ![p.x, p.y, p.z, *d].iter().all(|v| v.is_finite()))
    {
        return Err("Non-finite splat depth".into());
    }
    sorted.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    let mut out = Vec::with_capacity(sorted.len());
    for (i, p, d) in sorted {
        if d <= 0. || d < layer.near || d > layer.far {
            continue;
        }
        let (x, y) = (right.dot(p), up.dot(p));
        let center = [
            2. * fx * x / (d * width as f32),
            2. * fy * y / (d * height as f32),
        ];
        if center.iter().any(|v| v.abs() > 1.2) {
            continue;
        }
        let g = &layer.data[i];
        let c = g.covariance;
        let sigma = [[c[0], c[1], c[2]], [c[1], c[3], c[4]], [c[2], c[4], c[5]]];
        let tx: [f32; 3] = std::array::from_fn(|k| fx / d * rx[k] + fx * x / (d * d) * rz[k]);
        let ty: [f32; 3] = std::array::from_fn(|k| fy / d * ry[k] + fy * y / (d * d) * rz[k]);
        let cov = |a: [f32; 3], b: [f32; 3]| {
            (0..3)
                .map(|r| (0..3).map(|k| a[r] * sigma[r][k] * b[k]).sum::<f32>())
                .sum::<f32>()
        };
        let (xx, xy, yy) = (cov(tx, tx) + 0.3, cov(tx, ty), cov(ty, ty) + 0.3);
        let mid = 0.5 * (xx + yy);
        let radius = (0.5 * (xx - yy)).hypot(xy);
        let (l1, l2) = (mid + radius, (mid - radius).max(0.1));
        if ![xx, xy, yy, l1, l2].iter().all(|v| v.is_finite()) || l1 < 0. {
            return Err("Invalid projected splat covariance".into());
        }
        let axis = if xy.abs() < 1e-8 {
            if xx >= yy { [1., 0.] } else { [0., 1.] }
        } else {
            let length = xy.hypot(l1 - xx);
            [xy / length, (l1 - xx) / length]
        };
        let major = (2. * l1).sqrt().min(1024.);
        let minor = (2. * l2).sqrt().min(1024.);
        out.push(Projected {
            center,
            major: [
                major * axis[0] * 2. / width as f32,
                major * axis[1] * 2. / height as f32,
            ],
            minor: [
                minor * axis[1] * 2. / width as f32,
                -minor * axis[0] * 2. / height as f32,
            ],
            color: g.rgba.map(|v| v as f32 / 255.),
        });
    }
    Ok(out)
}
pub struct Rasterizer {
    pipeline: wgpu::RenderPipeline,
}
impl Rasterizer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm Gaussian splats"),
            source: wgpu::ShaderSource::Wgsl(include_str!("splats.wgsl").into()),
        });
        let attributes =
            wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Float32x2,3=>Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pm Gaussian splats"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Projected>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        layer: &Layer,
        target: &wgpu::Texture,
    ) -> Result<(), String> {
        let bytes = (layer.data.len() as u64)
            .checked_mul(size_of::<Projected>() as u64)
            .ok_or("Splat allocation overflow")?;
        if bytes > device.limits().max_buffer_size {
            return Err("Splat vertex data exceeds GPU buffer capacity".into());
        }
        let projected = project(layer, target.width(), target.height())?;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm sorted Gaussian splats"),
            size: (projected.len() * size_of::<Projected>()).max(4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !projected.is_empty() {
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&projected));
        }
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm Gaussian splat layer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if !projected.is_empty() {
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..4, 0..projected.len() as u32);
        }
        Ok(())
    }
}
