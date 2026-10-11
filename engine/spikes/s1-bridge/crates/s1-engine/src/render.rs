//! wgpu device, pipelines, and per-frame encoding into pool surfaces.

use crate::pool::Surface;
use s1_mach::wire::PixelFormat;

#[repr(C)]
#[derive(Clone, Copy)]
struct PatternUniforms {
    size: [f32; 2],
    time_s: f32,
    flags: u32,
    marker: [f32; 4],
    seq_lo: u32,
    rev_lo: u32,
    noise_off: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CameraUniforms {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 4],
}

fn bytes_of<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: T is a repr(C) plain-old-data struct with no padding-dependent reads.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()) }
}

pub const CUBES: u32 = 24 * 24 * 4;

/// What the frame shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneKind {
    Pattern,
    /// Orbit or walk camera over the cube field.
    Cubes { yaw: f32, pitch: f32, distance: f32, walk: f32 },
}

pub struct FrameParams {
    pub time_s: f32,
    pub seq: u64,
    pub revision: u64,
    pub marker: [f32; 4],
    pub scene: SceneKind,
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    format: PixelFormat,
    width: u32,
    height: u32,
    pattern: wgpu::RenderPipeline,
    pattern_overlay: wgpu::RenderPipeline,
    load: wgpu::RenderPipeline,
    cubes: wgpu::RenderPipeline,
    pattern_ubo: wgpu::Buffer,
    camera_ubo: wgpu::Buffer,
    pattern_bg: wgpu::BindGroup,
    camera_bg: wgpu::BindGroup,
    scratch: wgpu::TextureView,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    pub extra_passes: u32,
}

