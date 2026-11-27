//! The level 2 resampler on the GPU (`shaders/resample.wgsl`,
//! `shaders/mips.wgsl`): mip pyramids of sources, the content polygon and
//! inverse mapping of a draw, and its pipelines.

use std::{collections::VecDeque, sync::Arc};

use crate::pipelines::PassBlend;

use super::Sampling;

/// `Resample` in `resample.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ResampleUniform {
    pub inv: [[f32; 4]; 3],
    pub uvwin: [f32; 4],
    pub params: [f32; 4],
    pub view: [f32; 4],
    pub tex: [f32; 4],
    pub origin: [f32; 4],
    pub edges: [[f32; 4]; MAX_EDGES],
    pub bounds: [f32; 4],
}

/// Both quad and resample draws use one dynamic-uniform binding per 256-byte
/// slot. The resample uniform is the larger of the two structures.
pub(crate) const DRAW_UNIFORM_SIZE: u64 = size_of::<ResampleUniform>() as u64;
const _: () = assert!(DRAW_UNIFORM_SIZE == 224);
const _: () = assert!(DRAW_UNIFORM_SIZE <= crate::pipelines::UNIFORM_STRIDE);

/// A quad clipped at the camera plane has at most 5 edges.
pub(crate) const MAX_EDGES: usize = 5;

/// A source with its mip pyramid.
#[derive(Clone)]
pub(crate) struct Pyramid {
    /// Retained downsample levels for a physical pyramid. The source view is
    /// borrowed as logical level zero.
    tail: Option<Arc<TailStorage>>,
    pub view: wgpu::TextureView,
    tail_view: Option<wgpu::TextureView>,
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    inline_first: bool,
}

impl Pyramid {
    /// Borrow the source directly when the declared kernel uses mip zero.
    /// No copy, additional quantization, or retained pyramid allocation.
    pub fn base(source: &wgpu::Texture, view: Option<&wgpu::TextureView>) -> Self {
        Self {
            tail: None,
            view: view.cloned().unwrap_or_else(|| {
                source.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: 0,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            }),
            tail_view: None,
            width: source.width(),
            height: source.height(),
            levels: 1,
            inline_first: false,
        }
    }

    /// A still's own mip chain (`Resampler::fill_mips`): level 0 and every
    /// level below it from the same texture.
    fn own_mips(source: &wgpu::Texture) -> Self {
        let mut pyramid = Self::base(source, None);
        pyramid.tail_view = Some(source.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 1,
            ..Default::default()
        }));
        pyramid.levels = source.mip_level_count();
        pyramid
    }

    /// Evaluate the first box-filtered mip in the sampling draw. Small
    /// minifications need no retained texture or separate copy/downsample passes.
    pub fn inline_first(source: &wgpu::Texture, view: Option<&wgpu::TextureView>) -> Self {
        let mut pyramid = Self::base(source, view);
        pyramid.levels = 2;
        pyramid.inline_first = true;
        pyramid
    }
}

struct CachedPyramid {
    source: wgpu::Texture,
    revision: Option<u64>,
    pyramid: Pyramid,
}

/// Tail storage without a source handle, source view, or first downsample
/// binding. These are rebuilt if a different source takes the allocation.
struct IdleTail {
    storage: Arc<TailStorage>,
    used_frame: u64,
    width: u32,
    height: u32,
    levels: u32,
}

struct TailStorage {
    levels: Vec<wgpu::TextureView>,
    view: wgpu::TextureView,
}

impl TailStorage {
    fn texture(&self) -> &wgpu::Texture {
        self.view.texture()
    }
}

impl CachedPyramid {
    fn into_idle(self, used_frame: u64) -> IdleTail {
        let Pyramid {
            tail,
            width,
            height,
            levels,
            ..
        } = self.pyramid;
        IdleTail {
            storage: tail.expect("physical pyramid"),
            used_frame,
            width,
            height,
            levels,
        }
    }
}

/// Revision-valid source pyramids and source-free tail storage.
pub(crate) struct PyramidCache {
    entries: VecDeque<CachedPyramid>,
    /// Views of stills' own mip chains, kept so their draws' bind groups
    /// stay cached (the textures belong to the media cache).
    own: VecDeque<Pyramid>,
    idle: VecDeque<IdleTail>,
    frame: u64,
    next_revision: u64,
    budget: u64,
}

/// Sources whose pyramids are kept.
const CACHED_PYRAMIDS: usize = 16;
const CACHED_PYRAMID_BYTES: u64 = 16 * 1024 * 1024;
const IDLE_TAIL_FRAMES: u64 = 8;

impl PyramidCache {
    pub fn new() -> PyramidCache {
        PyramidCache {
            entries: VecDeque::new(),
            own: VecDeque::new(),
            idle: VecDeque::new(),
            frame: 0,
            next_revision: 0,
            budget: CACHED_PYRAMID_BYTES,
        }
    }

    /// Submitted commands keep their own source views. Retain only the
    /// reusable tail storage across frames, so retired sources can go.
    pub fn begin_frame(&mut self) {
        let previous = self.frame;
        self.frame = self.frame.wrapping_add(1);
        self.idle
            .retain(|tail| self.frame.wrapping_sub(tail.used_frame) <= IDLE_TAIL_FRAMES);
        self.idle.extend(
            self.entries
                .drain(..)
                .map(|entry| entry.into_idle(previous)),
        );
        self.enforce_budget();
    }

    /// A new write to a pooled composition texture changes its contents even
    /// when its allocation is reused. Multiple placements of that write share
    /// one revision; the next write gets a different one.
    pub fn changed(&mut self) -> u64 {
        self.next_revision += 1;
        self.next_revision
    }

