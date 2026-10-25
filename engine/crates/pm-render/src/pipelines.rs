//! Compiled pipelines and the uniform layouts they read.

use std::sync::{Mutex, OnceLock};

/// A pipeline compiled on first use. The factory owns only handles from its
/// device; neither it nor the device keeps a reference to this wrapper.
pub(crate) struct LazyRenderPipeline {
    pipeline: OnceLock<wgpu::RenderPipeline>,
    create: Mutex<Option<Box<dyn FnOnce() -> wgpu::RenderPipeline + Send>>>,
}

impl LazyRenderPipeline {
    pub(crate) fn new(create: impl FnOnce() -> wgpu::RenderPipeline + Send + 'static) -> Self {
        Self {
            pipeline: OnceLock::new(),
            create: Mutex::new(Some(Box::new(create))),
        }
    }

    pub(crate) fn get(&self) -> &wgpu::RenderPipeline {
        self.pipeline.get_or_init(|| {
            // OnceLock admits one initializer. Take the factory before running
            // it so its captured device, module, and layout handles drop as
            // soon as the new pipeline has been installed.
            let create = self
                .create
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("a lazy pipeline initializes only once");
            create()
        })
    }

    #[cfg(test)]
    pub(crate) fn is_built(&self) -> bool {
        self.pipeline.get().is_some()
    }
}

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
    pub scene: [f32; 4],
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

