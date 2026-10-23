//! Text rasters on the GPU: native Graphite primitives, platform glyph masks
//! and Vello coverage pages, painted in order into N32 or float canvases
//! (`shaders/text.wgsl`), with a content-keyed raster cache.
//!
//! Vello 0.11 renders only into `Rgba8Unorm` storage textures and submits
//! its own command buffers, so each coverage page is rendered (and
//! submitted) before the frame's encoder, which holds the paint passes.
//! Pages are taken from the frame's target pool and returned after the
//! frame is submitted, so a later layer of the same frame never overwrites
//! a page an earlier layer's paint pass still reads. When Vello cannot run
//! on the device, the same pages come from `vello_cpu`
//! (`pm_text::render::cpu`) and are uploaded.

use std::collections::HashMap;

use kurbo::{Affine, Rect};
use pm_gpu::{Target, TargetPool, working_format};
use pm_protocol::color::Precision;
use pm_text::render::atlas::{self, Atlas, Cell};
use pm_text::render::bitmap::Rgba8;
use pm_text::render::scene::{ItemContent, PathStyle, TextScene, blur_radius};

use super::TextPlan;
use crate::pipelines::{UNIFORM_STRIDE, bytes_of};

/// The largest coverage page side.
pub const MAX_PAGE: u32 = 4096;
/// Rasters kept across frames (bytes of texel storage).
pub const CACHE_BYTES: u64 = 192 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct ItemUniform {
    rect: [f32; 4],
    map: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
    inv0: [f32; 4],
    inv1: [f32; 4],
    isize: [f32; 4],
    /// The coverage curve, 17 values (20 slots).
    curve: [[f32; 4]; 5],
}

fn curve_slots(curve: &pm_text::render::profile::Curve) -> [[f32; 4]; 5] {
    let mut out = [[0.0f32; 4]; 5];
    for (i, v) in curve.iter().enumerate() {
        out[i / 4][i % 4] = *v;
    }
    out
}

/// How coverage is produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Vello on the GPU (the default), `vello_cpu` if Vello fails to start.
    Auto,
    /// `vello_cpu` only (the CPU fallback, and tests comparing the two).
    Cpu,
}

/// Counters for tests and performance reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStats {
    pub painted: u64,
    pub cache_hits: u64,
    pub pages: u64,
    pub cpu_pages: u64,
}

struct Cached {
    target: Target,
    used: u64,
}

pub struct TextGpu {
    graphite: Option<super::graphite::Gpu>,
    glyph_atlas: super::graphite::atlas::Atlas,
    vello: Option<vello::Renderer>,
    vello_failed: Option<String>,
    coverage: Coverage,
    item_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    paint: [wgpu::RenderPipeline; 2],
    paint_n32: wgpu::RenderPipeline,
    paint_bgra: wgpu::RenderPipeline,
    filter: crate::composite::path_masks::PathMaskPipelines,
    sampler: wgpu::Sampler,
    images: HashMap<u64, (wgpu::Texture, wgpu::TextureView)>,
    cache: HashMap<([u64; 2], Precision), Cached>,
    cache_bytes: u64,
    tick: u64,
    pub stats: TextStats,
}

fn format_index(precision: Precision) -> usize {
    match precision {
        Precision::Float16 => 0,
        Precision::Float32 => 1,
    }
}

fn image_key(img: &Rgba8) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (img.width, img.height).hash(&mut h);
    img.data.hash(&mut h);
    h.finish()
}