    fn get(
        &mut self,
        source: &wgpu::Texture,
        revision: Option<u64>,
        levels: u32,
    ) -> Option<Pyramid> {
        let revision = revision?;
        self.entries
            .iter()
            .find(|entry| {
                entry.source == *source
                    && entry.revision == Some(revision)
                    && entry.pyramid.levels >= levels
            })
            .map(|entry| entry.pyramid.clone())
    }

    fn take_tail(
        &mut self,
        width: u32,
        height: u32,
        levels: u32,
        format: wgpu::TextureFormat,
    ) -> Option<(Arc<TailStorage>, u32)> {
        // A tail referenced by this encoder stays in entries until the
        // submission. Only source-free storage can be overwritten.
        let i = self.idle.iter().position(|tail| {
            tail.width == width
                && tail.height == height
                && tail.levels >= levels
                && tail.storage.texture().format() == format
        })?;
        let idle = self.idle.remove(i).unwrap();
        Some((idle.storage, idle.levels))
    }

    fn put(&mut self, source: wgpu::Texture, revision: Option<u64>, pyramid: Pyramid) {
        self.entries.push_front(CachedPyramid {
            source,
            revision,
            pyramid,
        });
        self.enforce_budget();
    }

    fn enforce_budget(&mut self) {
        while self.entries.len() + self.idle.len() > CACHED_PYRAMIDS || self.bytes() > self.budget {
            if let Some(i) = self
                .idle
                .iter()
                .enumerate()
                .max_by_key(|(_, tail)| self.frame.wrapping_sub(tail.used_frame))
                .map(|(i, _)| i)
            {
                self.idle.remove(i);
                continue;
            }
            // All remaining tails are referenced by the current encoder.
            // Revisit the cap at the next frame boundary.
            break;
        }
    }

    /// `texture`'s own mip chain read to `levels`, its views kept.
    fn own_mips(&mut self, texture: &wgpu::Texture, levels: u32) -> Pyramid {
        let found = self.own.iter().position(|p| p.view.texture() == texture);
        let pyramid = match found {
            Some(i) => self.own.remove(i).expect("found"),
            None => Pyramid::own_mips(texture),
        };
        self.own.push_front(pyramid.clone());
        self.own.truncate(CACHED_PYRAMIDS);
        while self
            .own
            .iter()
            .map(|p| texture_bytes(p.view.texture()))
            .sum::<u64>()
            > self.budget
        {
            self.own.pop_back();
        }
        Pyramid {
            levels: levels.min(pyramid.levels),
            ..pyramid
        }
    }

    /// Drops the views of stills' own mip chains whose texture `kept`
    /// does not name any more: the media cache let the still go, and its
    /// views here would keep the whole texture allocated until they aged
    /// out.
    pub fn retain_own(&mut self, kept: impl Fn(&wgpu::Texture) -> bool) {
        self.own.retain(|pyramid| kept(pyramid.view.texture()));
    }

    /// Texel bytes of the stills' textures that only the views here still
    /// own (`kept`: the media cache has the still, and counts it).
    pub fn orphaned_bytes(&self, kept: impl Fn(&wgpu::Texture) -> bool) -> u64 {
        self.own
            .iter()
            .map(|pyramid| pyramid.view.texture())
            .filter(|texture| !kept(texture))
            .map(texture_bytes)
            .sum()
    }

    /// Drops every kept pyramid.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.own.clear();
        self.idle.clear();
    }

    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
        self.enforce_budget();
        while self
            .own
            .iter()
            .map(|p| texture_bytes(p.view.texture()))
            .sum::<u64>()
            > self.budget
        {
            self.own.pop_back();
        }
    }

    /// Retained downsample storage (the source belongs to its content cache).
    pub fn bytes(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| {
                tail_bytes(
                    entry.pyramid.width,
                    entry.pyramid.height,
                    entry.pyramid.levels,
                    entry
                        .pyramid
                        .tail
                        .as_ref()
                        .expect("physical pyramid")
                        .texture()
                        .format(),
                )
            })
            .chain(self.idle.iter().map(|tail| {
                tail_bytes(
                    tail.width,
                    tail.height,
                    tail.levels,
                    tail.storage.texture().format(),
                )
            }))
            .sum()
    }

    /// Storage to retain at the next boundary, including a few recent
    /// frames for cuts and intermittently visible layers.
    pub fn playback_demand(&self) -> u64 {
        self.bytes()
            - self
                .idle
                .iter()
                .filter(|tail| self.frame.wrapping_sub(tail.used_frame) > 1)
                .map(|tail| {
                    tail_bytes(
                        tail.width,
                        tail.height,
                        tail.levels,
                        tail.storage.texture().format(),
                    )
                })
                .sum::<u64>()
    }
}

fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    (0..texture.mip_level_count())
        .map(|level| {
            u64::from((texture.width() >> level).max(1))
                * u64::from((texture.height() >> level).max(1))
        })
        .sum::<u64>()
        * u64::from(texture.format().block_copy_size(None).unwrap_or(16))
}

fn tail_bytes(width: u32, height: u32, levels: u32, format: wgpu::TextureFormat) -> u64 {
    let px: u64 = (1..levels)
        .map(|level| u64::from((width >> level).max(1)) * u64::from((height >> level).max(1)))
        .sum();
    px * u64::from(
        format
            .block_copy_size(None)
            .expect("uncompressed raster format"),
    )
}

impl Default for PyramidCache {
    fn default() -> Self {
        Self::new()
    }
}