impl Gpu {
    pub fn new(width: u32, height: u32, format: PixelFormat, extra_passes: u32) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("request_adapter: {e}"))?;
        let adapter_info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("s1 engine"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() },
            ..Default::default()
        }))
        .map_err(|e| format!("request_device: {e}"))?;

        let target = match format {
            PixelFormat::Bgra8 => wgpu::TextureFormat::Bgra8Unorm,
            PixelFormat::Rgba16F => wgpu::TextureFormat::Rgba16Float,
        };
        let pattern_src = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pattern"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pattern.wgsl").into()),
        });
        let cubes_src = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cubes"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene3d.wgsl").into()),
        });

        let pattern_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pattern"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pattern_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pattern"),
            bind_group_layouts: &[Some(&pattern_bgl)],
            immediate_size: 0,
        });
        let camera_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("camera"),
            bind_group_layouts: &[Some(&camera_bgl)],
            immediate_size: 0,
        });

        let fullscreen = |label: &str, entry: &str, fmt: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pattern_layout),
                vertex: wgpu::VertexState {
                    module: &pattern_src,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &pattern_src,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(fmt.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pattern = fullscreen("pattern", "fs", target);
        let pattern_overlay = fullscreen("pattern overlay", "fs", target);
        let load = fullscreen("load", "fs_load", wgpu::TextureFormat::Rgba16Float);

        let cubes = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cubes"),
            layout: Some(&camera_layout),
            vertex: wgpu::VertexState {
                module: &cubes_src,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: 4, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &cubes_src,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(target.into())],
            }),
            multiview_mask: None,
            cache: None,
        });

        let pattern_ubo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pattern ubo"),
            size: std::mem::size_of::<PatternUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_ubo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera ubo"),
            size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Blue-noise thresholds, (rank + 0.5) / 4096 in [0, 1).
        let ranks = crate::noise::void_and_cluster(64);
        let texels: Vec<f32> = ranks.iter().map(|&r| (r as f32 + 0.5) / 4096.0).collect();
        let noise = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blue noise"),
            size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texel_bytes: Vec<u8> = texels.iter().flat_map(|v| v.to_ne_bytes()).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &noise, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &texel_bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64 * 4), rows_per_image: Some(64) },
            wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        );
        let noise_view = noise.create_view(&Default::default());
        let pattern_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pattern"),
            layout: &pattern_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: pattern_ubo.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&noise_view) },
            ],
        });
        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_ubo.as_entire_binding() }],
        });

        let tex = |label: &str, fmt: wgpu::TextureFormat, samples: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: fmt,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let scratch = tex("scratch", wgpu::TextureFormat::Rgba16Float, 1);
        let msaa = tex("msaa color", target, 4);
        let depth = tex("msaa depth", wgpu::TextureFormat::Depth32Float, 4);

        Ok(Self {
            device,
            queue,
            adapter_info,
            format,
            width,
            height,
            pattern,
            pattern_overlay,
            load,
            cubes,
            pattern_ubo,
            camera_ubo,
            pattern_bg,
            camera_bg,
            scratch,
            msaa,
            depth,
            extra_passes,
        })
    }

    /// Encodes and submits one frame into `surface`; returns the submission index.
    pub fn render(&self, surface: &Surface, p: &FrameParams) -> wgpu::SubmissionIndex {
        let eight_bit = self.format == PixelFormat::Bgra8;
        let mut flags = if eight_bit { 1 | 2 } else { 0 };
        if matches!(p.scene, SceneKind::Cubes { .. }) {
            flags |= 4;
        }
        let u = PatternUniforms {
            size: [self.width as f32, self.height as f32],
            time_s: p.time_s,
            flags,
            marker: p.marker,
            seq_lo: p.seq as u32,
            rev_lo: p.revision as u32,
            noise_off: [((p.seq * 17) % 64) as u32, ((p.seq * 29) % 64) as u32],
        };
        self.queue.write_buffer(&self.pattern_ubo, 0, bytes_of(&u));

        let mut enc = self.device.create_command_encoder(&Default::default());
        for _ in 0..self.extra_passes {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("load"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.scratch,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.load);
            pass.set_bind_group(0, &self.pattern_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        match p.scene {
            SceneKind::Pattern => {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pattern"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.pattern);
                pass.set_bind_group(0, &self.pattern_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            SceneKind::Cubes { yaw, pitch, distance, walk } => {
                let cam = CameraUniforms {
                    view_proj: camera_matrix(yaw, pitch, distance, walk, self.width as f32 / self.height as f32),
                    light_dir: [-0.4, -1.0, -0.3, 0.0],
                };
                self.queue.write_buffer(&self.camera_ubo, 0, bytes_of(&cam));
                {
                    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("cubes"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.msaa,
                            depth_slice: None,
                            resolve_target: Some(&surface.view),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.06, b: 0.08, a: 1.0 }),
                                store: wgpu::StoreOp::Discard,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.depth,
                            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_pipeline(&self.cubes);
                    pass.set_bind_group(0, &self.camera_bg, &[]);
                    pass.draw(0..36, 0..CUBES);
                }
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("overlay"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.pattern_overlay);
                pass.set_bind_group(0, &self.pattern_bg, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.queue.submit([enc.finish()])
    }

    /// Blocks until `index` has completed on the GPU.
    pub fn wait(&self, index: wgpu::SubmissionIndex) -> Result<(), String> {
        self.device
            .poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: Some(std::time::Duration::from_secs(2)) })
            .map(|_| ())
            .map_err(|e| format!("poll: {e}"))
    }
}

// --- camera math (column-major, wgpu clip space z in [0, 1]) ---------------------------

type Mat4 = [[f32; 4]; 4];

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [[0f32; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            r[c][row] = (0..4).map(|k| a[k][row] * b[c][k]).sum();
        }
    }
    r
}

fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fovy / 2.0).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Mat4 {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let norm = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let f = norm(sub(target, eye));
    let s = norm(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

/// Orbit around the field (yaw, pitch, distance); `walk` moves the orbit target
/// forward along the view direction (walk navigation).
pub fn camera_matrix(yaw: f32, pitch: f32, distance: f32, walk: f32, aspect: f32) -> Mat4 {
    let pitch = pitch.clamp(-1.4, 1.4);
    let dir = [pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()];
    let target = [-dir[0] * walk, 1.0, -dir[2] * walk];
    let eye = [target[0] + dir[0] * distance, target[1] + dir[1] * distance, target[2] + dir[2] * distance];
    mul(&perspective(0.9, aspect, 0.1, 200.0), &look_at(eye, target, [0.0, 1.0, 0.0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_maps_target_to_center() {
        let m = camera_matrix(0.3, 0.4, 20.0, 0.0, 16.0 / 9.0);
        let p = [0.0f32, 1.0, 0.0, 1.0];
        let clip: Vec<f32> = (0..4).map(|r| (0..4).map(|c| m[c][r] * p[c]).sum()).collect();
        assert!((clip[0] / clip[3]).abs() < 1e-5 && (clip[1] / clip[3]).abs() < 1e-5);
        let z = clip[2] / clip[3];
        assert!(z > 0.0 && z < 1.0);
    }
}
