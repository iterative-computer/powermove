//! The compositing pipelines (`ops.wgsl`) and their uniform.

use crate::pipelines::bytes_of;

/// `Op` in `ops.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OpUniform {
    pub m: [[f32; 4]; 3],
    pub res: [f32; 4],
    pub off: [i32; 4],
    pub f: [f32; 4],
    pub i: [i32; 4],
    pub inv: [[f32; 4]; 3],
    pub g: [[f32; 4]; 8],
    pub q: [[f32; 4]; 8],
}

pub(crate) const FLAG_MASKS: i32 = 1;
pub(crate) const FLAG_MATTE: i32 = 2;
pub(crate) const FLAG_MATTE_LUMA: i32 = 4;
pub(crate) const FLAG_MATTE_INVERTED: i32 = 8;
pub(crate) const FLAG_LINEAR: i32 = 16;
pub(crate) const FLAG_BLEND_IN_E: i32 = 32;
pub(crate) const FLAG_HALF: i32 = 64;

/// Which fragment entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// Masks and the track matte on a layer.
    Layer,
    /// `FRAG_COMPOSITE`.
    Composite,
    /// A rectangle copy.
    Copy,
    /// The final row flip, encoding linear light with E.
    EncodeFlip,
}

const FORMATS: [wgpu::TextureFormat; 2] = [
    wgpu::TextureFormat::Rgba16Float,
    wgpu::TextureFormat::Rgba32Float,
];

pub(crate) struct OpPipelines {
    pub layout: wgpu::BindGroupLayout,
    pipelines: Vec<(Op, wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl OpPipelines {
    pub fn new(device: &wgpu::Device) -> OpPipelines {
        let code = format!(
            "{}\n{}\n{}",
            pm_color::wgsl::COLOR,
            pm_color::wgsl::BLEND,
            include_str!("ops.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render composite ops"),
            source: wgpu::ShaderSource::Wgsl(code.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm composite op"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<OpUniform>() as u64),
                    },
                    count: None,
                },
                texture(1),
                texture(2),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm composite op"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mut pipelines = Vec::new();
        for op in [Op::Layer, Op::Composite, Op::Copy, Op::EncodeFlip] {
            let entry = match op {
                Op::Layer => "fs_layer",
                Op::Composite => "fs_composite",
                Op::Copy => "fs_copy",
                Op::EncodeFlip => "fs_encode_flip",
            };
            for format in FORMATS {
                let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                pipelines.push((op, format, p));
            }
        }
        OpPipelines { layout, pipelines }
    }

    pub fn get(&self, op: Op, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        &self
            .pipelines
            .iter()
            .find(|(o, f, _)| *o == op && *f == format)
            .expect("a working format")
            .2
    }

    /// Records one full-screen pass of `op` into `target`.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        op: Op,
        uniform: &OpUniform,
        a: &wgpu::TextureView,
        b: Option<&wgpu::TextureView>,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        viewport: Option<[f32; 4]>,
    ) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm composite op"),
            size: size_of::<OpUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytes_of(uniform));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm composite op"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(a),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(b.unwrap_or(a)),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm composite op"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
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
        if let Some([x, y, w, h]) = viewport {
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
        }
        pass.set_pipeline(self.get(op, format));
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..4, 0..1);
    }
}