/// The level 2 pipelines.
pub(crate) struct Resampler {
    uniform_layout: wgpu::BindGroupLayout,
    layout: wgpu::BindGroupLayout,
    /// Per format and dispatch length: the level above and the levels written.
    mip_layouts: [Vec<wgpu::BindGroupLayout>; 3],
    trilinear: wgpu::Sampler,
    /// `[format][blend]`: rgba16float, rgba32float, rgba8unorm × over, additive.
    draw: [[crate::pipelines::LazyRenderPipeline; 2]; 3],
    down: [Vec<crate::pipelines::LazyComputePipeline>; 3],
}

const FORMATS: [wgpu::TextureFormat; 3] = [
    wgpu::TextureFormat::Rgba16Float,
    wgpu::TextureFormat::Rgba32Float,
    wgpu::TextureFormat::Rgba8Unorm,
];
const MIP_FORMATS: [wgpu::TextureFormat; 3] = FORMATS;

fn format_index(format: wgpu::TextureFormat) -> usize {
    match format {
        wgpu::TextureFormat::Rgba16Float => 0,
        wgpu::TextureFormat::Rgba32Float => 1,
        wgpu::TextureFormat::Rgba8Unorm => 2,
        other => panic!("unsupported resampler format: {other:?}"),
    }
}

