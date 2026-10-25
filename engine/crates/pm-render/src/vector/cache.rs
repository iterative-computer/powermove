//! Resolving vector sources to textures, cached by content key, with every
//! byte accounted (section 19 "Memory and GPU": the render cache category of
//! the lease).
//!
//! - Coverage scenes use exact area coverage in `r32float` atlas pages,
//!   then composite their paint in float at working precision.
//! - SVG images draw at output resolution through the native GPU painter,
//!   or resvg for documents it cannot draw. No editor window is involved.

use std::collections::HashMap;

use pm_protocol::color::Precision;
use pm_vector::gpu::{GpuCoverage, SvgCell};
use pm_vector::render::scene::{AtlasLayout, CoverageScene, PixelRect};

use crate::coverage::{AreaCoverage, Batch, Item};

use super::VectorSource;

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
    pipeline_u8: wgpu::RenderPipeline,
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
    let cov = textureLoad(coverage, vec2<i32>(floor(i.cell)), 0).r;
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
            pipeline_u8: make(wgpu::TextureFormat::Rgba8Unorm),
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
    coverage: Option<AreaCoverage>,
    paint: Option<Paint>,
    premultiply: Premultiply,
    /// This frame's SVG batch: its signature and its images' keys
    /// ([`Self::prepare_svgs`]).
    svg_batch: Option<([u8; 32], Vec<[u8; 32]>)>,
    /// Which backend rendered the last SVG (tests).
    pub last_backend: Option<&'static str>,
    gpu: Option<Option<GpuCoverage>>,
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
            coverage: None,
            paint: None,
            premultiply: Premultiply::default(),
            svg_batch: None,
            last_backend: None,
            gpu: None,
        }
    }

    /// Bytes of cached textures.
    pub fn bytes(&self) -> u64 {
        self.bytes
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
        self.svg_batch = None;
        self.evict(self.max_bytes, self.max_entries, false);
    }

    /// Evicts least recently used textures until at most `budget` bytes
    /// stay (lease pressure, between frames: every entry may go).
    pub fn trim(&mut self, budget: u64) {
        self.evict(budget, self.max_entries, true);
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
    /// on a miss. Coverage scenes use the frame's precision.
    pub fn resolve(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &VectorSource,
        precision: Precision,
    ) -> Result<wgpu::TextureView, String> {
        self.tick += 1;
        let mut key = (source.key(), precision as u8);
        if let Some((sig, members)) = &self.svg_batch
            && members.contains(&key.0)
        {
            key.0 = batch_key(&key.0, sig);
        }
        if let Some(e) = self.entries.get_mut(&key) {
            e.used = self.tick;
            e.frame = self.frame;
            pm_perf::bump(pm_perf::Counter::VectorCacheHits);
            return Ok(e.view.clone());
        }
        pm_perf::bump(pm_perf::Counter::VectorCacheMisses);
        let (texture, bytes) = match source {
            VectorSource::Scene { scene, rect } => {
                self.render_scene(device, queue, scene, *rect, precision)?
            }
            VectorSource::SvgRaster { .. } => {
                let item = SvgItem::of(source).expect("an SVG source")?;
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("pm svg"),
                });
                let drawn = self
                    .render_svgs(device, queue, &mut encoder, vec![item], precision)?
                    .pop()
                    .expect("one texture per item");
                queue.submit([encoder.finish()]);
                drawn
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

    /// Draws a frame's SVG images ahead, together: one Vello pass per atlas
    /// page for the whole frame instead of one per image (each pass has a
    /// fixed cost). Vello's f32 arithmetic depends on where an image sits
    /// in its page (a gradient's position can round to the neighbouring
    /// ramp entry), so a frame's images are always drawn as the same
    /// batch, the frame's list of them, and cached under it: the frame is
    /// drawn the same in a viewer that holds other images as in a cold
    /// export. [`Self::resolve`] finds them. A frame with one SVG image, or
    /// images Vello cannot draw, leaves them to `resolve`.
    pub fn prepare_svgs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        plan: &crate::plan::CompPlan,
        precision: Precision,
    ) {
        self.svg_batch = None;
        let mut sources = Vec::new();
        super::svg::svg_sources(plan, &mut sources);
        let mut members: Vec<[u8; 32]> = Vec::new();
        let mut items = Vec::new();
        for source in sources {
            let key = source.key();
            if members.contains(&key) {
                continue;
            }
            if let Some(Ok(item)) = SvgItem::of(source)
                && item.doc.gpu_paintable()
            {
                members.push(key);
                items.push(item);
            }
        }
        if items.len() < 2 || self.gpu(device).is_none() {
            return;
        }
        let batch = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(b"pm-svg-batch/1");
            members.iter().for_each(|k| h.update(k));
            let sig: [u8; 32] = h.finalize().into();
            (sig, members)
        };
        let keys: Vec<([u8; 32], u8)> = batch
            .1
            .iter()
            .map(|k| (batch_key(k, &batch.0), precision as u8))
            .collect();
        self.svg_batch = Some(batch);
        if keys.iter().all(|k| self.entries.contains_key(k)) {
            return;
        }
        // A failure leaves every image to `resolve`, which reports it.
        if let Ok(textures) = self.render_svgs(device, queue, encoder, items, precision) {
            self.tick += 1;
            // Room for the whole batch at once (one eviction pass, not one
            // per texture).
            let total: u64 = textures.iter().map(|(_, b)| b).sum();
            self.evict(
                self.max_bytes.saturating_sub(total),
                self.max_entries.saturating_sub(textures.len()),
                false,
            );
            for (key, (texture, bytes)) in keys.into_iter().zip(textures) {
                pm_perf::bump(pm_perf::Counter::VectorCacheMisses);
                self.insert(key, texture, bytes);
            }
        }
    }

    /// SVG images (each under its transform, in its target rectangle,
    /// clipped to its polygon): Vello on the GPU in one pass per atlas page
    /// when it can draw them all, else resvg one by one.
    fn render_svgs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        items: Vec<SvgItem>,
        precision: Precision,
    ) -> Result<Vec<(wgpu::Texture, u64)>, String> {
        use pm_vector::kurbo::{Affine, Rect};
        let max_side = device.limits().max_texture_dimension_2d.min(8192);
        if !items.iter().all(|i| i.doc.gpu_paintable()) || self.gpu(device).is_none() {
            self.last_backend = Some("resvg");
            return Ok(items
                .iter()
                .map(|i| {
                    let b = i
                        .doc
                        .raster_device(i.transform, i.rect, Some(&i.clip));
                    (
                        upload_rgba8(device, queue, b.width, b.height, &b.rgba),
                        u64::from(b.width) * u64::from(b.height) * 4,
                    )
                })
                .collect());
        }
        let format = pm_gpu::working_format(precision);
        let sizes: Vec<(u32, u32)> = items
            .iter()
            .map(|i| (i.rect.width.max(1), i.rect.height.max(1)))
            .collect();
        let (pages, places) = shelf_pack(&sizes, max_side);
        let mut straight = Vec::with_capacity(pages.len());
        for (page, &(pw, ph)) in pages.iter().enumerate() {
            let cells: Vec<SvgCell<'_>> = items
                .iter()
                .zip(&places)
                .zip(&sizes)
                .filter(|((_, p), _)| p.0 == page)
                .map(|((i, &(_, x, y)), &(w, h))| {
                    let to_cell = Affine::translate((
                        f64::from(x) - f64::from(i.rect.x),
                        f64::from(y) - f64::from(i.rect.y),
                    ));
                    SvgCell {
                        doc: &i.doc,
                        transform: to_cell * i.transform,
                        clip: i.clip.map(|p| to_cell * p),
                        cell: Rect::new(
                            f64::from(x),
                            f64::from(y),
                            f64::from(x + w),
                            f64::from(y + h),
                        ),
                    }
                })
                .collect();
            let texture = device.create_texture(&GpuCoverage::page_descriptor(pw, ph));
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let gpu = self.gpu(device).expect("checked");
            gpu.render_svgs(device, queue, &cells, (pw, ph), &view)
                .map_err(|e| e.to_string())?;
            straight.push(view);
        }
        self.last_backend = Some("vello");
        let cuts: Vec<(&wgpu::TextureView, [u32; 2], (u32, u32))> = places
            .iter()
            .zip(&sizes)
            .map(|(&(page, x, y), &size)| (&straight[page], [x, y], size))
            .collect();
        Ok(self
            .premultiply
            .run(device, queue, encoder, &cuts, format)
            .into_iter()
            .zip(&sizes)
            .map(|(t, &(w, h))| (t, u64::from(w) * u64::from(h) * bytes_per_pixel(format)))
            .collect())
    }

    fn gpu(&mut self, device: &wgpu::Device) -> Option<&mut GpuCoverage> {
        self.gpu.get_or_insert_with(|| GpuCoverage::new(device).ok()).as_mut()
    }

    /// Renders a coverage scene: each item's exact coverage in `r32float`
    /// atlas pages, then the float paint pass into a `rect`-sized working
    /// texture, in one command buffer.
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
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pm vector"),
        });
        let coverage = self.coverage.get_or_insert_with(|| AreaCoverage::new(device));
        let outlines: Vec<_> = layout
            .cells
            .iter()
            .map(|c| scene.items[c.item].device_outline())
            .collect();
        let mut pages: Vec<wgpu::Texture> = Vec::with_capacity(layout.pages.len());
        for (i, &(w, h)) in layout.pages.iter().enumerate() {
            let items: Vec<Item<'_>> = layout
                .cells
                .iter()
                .zip(&outlines)
                .filter(|(c, _)| c.page == i)
                .map(|(c, path)| Item {
                    path,
                    bounds: c.device,
                    at: [c.x, c.y],
                })
                .collect();
            let batch = Batch::new(&items);
            let page = device.create_texture(&crate::coverage::page_descriptor(w, h));
            let view = page.create_view(&wgpu::TextureViewDescriptor::default());
            let values = crate::coverage::buffer(device, batch.bytes());
            coverage.encode(device, &mut encoder, &batch, &values, Some(&view));
            pages.push(page);
        }
        let format = pm_gpu::working_format(precision);
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
            } else if format == wgpu::TextureFormat::Rgba8Unorm {
                &paint.pipeline_u8
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
        Ok((
            out,
            u64::from(rect.width.max(1)) * u64::from(rect.height.max(1)) * bytes_per_pixel(format),
        ))
    }
}

