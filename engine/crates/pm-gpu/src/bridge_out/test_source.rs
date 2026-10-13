//! The bridge test source: what the frame bridge presents until pm-render
//! draws real frames. It is the S1 test pattern (frame code, ramps, HDR
//! patches, moving bar, marker) and, under an editor camera, S1's field of
//! 2,304 lit cubes with MSAA and depth, so the bridge's tests and the S1
//! harness measure the production path end to end.
//!
//! The engine uses it only when `POWERMOVE_ENGINE_FRAME_SOURCE=test-pattern`
//! is set (tests, the harness); otherwise no viewer frames are produced.

use pm_protocol::common::{Mat4, Size};
use pm_protocol::frame::{EditorCamera, Projection};

use super::display::{WORKING_FORMAT, WorkingTexture};

/// Instances in the cube field (24 × 24 × 4).
pub const CUBES: u32 = 24 * 24 * 4;

/// One test frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TestFrame {
    /// Drives the moving bar.
    pub time_s: f32,
    /// Written into the frame code's first 32 cells (low bits).
    pub seq: u64,
    /// Written into the frame code's last 32 cells (low bits).
    pub view_seq: u64,
    /// The composition rectangle in frame pixels (zoom and pan applied).
    pub composition: [f32; 4],
    /// The marker (overlay geometry) in frame pixels.
    pub marker: [f32; 4],
    /// The cube field's view-projection (column-major), drawn under the
    /// frame code and marker; `None` draws the 2D pattern.
    pub camera: Option<[[f32; 4]; 4]>,
}

