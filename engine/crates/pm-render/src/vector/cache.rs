//! Resolving vector sources to textures, cached by content key, with every
//! byte accounted (section 19 "Memory and GPU": the render cache category of
//! the lease).
//!
//! - Level 1 rasters are computed on the CPU by `pm_vector::canvas2d` and
//!   uploaded as premultiplied `Rgba8Unorm`, the texture today's compositor
//!   samples (Standard color: the canvas's sRGB values are working values).
//! - Level 1 SVG images are Chromium's rasters at today's buckets
//!   (decision S1, [`super::svg_rasters`]), uploaded like level 1 rasters:
//!   [`VectorCache::resolve_svg`].
//! - Level 2 SVG images that are not plain vectors are drawn by resvg at
//!   output resolution and uploaded like level 1 rasters.
//! - Level 2 scenes get analytic area coverage from Vello's GPU compute
//!   renderer (or `vello_cpu` when Vello is unavailable or forced) in an
//!   8-bit atlas; the float paint pass then composites `paint × coverage`
//!   per item with premultiplied source-over at working precision
//!   (amendment 10).

use std::collections::HashMap;

use pm_protocol::color::Precision;
use pm_vector::canvas2d::shape::{raster_paths, raster_shape};
use pm_vector::gpu::GpuCoverage;
use pm_vector::path::PathLayer;
use pm_vector::render::cpu::render_coverage;
use pm_vector::render::scene::{AtlasLayout, CoverageScene, PixelRect};

use super::VectorSource;
use super::svg_rasters::{SvgMissPolicy, SvgRasterNeed, SvgRasterizer, SvgRasters};
use pm_protocol::svg_raster::SvgRasterKey;

/// A level 1 SVG raster's texture for a frame ([`VectorCache::resolve_svg`]).
pub enum SvgResolved {
    /// The bucket the frame asked for.
    Drawn(wgpu::TextureView),
    /// Preview: another bucket of the same content while the asked one is
    /// on its way.
    Substitute(wgpu::TextureView),
    /// Nothing to draw this frame (the miss is recorded).
    Missing,
}

/// The precision slot of level 1 SVG raster textures (always `Rgba8Unorm`).
const SVG_SLOT: u8 = u8::MAX;

/// Which coverage backend level 2 uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageBackend {
    /// Vello's GPU compute renderer, falling back to `vello_cpu` when it
    /// cannot be created on the device.
    Auto,
    /// Always `vello_cpu` (the CPU fallback path).
    Cpu,
}

struct Entry {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bytes: u64,
    /// The resolve tick of the last use (least recently used order).
    used: u64,
    /// The frame of the last use; entries of the current frame are its
    /// working set and are never evicted while it records.
    frame: u64,
}

/// Today's raster cache bounds (legacy/gl/raster.ts: `MAX` entries and the
/// `raster` budget's 192 MiB default).
pub const DEFAULT_MAX_ENTRIES: usize = 4096;
pub const DEFAULT_MAX_BYTES: u64 = 192 * 1024 * 1024;

/// The float paint pass.
struct Paint {
    pipeline_f16: wgpu::RenderPipeline,
    pipeline_f32: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

const PAINT_WGSL: &str = r#"
struct Globals { size: vec4<f32> };
@group(0) @binding(0) var coverage: texture_2d<f32>;
@group(0) @binding(1) var<uniform> globals: Globals;

struct In {
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) cell: vec2<f32>,
    @location(2) color: vec4<f32>,
};
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) cell: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
};

@vertex fn vs(v: In) -> Out {
    let corner = vec2<f32>(f32(v.vertex & 1u), f32(v.vertex >> 1u));
    let p = v.rect.xy + corner * v.rect.zw;
    var o: Out;
    o.position = vec4<f32>(p.x / globals.size.x * 2.0 - 1.0, 1.0 - p.y / globals.size.y * 2.0, 0.0, 1.0);
    o.cell = v.cell + corner * v.rect.zw;
    o.color = v.color;
    return o;
}

@fragment fn fs(i: Out) -> @location(0) vec4<f32> {
    let cov = textureLoad(coverage, vec2<i32>(floor(i.cell)), 0).a;
    return i.color * cov;
}
"#;

impl Paint {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm vector paint"),
            source: wgpu::ShaderSource::Wgsl(PAINT_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm vector paint"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm vector paint"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("pm vector paint"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 40,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32x4],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            pipeline_f16: make(wgpu::TextureFormat::Rgba16Float),
            pipeline_f32: make(wgpu::TextureFormat::Rgba32Float),
            layout,
        }
    }
}

