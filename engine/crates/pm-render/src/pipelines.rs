//! Compiled pipelines and the uniform layouts they read.

use std::sync::{Arc, Mutex, OnceLock};

/// A pipeline compiled on first use. The factory owns only handles from its
/// device; neither it nor the device keeps a reference to this wrapper.
pub(crate) struct LazyPipeline<P> {
    pipeline: OnceLock<P>,
    create: Mutex<Option<Box<dyn FnOnce() -> P + Send>>>,
}

pub(crate) type LazyRenderPipeline = LazyPipeline<wgpu::RenderPipeline>;
pub(crate) type LazyComputePipeline = LazyPipeline<wgpu::ComputePipeline>;

impl<P> LazyPipeline<P> {
    pub(crate) fn new(create: impl FnOnce() -> P + Send + 'static) -> Self {
        Self {
            pipeline: OnceLock::new(),
            create: Mutex::new(Some(Box::new(create))),
        }
    }

    pub(crate) fn get(&self) -> &P {
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
    scene: [LazyRenderPipeline; 3],
    scene_add: [LazyRenderPipeline; 3],
    rect: [LazyRenderPipeline; 3],
    solid: [LazyRenderPipeline; 3],
    draw_add: [LazyRenderPipeline; 3],
    rect_add: [LazyRenderPipeline; 3],
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
        // Keep the analytic solid functions out of the module compiled for
        // texture, background and flip pipelines. Their first use otherwise
        // clones the entire solid WGSL AST for unrelated draws.
        let (shared, tail) = include_str!("shaders/composite.wgsl")
            .split_once("// Solid layers:")
            .expect("solid shader section");
        // Solid pipelines need only the quad uniform and vertex output from
        // the shared section. Texture sampling and color conversion belong
        // to the compositing module; parsing/cloning them here costs cold
        // renderer allocations even for an ordinary rectangle.
        let (solid_header, _) = shared
            .split_once("// Solid module header ends here.")
            .expect("solid module header");
        let (_, vertex_output_tail) = shared
            .split_once("// Vertex output starts.")
            .expect("vertex output section");
        let (vertex_output, _) = vertex_output_tail
            .split_once("// Vertex output ends.")
            .expect("vertex output end");
        let solid_shared = format!("{solid_header}\n{vertex_output}");
        let (solid_body, other_body) = tail
            .split_once("// `FRAG_BACKGROUND_FILL`")
            .expect("background shader section");
        let (rect_common, rounded_tail) = solid_body
            .split_once("// Rounded-only circle helpers.")
            .expect("rounded shader section");
        let (rect_vertex, polygon_tail) = rect_common
            .split_once("struct Poly {")
            .expect("rectangle polygon section");
        let (_, from_inverse) = rounded_tail
            .split_once("// The inverse")
            .expect("shared inverse section");
        let (inverse_and_round, rect_fragment) = from_inverse
            .split_once("// Rect-only fragment.")
            .expect("rectangle shader section");
        let (inverse, _) = inverse_and_round
            .split_once("@fragment\nfn fs_solid")
            .expect("rounded fragment section");
        let composite = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render composite"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}// `FRAG_BACKGROUND_FILL`{}",
                    pm_color::wgsl::COLOR,
                    shared,
                    other_body,
                )
                .into(),
            ),
        });
        let solid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render analytic solid"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{solid_shared}\n// Solid layers:{solid_body}").into(),
            ),
        });
        let rect_vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render rectangle vertex"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{solid_shared}\n// Solid layers:{rect_vertex}").into(),
            ),
        });
        let rect_fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render rectangle fragment"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\nstruct Poly {{{}// The inverse{}// Rect-only fragment.{}",
                    solid_shared, polygon_tail, inverse, rect_fragment,
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
        let resample = crate::sampling::gpu::Resampler::new(device);
        let quad_layout = wgpu::BindGroupLayout::clone(resample.uniform_layout());
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
        let scene_module = Arc::new(OnceLock::<wgpu::ShaderModule>::new());
        let half_modules = Arc::new(HalfModules::new(
            format!("{}\n// `FRAG_BACKGROUND_FILL`{}", shared, other_body),
            format!("{solid_shared}\n// Solid layers:{solid_body}"),
            format!(
                "{}\nstruct Poly {{{}// The inverse{}// Rect-only fragment.{}",
                solid_shared, polygon_tail, inverse, rect_fragment,
            ),
        ));
        let blended = |layout: &wgpu::PipelineLayout,
                       entry: &'static str,
                       format: wgpu::TextureFormat,
                       blend: wgpu::BlendState| {
            let device = device.clone();
            let (vertex_shader, fragment_shader) = match entry {
                "fs_solid" => (solid_shader.clone(), solid_shader.clone()),
                "fs_rect" => (rect_vertex_shader.clone(), rect_fragment_shader.clone()),
                _ => (composite.clone(), composite.clone()),
            };
            let layout = layout.clone();
            let scene_module = scene_module.clone();
            let half_modules = half_modules.clone();
            LazyRenderPipeline::new(move || {
                // A linear 3D source compiles the scene output curve; an
                // RGBA16F target stores the nearest half of each value.
                let fragment_shader = if entry == "fs_scene" {
                    scene_module.get_or_init(|| {
                        device.create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("pm-render scene composite"),
                            source: wgpu::ShaderSource::Wgsl(
                                format!(
                                    "{}\n{}\n{}\n{}\n{}",
                                    pm_color::wgsl::COLOR,
                                    crate::scene3d::NEAREST_HALF_WGSL,
                                    include_str!("shaders/scene_output.wgsl"),
                                    include_str!("shaders/composite.wgsl"),
                                    include_str!("shaders/scene_quad.wgsl")
                                )
                                .into(),
                            ),
                        })
                    }).clone()
                } else if format == wgpu::TextureFormat::Rgba16Float {
                    half_modules.module(&device, entry)
                } else {
                    fragment_shader
                };
                let vertex_shader = if entry == "fs_scene" { fragment_shader.clone() } else { vertex_shader };
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &vertex_shader,
                        // Solids grow by their antialiased fringe.
                        entry_point: Some(if entry == "fs_solid" || entry == "fs_rect" {
                            "vs_solid"
                        } else {
                            "vs"
                        }),
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
                        module: &fragment_shader,
                        entry_point: Some(&if format == wgpu::TextureFormat::Rgba16Float {
                            format!("{entry}_half")
                        } else {
                            entry.to_owned()
                        }),
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
        let scene = TARGET_FORMATS.map(|f| pipeline(&draw_layout, "fs_scene", f));
        let scene_add = TARGET_FORMATS.map(|f| blended(&draw_layout, "fs_scene", f, add));
        let rect = TARGET_FORMATS.map(|f| pipeline(&solid_layout, "fs_rect", f));
        let solid = TARGET_FORMATS.map(|f| pipeline(&solid_layout, "fs_solid", f));
        let draw_add = TARGET_FORMATS.map(|f| blended(&draw_layout, "fs_draw", f, add));
        let rect_add = TARGET_FORMATS.map(|f| blended(&solid_layout, "fs_rect", f, add));
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
            scene,
            scene_add,
            rect,
            solid,
            draw_add,
            rect_add,
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

    /// The scene output curve is compiled only when a linear 3D source draws.
    pub fn scene_with(
        &self,
        format: wgpu::TextureFormat,
        blend: PassBlend,
    ) -> &wgpu::RenderPipeline {
        match blend {
            PassBlend::Over => self.scene[format_index(format)].get(),
            PassBlend::Additive => self.scene_add[format_index(format)].get(),
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

    /// Exact area coverage for square-corner solids, without the disk shader.
    pub fn rect_with(
        &self,
        format: wgpu::TextureFormat,
        blend: PassBlend,
    ) -> &wgpu::RenderPipeline {
        match blend {
            PassBlend::Over => self.rect[format_index(format)].get(),
            PassBlend::Additive => self.rect_add[format_index(format)].get(),
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
            &self.rect[i],
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
            &self.rect,
            &self.solid,
            &self.draw_add,
            &self.rect_add,
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

/// The 2D shader modules for RGBA16F targets: each fragment entry `fs_x`
/// becomes `fs_x_value`, wrapped by `fs_x` and by `fs_x_half`, which stores
/// the nearest half (Metal's render-target conversion truncates). Compiled on
/// first use of a half-float target.
struct HalfModules {
    sources: [(String, OnceLock<wgpu::ShaderModule>); 3],
}

impl HalfModules {
    fn new(composite: String, solid: String, rect: String) -> HalfModules {
        HalfModules {
            sources: [composite, solid, rect].map(|s| (s, OnceLock::new())),
        }
    }

    fn module(&self, device: &wgpu::Device, entry: &str) -> wgpu::ShaderModule {
        let (source, module) = &self.sources[match entry {
            "fs_solid" => 1,
            "fs_rect" => 2,
            _ => 0,
        }];
        module
            .get_or_init(|| {
                let mut wgsl = format!(
                    "{}\n{}\n{}",
                    pm_color::wgsl::COLOR,
                    crate::scene3d::NEAREST_HALF_WGSL,
                    source
                );
                for name in ["fs_draw", "fs_background", "fs_solid", "fs_rect"] {
                    let entry = format!("@fragment\nfn {name}(in: VsOut) -> @location(0) vec4<f32> {{");
                    if wgsl.contains(&entry) {
                        wgsl = wgsl.replace(&entry, &format!("fn {name}_value(in: VsOut) -> vec4<f32> {{"));
                        wgsl.push_str(&format!(
                            "\n@fragment fn {name}_half(in: VsOut) -> @location(0) vec4<f32> {{ return pm_scene_half_color({name}_value(in)); }}\n"
                        ));
                    }
                }
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("pm-render half-float 2D"),
                    source: wgpu::ShaderSource::Wgsl(wgsl.into()),
                })
            })
            .clone()
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
            "lazy 2D pipelines: construct={construction:?} first={first_use:?} repeat={repeat:?} built=2/24"
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
            timed!(
                format!("2D rectangle {format:?}"),
                pipelines.rect_with(format, PassBlend::Over)
            );
            timed!(format!("2D solid {format:?}"), pipelines.solid(format));
            timed!(
                format!("2D draw additive {format:?}"),
                pipelines.draw_with(format, PassBlend::Additive)
            );
            timed!(
                format!("2D rectangle additive {format:?}"),
                pipelines.rect_with(format, PassBlend::Additive)
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
        assert_eq!(pipelines.initialized_2d_pipelines(), 24);
        times.extend(pipelines.ops.probe_all());
        times.extend(pipelines.ops.path_masks.probe_all());
        times.extend(pipelines.resample.probe_all());
        times.extend(text.probe_all());
        // 24 2D, 21 composite (7 ops), 6 resample draws and 3 mip writers
        // over rgba16float, rgba32float and colordepth's rgba8unorm; path
        // masks and 6 text outline/image/blur.
        assert_eq!(times.len(), 72);
        assert_eq!(built(), times.len());
        for (name, duration) in times {
            eprintln!("lazy pipeline first use: {name}={duration:?}");
        }
    }
}