fn mip_view(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("pm pyramid level"),
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

impl Resampler {
    pub fn new(device: &wgpu::Device) -> Resampler {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm draw uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(DRAW_UNIFORM_SIZE),
                },
                count: None,
            }],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render resample"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    crate::nearest_half::NEAREST_HALF_WGSL,
                    include_str!("../shaders/resample.wgsl")
                )
                .into(),
            ),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm resample source"),
            entries: &[
                texture(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(2),
            ],
        });
        let mip_layouts = MIP_FORMATS.map(|format| {
            let batches = if format == wgpu::TextureFormat::Rgba8Unorm {
                1
            } else {
                4
            };
            (0..batches)
                .map(|step| {
                    let mut entries = vec![wgpu::BindGroupLayoutEntry {
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ..texture(0)
                    }];
                    for binding in 1..=step as u32 + 1 {
                        entries.push(wgpu::BindGroupLayoutEntry {
                            binding,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                            count: None,
                        });
                    }
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("pm mips"),
                        entries: &entries,
                    })
                })
                .collect::<Vec<_>>()
        });
        let trilinear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pm trilinear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let draw_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm resample"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&layout)],
            immediate_size: 0,
        });
        let component = |dst| wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: dst,
            operation: wgpu::BlendOperation::Add,
        };
        let blend = |b: PassBlend| {
            let dst = match b {
                PassBlend::Over => wgpu::BlendFactor::OneMinusSrcAlpha,
                PassBlend::Additive => wgpu::BlendFactor::One,
            };
            wgpu::BlendState {
                color: component(dst),
                alpha: component(dst),
            }
        };
        let pipeline = |layout: &wgpu::PipelineLayout,
                        module: &wgpu::ShaderModule,
                        vs: &'static str,
                        fs: &'static str,
                        format: wgpu::TextureFormat,
                        blend: Option<wgpu::BlendState>| {
            let device = device.clone();
            let module = module.clone();
            let layout = layout.clone();
            crate::pipelines::LazyRenderPipeline::new(move || {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(fs),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(vs),
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
                        entry_point: Some(fs),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
        };
        let draw = FORMATS.map(|f| {
            [PassBlend::Over, PassBlend::Additive].map(|b| {
                pipeline(
                    &draw_layout,
                    &module,
                    "vs",
                    if f == wgpu::TextureFormat::Rgba16Float {
                        "fs_resample_half"
                    } else {
                        "fs_resample"
                    },
                    f,
                    Some(blend(b)),
                )
            })
        });
        let down = std::array::from_fn(|i| {
            let (storage, store) = match MIP_FORMATS[i] {
                wgpu::TextureFormat::Rgba16Float => (
                    "rgba16float",
                    "fn store(c: vec4<f32>) -> vec4<f32> { return pm_scene_half_color(c); }",
                ),
                wgpu::TextureFormat::Rgba32Float => (
                    "rgba32float",
                    "fn store(c: vec4<f32>) -> vec4<f32> { return c; }",
                ),
                _ => (
                    "rgba8unorm",
                    "fn store(c: vec4<f32>) -> vec4<f32> { return c; }",
                ),
            };
            let module = Arc::new(std::sync::OnceLock::new());
            mip_layouts[i]
                .iter()
                .enumerate()
                .map(|(step, layout)| {
                    let device = device.clone();
                    let layout = layout.clone();
                    let module = module.clone();
                    let entry = ["cs_down", "cs_down2", "cs_down3", "cs_down4"][step];
                    crate::pipelines::LazyPipeline::new(move || {
                        let module = module.get_or_init(|| {
                            let source = format!(
                                "{}\n{store}\n{}",
                                crate::nearest_half::NEAREST_HALF_WGSL,
                                include_str!("../shaders/mips.wgsl").replace("STORE", storage)
                            );
                            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                                label: Some("pm-render mips"),
                                source: wgpu::ShaderSource::Wgsl(source.into()),
                            })
                        });
                        let layout =
                            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                                label: Some("pm mips"),
                                bind_group_layouts: &[Some(&layout)],
                                immediate_size: 0,
                            });
                        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(entry),
                            layout: Some(&layout),
                            module,
                            entry_point: Some(entry),
                            compilation_options: Default::default(),
                            cache: None,
                        })
                    })
                })
                .collect()
        });
        Resampler {
            uniform_layout,
            layout,
            mip_layouts,
            trilinear,
            draw,
            down,
        }
    }

    /// The mip pyramid of `source` at its known content revision. Unknown
    /// revisions reuse storage but regenerate texels before sampling.
    pub fn pyramid(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        levels: u32,
        revision: Option<u64>,
        cache: &mut PyramidCache,
    ) -> Pyramid {
        if let Some(pyramid) = cache.get(source, revision, levels) {
            return pyramid;
        }
        let (width, height) = (source.width().max(1), source.height().max(1));
        // RGBA8 source texels must not quantize each box mean back to eight
        // bits. Half precision is sufficient storage for that source; float
        // sources retain their original precision.
        let format = match source.format() {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba16Float => {
                wgpu::TextureFormat::Rgba16Float
            }
            wgpu::TextureFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
            other => panic!("unsupported pyramid source format: {other:?}"),
        };
        // Multiple draws in one encoder must keep distinct tails unless
        // they read the same revision. Prior-frame storage can change source
        // identity after its old source bindings are dropped.
        let reused = cache.take_tail(width, height, levels, format);
        let (tail, levels) = reused.unwrap_or_else(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pm raster pyramid"),
                size: wgpu::Extent3d {
                    width: (width >> 1).max(1),
                    height: (height >> 1).max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels - 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let storage = TailStorage {
                levels: (0..levels - 1)
                    .map(|level| mip_view(&texture, level))
                    .collect(),
                view: texture.create_view(&Default::default()),
            };
            (Arc::new(storage), levels)
        });
        let view = mip_view(source, 0);
        self.write_levels(device, encoder, &view, &tail.levels, 0, 4);
        let pyramid = Pyramid {
            view,
            tail_view: Some(tail.view.clone()),
            tail: Some(tail),
            width,
            height,
            levels,
            inline_first: false,
        };
        cache.put(source.clone(), revision, pyramid.clone());
        pyramid
    }

    /// Dispatches up to four levels at once, carrying the stored values
    /// through workgroup memory between levels.
    fn write_levels<'a>(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        mut above: &'a wgpu::TextureView,
        levels: &'a [wgpu::TextureView],
        mut level: u32,
        batch: usize,
    ) {
        let Some(first) = levels.first() else {
            return;
        };
        let i = format_index(first.texture().format());
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pm pyramid"),
            timestamp_writes: None,
        });
        for dest in levels.chunks(batch) {
            let step = dest.len() - 1;
            let entries = std::array::from_fn::<_, 5, _>(|binding| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::TextureView(if binding == 0 {
                    above
                } else {
                    dest.get(binding - 1).unwrap_or(above)
                }),
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pm pyramid"),
                layout: &self.mip_layouts[i][step],
                entries: &entries[..=dest.len()],
            });
            pass.set_pipeline(self.down[i][step].get());
            pass.set_bind_group(0, &bind, &[]);
            let target = dest[0].texture();
            let (w, h) = (target.width(), target.height());
            pass.dispatch_workgroups(
                (w >> level).max(1).div_ceil(8),
                (h >> level).max(1).div_ceil(8),
                1,
            );
            above = dest.last().unwrap();
            level += dest.len() as u32;
        }
    }

    /// A still is converted into mip 0; fill its own mip chain in place.
    pub fn fill_mips(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) {
        let views: Vec<_> = (0..texture.mip_level_count())
            .map(|level| mip_view(texture, level))
            .collect();
        // Keep hardware UNORM quantization between levels.
        let batch = self.down[format_index(texture.format())].len();
        self.write_levels(device, encoder, &views[0], &views[1..], 1, batch);
    }

    pub fn pipeline(&self, format: wgpu::TextureFormat, blend: PassBlend) -> &wgpu::RenderPipeline {
        self.draw[format_index(format)][match blend {
            PassBlend::Over => 0,
            PassBlend::Additive => 1,
        }]
        .get()
    }

    #[cfg(test)]
    pub(crate) fn built_count(&self) -> usize {
        self.draw
            .iter()
            .flat_map(|variants| variants.iter())
            .filter(|pipeline| pipeline.is_built())
            .count()
            + self
                .down
                .iter()
                .flatten()
                .filter(|pipeline| pipeline.is_built())
                .count()
    }

    #[cfg(test)]
    pub(crate) fn probe_all(&self) -> Vec<(String, std::time::Duration)> {
        let mut times = Vec::new();
        for (i, format) in FORMATS.into_iter().enumerate() {
            for (name, pipeline) in [
                ("draw over", &self.draw[i][0]),
                ("draw additive", &self.draw[i][1]),
            ] {
                let start = std::time::Instant::now();
                pipeline.get();
                times.push((format!("resample {name} {format:?}"), start.elapsed()));
            }
        }
        for (i, format) in MIP_FORMATS.into_iter().enumerate() {
            for (step, pipeline) in self.down[i].iter().enumerate() {
                let start = std::time::Instant::now();
                pipeline.get();
                times.push((
                    format!("resample down {} {format:?}", step + 1),
                    start.elapsed(),
                ));
            }
        }
        times
    }

    /// The dynamic uniform layout shared with quad draws.
    pub(crate) fn uniform_layout(&self) -> &wgpu::BindGroupLayout {
        &self.uniform_layout
    }

    /// The draw uniform at group zero, shared with quad draws.
    pub(crate) fn bind_uniform(
        &self,
        device: &wgpu::Device,
        cache: &mut crate::binds::BindCache,
        buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        cache.quads(device, &self.uniform_layout, buffer, DRAW_UNIFORM_SIZE)
    }

    /// The source binding at group one.
    pub fn bind(
        &self,
        device: &wgpu::Device,
        cache: &mut crate::binds::BindCache,
        pyramid: &Pyramid,
    ) -> wgpu::BindGroup {
        cache.resample(
            device,
            &self.layout,
            &self.trilinear,
            &pyramid.view,
            pyramid.tail_view.as_ref().unwrap_or(&pyramid.view),
        )
    }
}