impl TestFrame {
    /// Width of one frame-code cell for a frame `width` pixels wide.
    pub fn code_cell(width: u32) -> u32 {
        (width / 64).clamp(1, 32)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PatternUniforms {
    size: [f32; 2],
    time_s: f32,
    flags: u32,
    marker: [f32; 4],
    composition: [f32; 4],
    seq_lo: u32,
    view_lo: u32,
    cell: f32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CameraUniforms {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 4],
}

fn bytes_of<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: T is a repr(C) plain-old-data struct without padding reads.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

pub struct TestSource {
    size: Size,
    pattern: wgpu::RenderPipeline,
    overlay: wgpu::RenderPipeline,
    load: wgpu::RenderPipeline,
    cubes: wgpu::RenderPipeline,
    pattern_uniforms: wgpu::Buffer,
    camera_uniforms: wgpu::Buffer,
    pattern_group: wgpu::BindGroup,
    camera_group: wgpu::BindGroup,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    scratch: wgpu::TextureView,
    /// Extra full-screen ALU passes per frame (the S1 harness's synthetic
    /// engine load; 0 in tests).
    pub extra_passes: u32,
}

impl TestSource {
    pub fn new(device: &wgpu::Device, size: Size, extra_passes: u32) -> Self {
        let pattern_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bridge test pattern"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pattern.wgsl").into()),
        });
        let cubes_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bridge test cubes"),
            source: wgpu::ShaderSource::Wgsl(include_str!("cubes.wgsl").into()),
        });
        let uniform_entry = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let pattern_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bridge test pattern"),
            entries: &[uniform_entry(wgpu::ShaderStages::FRAGMENT)],
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bridge test camera"),
            entries: &[uniform_entry(wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let pattern_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bridge test pattern"),
                bind_group_layouts: &[Some(&pattern_layout)],
                immediate_size: 0,
            });
        let camera_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bridge test camera"),
                bind_group_layouts: &[Some(&camera_layout)],
                immediate_size: 0,
            });
        let fullscreen = |label: &str, entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pattern_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &pattern_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &pattern_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(WORKING_FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pattern = fullscreen("bridge test pattern", "fs");
        let overlay = fullscreen("bridge test overlay", "fs");
        let load = fullscreen("bridge test load", "fs_load");
        let cubes = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bridge test cubes"),
            layout: Some(&camera_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &cubes_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 4,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &cubes_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(WORKING_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let pattern_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bridge test pattern"),
            size: size_of::<PatternUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bridge test camera"),
            size: size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pattern_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bridge test pattern"),
            layout: &pattern_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: pattern_uniforms.as_entire_binding(),
            }],
        });
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bridge test camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniforms.as_entire_binding(),
            }],
        });
        let target = |label: &str, format, samples| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.width,
                        height: size.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        Self {
            size,
            msaa: target("bridge test msaa", WORKING_FORMAT, 4),
            depth: target("bridge test depth", wgpu::TextureFormat::Depth32Float, 4),
            scratch: target("bridge test load scratch", WORKING_FORMAT, 1),
            pattern,
            overlay,
            load,
            cubes,
            pattern_uniforms,
            camera_uniforms,
            pattern_group,
            camera_group,
            extra_passes,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Records one frame into `target` (the working texture of the same size).
    pub fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &WorkingTexture,
        frame: &TestFrame,
    ) {
        debug_assert_eq!(target.size, self.size);
        let uniforms = PatternUniforms {
            size: [self.size.width as f32, self.size.height as f32],
            time_s: frame.time_s,
            flags: u32::from(frame.camera.is_some()),
            marker: frame.marker,
            composition: frame.composition,
            seq_lo: frame.seq as u32,
            view_lo: frame.view_seq as u32,
            cell: TestFrame::code_cell(self.size.width) as f32,
            _pad: 0,
        };
        queue.write_buffer(&self.pattern_uniforms, 0, bytes_of(&uniforms));
        for _ in 0..self.extra_passes {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bridge test load"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.scratch,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.load);
            pass.set_bind_group(0, &self.pattern_group, &[]);
            pass.draw(0..3, 0..1);
        }
        match frame.camera {
            None => {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bridge test pattern"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.pattern);
                pass.set_bind_group(0, &self.pattern_group, &[]);
                pass.draw(0..3, 0..1);
            }
            Some(view_proj) => {
                let camera = CameraUniforms {
                    view_proj,
                    light_dir: [-0.4, -1.0, -0.3, 0.0],
                };
                queue.write_buffer(&self.camera_uniforms, 0, bytes_of(&camera));
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("bridge test cubes"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.msaa,
                            depth_slice: None,
                            resolve_target: Some(&target.view),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.05,
                                    g: 0.06,
                                    b: 0.08,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Discard,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.depth,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_pipeline(&self.cubes);
                    pass.set_bind_group(0, &self.camera_group, &[]);
                    pass.draw(0..36, 0..CUBES);
                }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bridge test overlay"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.overlay);
                pass.set_bind_group(0, &self.pattern_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
    }
}

// --- Camera math (column-major; wgpu clip space, z in 0..1) -----------------

type M4 = [[f64; 4]; 4];

fn columns(m: &Mat4) -> M4 {
    let mut out = [[0.0; 4]; 4];
    for (c, column) in out.iter_mut().enumerate() {
        column.copy_from_slice(&m[c * 4..c * 4 + 4]);
    }
    out
}

fn mul(a: &M4, b: &M4) -> M4 {
    let mut r = [[0.0; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            r[c][row] = (0..4).map(|k| a[k][row] * b[c][k]).sum();
        }
    }
    r
}

/// The projection matrix of an editor camera at `aspect` (width / height).
pub fn projection_matrix(projection: &Projection, aspect: f64) -> M4 {
    match *projection {
        Projection::Perspective {
            fov_y_degrees,
            near,
            far,
        } => {
            let f = 1.0 / (fov_y_degrees.to_radians() / 2.0).tan();
            [
                [f / aspect, 0.0, 0.0, 0.0],
                [0.0, f, 0.0, 0.0],
                [0.0, 0.0, far / (near - far), -1.0],
                [0.0, 0.0, near * far / (near - far), 0.0],
            ]
        }
        Projection::Orthographic {
            half_height,
            near,
            far,
        } => {
            let half_width = half_height * aspect;
            [
                [1.0 / half_width, 0.0, 0.0, 0.0],
                [0.0, 1.0 / half_height, 0.0, 0.0],
                [0.0, 0.0, 1.0 / (near - far), 0.0],
                [0.0, 0.0, near / (near - far), 1.0],
            ]
        }
    }
}

/// The view-projection the cube field draws with, as f32 columns.
pub fn view_projection(camera: &EditorCamera, aspect: f64) -> [[f32; 4]; 4] {
    let m = mul(
        &projection_matrix(&camera.projection, aspect),
        &columns(&camera.view),
    );
    m.map(|column| column.map(|v| v as f32))
}

/// A world-to-camera matrix orbiting the cube field: yaw and pitch in
/// radians, distance from the target, and `walk` moving the target along the
/// view direction (walk navigation). Column-major.
pub fn orbit_view(yaw: f64, pitch: f64, distance: f64, walk: f64) -> Mat4 {
    let pitch = pitch.clamp(-1.4, 1.4);
    let dir = [
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        pitch.cos() * yaw.cos(),
    ];
    let target = [-dir[0] * walk, 1.0, -dir[2] * walk];
    let eye = [
        target[0] + dir[0] * distance,
        target[1] + dir[1] * distance,
        target[2] + dir[2] * distance,
    ];
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let norm = |v: [f64; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let f = norm(sub(target, eye));
    let s = norm(cross(f, [0.0, 1.0, 0.0]));
    let u = cross(s, f);
    [
        s[0],
        u[0],
        -f[0],
        0.0,
        s[1],
        u[1],
        -f[1],
        0.0,
        s[2],
        u[2],
        -f[2],
        0.0,
        -dot(s, eye),
        -dot(u, eye),
        dot(f, eye),
        1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_orbit_target_projects_to_the_center() {
        let camera = EditorCamera {
            view: orbit_view(0.3, 0.4, 20.0, 0.0),
            projection: Projection::Perspective {
                fov_y_degrees: 51.566,
                near: 0.1,
                far: 200.0,
            },
        };
        let m = view_projection(&camera, 16.0 / 9.0);
        let p = [0.0f32, 1.0, 0.0, 1.0];
        let clip: Vec<f32> = (0..4)
            .map(|r| (0..4).map(|c| m[c][r] * p[c]).sum())
            .collect();
        assert!((clip[0] / clip[3]).abs() < 1e-5 && (clip[1] / clip[3]).abs() < 1e-5);
        let z = clip[2] / clip[3];
        assert!(z > 0.0 && z < 1.0);
    }

    #[test]
    fn orthographic_maps_the_half_height_to_the_edge() {
        let m = projection_matrix(
            &Projection::Orthographic {
                half_height: 3.0,
                near: 0.1,
                far: 100.0,
            },
            2.0,
        );
        // (6, 3, -0.1) in view space lands on the top-right edge at depth 0.
        let p = [6.0, 3.0, -0.1, 1.0];
        let clip: Vec<f64> = (0..4)
            .map(|r| (0..4).map(|c| m[c][r] * p[c]).sum())
            .collect();
        assert!((clip[0] - 1.0).abs() < 1e-12 && (clip[1] - 1.0).abs() < 1e-12);
        assert!(clip[2].abs() < 1e-12);
    }

    #[test]
    fn code_cells_fit_the_frame() {
        assert_eq!(TestFrame::code_cell(3840), 32);
        assert_eq!(TestFrame::code_cell(1600), 25);
        assert_eq!(TestFrame::code_cell(32), 1);
    }
}
