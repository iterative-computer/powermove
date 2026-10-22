//! Native Graphite primitive execution. Analytic shader bodies are emitted
//! by the pinned Skia compiler; rasterization, interpolation, per-sample N32
//! blending and resolve execute on Metal instead of a CPU approximation.
pub(super) mod atlas;
use crate::pipelines::bytes_of;
use pm_gpu::{Target, TargetPool};
use pm_text::render::graphite::{Geometry, Raster};

const INDICES: [u16; 69] = [
    0, 4, 1, 5, 2, 3, 5, 9, 13, 10, 14, 11, 12, 14, 18, 22, 19, 23, 20, 21, 23, 27, 31, 28, 32, 29,
    30, 32, 0, 4, 4, 6, 5, 7, 13, 15, 14, 16, 22, 24, 23, 25, 31, 33, 32, 34, 4, 6, 6, 8, 7, 7, 17,
    15, 17, 16, 16, 26, 24, 26, 25, 25, 35, 33, 35, 34, 34, 8, 6,
];

struct Pipelines {
    analytic: wgpu::RenderPipeline,
    triangles: wgpu::RenderPipeline,
    winding: [wgpu::RenderPipeline; 2],
    cover: wgpu::RenderPipeline,
    fill: [wgpu::RenderPipeline; 3],
    stroke: wgpu::RenderPipeline,
    sdf: wgpu::RenderPipeline,
}

pub struct Gpu {
    layout: wgpu::BindGroupLayout,
    indices: wgpu::Buffer,
    fill_vertices: wgpu::Buffer,
    fill_indices: wgpu::Buffer,
    pipelines: [Pipelines; 2],
    dummy: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

fn buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let b = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Graphite primitive data"),
        size: bytes.len().max(8).next_multiple_of(4) as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !bytes.is_empty() {
        if bytes.len().is_multiple_of(4) {
            queue.write_buffer(&b, 0, bytes);
        } else {
            let mut aligned = bytes.to_vec();
            aligned.resize(bytes.len().next_multiple_of(4), 0);
            queue.write_buffer(&b, 0, &aligned);
        }
    }
    b
}

fn vertices(device: &wgpu::Device, queue: &wgpu::Queue, points: &[[f32; 2]]) -> wgpu::Buffer {
    let bytes: Vec<u8> = points
        .iter()
        .flatten()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    buffer(device, queue, &bytes, wgpu::BufferUsages::VERTEX)
}

fn matrix_uniform(u: &mut [[f32; 4]], m: [f32; 6]) {
    u[1] = [m[0], m[1], 0., 0.];
    u[2] = [m[2], m[3], 0., 0.];
    u[3] = [0., 0., 1., 0.];
    u[4] = [m[4], m[5], 0., 1.];
}

/// FixedCountWedges' static middle-out vertices and index order.
fn wedge_buffers() -> (Vec<[f32; 2]>, Vec<u16>) {
    let mut points = vec![[-1., -1.], [0., 0.], [0., 1.]];
    for level in 1..=5 {
        for i in (1..(1 << level)).step_by(2) {
            points.push([level as f32, i as f32]);
        }
    }
    let mut indices = vec![0, 1, 2];
    let mut previous = vec![[1u16, 3, 2]];
    indices.extend(previous[0]);
    let mut next = 4u16;
    for _ in 2..=5 {
        let mut current = Vec::new();
        for t in previous {
            current.push([t[0], next, t[1]]);
            next += 1;
            current.push([t[1], next, t[2]]);
            next += 1;
        }
        indices.extend(current.iter().flatten());
        previous = current;
    }
    (points, indices)
}