/// How a pass blends its draws into the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassBlend {
    /// Premultiplied source-over (`ONE, ONE_MINUS_SRC_ALPHA`).
    Over,
    /// Accumulation (`ONE, ONE`): motion-blur samples.
    Additive,
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
    draw: [LazyRenderPipeline; 3],
    solid: [LazyRenderPipeline; 3],
    draw_add: [LazyRenderPipeline; 3],
    solid_add: [LazyRenderPipeline; 3],
    background: [LazyRenderPipeline; 3],
    flip: [LazyRenderPipeline; 3],
    source: [wgpu::ComputePipeline; 3],
    /// Level 2 sampling (`crate::sampling`).
    pub(crate) resample: crate::sampling::gpu::Resampler,
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
                    "{}\n{}\n{}",
                    pm_color::wgsl::COLOR,
                    include_str!("shaders/scene_output.wgsl"),
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
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let blended = |layout: &wgpu::PipelineLayout,
                       entry: &'static str,
                       format: wgpu::TextureFormat,
                       blend: wgpu::BlendState| {
            let device = device.clone();
            let composite = composite.clone();
            let layout = layout.clone();
            LazyRenderPipeline::new(move || {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
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
                            blend: Some(blend),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
        };
        let pipeline =
            |layout: &wgpu::PipelineLayout, entry: &'static str, format: wgpu::TextureFormat| {
                blended(layout, entry, format, over)
            };
        let draw = TARGET_FORMATS.map(|f| pipeline(&draw_layout, "fs_draw", f));
        let solid = TARGET_FORMATS.map(|f| pipeline(&solid_layout, "fs_solid", f));
        let draw_add = TARGET_FORMATS.map(|f| blended(&draw_layout, "fs_draw", f, add));
        let solid_add = TARGET_FORMATS.map(|f| blended(&solid_layout, "fs_solid", f, add));
        let background = TARGET_FORMATS.map(|f| pipeline(&background_layout, "fs_background", f));
        let flip_pipeline = |format: wgpu::TextureFormat| {
            let device = device.clone();
            let composite = composite.clone();
            let draw_layout = draw_layout.clone();
            LazyRenderPipeline::new(move || {
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
        let source_layout_rgba8 =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pm source rgba8"),
                entries: &source_entries(wgpu::TextureFormat::Rgba8Unorm),
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
            (
                wgpu::TextureFormat::Rgba8Unorm,
                "rgba8unorm",
                &source_layout_rgba8,
            ),
        ]
        .map(|(_, name, layout)| {
            let code = format!(
                "{}\n{}\n{}",
                pm_color::wgsl::COLOR,
                pm_color::tonemap::wgsl(),
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
        let resample = crate::sampling::gpu::Resampler::new(device, &quad_layout);
        Pipelines {
            ops: crate::composite::gpu::OpPipelines::new(device),
            resample,
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
            draw_add,
            solid_add,
            background,
            flip,
            source,
        }
    }

    pub fn draw(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.draw[format_index(format)].get()
    }

    pub fn solid(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.solid[format_index(format)].get()
    }

    /// [`Self::draw`] with a pass's blending.
    pub fn draw_with(
        &self,
        format: wgpu::TextureFormat,
        blend: PassBlend,
    ) -> &wgpu::RenderPipeline {
        match blend {
            PassBlend::Over => self.draw[format_index(format)].get(),
            PassBlend::Additive => self.draw_add[format_index(format)].get(),
        }
    }

    /// [`Self::solid`] with a pass's blending.
    pub fn solid_with(
        &self,
        format: wgpu::TextureFormat,
        blend: PassBlend,
    ) -> &wgpu::RenderPipeline {
        match blend {
            PassBlend::Over => self.solid[format_index(format)].get(),
            PassBlend::Additive => self.solid_add[format_index(format)].get(),
        }
    }

    pub fn background(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.background[format_index(format)].get()
    }

    /// The exact row flip from GL row order into a top-down target.
    pub fn flip(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.flip[format_index(format)].get()
    }

    /// Compiles the deferred 2D pipelines every drawn frame at `format`
    /// uses (quads, solids, the row flip, the background), ahead of the
    /// frame that would otherwise compile them.
    pub fn prewarm(&self, format: wgpu::TextureFormat) {
        let i = format_index(format);
        for pipeline in [
            &self.draw[i],
            &self.solid[i],
            &self.flip[i],
            &self.background[i],
        ] {
            pipeline.get();
        }
    }

    pub fn source(&self, format: wgpu::TextureFormat) -> &wgpu::ComputePipeline {
        &self.source[format_index(format)]
    }

    /// The source pass bind group layout for its output storage format.
    pub fn source_layout(&self, format: wgpu::TextureFormat) -> wgpu::BindGroupLayout {
        self.source(format).get_bind_group_layout(0)
    }

    #[cfg(test)]
    fn initialized_2d_pipelines(&self) -> usize {
        [
            &self.draw,
            &self.solid,
            &self.draw_add,
            &self.solid_add,
            &self.background,
            &self.flip,
        ]
        .into_iter()
        .flat_map(|variants| variants.iter())
        .filter(|pipeline| pipeline.is_built())
        .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn fixed_2d_variants_compile_once_on_demand() {
        let gpu = pm_gpu::Gpu::new("lazy pipeline probe").unwrap();
        let start = Instant::now();
        let pipelines = Pipelines::new(&gpu.device);
        let construction = start.elapsed();
        assert_eq!(pipelines.initialized_2d_pipelines(), 0);

        let start = Instant::now();
        let first = pipelines.solid(wgpu::TextureFormat::Rgba16Float);
        let first_use = start.elapsed();
        assert_eq!(pipelines.initialized_2d_pipelines(), 1);
        let start = Instant::now();
        let again = pipelines.solid(wgpu::TextureFormat::Rgba16Float);
        let repeat = start.elapsed();
        assert!(std::ptr::eq(first, again));
        assert_eq!(pipelines.initialized_2d_pipelines(), 1);

        pipelines.solid_with(wgpu::TextureFormat::Rgba32Float, PassBlend::Additive);
        assert_eq!(pipelines.initialized_2d_pipelines(), 2);
        eprintln!(
            "lazy 2D pipelines: construct={construction:?} first={first_use:?} repeat={repeat:?} built=2/18"
        );

        let captured = std::sync::Arc::new(());
        let weak = std::sync::Arc::downgrade(&captured);
        let handle = first.clone();
        let one_shot = LazyRenderPipeline::new(move || {
            std::hint::black_box(&captured);
            handle
        });
        assert!(weak.upgrade().is_some());
        let cached = one_shot.get();
        assert!(
            weak.upgrade().is_none(),
            "the finished factory kept its captures"
        );
        assert!(std::ptr::eq(cached, one_shot.get()));
    }

    #[test]
    fn prewarm_compiles_one_formats_frame_pipelines_only() {
        let gpu = pm_gpu::Gpu::new("lazy pipeline prewarm").unwrap();
        let pipelines = Pipelines::new(&gpu.device);
        pipelines.prewarm(wgpu::TextureFormat::Rgba16Float);
        assert_eq!(pipelines.initialized_2d_pipelines(), 4);
        pipelines.draw(wgpu::TextureFormat::Rgba16Float);
        pipelines.flip(wgpu::TextureFormat::Rgba16Float);
        assert_eq!(
            pipelines.initialized_2d_pipelines(),
            4,
            "a frame recompiled them"
        );
        pipelines.prewarm(wgpu::TextureFormat::Rgba16Float);
        assert_eq!(pipelines.initialized_2d_pipelines(), 4);
    }

    #[test]
    fn probe_first_use_of_every_deferred_render_pipeline() {
        let gpu = pm_gpu::Gpu::new("lazy pipeline family probe").unwrap();
        let pipelines = Pipelines::new(&gpu.device);
        let text = crate::text::gpu::TextGpu::new(&gpu.device);
        let built = || {
            pipelines.initialized_2d_pipelines()
                + pipelines.ops.built_count()
                + pipelines.ops.path_masks.built_count()
                + pipelines.resample.built_count()
                + text.built_count()
        };
        assert_eq!(built(), 0, "constructors compiled a deferred pipeline");
        let mut times: Vec<(String, std::time::Duration)> = Vec::new();
        macro_rules! timed {
            ($name:expr, $pipeline:expr) => {{
                let start = Instant::now();
                std::hint::black_box($pipeline);
                times.push(($name, start.elapsed()));
            }};
        }
        for format in TARGET_FORMATS {
            timed!(format!("2D draw {format:?}"), pipelines.draw(format));
            timed!(format!("2D solid {format:?}"), pipelines.solid(format));
            timed!(
                format!("2D draw additive {format:?}"),
                pipelines.draw_with(format, PassBlend::Additive)
            );
            timed!(
                format!("2D solid additive {format:?}"),
                pipelines.solid_with(format, PassBlend::Additive)
            );
            timed!(
                format!("2D background {format:?}"),
                pipelines.background(format)
            );
            timed!(format!("2D flip {format:?}"), pipelines.flip(format));
        }
        assert_eq!(pipelines.initialized_2d_pipelines(), 18);
        times.extend(pipelines.ops.probe_all());
        times.extend(pipelines.ops.path_masks.probe_all());
        times.extend(pipelines.resample.probe_all());
        times.extend(text.probe_all());
        // 18 2D, 12 composite and 12 resample (rgba16float, rgba32float and
        // colordepth's rgba8unorm), path masks and text.
        assert_eq!(times.len(), 56);
        assert_eq!(built(), 56);
        for (name, duration) in times {
            eprintln!("lazy pipeline first use: {name}={duration:?}");
        }
    }
}