/// A level 2 draw's geometry: the content polygon in output pixels
/// (top-down, full frame), its pixel bounds, and the inverse mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Unit quad to output pixels (column-major 3×3).
    pub matrix: [f64; 9],
    /// Output pixels to unit quad.
    pub inverse: [f64; 9],
    /// The polygon's edges `(a, b, c)`: `a·x + b·y + c ≥ 0` inside.
    pub edges: Vec<[f64; 3]>,
    /// The polygon's vertices.
    pub polygon: Vec<[f64; 2]>,
    /// Pixel bounds `[x0, y0, x1, y1]` grown by one pixel.
    pub bounds: [f64; 4],
}

/// A 2×3 affine matrix `[a, b, c, d, e, f]` as a column-major 3×3.
pub fn affine3(m: &[f64; 6]) -> [f64; 9] {
    [m[0], m[1], 0.0, m[2], m[3], 0.0, m[4], m[5], 1.0]
}

fn invert3(m: &[f64; 9]) -> Option<[f64; 9]> {
    let [a, d, g, b, e, h, c, f, i] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    Some([
        (e * i - f * h) / det,
        (f * g - d * i) / det,
        (d * h - e * g) / det,
        (c * h - b * i) / det,
        (a * i - c * g) / det,
        (b * g - a * h) / det,
        (b * f - c * e) / det,
        (c * d - a * f) / det,
        (a * e - b * d) / det,
    ])
}

/// The smallest homogeneous w kept in front of the camera plane.
const NEAR_W: f64 = 1e-6;

/// The placement of a level 2 draw: `matrix` maps the unit quad to output
/// pixels; `uv` is the content UV window (offset, scale), whose part
/// outside 0..1 is transparent. `None` when nothing is in front of the
/// camera or the mapping is singular.
pub fn placement(matrix: [f64; 9], uv: [f64; 4]) -> Option<Placement> {
    let inverse = invert3(&matrix)?;
    // The part of the unit quad the UV window fills.
    let span = |offset: f64, scale: f64| -> Option<(f64, f64)> {
        if scale == 0.0 || !scale.is_finite() {
            return None;
        }
        let (a, b) = ((0.0 - offset) / scale, (1.0 - offset) / scale);
        let (lo, hi) = (a.min(b).max(0.0), a.max(b).min(1.0));
        (hi > lo).then_some((lo, hi))
    };
    let (u0, u1) = span(uv[0], uv[2])?;
    let (v0, v1) = span(uv[1], uv[3])?;
    let m = &matrix;
    let hom = |u: f64, v: f64| {
        [
            m[0] * u + m[3] * v + m[6],
            m[1] * u + m[4] * v + m[7],
            m[2] * u + m[5] * v + m[8],
        ]
    };
    // Clip the quad at w ≥ NEAR_W in homogeneous space.
    let quad = [hom(u0, v0), hom(u1, v0), hom(u1, v1), hom(u0, v1)];
    let mut clipped: Vec<[f64; 3]> = Vec::with_capacity(5);
    for i in 0..4 {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        let (ia, ib) = (a[2] >= NEAR_W, b[2] >= NEAR_W);
        if ia {
            clipped.push(a);
        }
        if ia != ib {
            let t = (NEAR_W - a[2]) / (b[2] - a[2]);
            clipped.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, NEAR_W]);
        }
    }
    if clipped.len() < 3 {
        return None;
    }
    let polygon: Vec<[f64; 2]> = clipped.iter().map(|p| [p[0] / p[2], p[1] / p[2]]).collect();
    if polygon
        .iter()
        .any(|p| !p[0].is_finite() || !p[1].is_finite())
    {
        return None;
    }
    // Orientation: the signed area's sign makes every edge's inside ≥ 0.
    let area: f64 = (0..polygon.len())
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if area.abs() < 1e-12 {
        return None;
    }
    let sign = area.signum();
    let edges = (0..polygon.len())
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            // Inside is to the left of a → b for counterclockwise polygons
            // (positive area in y-down coordinates means clockwise on screen;
            // the sign covers both).
            let (nx, ny) = (-(b[1] - a[1]) * sign, (b[0] - a[0]) * sign);
            [nx, ny, -(nx * a[0] + ny * a[1])]
        })
        .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &polygon {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    Some(Placement {
        matrix,
        inverse,
        edges,
        polygon,
        bounds: [
            x0.floor() - 1.0,
            y0.floor() - 1.0,
            x1.ceil() + 1.0,
            y1.ceil() + 1.0,
        ],
    })
}

impl Placement {
    /// The draw's bounding quad (unit quad to output pixels), clamped to
    /// the frame grown by a pixel; `None` when it misses the frame.
    pub fn bounds_matrix(&self, width: u32, height: u32) -> Option<[f64; 6]> {
        let x0 = self.bounds[0].max(-1.0);
        let y0 = self.bounds[1].max(-1.0);
        let x1 = self.bounds[2].min(f64::from(width) + 1.0);
        let y1 = self.bounds[3].min(f64::from(height) + 1.0);
        (x1 > x0 && y1 > y0).then_some([x1 - x0, 0.0, 0.0, y1 - y0, x0, y0])
    }