fn instance_bytes(geometry: &Geometry) -> Vec<u8> {
    fn floats(out: &mut Vec<u8>, v: &[f32]) {
        out.extend(v.iter().flat_map(|f| f.to_ne_bytes()));
    }
    let mut out = Vec::new();
    match geometry {
        Geometry::Fill { patches, .. } => {
            for p in patches {
                floats(&mut out, &p[0]);
                floats(&mut out, &p[1]);
                floats(&mut out, &p[4][..2]);
                floats(&mut out, &[0.5]);
                out.extend(0u32.to_ne_bytes());
                floats(&mut out, &[p[3][3]]);
            }
        }
        Geometry::Stroke { patches, .. } => {
            for p in patches {
                floats(&mut out, &p[0]);
                floats(&mut out, &p[1]);
                floats(&mut out, &p[4]);
                floats(&mut out, &[0.5]);
                out.extend(0u32.to_ne_bytes());
                floats(&mut out, &[p[3][3]]);
            }
        }
        Geometry::Sdf {
            field, left_top, ..
        } => {
            for v in [field.width - 4, field.height - 4, 2, 2] {
                out.extend((v as u16).to_ne_bytes());
            }
            floats(&mut out, &[left_top.0, left_top.1]);
            out.extend([0u8; 4]);
            floats(&mut out, &[1., 0.5]);
            out.extend(0u32.to_ne_bytes());
        }
        _ => {}
    }
    out
}

/// The Metal bodies are emitted by Chromium 152's pinned Dawn/Tint from
/// the source WGSL, with Dawn's default relaxed math mode. Other backends
/// consume that same WGSL through wgpu.
fn shader(
    device: &wgpu::Device,
    label: &str,
    wgsl: &'static str,
    msl: &'static str,
    entry: &str,
) -> pm_gpu::msl::MslStage {
    if pm_gpu::msl::supported(device) {
        return pm_gpu::msl::from_source(device, label, msl, "tint_entry_point")
            .expect("pinned Dawn Metal shader");
    }
    pm_gpu::msl::MslStage {
        module: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        }),
        entry_point: entry.into(),
        source: wgsl.into(),
    }
}

