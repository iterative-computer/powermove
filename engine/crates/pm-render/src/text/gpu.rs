//! Text rasters on the GPU: exact float coverage pages and color glyph images,
//! painted in order into float canvases
//! (`shaders/text.wgsl`), with a content-keyed raster cache.
//!
//! Coverage is written as R32Float by the shared area renderer in the same
//! command encoder as paint. Pages are returned to the pool after submission,
//! so later layers cannot overwrite coverage still in use. The CPU mode is
//! retained for raster comparisons.

use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Rect};
use pm_gpu::{Target, TargetPool, working_format};
use pm_protocol::color::{Precision, WorkingEncoding};
use pm_text::render::atlas::{self, Atlas, Cell};
use pm_text::render::scene::{ImagePixels, ItemContent, TextScene, blur_radius};
use pm_vector::render::scene::PixelRect;

use super::TextPlan;
use crate::coverage::{self, AreaCoverage};
use crate::pipelines::{UNIFORM_STRIDE, bytes_of};

/// The largest coverage page side.
pub const MAX_PAGE: u32 = 2048;
/// Rasters kept across frames (bytes of texel storage).
pub const CACHE_BYTES: u64 = 32 * 1024 * 1024;
const IMAGE_CACHE_BYTES: u64 = 8 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct ItemUniform {
    rect: [f32; 4],
    map: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
    inv0: [f32; 4],
    inv1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BlurUniform {
    width: u32,
    height: u32,
    radius: u32,
    axis: u32,
}

/// How coverage is produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Exact GPU area coverage in R32Float pages.
    Auto,
    /// CPU coverage for reference comparisons.
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
    area: AreaCoverage,
    batch: coverage::Batch,
    cpu: pm_text::render::cpu::Rasterizer,
    coverage: Coverage,
    item_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    paint: [crate::pipelines::LazyRenderPipeline; 2],
    blur_layout: wgpu::BindGroupLayout,
    blur: [crate::pipelines::LazyRenderPipeline; 2],
    images: HashMap<u64, (wgpu::Texture, wgpu::TextureView)>,
    image_bytes: u64,
    cache: HashMap<([u64; 2], Precision, WorkingEncoding), Cached>,
    cache_bytes: u64,
    tick: u64,
    pub stats: TextStats,
}

fn format_index(precision: Precision) -> usize {
    match precision {
        Precision::Unorm8 => 0,
        Precision::Float16 => 0,
        Precision::Float32 => 1,
    }
}

fn text_format(precision: Precision) -> wgpu::TextureFormat {
    working_format(match precision {
        Precision::Unorm8 => Precision::Float16,
        other => other,
    })
}

fn image_key(img: &ImagePixels, working: WorkingEncoding) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    working.hash(&mut h);
    match img {
        ImagePixels::Rgba8(i) => {
            (0u8, i.width, i.height).hash(&mut h);
            i.data.hash(&mut h);
        }
        ImagePixels::WorkingF32(i) => {
            (1u8, i.width, i.height).hash(&mut h);
            for v in &i.data {
                v.to_bits().hash(&mut h);
            }
        }
        ImagePixels::Colr(program) => {
            (2u8, program.content_key()).hash(&mut h);
        }
    }
    h.finish()
}

fn to_working(mut rgba: [f32; 4], working: WorkingEncoding) -> [f32; 4] {
    if working == WorkingEncoding::LinearLight {
        let a = rgba[3];
        if a > 0.0 {
            for c in &mut rgba[..3] {
                *c = pm_color::transfer::display_decode_f32(*c / a) * a;
            }
        } else {
            rgba[..3].fill(0.0);
        }
    }
    rgba
}