impl TextGpu {
    pub fn new(device: &wgpu::Device) -> TextGpu {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render text"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/text.wgsl").into()),
        });
        let uniform = |binding, dynamic, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let item_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm text item"),
            entries: &[uniform(0, true, size_of::<ItemUniform>() as u64)],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm text source"),
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
        let layout = |groups: &[&wgpu::BindGroupLayout]| {
            let groups: Vec<Option<&wgpu::BindGroupLayout>> =
                groups.iter().map(|g| Some(*g)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &groups,
                immediate_size: 0,
            })
        };
        let paint_layout = layout(&[&item_layout, &texture_layout]);
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
        let pipeline = |layout: &wgpu::PipelineLayout,
                        vs: &str,
                        fs: &str,
                        format: wgpu::TextureFormat,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(layout),
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
        };
        let formats = [Precision::Float16, Precision::Float32].map(working_format);
        let paint = formats.map(|f| pipeline(&paint_layout, "vs", "fs_paint", f, Some(over)));
        let paint_n32 = pipeline(
            &paint_layout,
            "vs",
            "fs_paint",
            wgpu::TextureFormat::Rgba8Unorm,
            Some(over),
        );
        let paint_bgra = pipeline(
            &paint_layout,
            "vs",
            "fs_paint",
            wgpu::TextureFormat::Bgra8Unorm,
            Some(over),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pm text bilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        TextGpu {
            graphite: None,
            glyph_atlas: super::graphite::atlas::Atlas::new(),
            vello: None,
            vello_failed: None,
            coverage: Coverage::Auto,
            item_layout,
            texture_layout,
            paint,
            paint_n32,
            paint_bgra,
            filter: crate::composite::path_masks::PathMaskPipelines::rgba(device),
            sampler,
            images: HashMap::new(),
            cache: HashMap::new(),
            cache_bytes: 0,
            tick: 0,
            stats: TextStats::default(),
        }
    }

    pub fn set_coverage(&mut self, coverage: Coverage) {
        self.coverage = coverage;
        self.clear();
    }

    /// Why Vello is not used, when it failed to start.
    pub fn vello_failure(&self) -> Option<&str> {
        self.vello_failed.as_deref()
    }

    /// Drops cached rasters and color glyph textures.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.cache_bytes = 0;
        self.images.clear();
        self.glyph_atlas = super::graphite::atlas::Atlas::new();
    }

    pub fn cached_bytes(&self) -> u64 {
        self.cache_bytes
    }

    fn vello(&mut self, device: &wgpu::Device) -> Option<&mut vello::Renderer> {
        if self.coverage == Coverage::Cpu || self.vello_failed.is_some() {
            return None;
        }
        if self.vello.is_none() {
            let options = vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1),
                pipeline_cache: None,
            };
            match vello::Renderer::new(device, options) {
                Ok(r) => self.vello = Some(r),
                Err(e) => {
                    self.vello_failed = Some(e.to_string());
                    return None;
                }
            }
        }
        self.vello.as_mut()
    }

    fn image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &Rgba8,
    ) -> (wgpu::TextureView, [f32; 2]) {
        let key = image_key(img);
        let size = [(img.width + 2) as f32, (img.height + 2) as f32];
        if let Some((_, v)) = self.images.get(&key) {
            return (v.clone(), size);
        }
        // A one-pixel transparent border: bilinear filtering at the edges
        // then fades to transparent, as the CPU reference samples.
        let (w, h) = (img.width + 2, img.height + 2);
        let mut data = vec![0u8; w as usize * h as usize * 4];
        for y in 0..img.height as usize {
            let src = &img.data[y * img.width as usize * 4..(y + 1) * img.width as usize * 4];
            let o = ((y + 1) * w as usize + 1) * 4;
            data[o..o + src.len()].copy_from_slice(src);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm text color glyph"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        if self.images.len() > 1024 {
            self.images.clear();
        }
        self.images.insert(key, (texture, view.clone()));
        (view, size)
    }

    /// Renders the coverage pages of `atlas` into pool targets (pushed to
    /// `taken`, returned after the frame is submitted).
    fn pages(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &mut TargetPool,
        taken: &mut Vec<Target>,
        scene: &TextScene,
        atlas: &Atlas,
    ) -> Result<Vec<wgpu::TextureView>, String> {
        let mut views = Vec::with_capacity(atlas.pages.len());
        for (page, &(w, h)) in atlas.pages.iter().enumerate() {
            let target = pool.take(
                device,
                w.max(1),
                h.max(1),
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
            );
            self.stats.pages += 1;
            let drawn = match self.vello(device) {
                Some(renderer) => {
                    let s = vello_page(scene, atlas, page);
                    renderer
                        .render_to_texture(
                            device,
                            queue,
                            &s,
                            &target.view,
                            &vello::RenderParams {
                                base_color: vello::peniko::Color::TRANSPARENT,
                                width: w.max(1),
                                height: h.max(1),
                                antialiasing_method: vello::AaConfig::Area,
                            },
                        )
                        .map_err(|e| format!("Vello coverage: {e}"))
                        .is_ok()
                }
                None => false,
            };
            if !drawn {
                let (pw, ph, data) = pm_text::render::cpu::render_page(scene, atlas, page);
                self.stats.cpu_pages += 1;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &target.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(pw * 4),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: pw,
                        height: ph,
                        depth_or_array_layers: 1,
                    },
                );
            }
            views.push(target.view.clone());
            taken.push(target);
        }
        Ok(views)
    }

    fn source_bind(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm text source"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// The painted raster of `plan`: a top-down working texture of the
    /// plan's crop, premultiplied.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        taken: &mut Vec<Target>,
        plan: &TextPlan,
        precision: Precision,
    ) -> Result<wgpu::TextureView, String> {
        self.tick += 1;
        if let Some(c) = self.cache.get_mut(&(plan.key, precision)) {
            c.used = self.tick;
            self.stats.cache_hits += 1;
            pm_perf::bump(pm_perf::Counter::TextCacheHits);
            return Ok(c.target.view.clone());
        }
        pm_perf::bump(pm_perf::Counter::TextCacheMisses);
        let scene = &plan.scene;
        let atlas = atlas::pack(scene, MAX_PAGE);
        let pages = self.pages(device, queue, pool, taken, scene, &atlas)?;
        let (tw, th) = (plan.width(), plan.height());
        let format = if plan.canvas_n32 {
            // Chromium SharedImageFormat::N32Format on desktop platforms.
            wgpu::TextureFormat::Bgra8Unorm
        } else {
            working_format(precision)
        };
        let raster = Target::new(
            device,
            "pm text raster",
            tw,
            th,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let crop = [f64::from(plan.crop[0]), f64::from(plan.crop[1])];
        let mut cells: HashMap<usize, Vec<Cell>> = HashMap::new();
        for c in &atlas.cells {
            cells.entry(c.item).or_default().push(*c);
        }
        // Color glyph textures (uploaded before the passes are recorded).
        let mut images: HashMap<usize, (wgpu::TextureView, [f32; 2])> = HashMap::new();
        self.glyph_atlas.begin_run();
        for (i, item) in scene.items.iter().enumerate() {
            if let ItemContent::GlyphMask { mask, image, .. } = &item.content {
                // Direct A8 and SDF glyphs share Skia's atlas allocator. Even
                // when the direct mask uses its cached paint texture, its
                // allocation affects SDF UV interpolation and rounding.
                self.glyph_atlas
                    .reserve_mask(device, mask, image_key(image));
            }
            // A full Canvas viewport may exceed Metal's texture limit even
            // when its ink crop fits. Keep the existing CPU primitive image
            // for those canvases; never shrink the viewport and change AA.
            if let ItemContent::Graphite { raster, image, .. } = &item.content
                && raster.width <= device.limits().max_texture_dimension_2d
                && raster.height <= device.limits().max_texture_dimension_2d
            {
                let native = self
                    .graphite
                    .get_or_insert_with(|| super::graphite::Gpu::new(device, queue));
                let drawn = native.render(
                    device,
                    queue,
                    encoder,
                    pool,
                    taken,
                    raster,
                    &mut self.glyph_atlas,
                );
                // A distance field beyond an atlas plot keeps the CPU
                // primitive image of the same draws.
                let drawn = match drawn {
                    Some(drawn) => drawn,
                    None => self.image(device, queue, image),
                };
                images.insert(i, drawn);
            } else if let ItemContent::Image { image, .. }
            | ItemContent::GlyphMask { image, .. }
            | ItemContent::Graphite { image, .. } = &item.content
            {
                images.insert(i, self.image(device, queue, image));
            }
        }
        let mut cleared = false;
        let mut i = 0;
        while i < scene.items.len() {
            if scene.items[i].blur > 0.0 {
                self.blurred(
                    device,
                    queue,
                    encoder,
                    pool,
                    taken,
                    plan,
                    i,
                    &cells,
                    &images,
                    &pages,
                    &raster,
                    precision,
                    &mut cleared,
                );
                i += 1;
                continue;
            }
            // A run of unblurred draws in one pass.
            let start = i;
            while i < scene.items.len() && scene.items[i].blur <= 0.0 {
                i += 1;
            }
            let draws = self.draws(scene, start..i, crop, (tw, th), &cells, &images);
            self.pass(
                device,
                queue,
                encoder,
                &raster,
                &draws,
                &pages,
                &images,
                precision,
                &mut cleared,
            );
        }
        if !cleared {
            self.pass(
                device,
                queue,
                encoder,
                &raster,
                &[],
                &pages,
                &images,
                precision,
                &mut cleared,
            );
        }
        self.stats.painted += 1;
        self.insert(plan, precision, raster.clone());
        Ok(raster.view)
    }

    /// Evicts least recently used rasters until the cache holds at most
    /// `bytes` (lease pressure).
    pub fn trim(&mut self, bytes: u64) {
        while self.cache_bytes > bytes {
            let Some((&k, _)) = self.cache.iter().min_by_key(|(_, c)| c.used) else {
                break;
            };
            // Dropped, not destroyed: a command encoder of the frame being
            // recorded may still read it (wgpu keeps it alive until then).
            if let Some(c) = self.cache.remove(&k) {
                self.cache_bytes = self.cache_bytes.saturating_sub(c.target.bytes());
            }
        }
    }

    fn insert(&mut self, plan: &TextPlan, precision: Precision, target: Target) {
        let bytes = target.bytes();
        if bytes > CACHE_BYTES / 2 {
            return;
        }
        self.trim(CACHE_BYTES.saturating_sub(bytes));
        self.cache_bytes += bytes;
        self.cache.insert(
            (plan.key, precision),
            Cached {
                target,
                used: self.tick,
            },
        );
    }

    /// The draw uniforms of items `range`, in a target whose pixel (0, 0)
    /// is device pixel `origin`.
    fn draws(
        &self,
        scene: &TextScene,
        range: std::ops::Range<usize>,
        origin: [f64; 2],
        size: (u32, u32),
        cells: &HashMap<usize, Vec<Cell>>,
        images: &HashMap<usize, (wgpu::TextureView, [f32; 2])>,
    ) -> Vec<Draw> {
        let (tw, th) = (size.0 as f32, size.1 as f32);
        let mut out = Vec::new();
        for i in range {
            let item = &scene.items[i];
            match &item.content {
                ItemContent::Coverage { curve, .. } => {
                    let a = item.color[3] * item.opacity;
                    let color = [
                        (item.color[0] * a) as f32,
                        (item.color[1] * a) as f32,
                        (item.color[2] * a) as f32,
                        a as f32,
                    ];
                    for c in cells.get(&i).map(Vec::as_slice).unwrap_or(&[]) {
                        let x0 = c.device_x as f64 - origin[0];
                        let y0 = c.device_y as f64 - origin[1];
                        out.push(Draw {
                            page: Some(c.page),
                            image: None,
                            u: ItemUniform {
                                rect: [
                                    x0 as f32,
                                    y0 as f32,
                                    (x0 + f64::from(c.width)) as f32,
                                    (y0 + f64::from(c.height)) as f32,
                                ],
                                map: [
                                    (f64::from(c.x) - x0) as f32,
                                    (f64::from(c.y) - y0) as f32,
                                    0.0,
                                    0.0,
                                ],
                                color,
                                params: [0.0, 0.0, tw, th],
                                curve: curve_slots(curve),
                                ..Default::default()
                            },
                        });
                    }
                }
                ItemContent::Image { transform, .. }
                | ItemContent::GlyphMask { transform, .. }
                | ItemContent::Graphite { transform, .. } => {
                    let Some((_, isize)) = images.get(&i) else {
                        continue;
                    };
                    let b: Rect = item.bounds.inflate(1.0, 1.0);
                    let rect = [
                        (b.x0.floor() - origin[0]) as f32,
                        (b.y0.floor() - origin[1]) as f32,
                        (b.x1.ceil() - origin[0]) as f32,
                        (b.y1.ceil() - origin[1]) as f32,
                    ];
                    // Target px → device px → image px → padded image px.
                    let inv = Affine::translate((1.0, 1.0))
                        * transform.inverse()
                        * Affine::translate((origin[0], origin[1]));
                    let [a, bb, c, d, e, f] = inv.as_coeffs();
                    let mask = matches!(item.content, ItemContent::GlyphMask { .. });
                    let alpha = (item.color[3] * item.opacity) as f32;
                    out.push(Draw {
                        page: None,
                        image: Some(i),
                        u: ItemUniform {
                            rect,
                            color: if mask {
                                [
                                    item.color[0] as f32 * alpha,
                                    item.color[1] as f32 * alpha,
                                    item.color[2] as f32 * alpha,
                                    alpha,
                                ]
                            } else {
                                [0.0, 0.0, 0.0, alpha]
                            },
                            params: [if mask { 3.0 } else { 1.0 }, 1.0, tw, th],
                            inv0: [a as f32, c as f32, e as f32, 0.0],
                            inv1: [bb as f32, d as f32, f as f32, 0.0],
                            isize: [isize[0], isize[1], 0.0, 0.0],
                            ..Default::default()
                        },
                    });
                }
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        draws: &[Draw],
        pages: &[wgpu::TextureView],
        images: &HashMap<usize, (wgpu::TextureView, [f32; 2])>,
        precision: Precision,
        cleared: &mut bool,
    ) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm text items"),
            size: (draws.len().max(1) as u64) * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut data = vec![0u8; draws.len().max(1) * UNIFORM_STRIDE as usize];
        for (k, d) in draws.iter().enumerate() {
            let at = k * UNIFORM_STRIDE as usize;
            data[at..at + size_of::<ItemUniform>()].copy_from_slice(bytes_of(&d.u));
        }
        queue.write_buffer(&buffer, 0, &data);
        let item_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm text items"),
            layout: &self.item_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<ItemUniform>() as u64),
                }),
            }],
        });
        let page_binds: Vec<wgpu::BindGroup> =
            pages.iter().map(|v| self.source_bind(device, v)).collect();
        let image_binds: HashMap<usize, wgpu::BindGroup> = draws
            .iter()
            .filter_map(|d| d.image)
            .map(|i| (i, self.source_bind(device, &images[&i].0)))
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm text paint"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if *cleared {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        *cleared = true;
        pass.set_pipeline(match target.format() {
            wgpu::TextureFormat::Rgba8Unorm => &self.paint_n32,
            wgpu::TextureFormat::Bgra8Unorm => &self.paint_bgra,
            _ => &self.paint[format_index(precision)],
        });
        for (k, d) in draws.iter().enumerate() {
            let bind = match (d.page, d.image) {
                (Some(p), _) => &page_binds[p],
                (None, Some(i)) => &image_binds[&i],
                (None, None) => continue,
            };
            pass.set_bind_group(0, &item_bind, &[(k as u64 * UNIFORM_STRIDE) as u32]);
            pass.set_bind_group(1, bind, &[]);
            pass.draw(0..4, 0..1);
        }
    }

    /// A draw painted into an N32 save layer, filtered through the shared
    /// Canvas2D/Graphite image-filter executor, then composited.
    #[allow(clippy::too_many_arguments)]
    fn blurred(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        taken: &mut Vec<Target>,
        plan: &TextPlan,
        index: usize,
        cells: &HashMap<usize, Vec<Cell>>,
        images: &HashMap<usize, (wgpu::TextureView, [f32; 2])>,
        pages: &[wgpu::TextureView],
        raster: &Target,
        precision: Precision,
        cleared: &mut bool,
    ) {
        let scene = &plan.scene;
        let item = &scene.items[index];
        let r = blur_radius(item.blur) as f64;
        // The draw's own pixels, grown by the kernel, within what can reach
        // the raster.
        let own = match &item.content {
            ItemContent::Coverage { .. } => {
                let mut u: Option<Rect> = None;
                for c in cells.get(&index).map(Vec::as_slice).unwrap_or(&[]) {
                    let rc = Rect::new(
                        c.device_x as f64,
                        c.device_y as f64,
                        (c.device_x + i64::from(c.width)) as f64,
                        (c.device_y + i64::from(c.height)) as f64,
                    );
                    u = Some(u.map_or(rc, |a| a.union(rc)));
                }
                u
            }
            ItemContent::Image { .. }
            | ItemContent::GlyphMask { .. }
            | ItemContent::Graphite { .. } => Some(item.bounds.inflate(1.0, 1.0)),
        };
        let Some(own) = own else { return };
        let crop = Rect::new(
            f64::from(plan.crop[0]),
            f64::from(plan.crop[1]),
            f64::from(plan.crop[2]),
            f64::from(plan.crop[3]),
        );
        let reach = own.inflate(r, r).intersect(crop.inflate(r, r));
        if reach.width() <= 0.0 || reach.height() <= 0.0 {
            return;
        }
        let rx0 = reach.x0.floor();
        let ry0 = reach.y0.floor();
        let lw = (reach.x1.ceil() - rx0) as u32;
        let lh = (reach.y1.ceil() - ry0) as u32;
        let a = pool.take(
            device,
            lw,
            lh,
            wgpu::TextureFormat::Rgba8Unorm,
            pm_gpu::target::working_usage(),
        );
        let draws = self.draws(scene, index..index + 1, [rx0, ry0], (lw, lh), cells, images);
        let mut fresh = false;
        self.pass(
            device, queue, encoder, &a, &draws, pages, images, precision, &mut fresh,
        );
        let filtered = self.filter.blur_rgba(
            device,
            queue,
            encoder,
            &a,
            [rx0 as i32, ry0 as i32],
            (scene.width, scene.height),
            plan.crop,
            item.blur as f32,
        );
        let (tw, th) = (plan.width() as f32, plan.height() as f32);
        let copy = Draw {
            page: Some(0),
            image: None,
            u: ItemUniform {
                rect: [0.0, 0.0, tw, th],
                map: [0.0; 4],
                params: [2.0, 1.0, tw, th],
                ..Default::default()
            },
        };
        self.pass(
            device,
            queue,
            encoder,
            raster,
            &[copy],
            std::slice::from_ref(&filtered.view),
            images,
            precision,
            cleared,
        );
        taken.push(a);
        // The encoder retains this non-pooled filter result until submission.
    }
}

