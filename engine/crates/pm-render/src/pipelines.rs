//! Compiled pipelines and the uniform layouts they read.

use pm_protocol::color::Precision;

/// Dynamic uniform offsets are aligned to this (Metal's 256-byte rule).
pub const UNIFORM_STRIDE: u64 = 256;

/// `Quad` in `composite.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct QuadUniform {
    pub m: [[f32; 4]; 3],
    pub res: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
}

/// `Fill` in `composite.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FillUniform {
    pub stops: [[f32; 4]; 8],
    pub params: [f32; 4],
    pub view: [f32; 4],
    /// x: 1 when the working encoding is linear light.
    pub flags: [f32; 4],
}

/// `Params` in `source.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceUniform {
    pub a: [u32; 4],
    pub b: [u32; 4],
    pub c: [u32; 4],
    pub origin: [f32; 4],
    pub m: [[f32; 4]; 3],
    pub p: [[f32; 4]; 3],
    pub matte: [f32; 4],
    /// HDR input tone mapping: x is the content peak in cd/m² (L_HDR).
    pub hdr: [f32; 4],
}

/// The bytes of a plain `repr(C)` value.
pub fn bytes_of<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: only used with the repr(C) uniform structs above, which hold
    // f32 and u32 fields without padding.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Every 2D pipeline, for both working formats.
pub struct Pipelines {
    pub quad_layout: wgpu::BindGroupLayout,
    pub texture_layout: wgpu::BindGroupLayout,
    pub fill_layout: wgpu::BindGroupLayout,
    pub source_layout: wgpu::BindGroupLayout,
    pub depth_layout: wgpu::BindGroupLayout,
    pub linear: wgpu::Sampler,
    pub id_draw: wgpu::RenderPipeline,
    pub id_solid: wgpu::RenderPipeline,
    pub id_3d: wgpu::RenderPipeline,
    /// The compositing passes (`crate::composite`).
    pub(crate) ops: crate::composite::gpu::OpPipelines,
    draw: [wgpu::RenderPipeline; 3],
    solid: [wgpu::RenderPipeline; 3],
    background: [wgpu::RenderPipeline; 3],
    flip: [wgpu::RenderPipeline; 3],
    source: [wgpu::ComputePipeline; 2],
}

/// Formats a 2D target can have: the two working formats, and 8-bit
/// (the 3D pass resolves into it).
const TARGET_FORMATS: [wgpu::TextureFormat; 3] = [
    wgpu::TextureFormat::Rgba16Float,
    wgpu::TextureFormat::Rgba32Float,
    wgpu::TextureFormat::Rgba8Unorm,
];

fn format_index(format: wgpu::TextureFormat) -> usize {
    TARGET_FORMATS
        .iter()
        .position(|f| *f == format)
        .expect("a 2D target format")
}

impl Pipelines {
    pub fn new(device: &wgpu::Device) -> Pipelines {
        let composite = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render composite"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    pm_color::wgsl::COLOR,
                    include_str!("shaders/composite.wgsl")
                )
                .into(),
            ),
        });
        let uniform = |binding, visibility, dynamic, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let quad_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm quad"),
            entries: &[uniform(
                0,
                wgpu::ShaderStages::VERTEX_FRAGMENT,
                true,
                size_of::<QuadUniform>() as u64,
            )],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let fill_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm fill"),
            entries: &[uniform(
                2,
                wgpu::ShaderStages::FRAGMENT,
                false,
                size_of::<FillUniform>() as u64,
            )],
        });
        // Today's texture parameters: LINEAR min and mag, CLAMP_TO_EDGE, no mipmaps.
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pm linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let layout_with = |groups: &[&wgpu::BindGroupLayout]| {
            let groups: Vec<Option<&wgpu::BindGroupLayout>> =
                groups.iter().map(|g| Some(*g)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &groups,
                immediate_size: 0,
            })
        };
        let draw_layout = layout_with(&[&quad_layout, &texture_layout]);
        let solid_layout = layout_with(&[&quad_layout]);
        let background_layout = layout_with(&[&quad_layout, &fill_layout]);
        let over = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = |layout: &wgpu::PipelineLayout, entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &composite,
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
                    module: &composite,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(over),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let draw = TARGET_FORMATS.map(|f| pipeline(&draw_layout, "fs_draw", f));
        let solid = TARGET_FORMATS.map(|f| pipeline(&solid_layout, "fs_solid", f));
        let background = TARGET_FORMATS.map(|f| pipeline(&background_layout, "fs_background", f));
        let flip_pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("fs_flip"),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState {
                    module: &composite,
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
                    module: &composite,
                    entry_point: Some("fs_flip"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let flip = TARGET_FORMATS.map(flip_pipeline);

        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let storage = |format| wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let source_entries = |format| {
            [
                tex(0),
                tex(1),
                uniform(
                    2,
                    wgpu::ShaderStages::COMPUTE,
                    false,
                    size_of::<SourceUniform>() as u64,
                ),
                storage(format),
            ]
        };
        let source_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm source f16"),
            entries: &source_entries(wgpu::TextureFormat::Rgba16Float),
        });
        let source_layout_f32 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm source f32"),
            entries: &source_entries(wgpu::TextureFormat::Rgba32Float),
        });
        let source = [
            (
                wgpu::TextureFormat::Rgba16Float,
                "rgba16float",
                &source_layout,
            ),
            (
                wgpu::TextureFormat::Rgba32Float,
                "rgba32float",
                &source_layout_f32,
            ),
        ]
        .map(|(_, name, layout)| {
            let code = format!(
                "{}\n{}\n{}",
                pm_color::wgsl::COLOR,
                pm_color::tonemap::WGSL,
                include_str!("shaders/source.wgsl").replace("OUT_FORMAT", name)
            );
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pm-render source"),
                source: wgpu::ShaderSource::Wgsl(code.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pm-render source"),
                layout: Some(&layout_with(&[layout])),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        // The depth and id surface.
        let id_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render id"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/id.wgsl").into()),
        });
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let id_pipeline = |layout: &wgpu::PipelineLayout, entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &id_module,
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
                    module: &id_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let id_draw = id_pipeline(&draw_layout, "fs_id_draw");
        let id_solid = id_pipeline(&solid_layout, "fs_id_solid");
        let id_3d = id_pipeline(
            &layout_with(&[&quad_layout, &texture_layout, &depth_layout]),
            "fs_id_3d",
        );
        Pipelines {
            ops: crate::composite::gpu::OpPipelines::new(device),
            depth_layout,
            id_draw,
            id_solid,
            id_3d,
            quad_layout,
            texture_layout,
            fill_layout,
            source_layout,
            linear,
            draw,
            solid,
            background,
            flip,
            source,
        }
    }

    pub fn draw(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        &self.draw[format_index(format)]
    }

    pub fn solid(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        &self.solid[format_index(format)]
    }

    pub fn background(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        &self.background[format_index(format)]
    }

    /// The exact row flip from GL row order into a top-down target.
    pub fn flip(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        &self.flip[format_index(format)]
    }

    pub fn source(&self, precision: Precision) -> &wgpu::ComputePipeline {
        match precision {
            Precision::Float16 => &self.source[0],
            Precision::Float32 => &self.source[1],
        }
    }

    /// The source pass bind group layout for a precision (each storage
    /// format has its own).
    pub fn source_layout(&self, precision: Precision) -> wgpu::BindGroupLayout {
        self.source(precision).get_bind_group_layout(0)
    }
}