impl TextGpu {
    pub fn new(device: &wgpu::Device) -> TextGpu {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render text"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/text.wgsl").into()),
        });
        let blur_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render text Gaussian"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/text_blur.wgsl").into()),
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
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm text Gaussian"),
            entries: &[
                uniform(0, false, size_of::<BlurUniform>() as u64),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let blur_pipeline_layout = layout(&[&blur_layout]);
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
        let pipeline = |module: &wgpu::ShaderModule,
                        layout: &wgpu::PipelineLayout,
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
        let formats = [Precision::Float16, Precision::Float32].map(working_format);
        let paint =
            formats.map(|f| pipeline(&module, &paint_layout, "vs", "fs_paint", f, Some(over)));
        let blur = formats.map(|f| {
            pipeline(
                &blur_module,
                &blur_pipeline_layout,
                "vs_blur",
                "fs_blur",
                f,
                None,
            )
        });
        TextGpu {
            area: AreaCoverage::new(device),
            batch: coverage::Batch::default(),
            cpu: pm_text::render::cpu::Rasterizer::new(),
            coverage: Coverage::Auto,
            item_layout,
            texture_layout,
            paint,
            blur_layout,
            blur,
            images: HashMap::new(),
            image_bytes: 0,
            cache: HashMap::new(),
            cache_bytes: 0,
            tick: 0,
            stats: TextStats::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn built_count(&self) -> usize {
        self.paint
            .iter()
            .filter(|pipeline| pipeline.is_built())
            .count()
            + self
                .blur
                .iter()
                .filter(|pipeline| pipeline.is_built())
                .count()
    }

    #[cfg(test)]
    pub(crate) fn probe_all(&self) -> Vec<(String, std::time::Duration)> {
        let mut times = Vec::new();
        for (name, pipeline) in [
            ("paint f16", &self.paint[0]),
            ("paint f32", &self.paint[1]),
            ("Gaussian f16", &self.blur[0]),
            ("Gaussian f32", &self.blur[1]),
        ] {
            let start = std::time::Instant::now();
            pipeline.get();
            times.push((format!("text {name}"), start.elapsed()));
        }
        times
    }

    pub fn set_coverage(&mut self, coverage: Coverage) {
        self.coverage = coverage;
        self.clear();
    }

    /// Marks a new submitted frame so area buffers can be reused without
    /// aliasing two text pages recorded in the same command encoder.
    pub fn begin_frame(&mut self) {
        self.area.begin_frame();
        self.batch.trim_scratch(8 * 1024 * 1024);
    }

    /// Drops cached rasters and color glyph textures.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.cache_bytes = 0;
        self.images.clear();
        self.image_bytes = 0;
    }

    pub fn cached_bytes(&self) -> u64 {
        self.cache_bytes
    }

    fn image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &ImagePixels,
        working: WorkingEncoding,
    ) -> wgpu::TextureView {
        let key = image_key(img, working);
        let (width, height) = img.dimensions();
        if let Some((_, view)) = self.images.get(&key) {
            return view.clone();
        }
        // Decode premultiplied E into the project's working domain before
        // area filtering. The original texels are retained so the shader can
        // integrate each inverse output pixel against source texel squares.
        let base = if let ImagePixels::Colr(program) = img {
            let raster = program.rasterize(working);
            debug_assert_eq!((raster.width, raster.height), (width, height));
            raster.data
        } else {
            let mut base = vec![0.0f32; width as usize * height as usize * 4];
            for (index, rgba) in base.chunks_exact_mut(4).enumerate() {
                let o = index * 4;
                let source: [f32; 4] = match img {
                    ImagePixels::Rgba8(i) => {
                        std::array::from_fn(|c| f32::from(i.data[o + c]) / 255.0)
                    }
                    ImagePixels::WorkingF32(i) => i.data[o..o + 4].try_into().expect("RGBA pixel"),
                    ImagePixels::Colr(_) => unreachable!(),
                };
                let pixel = match img {
                    ImagePixels::Rgba8(_) => to_working(source, working),
                    ImagePixels::WorkingF32(_) => source,
                    ImagePixels::Colr(_) => unreachable!(),
                };
                rgba.copy_from_slice(&pixel);
            }
            base
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm text color glyph source"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let data: Vec<u8> = base.iter().flat_map(|v| v.to_ne_bytes()).collect();
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
                bytes_per_row: Some(width * 16),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let bytes = u64::from(width) * u64::from(height) * 16;
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        if bytes <= IMAGE_CACHE_BYTES / 2 {
            if self.image_bytes + bytes > IMAGE_CACHE_BYTES {
                self.images.clear();
                self.image_bytes = 0;
            }
            self.image_bytes += bytes;
            self.images.insert(key, (texture, view.clone()));
        }
        view
    }

    /// Renders the coverage pages of `atlas` into pool targets (pushed to
    /// `taken`, returned after the frame is submitted).
    fn pages(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        taken: &mut Vec<Target>,
        scene: &TextScene,
        atlas: &Atlas,
    ) -> Result<Vec<wgpu::TextureView>, String> {
        let mut views = Vec::with_capacity(atlas.pages.len());
        for (page, &(w, h)) in atlas.pages.iter().enumerate() {
            let use_gpu = self.coverage == Coverage::Auto;
            // The atlas cells keep their exact logical coordinates. Only the
            // R32 target allocation is bucketed so a growing text raster can
            // reuse the same pool texture across several adjacent sizes.
            // Paint loads integer cell texels and never samples the spare area.
            let (target_w, target_h) = if use_gpu {
                (w.max(1).next_power_of_two(), h.max(1).next_power_of_two())
            } else {
                (w.max(1), h.max(1))
            };
            let target = pool.take(
                device,
                target_w,
                target_h,
                if use_gpu {
                    wgpu::TextureFormat::R32Float
                } else {
                    wgpu::TextureFormat::R8Unorm
                },
                wgpu::TextureUsages::TEXTURE_BINDING
                    | if use_gpu {
                        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::COPY_DST
                    },
            );
            self.stats.pages += 1;
            if use_gpu {
                let entries = area_items(scene, atlas, page, self.cpu.coverage_cache())?;
                let items: Vec<_> = entries
                    .iter()
                    .map(|entry| coverage::Item {
                        path: &entry.path,
                        translation: entry.translation,
                        bounds: entry.bounds,
                        at: entry.at,
                    })
                    .collect();
                self.batch.rebuild(&items);
                self.area
                    .encode_reusing(device, queue, encoder, &self.batch, Some(&target.view));
            } else {
                let (pw, ph, data) = self.cpu.render_page_r8(scene, atlas, page);
                self.stats.cpu_pages += 1;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &target.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(pw),
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
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            }],
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
        working: WorkingEncoding,
    ) -> Result<wgpu::TextureView, String> {
        self.tick += 1;
        if let Some(c) = self.cache.get_mut(&(plan.key, precision, working)) {
            c.used = self.tick;
            self.stats.cache_hits += 1;
            pm_perf::bump(pm_perf::Counter::TextCacheHits);
            return Ok(c.target.view.clone());
        }
        pm_perf::bump(pm_perf::Counter::TextCacheMisses);
        let scene = &plan.scene;
        let atlas = atlas::pack(scene, MAX_PAGE);
        let pages = self.pages(device, queue, encoder, pool, taken, scene, &atlas)?;
        let (tw, th) = (plan.width(), plan.height());
        let format = text_format(precision);
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
        let mut images: HashMap<usize, wgpu::TextureView> = HashMap::new();
        for (i, item) in scene.items.iter().enumerate() {
            if let ItemContent::Image { image, .. } = &item.content {
                images.insert(i, self.image(device, queue, image, working));
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
                    working,
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
            let draws = self.draws(scene, start..i, crop, (tw, th), &cells, &images, working);
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
        self.insert(plan, precision, working, raster.clone());
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

    fn insert(
        &mut self,
        plan: &TextPlan,
        precision: Precision,
        working: WorkingEncoding,
        target: Target,
    ) {
        let bytes = target.bytes();
        if bytes > CACHE_BYTES / 2 {
            return;
        }
        self.trim(CACHE_BYTES.saturating_sub(bytes));
        self.cache_bytes += bytes;
        self.cache.insert(
            (plan.key, precision, working),
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
        images: &HashMap<usize, wgpu::TextureView>,
        working: WorkingEncoding,
    ) -> Vec<Draw> {
        let (tw, th) = (size.0 as f32, size.1 as f32);
        let mut out = Vec::new();
        for i in range {
            let item = &scene.items[i];
            match &item.content {
                ItemContent::Coverage { .. } => {
                    let a = item.color[3] * item.opacity;
                    let rgb = pm_color::linear::color_to_working(
                        [item.color[0], item.color[1], item.color[2]],
                        working,
                    );
                    let color = [
                        (rgb[0] * a) as f32,
                        (rgb[1] * a) as f32,
                        (rgb[2] * a) as f32,
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
                                ..Default::default()
                            },
                        });
                    }
                }
                ItemContent::Image { transform, .. } => {
                    if !images.contains_key(&i) {
                        continue;
                    }
                    let b: Rect = item.bounds.inflate(1.0, 1.0);
                    let rect = [
                        (b.x0.floor() - origin[0]) as f32,
                        (b.y0.floor() - origin[1]) as f32,
                        (b.x1.ceil() - origin[0]) as f32,
                        (b.y1.ceil() - origin[1]) as f32,
                    ];
                    // Target px → device px → image px → padded image px.
                    let inv = transform.inverse() * Affine::translate((origin[0], origin[1]));
                    let [a, bb, c, d, e, f] = inv.as_coeffs();
                    // Any source-axis footprint wider than one texel needs
                    // area integration, including mixed shrink/enlarge maps.
                    let xx = a * a + c * c;
                    let xy = a * bb + c * d;
                    let yy = bb * bb + d * d;
                    let major =
                        ((xx + yy + ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt()) * 0.5).sqrt();
                    let downscale = major > 1.0;
                    let alpha = (item.color[3] * item.opacity) as f32;
                    out.push(Draw {
                        page: None,
                        image: Some(i),
                        u: ItemUniform {
                            rect,
                            color: [0.0, 0.0, 0.0, alpha],
                            params: [1.0, if downscale { 1.0 } else { 0.0 }, tw, th],
                            inv0: [a as f32, c as f32, e as f32, 0.0],
                            inv1: [bb as f32, d as f32, f as f32, 0.0],
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
        images: &HashMap<usize, wgpu::TextureView>,
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
            .map(|i| (i, self.source_bind(device, &images[&i])))
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
        pass.set_pipeline(self.paint[format_index(precision)].get());
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

    /// A draw painted alone, convolved with the pixel-integrated Gaussian,
    /// then composited into the float text target.
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
        images: &HashMap<usize, wgpu::TextureView>,
        pages: &[wgpu::TextureView],
        raster: &Target,
        precision: Precision,
        working: WorkingEncoding,
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
            ItemContent::Image { .. } => Some(item.bounds.inflate(1.0, 1.0)),
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
            text_format(precision),
            pm_gpu::target::working_usage(),
        );
        let draws = self.draws(
            scene,
            index..index + 1,
            [rx0, ry0],
            (lw, lh),
            cells,
            images,
            working,
        );
        let mut fresh = false;
        self.pass(
            device, queue, encoder, &a, &draws, pages, images, precision, &mut fresh,
        );
        let weights = gaussian_kernel(item.blur);
        let tmp = pool.take(
            device,
            lw,
            lh,
            text_format(precision),
            pm_gpu::target::working_usage(),
        );
        let filtered = pool.take(
            device,
            lw,
            lh,
            text_format(precision),
            pm_gpu::target::working_usage(),
        );
        self.blur_pass(device, queue, encoder, &a, &tmp, &weights, 0, precision);
        self.blur_pass(
            device, queue, encoder, &tmp, &filtered, &weights, 1, precision,
        );
        let (tw, th) = (plan.width() as f32, plan.height() as f32);
        let x0 = (rx0 - crop.x0).max(0.0);
        let y0 = (ry0 - crop.y0).max(0.0);
        let x1 = (rx0 + f64::from(lw) - crop.x0).min(f64::from(plan.width()));
        let y1 = (ry0 + f64::from(lh) - crop.y0).min(f64::from(plan.height()));
        let copy = Draw {
            page: Some(0),
            image: None,
            u: ItemUniform {
                rect: [x0 as f32, y0 as f32, x1 as f32, y1 as f32],
                map: [(crop.x0 - rx0) as f32, (crop.y0 - ry0) as f32, 0.0, 0.0],
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
        taken.extend([a, tmp, filtered]);
    }

    fn blur_pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        target: &Target,
        weights: &[f32],
        axis: u32,
        precision: Precision,
    ) {
        let (width, height) = (target.width(), target.height());
        let uniform = BlurUniform {
            width,
            height,
            radius: (weights.len() / 2) as u32,
            axis,
        };
        let ub = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm text blur uniform"),
            size: size_of::<BlurUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ub, 0, bytes_of(&uniform));
        let wb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm text Gaussian weights"),
            size: std::mem::size_of_val(weights) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let weight_bytes: Vec<u8> = weights.iter().flat_map(|w| w.to_ne_bytes()).collect();
        queue.write_buffer(&wb, 0, &weight_bytes);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm text Gaussian"),
            layout: &self.blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wb.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm text Gaussian"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
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
        pass.set_pipeline(self.blur[format_index(precision)].get());
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..4, 0..1);
    }
}

fn gaussian_kernel(sigma: f64) -> Vec<f32> {
    pm_text::render::paint::gaussian(sigma)
        .into_iter()
        .map(|w| w as f32)
        .collect()
}

struct Draw {
    /// Page index (coverage) or layer (kind 2, page 0 of the slice passed).
    page: Option<usize>,
    image: Option<usize>,
    u: ItemUniform,
}

struct AreaItem {
    path: Arc<BezPath>,
    bounds: PixelRect,
    at: [u32; 2],
    translation: [f64; 2],
}

/// Device-space outlines and the atlas cell each outline covers. The cache
/// holds the f64 outline after its linear transform; glyph placement stays
/// f64 until area rasterization.
fn area_items(
    scene: &TextScene,
    atlas: &Atlas,
    page: usize,
    cache: &mut pm_text::render::scene::CoveragePathCache,
) -> Result<Vec<AreaItem>, String> {
    let mut out = Vec::new();
    for cell in atlas.cells.iter().filter(|c| c.page == page) {
        let ItemContent::Coverage { paths, .. } = &scene.items[cell.item].content else {
            continue;
        };
        let (path, translation) = if let [one] = paths.as_slice() {
            let [_, _, _, _, x, y] = one.transform.as_coeffs();
            (cache.get(one), [x, y])
        } else {
            let mut combined = BezPath::new();
            for p in paths {
                let mut part = cache.get(p).as_ref().clone();
                let [_, _, _, _, x, y] = p.transform.as_coeffs();
                part.apply_affine(Affine::translate((x, y)));
                for el in part.iter() {
                    combined.push(el);
                }
            }
            (Arc::new(combined), [0.0, 0.0])
        };
        let x = i32::try_from(cell.device_x).map_err(|_| "text cell x outside i32".to_owned())?;
        let y = i32::try_from(cell.device_y).map_err(|_| "text cell y outside i32".to_owned())?;
        out.push(AreaItem {
            path,
            bounds: PixelRect::new(x, y, cell.width, cell.height),
            at: [cell.x, cell.y],
            translation,
        });
    }
    Ok(out)
}

/// The CPU reference of a plan's raster (tests).
pub fn render_cpu(plan: &TextPlan) -> pm_text::render::paint::Image {
    let full = pm_text::render::paint::render_cpu(&plan.scene);
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