struct Draw {
    /// Page index (coverage) or layer (kind 2, page 0 of the slice passed).
    page: Option<usize>,
    image: Option<usize>,
    u: ItemUniform,
}

/// Vello's scene for one coverage page: every cell's paths in white.
fn vello_page(scene: &TextScene, atlas: &Atlas, page: usize) -> vello::Scene {
    use vello::peniko::{Color, Fill};
    let mut s = vello::Scene::new();
    for cell in atlas.cells.iter().filter(|c| c.page == page) {
        let ItemContent::Coverage { paths, .. } = &scene.items[cell.item].content else {
            continue;
        };
        if cell.clip {
            s.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &cell.rect());
        }
        let to_page = cell.transform();
        for p in paths {
            let t = to_page * p.transform;
            match &p.style {
                PathStyle::Fill => s.fill(Fill::NonZero, t, Color::WHITE, None, &*p.path),
                PathStyle::Stroke(stroke) => s.stroke(stroke, t, Color::WHITE, None, &*p.path),
            }
        }
        if cell.clip {
            s.pop_layer();
        }
    }
    s
}

/// The CPU reference of a plan's raster (tests).
pub fn render_cpu(plan: &TextPlan) -> pm_text::render::paint::Image {
    let full = pm_text::render::paint::render_cpu_with_format(
        &plan.scene,
        Some(&|img, origin, canvas, sigma| {
            use pm_vector::canvas2d::filter::{BlurPlan, Rect};
            let rgba = img.to_rgba8();
            let data = if let Some(filter) = BlurPlan::new(canvas.0, canvas.1, sigma as f32) {
                filter.run_cpu_rgba_region(
                    &rgba,
                    Rect {
                        x0: origin[0],
                        y0: origin[1],
                        w: img.width,
                        h: img.height,
                    },
                    canvas.0,
                    canvas.1,
                )
            } else {
                let mut data = vec![0.0; (canvas.0 * canvas.1 * 4) as usize];
                let x0 = origin[0].max(0);
                let x1 = (origin[0] + img.width as i32).min(canvas.0 as i32);
                for y in origin[1].max(0)..(origin[1] + img.height as i32).min(canvas.1 as i32) {
                    for x in x0..x1 {
                        let src = (((y - origin[1]) as u32 * img.width + (x - origin[0]) as u32)
                            * 4) as usize;
                        let dst = ((y as u32 * canvas.0 + x as u32) * 4) as usize;
                        for c in 0..4 {
                            data[dst + c] = f32::from(rgba[src + c]) / 255.0;
                        }
                    }
                }
                data
            };
            pm_text::render::paint::Image {
                width: canvas.0,
                height: canvas.1,
                data,
            }
        }),
        plan.canvas_n32,
    );
    let (w, h) = (plan.width(), plan.height());
    let mut out = pm_text::render::paint::Image::new(w, h);
    for y in 0..h as usize {
        let sy = y + plan.crop[1] as usize;
        let src =
            &full.data[(sy * full.width as usize + plan.crop[0] as usize) * 4..][..w as usize * 4];
        out.data[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(src);
    }
    out
}