impl Gpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let av = shader(
            device,
            "Skia 152 analytic vertex",
            include_str!("graphite/analytic-vs.wgsl"),
            include_str!("graphite/analytic-vs.metal"),
            "main",
        );
        let af = shader(
            device,
            "Skia 152 analytic coverage",
            include_str!("graphite/analytic-fs.wgsl"),
            include_str!("graphite/analytic-fs.metal"),
            "main",
        );
        let pv = shader(
            device,
            "Graphite primitive vertex",
            include_str!("graphite/primitive.wgsl"),
            include_str!("graphite/primitive-vs.metal"),
            "primitive_vs",
        );
        let pf = shader(
            device,
            "Graphite primitive fragment",
            include_str!("graphite/primitive.wgsl"),
            include_str!("graphite/primitive-fs.metal"),
            "primitive_fs",
        );
        let fv = shader(
            device,
            "Skia 152 fill vertex",
            include_str!("graphite/fill-vs.wgsl"),
            include_str!("graphite/fill-vs.metal"),
            "main",
        );
        let sv = shader(
            device,
            "Skia 152 stroke vertex",
            include_str!("graphite/stroke-vs.wgsl"),
            include_str!("graphite/stroke-vs.metal"),
            "main",
        );
        let tv = shader(
            device,
            "Skia 152 SDF vertex",
            include_str!("graphite/sdf-vs.wgsl"),
            include_str!("graphite/sdf-vs.metal"),
            "main",
        );
        let tf = shader(
            device,
            "Skia 152 SDF coverage",
            include_str!("graphite/sdf-fs.wgsl"),
            include_str!("graphite/sdf-fs.metal"),
            "main",
        );
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(192),
            },
            count: None,
        }];
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(160),
            },
            count: None,
        });
        for n in 0..4 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 + 2 * n,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 3 + 2 * n,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Graphite source inputs"),
            entries: &entries,
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Graphite primitive pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |samples, kind: u8, mode: u8| {
            let analytic = kind == 1;
            let (vs, entry) = match kind {
                1 => (&av.module, av.entry_point.as_str()),
                2 => (&fv.module, fv.entry_point.as_str()),
                3 => (&sv.module, sv.entry_point.as_str()),
                4 => (&tv.module, tv.entry_point.as_str()),
                _ => (&pv.module, pv.entry_point.as_str()),
            };
            let face = |op| wgpu::StencilFaceState {
                compare: wgpu::CompareFunction::Always,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op: op,
            };
            let stencil = match mode {
                1 => wgpu::StencilState {
                    front: face(wgpu::StencilOperation::IncrementWrap),
                    back: face(wgpu::StencilOperation::DecrementWrap),
                    read_mask: 255,
                    write_mask: 255,
                },
                2 => wgpu::StencilState {
                    front: face(wgpu::StencilOperation::Invert),
                    back: face(wgpu::StencilOperation::Invert),
                    read_mask: 255,
                    write_mask: 255,
                },
                3 => {
                    let f = wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::NotEqual,
                        fail_op: wgpu::StencilOperation::Zero,
                        depth_fail_op: wgpu::StencilOperation::Zero,
                        pass_op: wgpu::StencilOperation::Zero,
                    };
                    wgpu::StencilState {
                        front: f,
                        back: f,
                        read_mask: 255,
                        write_mask: 255,
                    }
                }
                _ => wgpu::StencilState::default(),
            };
            use wgpu::VertexFormat::{Float32, Float32x2, Float32x4, Uint16x2, Uint32};
            let attr = |shader_location, offset, format| wgpu::VertexAttribute {
                shader_location,
                offset,
                format,
            };
            let attrs = [attr(0, 0, Float32x2)];
            let fill_attrs = [
                attr(1, 0, Float32x4),
                attr(2, 16, Float32x4),
                attr(3, 32, Float32x2),
                attr(4, 40, Float32),
                attr(5, 44, Uint32),
                attr(6, 48, Float32),
            ];
            let stroke_attrs = [
                attr(0, 0, Float32x4),
                attr(1, 16, Float32x4),
                attr(2, 32, Float32x2),
                attr(3, 40, Float32x2),
                attr(4, 48, Float32),
                attr(5, 52, Uint32),
                attr(6, 56, Float32),
            ];
            let sdf_attrs = [
                attr(0, 0, Uint16x2),
                attr(1, 4, Uint16x2),
                attr(2, 8, Float32x2),
                attr(3, 16, Uint16x2),
                attr(4, 20, Float32),
                attr(5, 24, Float32),
                attr(6, 28, Uint32),
            ];
            let vl = [Some(wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attrs,
            })];
            let fill_vl = [
                Some(wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: 52,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &fill_attrs,
                }),
            ];
            let stroke_vl = [Some(wgpu::VertexBufferLayout {
                array_stride: 60,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &stroke_attrs,
            })];
            let sdf_vl = [Some(wgpu::VertexBufferLayout {
                array_stride: 32,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &sdf_attrs,
            })];
            let buffers = match kind {
                1 => &[][..],
                2 => &fill_vl[..],
                3 => &stroke_vl[..],
                4 => &sdf_vl[..],
                _ => &vl[..],
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Graphite native primitive"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: vs,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: if analytic || kind == 3 || kind == 4 || mode == 3 {
                        wgpu::PrimitiveTopology::TriangleStrip
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    strip_index_format: if analytic {
                        Some(wgpu::IndexFormat::Uint16)
                    } else {
                        None
                    },
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24PlusStencil8,
                    depth_write_enabled: Some(mode == 0),
                    depth_compare: Some(if mode == 0 && kind == 4 {
                        wgpu::CompareFunction::LessEqual
                    } else if mode == 0 {
                        wgpu::CompareFunction::Less
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil,
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: if analytic {
                        &af.module
                    } else if kind == 4 {
                        &tf.module
                    } else {
                        &pf.module
                    },
                    entry_point: Some(if analytic {
                        &af.entry_point
                    } else if kind == 4 {
                        &tf.entry_point
                    } else {
                        &pf.entry_point
                    }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: if mode == 1 || mode == 2 {
                            wgpu::ColorWrites::empty()
                        } else {
                            wgpu::ColorWrites::ALL
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = [1, 4].map(|n| Pipelines {
            analytic: make(n, 1, 0),
            triangles: make(n, 0, 0),
            winding: [make(n, 0, 1), make(n, 0, 2)],
            cover: make(n, 0, 3),
            fill: [make(n, 2, 0), make(n, 2, 1), make(n, 2, 2)],
            stroke: make(n, 3, 0),
            sdf: make(n, 4, 0),
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Graphite unused atlas"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Graphite bilinear atlas"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let (fill_points, fill_indices) = wedge_buffers();
        Self {
            fill_vertices: vertices(device, queue, &fill_points),
            fill_indices: buffer(
                device,
                queue,
                &fill_indices
                    .iter()
                    .flat_map(|v| v.to_ne_bytes())
                    .collect::<Vec<_>>(),
                wgpu::BufferUsages::INDEX,
            ),
            layout,
            dummy,
            sampler,
            indices: buffer(device, queue, bytes_of(&INDICES), wgpu::BufferUsages::INDEX),
            pipelines,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        taken: &mut Vec<Target>,
        raster: &Raster,
        atlas: &mut atlas::Atlas,
    ) -> (wgpu::TextureView, [f32; 2]) {
        atlas.begin_run();
        let (w, h) = (raster.width, raster.height);
        let samples = if raster.multisampled { 4 } else { 1 };
        let p = &self.pipelines[usize::from(raster.multisampled)];
        let resolved = pool.take(
            device,
            w,
            h,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let attachment = |format, label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        };
        let msaa = raster
            .multisampled
            .then(|| attachment(wgpu::TextureFormat::Bgra8Unorm, "Graphite MSAA"));
        let msaa_view = msaa.as_ref().map(|t| t.create_view(&Default::default()));
        let depth = attachment(
            wgpu::TextureFormat::Depth24PlusStencil8,
            "Graphite primitive depth/stencil",
        );
        let depth_view = depth.create_view(&Default::default());
        let cover = vertices(
            device,
            queue,
            &[
                [0., 0.],
                [0., h as f32],
                [w as f32, 0.],
                [w as f32, h as f32],
            ],
        );
        for (i, draw) in raster.draws.iter().enumerate() {
            let mut uniforms = [[0f32; 4]; 12];
            uniforms[0] = [w as f32, h as f32, 2. / w as f32, -2. / h as f32];
            uniforms[8] = draw.color;
            if let Geometry::Analytic(a) = &draw.geometry {
                uniforms[1..8].copy_from_slice(a);
            }
            if let Geometry::Fill { patches, .. } = &draw.geometry
                && let Some(p) = patches.first()
            {
                matrix_uniform(
                    &mut uniforms,
                    [p[2][0], p[2][1], p[2][2], p[2][3], p[3][0], p[3][1]],
                );
            }
            if let Geometry::Stroke { patches, .. } = &draw.geometry
                && let Some(p) = patches.first()
            {
                uniforms[1] = p[2];
                uniforms[2] = p[3];
            }
            let sdf_location = if let Geometry::Sdf {
                field,
                matrix,
                font_size,
                ..
            } = &draw.geometry
            {
                matrix_uniform(&mut uniforms, *matrix);
                uniforms[5] = [1., 0., 0., 0.];
                uniforms[6] = [0., 1., 0., 0.];
                uniforms[7] = [0., 0., 1., 0.];
                uniforms[8] = [0., 0., 0., 1.];
                let inverse = atlas::Atlas::inverse_size();
                uniforms[9] = [inverse, inverse, 0., 0.];
                uniforms[11] = draw.color;
                Some(atlas.locate(device, queue, field, *font_size))
            } else {
                None
            };
            let uniform = buffer(
                device,
                queue,
                bytes_of(&uniforms),
                wgpu::BufferUsages::UNIFORM,
            );
            let mut storage_data = [[0.; 4]; 10];
            storage_data.copy_from_slice(&uniforms[..10]);
            storage_data[0] = draw.color;
            let storage = buffer(
                device,
                queue,
                bytes_of(&storage_data),
                wgpu::BufferUsages::STORAGE,
            );
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: storage.as_entire_binding(),
                },
            ];
            for n in 0..4 {
                entries.push(wgpu::BindGroupEntry {
                    binding: 2 + 2 * n,
                    resource: wgpu::BindingResource::TextureView(if sdf_location.is_some() {
                        atlas.view(n as usize)
                    } else {
                        &self.dummy
                    }),
                });
                entries.push(wgpu::BindGroupEntry {
                    binding: 3 + 2 * n,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                });
            }
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &entries,
            });
            let mesh = match &draw.geometry {
                Geometry::Triangles(v) | Geometry::Winding { vertices: v, .. } => {
                    Some(vertices(device, queue, v))
                }
                Geometry::Fill { .. } | Geometry::Stroke { .. } | Geometry::Sdf { .. } => {
                    let mut data = instance_bytes(&draw.geometry);
                    if let Some((pos, page)) = sdf_location {
                        data[4..6].copy_from_slice(&((pos[0] + 2) as u16).to_ne_bytes());
                        data[6..8].copy_from_slice(&((pos[1] + 2) as u16).to_ne_bytes());
                        data[16..18].copy_from_slice(&(page as u16).to_ne_bytes());
                    }
                    Some(buffer(
                        device,
                        queue,
                        if data.is_empty() { &[0u8; 80] } else { &data },
                        wgpu::BufferUsages::VERTEX,
                    ))
                }
                _ => None,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Graphite N32 primitive"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa_view.as_ref().unwrap_or(&resolved.view),
                    depth_slice: None,
                    resolve_target: raster.multisampled.then_some(&resolved.view),
                    ops: wgpu::Operations {
                        load: if i == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &bind, &[]);
            match &draw.geometry {
                Geometry::Analytic(_) => {
                    pass.set_pipeline(&p.analytic);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
                }
                Geometry::Triangles(v) => {
                    pass.set_pipeline(&p.triangles);
                    pass.set_vertex_buffer(0, mesh.as_ref().unwrap().slice(..));
                    pass.draw(0..v.len() as u32, 0..1);
                }
                Geometry::Sdf { .. } => {
                    pass.set_pipeline(&p.sdf);
                    pass.set_vertex_buffer(0, mesh.as_ref().unwrap().slice(..));
                    pass.draw(0..4, 0..1);
                }
                Geometry::Fill {
                    patches,
                    vertex_count,
                    winding,
                    even_odd,
                } => {
                    let mode = if *winding {
                        1 + usize::from(*even_odd)
                    } else {
                        0
                    };
                    pass.set_pipeline(&p.fill[mode]);
                    pass.set_stencil_reference(0);
                    pass.set_vertex_buffer(0, self.fill_vertices.slice(..));
                    pass.set_vertex_buffer(1, mesh.as_ref().unwrap().slice(..));
                    pass.set_index_buffer(self.fill_indices.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..*vertex_count, 0, 0..patches.len() as u32);
                    if *winding {
                        pass.set_pipeline(&p.cover);
                        pass.set_vertex_buffer(0, cover.slice(..));
                        pass.draw(0..4, 0..1);
                    }
                }
                Geometry::Stroke {
                    patches,
                    vertex_count,
                } => {
                    pass.set_pipeline(&p.stroke);
                    pass.set_vertex_buffer(0, mesh.as_ref().unwrap().slice(..));
                    pass.draw(0..*vertex_count, 0..patches.len() as u32);
                }
                Geometry::Winding {
                    vertices: v,
                    even_odd,
                } => {
                    pass.set_pipeline(&p.winding[usize::from(*even_odd)]);
                    pass.set_stencil_reference(0);
                    pass.set_vertex_buffer(0, mesh.as_ref().unwrap().slice(..));
                    pass.draw(0..v.len() as u32, 0..1);
                    pass.set_pipeline(&p.cover);
                    pass.set_vertex_buffer(0, cover.slice(..));
                    pass.draw(0..4, 0..1);
                }
            }
        }
        if raster.draws.is_empty() {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Graphite empty primitive image"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &resolved.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        let [x0, y0, x1, y1] = raster.crop;
        let (cw, ch) = (x1 - x0, y1 - y0);
        let image = pool.take(
            device,
            cw + 2,
            ch + 2,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        // A pooled crop can have been used for a different draw. Its
        // transparent filtering border must be cleared as well as its ink.
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Graphite crop border"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &image.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &resolved.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: x0, y: y0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &image.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 1, y: 1, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: cw,
                height: ch,
                depth_or_array_layers: 1,
            },
        );
        let result = (image.view.clone(), [(cw + 2) as f32, (ch + 2) as f32]);
        taken.extend([resolved, image]);
        result
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pinned_graphite_wgsl_validates() {
        use pm_gpu::glsl::naga;
        for source in [
            include_str!("graphite/analytic-vs.wgsl"),
            include_str!("graphite/analytic-fs.wgsl"),
            include_str!("graphite/fill-vs.wgsl"),
            include_str!("graphite/stroke-vs.wgsl"),
            include_str!("graphite/sdf-vs.wgsl"),
            include_str!("graphite/sdf-fs.wgsl"),
            include_str!("graphite/primitive.wgsl"),
        ] {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("pinned render-step WGSL");
        }
    }
}