    /// Whether every output pixel center lands on a texel center one to one
    /// (an unscaled, unrotated draw at an integer offset): the sample is that
    /// texel exactly, with no kernel arithmetic.
    fn texel_aligned(&self, uv: [f64; 4], from_fbo: bool, pyramid: &Pyramid) -> bool {
        let i = &self.inverse;
        if i[2].abs() > 1e-12 || i[5].abs() > 1e-12 || (i[8] - 1.0).abs() > 1e-12 {
            return false;
        }
        let (w, h) = (f64::from(pyramid.width), f64::from(pyramid.height));
        let flip = if from_fbo { -1.0 } else { 1.0 };
        // Texel coordinates per output pixel, and at the first pixel center.
        let (jxx, jxy) = (i[0] * uv[2] * w, i[3] * uv[2] * w);
        let (jyx, jyy) = (flip * i[1] * uv[3] * h, flip * i[4] * uv[3] * h);
        let tx = (uv[0] + (i[0] * 0.5 + i[3] * 0.5 + i[6]) * uv[2]) * w;
        let qy = uv[1] + (i[1] * 0.5 + i[4] * 0.5 + i[7]) * uv[3];
        let ty = (if from_fbo { 1.0 - qy } else { qy }) * h;
        let unit = |v: f64| (v.abs() - 1.0).abs() < 1e-9;
        let centered = |t: f64| (t - t.floor() - 0.5).abs() < 1e-6;
        unit(jxx)
            && jxy.abs() < 1e-9
            && jyx.abs() < 1e-9
            && unit(jyy)
            && centered(tx)
            && centered(ty)
    }

    /// The draw uniform of this placement in a `frame`-sized output, or
    /// `None` when the draw misses the frame.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn uniform(
        &self,
        uv: [f64; 4],
        alpha: f64,
        from_fbo: bool,
        sampling: Sampling,
        region_origin: (u32, u32),
        target_height: u32,
        frame: (u32, u32),
        pyramid: &Pyramid,
    ) -> Option<ResampleUniform> {
        let bounds = self.bounds_matrix(frame.0, frame.1)?;
        // Near-plane intersections can project billions of pixels away.
        // Use the frame-clamped origin so f32 retains subpixel distances.
        let (ox, oy) = (bounds[4], bounds[5]);
        let i = &self.inverse;
        let mut u = ResampleUniform {
            inv: [
                [i[0] as f32, i[1] as f32, i[2] as f32, 0.0],
                [i[3] as f32, i[4] as f32, i[5] as f32, 0.0],
                [i[6] as f32, i[7] as f32, i[8] as f32, 0.0],
            ],
            uvwin: uv.map(|v| v as f32),
            params: [
                alpha as f32,
                if from_fbo { 1.0 } else { 0.0 },
                match sampling {
                    Sampling::Playback => 0.0,
                    Sampling::Final => 1.0,
                },
                self.edges.len().min(MAX_EDGES) as f32,
            ],
            view: [
                region_origin.0 as f32,
                region_origin.1 as f32,
                target_height as f32,
                frame.0 as f32,
            ],
            tex: [
                pyramid.width as f32,
                pyramid.height as f32,
                pyramid.levels as f32,
                frame.1 as f32,
            ],
            origin: [
                ox as f32,
                oy as f32,
                f32::from(pyramid.inline_first),
                f32::from(u8::from(self.texel_aligned(uv, from_fbo, pyramid))),
            ],
            edges: [[0.0; 4]; MAX_EDGES],
            bounds: [
                bounds[4] as f32,
                bounds[5] as f32,
                bounds[0] as f32,
                bounds[3] as f32,
            ],
        };
        // Normalize and rebase the edges in f64 before uploading them.
        for (k, e) in self.edges.iter().take(MAX_EDGES).enumerate() {
            let len = e[0].hypot(e[1]);
            let (a, b) = (e[0] / len, e[1] / len);
            let c = (e[2] + e[0] * ox + e[1] * oy) / len;
            u.edges[k] = [a as f32, b as f32, c as f32, 0.0];
        }
        Some(u)
    }
}

/// The pyramid levels an affine draw (unit quad to output pixels) of
/// `texture`'s `uv` window can read: none beyond mip zero until its major
/// axis minifies past the kernel's reach.
pub(crate) fn mip_levels(
    matrix: &[f64; 6],
    uv: [f64; 4],
    texture: &wgpu::Texture,
    sampling: Sampling,
) -> u32 {
    let available = texture.width().max(texture.height()).ilog2() + 1;
    let Some(inv) = pm_eval::transform::invert(matrix) else {
        return 1;
    };
    let sx = uv[2] * f64::from(texture.width());
    let sy = uv[3] * f64::from(texture.height());
    let (a, b, c, d) = (inv[0] * sx, inv[1] * sy, inv[2] * sx, inv[3] * sy);
    let (xx, xy, yy) = (a * a + c * c, a * b + c * d, b * b + d * d);
    let major = (0.5 * (xx + yy) + (0.25 * (xx - yy).powi(2) + xy * xy).sqrt()).sqrt();
    let threshold = match sampling {
        Sampling::Final => 4.0,
        Sampling::Playback => 1.0,
    };
    (1 + (major / threshold).log2().ceil().max(0.0) as u32).min(available)
}