fn bytes_per_pixel(format: wgpu::TextureFormat) -> u64 {
    match format {
        wgpu::TextureFormat::Rgba32Float => 16,
        wgpu::TextureFormat::Rgba16Float => 8,
        wgpu::TextureFormat::Rgba8Unorm => 4,
        other => panic!("unsupported vector format: {other:?}"),
    }
}

/// Vello's straight-alpha output premultiplied into working textures: one
/// compute pass, one dispatch per image, recorded into the caller's
/// encoder.
#[derive(Default)]
struct Premultiply {
    formats: HashMap<wgpu::TextureFormat, (wgpu::BindGroupLayout, wgpu::ComputePipeline)>,
}

const PREMULTIPLY_WGSL: &str = r#"
@group(0) @binding(0) var straight: texture_2d<f32>;
@group(0) @binding(1) var<uniform> cut: vec4<u32>;
@group(0) @binding(2) var premultiplied: texture_storage_2d<FORMAT, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= cut.z || id.y >= cut.w) {
        return;
    }
    let c = textureLoad(straight, vec2<i32>(id.xy + cut.xy), 0);
    textureStore(premultiplied, vec2<i32>(id.xy), vec4<f32>(c.rgb * c.a, c.a));
}
"#;

impl Premultiply {
    fn format(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &(wgpu::BindGroupLayout, wgpu::ComputePipeline) {
        self.formats.entry(format).or_insert_with(|| {
            let name = match format {
                wgpu::TextureFormat::Rgba32Float => "rgba32float",
                wgpu::TextureFormat::Rgba16Float => "rgba16float",
                _ => "rgba8unorm",
            };
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pm svg premultiply"),
                source: wgpu::ShaderSource::Wgsl(PREMULTIPLY_WGSL.replace("FORMAT", name).into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pm svg premultiply"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(16),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pm svg premultiply"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pm svg premultiply"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            (layout, pipeline)
        })
    }

    /// Each cut (a straight-alpha page, the cut's origin in it, its size)
    /// premultiplied into its own `format` texture, recorded into
    /// `encoder`.
    fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cuts: &[(&wgpu::TextureView, [u32; 2], (u32, u32))],
        format: wgpu::TextureFormat,
    ) -> Vec<wgpu::Texture> {
        let (layout, pipeline) = self.format(device, format);
        // Every cut's rectangle in one buffer, at the uniform alignment.
        let stride = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(16);
        let mut words = vec![0u8; (stride * cuts.len() as u64) as usize];
        for (i, &(_, [x, y], (w, h))) in cuts.iter().enumerate() {
            let at = i * stride as usize;
            for (k, v) in [x, y, w, h].into_iter().enumerate() {
                words[at + 4 * k..at + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        let rects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm svg premultiply cuts"),
            size: words.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&rects, 0, &words);
        let mut out = Vec::with_capacity(cuts.len());
        let mut binds = Vec::with_capacity(cuts.len());
        for (i, &(straight, _, (width, height))) in cuts.iter().enumerate() {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pm svg"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            binds.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pm svg premultiply"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(straight),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &rects,
                            offset: i as u64 * stride,
                            size: wgpu::BufferSize::new(16),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            }));
            out.push(texture);
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pm svg premultiply"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        for (bind, &(_, _, (w, h))) in binds.iter().zip(cuts) {
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(pass);
        out
    }
}

/// The cache key of an image drawn in the batch `sig`.
fn batch_key(key: &[u8; 32], sig: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(key);
    h.update(sig);
    h.finalize().into()
}

/// One SVG image to draw: its document, user units to target pixels, its
/// target rectangle and clip polygon.
struct SvgItem {
    doc: std::sync::Arc<pm_vector::render::svg::SvgDocument>,
    transform: pm_vector::kurbo::Affine,
    rect: PixelRect,
    clip: [pm_vector::kurbo::Point; 4],
}

impl SvgItem {
    /// `None` for a source that is not an SVG image.
    fn of(source: &VectorSource) -> Option<Result<SvgItem, String>> {
        let VectorSource::SvgRaster {
            path,
            transform,
            rect,
            clip,
            ..
        } = source
        else {
            return None;
        };
        Some(super::svg::document(path).map(|(doc, _)| SvgItem {
            doc,
            transform: pm_vector::kurbo::Affine::new(*transform),
            rect: *rect,
            clip: clip.map(|[x, y]| pm_vector::kurbo::Point::new(x, y)),
        }))
    }
}

/// Shelf packing of `sizes` (each at most `max_side` on a side) into pages
/// of at most `max_side`: the pages' used sizes and each size's (page, x,
/// y), in input order.
fn shelf_pack(sizes: &[(u32, u32)], max_side: u32) -> (Vec<(u32, u32)>, Vec<(usize, u32, u32)>) {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sizes[i].1));
    let mut pages: Vec<(u32, u32)> = vec![(0, 0)];
    let mut places = vec![(0, 0, 0); sizes.len()];
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    for i in order {
        let (w, h) = (sizes[i].0.min(max_side), sizes[i].1.min(max_side));
        if x + w > max_side {
            (x, y, shelf) = (0, y + shelf, 0);
        }
        if y + h > max_side {
            pages.push((0, 0));
            (x, y, shelf) = (0, 0, 0);
        }
        let page = pages.len() - 1;
        places[i] = (page, x, y);
        let p = &mut pages[page];
        *p = (p.0.max(x + w), p.1.max(y + h));
        x += w;
        shelf = shelf.max(h);
    }
    (pages, places)
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