/// The cache of vector textures.
pub struct VectorCache {
    entries: HashMap<([u8; 32], u8), Entry>,
    tick: u64,
    frame: u64,
    bytes: u64,
    max_bytes: u64,
    max_entries: usize,
    backend: CoverageBackend,
    gpu: Option<Option<GpuCoverage>>,
    paint: Option<Paint>,
    /// Which backend rendered the last level 2 scene (tests).
    pub last_backend: Option<&'static str>,
    /// Level 1 SVG rasters the renderers uploaded (decision S1).
    pub svg: SvgRasters,
}

impl Default for VectorCache {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            tick: 0,
            frame: 0,
            bytes: 0,
            max_bytes: DEFAULT_MAX_BYTES,
            max_entries: DEFAULT_MAX_ENTRIES,
            backend: CoverageBackend::Auto,
            gpu: None,
            paint: None,
            last_backend: None,
            svg: SvgRasters::default(),
        }
    }

    /// Selects the level 2 coverage backend; cached textures are dropped
    /// when it changes, so the next frame is rendered by the new one.
    pub fn set_backend(&mut self, backend: CoverageBackend) {
        if backend != self.backend {
            self.entries.clear();
            self.bytes = 0;
        }
        self.backend = backend;
    }

    /// Bytes of cached textures and of the stored level 1 SVG rasters.
    pub fn bytes(&self) -> u64 {
        self.bytes + self.svg.bytes()
    }

    /// Cached textures.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The cache's standing bound: at most `max_bytes` and `max_entries`
    /// outside the working set of the frame being recorded (a frame's own
    /// textures are never evicted while it records, so one frame may
    /// exceed the bound until the next one starts). Evicts at once.
    pub fn set_budget(&mut self, max_bytes: u64, max_entries: usize) {
        self.max_bytes = max_bytes;
        self.max_entries = max_entries.max(1);
        self.evict(self.max_bytes, self.max_entries, false);
    }

    /// The standing bound in bytes.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Starts a frame: the previous frame's textures become evictable, and
    /// the cache is brought within its bound (animated content misses on
    /// every frame, so this is what keeps playback and long exports
    /// bounded).
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.svg.begin_frame();
        self.evict(self.max_bytes, self.max_entries, false);
    }

    /// Evicts least recently used textures until at most `budget` bytes
    /// stay (lease pressure, between frames: every entry may go), then
    /// stored SVG rasters within what the textures leave.
    pub fn trim(&mut self, budget: u64) {
        self.evict(budget, self.max_entries, true);
        self.svg.evict(budget.saturating_sub(self.bytes));
    }

    /// Evicts least recently used entries until at most `bytes` and
    /// `entries` stay; the current frame's entries stay unless `all`.
    fn evict(&mut self, bytes: u64, entries: usize, all: bool) {
        if self.bytes <= bytes && self.entries.len() <= entries {
            return;
        }
        let frame = self.frame;
        let mut by_age: Vec<(u64, ([u8; 32], u8))> = self
            .entries
            .iter()
            .filter(|(_, e)| all || e.frame != frame)
            .map(|(k, e)| (e.used, *k))
            .collect();
        by_age.sort_unstable();
        for (_, k) in by_age {
            if self.bytes <= bytes && self.entries.len() <= entries {
                break;
            }
            // Dropping the texture is safe while recorded commands still
            // use it: wgpu keeps it alive until they complete.
            if let Some(e) = self.entries.remove(&k) {
                self.bytes -= e.bytes;
            }
        }
    }

    /// The texture of `source` (premultiplied working values), creating it
    /// on a miss. Level 1 textures are `Rgba8Unorm` top-down; level 2 ones
    /// are at the frame's precision.
    pub fn resolve(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &VectorSource,
        precision: Precision,
    ) -> Result<wgpu::TextureView, String> {
        self.tick += 1;
        let key = (source.key(), precision as u8);
        if let Some(e) = self.entries.get_mut(&key) {
            e.used = self.tick;
            e.frame = self.frame;
            pm_perf::bump(pm_perf::Counter::VectorCacheHits);
            return Ok(e.view.clone());
        }
        pm_perf::bump(pm_perf::Counter::VectorCacheMisses);
        let (texture, bytes) = match source {
            VectorSource::Shape {
                content,
                colors,
                scale,
            } => {
                let b = raster_shape(content, colors, *scale).map_err(|e| e.to_string())?;
                (
                    upload_rgba8(device, queue, b.width, b.height, &b.rgba),
                    u64::from(b.width) * u64::from(b.height) * 4,
                )
            }
            VectorSource::Paths {
                nodes,
                colors,
                scale,
            } => {
                let layer = PathLayer::new(nodes);
                let (b, _) = raster_paths(&layer, colors, *scale).map_err(|e| e.to_string())?;
                (
                    upload_rgba8(device, queue, b.width, b.height, &b.rgba),
                    u64::from(b.width) * u64::from(b.height) * 4,
                )
            }
            VectorSource::Scene { scene, rect } => {
                self.render_scene(device, queue, scene, *rect, precision)?
            }
            VectorSource::SvgImage { key, asset, path } => {
                // Through `resolve_svg` with the policy of a frame that waits.
                return match self.resolve_svg(
                    device,
                    queue,
                    key,
                    asset,
                    path,
                    SvgMissPolicy::Wait,
                    ("", ""),
                )? {
                    SvgResolved::Drawn(view) | SvgResolved::Substitute(view) => Ok(view),
                    SvgResolved::Missing => Err(format!(
                        "the SVG raster {}×{} has not arrived",
                        key.width, key.height
                    )),
                };
            }
            VectorSource::SvgRaster {
                path,
                transform,
                rect,
                clip,
                ..
            } => {
                let (doc, _) = super::svg::document(path)?;
                let clip = clip.map(|[x, y]| pm_vector::kurbo::Point::new(x, y));
                let b = doc.raster_device(
                    pm_vector::kurbo::Affine::new(*transform),
                    *rect,
                    Some(&clip),
                );
                (
                    upload_rgba8(device, queue, b.width, b.height, &b.rgba),
                    u64::from(b.width) * u64::from(b.height) * 4,
                )
            }
        };
        // Make room before inserting: earlier frames' entries go first, so
        // the bound holds within a frame too (a work item can be a whole
        // export).
        Ok(self.insert(key, texture, bytes))
    }

    /// Caches a new texture under `key`, making room first.
    fn insert(
        &mut self,
        key: ([u8; 32], u8),
        texture: wgpu::Texture,
        bytes: u64,
    ) -> wgpu::TextureView {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.evict(
            self.max_bytes.saturating_sub(bytes),
            self.max_entries.saturating_sub(1),
            false,
        );
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                _texture: texture,
                view: view.clone(),
                bytes,
                used: self.tick,
                frame: self.frame,
            },
        );
        view
    }

    /// A cached texture, marked used by this frame.
    fn touch(&mut self, key: &([u8; 32], u8)) -> Option<wgpu::TextureView> {
        let (tick, frame) = (self.tick, self.frame);
        self.entries.get_mut(key).map(|e| {
            e.used = tick;
            e.frame = frame;
            e.view.clone()
        })
    }

    /// The texture of a level 1 SVG raster (`Rgba8Unorm`, premultiplied,
    /// top-down: the bytes today's `texImage2D` of the bucket canvas
    /// stores). On a miss the need is recorded ([`SvgRasters::take_misses`])
    /// and, under [`SvgMissPolicy::Substitute`], the content's latest
    /// bucket stands in.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_svg(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &SvgRasterKey,
        asset: &str,
        path: &std::path::Path,
        policy: SvgMissPolicy,
        at: (&str, &str),
    ) -> Result<SvgResolved, String> {
        self.tick += 1;
        let source_key = |k: &SvgRasterKey| {
            let source = VectorSource::SvgImage {
                key: k.clone(),
                asset: String::new(),
                path: Default::default(),
            };
            (source.key(), SVG_SLOT)
        };
        let id = source_key(key);
        if self.svg.rasterizer == SvgRasterizer::Resvg {
            self.svg.approximated += 1;
        }
        if let Some(view) = self.touch(&id) {
            self.svg.drawn(key);
            pm_perf::bump(pm_perf::Counter::SvgCacheHits);
            return Ok(SvgResolved::Drawn(view));
        }
        pm_perf::bump(pm_perf::Counter::SvgCacheMisses);
        let pixels = match self.svg.rasterizer {
            SvgRasterizer::Resvg => {
                let (doc, _) = super::svg::document(path)?;
                Some(std::sync::Arc::new(doc.raster(key.width, key.height).rgba))
            }
            SvgRasterizer::Chromium => self.svg.get(key),
        };
        if let Some(pixels) = pixels {
            let texture = upload_rgba8(device, queue, key.width, key.height, &pixels);
            let view = self.insert(id, texture, key.bytes());
            self.svg.drawn(key);
            return Ok(SvgResolved::Drawn(view));
        }
        pm_perf::bump(pm_perf::Counter::RasterRequests);
        self.svg.miss(SvgRasterNeed {
            request: pm_protocol::svg_raster::SvgRasterRequest {
                key: key.clone(),
                asset: asset.to_owned(),
            },
            path: path.to_path_buf(),
            composition: at.0.to_owned(),
            layer: at.1.to_owned(),
        });
        if policy == SvgMissPolicy::Substitute
            && let Some(latest) = self.svg.latest(key).cloned()
            && let Some(view) = self.touch(&source_key(&latest))
        {
            return Ok(SvgResolved::Substitute(view));
        }
        Ok(SvgResolved::Missing)
    }

    fn gpu(&mut self, device: &wgpu::Device) -> Option<&mut GpuCoverage> {
        if self.backend == CoverageBackend::Cpu {
            return None;
        }
        self.gpu
            .get_or_insert_with(|| GpuCoverage::new(device).ok())
            .as_mut()
    }

    /// Renders a level 2 scene: coverage pages, then the float paint pass
    /// into a `rect`-sized working texture.
    fn render_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &CoverageScene,
        rect: PixelRect,
        precision: Precision,
    ) -> Result<(wgpu::Texture, u64), String> {
        let max_side = device.limits().max_texture_dimension_2d.min(8192);
        let layout = AtlasLayout::pack(scene, rect, max_side);
        let mut pages: Vec<wgpu::Texture> = Vec::with_capacity(layout.pages.len());
        let mut used_gpu = false;
        if let Some(gpu) = self.gpu(device) {
            used_gpu = true;
            for (i, &(w, h)) in layout.pages.iter().enumerate() {
                let tex = device.create_texture(&GpuCoverage::page_descriptor(w, h));
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                gpu.render_page(device, queue, scene, &layout, i, &view)
                    .map_err(|e| e.to_string())?;
                pages.push(tex);
            }
        } else {
            let cpu = render_coverage(scene, &layout);
            for (i, &(w, h)) in layout.pages.iter().enumerate() {
                pages.push(upload_rgba8(device, queue, w, h, &cpu.pages[i]));
            }
        }
        self.last_backend = Some(if used_gpu { "vello" } else { "vello_cpu" });
        let format = match precision {
            Precision::Float32 => wgpu::TextureFormat::Rgba32Float,
            _ => wgpu::TextureFormat::Rgba16Float,
        };
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm vector level 2"),
            size: wgpu::Extent3d {
                width: rect.width.max(1),
                height: rect.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let paint = self.paint.get_or_insert_with(|| Paint::new(device));
        let globals = [rect.width as f32, rect.height as f32, 0.0, 0.0];
        let gbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm vector paint globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&gbuf, 0, &f32_bytes(&globals));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pm vector paint"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pm vector paint"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out_view,
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
            pass.set_pipeline(if format == wgpu::TextureFormat::Rgba32Float {
                &paint.pipeline_f32
            } else {
                &paint.pipeline_f16
            });
            // Items draw in scene order; consecutive items on one page share
            // a draw.
            let mut i = 0;
            let cells = &layout.cells;
            let mut binds: Vec<Option<wgpu::BindGroup>> = (0..pages.len()).map(|_| None).collect();
            while i < cells.len() {
                let page = cells[i].page;
                let mut j = i;
                let mut data: Vec<f32> = Vec::new();
                while j < cells.len() && cells[j].page == page {
                    let c = &cells[j];
                    let item = &scene.items[c.item];
                    let paint_color = item.premultiplied();
                    data.extend_from_slice(&[
                        (c.device.x - rect.x) as f32,
                        (c.device.y - rect.y) as f32,
                        c.device.width as f32,
                        c.device.height as f32,
                        c.x as f32,
                        c.y as f32,
                    ]);
                    data.extend_from_slice(&paint_color);
                    j += 1;
                }
                let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pm vector paint items"),
                    size: (data.len() * 4) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&vbuf, 0, &f32_bytes(&data));
                let bind = binds[page].get_or_insert_with(|| {
                    let view = pages[page].create_view(&wgpu::TextureViewDescriptor::default());
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pm vector paint"),
                        layout: &paint.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: gbuf.as_entire_binding(),
                            },
                        ],
                    })
                });
                pass.set_bind_group(0, &*bind, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.draw(0..4, 0..(j - i) as u32);
                i = j;
            }
        }
        queue.submit([encoder.finish()]);
        let bpp = if format == wgpu::TextureFormat::Rgba32Float {
            16
        } else {
            8
        };
        Ok((
            out,
            u64::from(rect.width.max(1)) * u64::from(rect.height.max(1)) * bpp,
        ))
    }
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// A premultiplied RGBA8 texture, top-down.
fn upload_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> wgpu::Texture {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pm vector raster"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    tex
}