impl Resampler {
    /// `texture`'s pyramid for a draw that reads `levels`: the source alone,
    /// its first mip box-filtered inline, or a kept pyramid.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pyramid_for(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        view: Option<&wgpu::TextureView>,
        levels: u32,
        revision: Option<u64>,
        cache: &mut PyramidCache,
    ) -> Pyramid {
        // A still's draws all bind one kept pair of views.
        if texture.mip_level_count() > 1 {
            return cache.own_mips(texture, levels);
        }
        match levels {
            1 => Pyramid::base(texture, view),
            2 => Pyramid::inline_first(texture, view),
            _ => self.pyramid(device, encoder, texture, levels, revision, cache),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fused_mips_match_each_separately_stored_box_mean() {
        let gpu = pm_gpu::Gpu::new("fused mips parity").unwrap();
        let resampler = Resampler::new(&gpu.device);
        let mut seed = 1u32;
        for (w, h) in [(67, 35), (5, 33), (9, 1), (1, 17)] {
            for format in [
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureFormat::Rgba32Float,
            ] {
                let make = || {
                    pm_gpu::Target::with_mips(
                        &gpu.device,
                        "fused mip parity",
                        w,
                        h,
                        format,
                        pm_gpu::target::working_usage(),
                    )
                    .texture
                };
                let fused = make();
                let separate = make();
                let bpp = format.block_copy_size(None).unwrap();
                let mut bytes = Vec::new();
                for i in 0..w * h * 4 {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let scale = if i % 3 == 0 { 1.0e-5 } else { 3.0 };
                    let value = (f32::from((seed >> 16) as u16) / 32768.0 - 1.0) * scale;
                    if format == wgpu::TextureFormat::Rgba16Float {
                        bytes.extend(pm_gpu::readback::f32_to_half(value).to_le_bytes());
                    } else {
                        bytes.extend(value.to_le_bytes());
                    }
                }
                for texture in [&fused, &separate] {
                    gpu.queue.write_texture(
                        texture.as_image_copy(),
                        &bytes,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(w * bpp),
                            rows_per_image: None,
                        },
                        texture.size(),
                    );
                }
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                resampler.fill_mips(&gpu.device, &mut encoder, &fused);
                let views: Vec<_> = (0..separate.mip_level_count())
                    .map(|level| mip_view(&separate, level))
                    .collect();
                resampler.write_levels(&gpu.device, &mut encoder, &views[0], &views[1..], 1, 1);
                gpu.queue.submit([encoder.finish()]);
                for level in 1..fused.mip_level_count() {
                    let size = wgpu::Extent3d {
                        width: (w >> level).max(1),
                        height: (h >> level).max(1),
                        depth_or_array_layers: 1,
                    };
                    let read = |texture: &wgpu::Texture| {
                        let copy = pm_gpu::Target::new(
                            &gpu.device,
                            "mip read",
                            size.width,
                            size.height,
                            format,
                            pm_gpu::target::working_usage(),
                        );
                        let mut encoder = gpu.device.create_command_encoder(&Default::default());
                        encoder.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                mip_level: level,
                                ..texture.as_image_copy()
                            },
                            copy.texture.as_image_copy(),
                            size,
                        );
                        gpu.queue.submit([encoder.finish()]);
                        pm_gpu::readback::read_raw(&gpu.device, &gpu.queue, &copy.texture).unwrap()
                    };
                    assert_eq!(
                        read(&fused),
                        read(&separate),
                        "{format:?} {w}×{h} mip {level}"
                    );
                }
            }
        }
    }

    #[test]
    fn playback_reuses_tail_storage_and_views_after_releasing_the_source() {
        let gpu = pm_gpu::Gpu::new("playback mip storage reuse").unwrap();
        let resampler = Resampler::new(&gpu.device);
        let mut cache = PyramidCache::new();
        let format = wgpu::TextureFormat::Rgba16Float;
        let (w, h, n) = (64, 36, 7);
        cache.set_budget(tail_bytes(w, h, n, format));
        let draw = |cache: &mut PyramidCache| {
            let source = pm_gpu::Target::new(
                &gpu.device,
                "different source frame",
                w,
                h,
                format,
                pm_gpu::target::working_usage(),
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let revision = cache.changed();
            let tail = resampler
                .pyramid(
                    &gpu.device,
                    &mut encoder,
                    &source.texture,
                    n,
                    Some(revision),
                    cache,
                )
                .tail
                .unwrap();
            gpu.queue.submit([encoder.finish()]);
            tail
        };
        cache.begin_frame();
        let first = draw(&mut cache);
        cache.begin_frame();
        assert!(cache.entries.is_empty());
        assert_eq!(cache.idle.len(), 1);
        let second = draw(&mut cache);
        assert!(Arc::ptr_eq(&first, &second));
    }

    /// A still's own mips are box means rounded to nearest at every
    /// storage depth: 2×2 texels 0, 1, 1, 1 (in 1/255) average 0.75/255.
    #[test]
    fn still_mips_store_the_nearest_box_mean() {
        let gpu = pm_gpu::Gpu::new("still mips").expect("GPU device");
        let resampler = Resampler::new(&gpu.device);
        for format in FORMATS {
            let target = pm_gpu::Target::with_mips(
                &gpu.device,
                "still",
                2,
                2,
                format,
                pm_gpu::target::working_usage(),
            );
            assert_eq!(target.texture.mip_level_count(), 2);
            let texel = |v: f32| -> Vec<u8> {
                match format {
                    wgpu::TextureFormat::Rgba8Unorm => vec![(v * 255.0) as u8; 4],
                    wgpu::TextureFormat::Rgba16Float => [v; 4]
                        .iter()
                        .flat_map(|c| pm_gpu::readback::f32_to_half(*c).to_le_bytes())
                        .collect(),
                    _ => [v; 4].iter().flat_map(|c| c.to_le_bytes()).collect(),
                }
            };
            let bytes: Vec<u8> = [0.0, 1.0 / 255.0, 1.0 / 255.0, 1.0 / 255.0]
                .into_iter()
                .flat_map(texel)
                .collect();
            let bpp = format.block_copy_size(None).unwrap();
            gpu.queue.write_texture(
                target.texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(2 * bpp),
                    rows_per_image: Some(2),
                },
                wgpu::Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 1,
                },
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            resampler.fill_mips(&gpu.device, &mut encoder, &target.texture);
            let out = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target.texture,
                    mip_level: 1,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &out,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            gpu.queue.submit([encoder.finish()]);
            out.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let data = out.slice(..).get_mapped_range().unwrap();
            let red = match format {
                wgpu::TextureFormat::Rgba8Unorm => f64::from(data[0]) / 255.0,
                wgpu::TextureFormat::Rgba16Float => {
                    f64::from(pm_gpu::readback::half_to_f32(u16::from_le_bytes([
                        data[0], data[1],
                    ])))
                }
                _ => f64::from(f32::from_le_bytes([data[0], data[1], data[2], data[3]])),
            };
            let exact = 0.75 / 255.0;
            let bound = match format {
                wgpu::TextureFormat::Rgba8Unorm => 0.5 / 255.0,
                wgpu::TextureFormat::Rgba16Float => 2f64.powi(-23),
                _ => 1e-9,
            };
            assert!((red - exact).abs() <= bound, "{format:?}: {red} vs {exact}");
        }
    }

    use super::*;

    #[test]
    fn an_axis_aligned_quad_has_four_edges_inward() {
        let p = placement(
            affine3(&[100.0, 0.0, 0.0, 50.0, 10.0, 20.0]),
            [0.0, 0.0, 1.0, 1.0],
        )
        .unwrap();
        assert_eq!(p.polygon.len(), 4);
        let inside = |x: f64, y: f64| p.edges.iter().all(|e| e[0] * x + e[1] * y + e[2] >= 0.0);
        assert!(inside(60.0, 45.0));
        assert!(!inside(5.0, 45.0) && !inside(60.0, 75.0) && !inside(111.0, 21.0));
        assert_eq!(p.bounds, [9.0, 19.0, 111.0, 71.0]);
        // A mirrored quad keeps its inside.
        let m = placement(
            affine3(&[-100.0, 0.0, 0.0, 50.0, 110.0, 20.0]),
            [0.0, 0.0, 1.0, 1.0],
        )
        .unwrap();
        let inside = |x: f64, y: f64| m.edges.iter().all(|e| e[0] * x + e[1] * y + e[2] >= 0.0);
        assert!(inside(60.0, 45.0));
    }

    #[test]
    fn the_uv_window_limits_the_polygon() {
        // `contain` letterboxing: v from 0.25 to 0.75 of the quad.
        let p = placement(
            affine3(&[100.0, 0.0, 0.0, 100.0, 0.0, 0.0]),
            [0.0, -0.5, 1.0, 2.0],
        )
        .unwrap();
        let ys: Vec<f64> = p.polygon.iter().map(|q| q[1]).collect();
        assert_eq!(ys.iter().cloned().fold(f64::MAX, f64::min), 25.0);
        assert_eq!(ys.iter().cloned().fold(f64::MIN, f64::max), 75.0);
    }

    #[test]
    fn planes_are_clipped_at_the_camera() {
        // w = 1 − 2u: the u > 0.5 half is behind the camera.
        let m = [100.0, 0.0, -2.0, 0.0, 100.0, 0.0, 0.0, 0.0, 1.0];
        let p = placement(m, [0.0, 0.0, 1.0, 1.0]).unwrap();
        assert_eq!(p.polygon.len(), 4);
        assert!(p.polygon.iter().all(|q| q[0].is_finite()));
        // Entirely behind: nothing.
        let m = [100.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, -1.0];
        assert!(placement(m, [0.0, 0.0, 1.0, 1.0]).is_none());
    }

    #[test]
    fn near_plane_edge_distances_resolve_each_row() {
        // w = 2v - 1 puts the top two corners behind the eye. The left
        // edge projects to x = 40 + y/2, even at the near-plane intersection.
        let matrix = [
            300.0, 0.0, 0.0, 20_220.0, 40_280.0, 2.0, -20_110.0, -40_140.0, -1.0,
        ];
        let uv = [0.0, 0.0, 1.0, 1.0];
        let placement = placement(matrix, uv).unwrap();
        assert_eq!(placement.polygon.iter().filter(|p| p[1] < -1e9).count(), 2);

        let gpu = pm_gpu::Gpu::new("near-plane coverage uniform").expect("GPU device");
        let source = pm_gpu::Target::new(
            &gpu.device,
            "near-plane source",
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            pm_gpu::target::working_usage(),
        );
        let pyramid = Pyramid::base(&source.texture, Some(&source.view));
        let uniform = placement
            .uniform(
                uv,
                1.0,
                false,
                Sampling::Final,
                (0, 0),
                160,
                (240, 160),
                &pyramid,
            )
            .unwrap();

        let [a, b, c, _] = uniform.edges[2]; // Left edge, from v = 1 to the near plane.
        for row in 60..=140 {
            let (x, y) = (100.5, row as f32 + 0.5);
            let actual = c + (a * (x - uniform.origin[0]) + b * (y - uniform.origin[1]));
            let expected = (x - 40.0 - y * 0.5) / 1.0_f32.hypot(0.5);
            assert!(
                (actual - expected).abs() < 1e-4,
                "row {row}: {actual} vs {expected}"
            );
        }
    }

    #[test]
    fn clipping_one_corner_uses_all_five_uniform_edges() {
        // w = u + v − 0.2 puts only the first corner behind the camera.
        let m = [100.0, 0.0, 1.0, 0.0, 100.0, 1.0, 0.0, 0.0, -0.2];
        let p = placement(m, [0.0, 0.0, 1.0, 1.0]).unwrap();
        assert_eq!(p.polygon.len(), 5);
        assert_eq!(p.edges.len(), MAX_EDGES);
    }
}
