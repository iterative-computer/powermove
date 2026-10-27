//! Resolving vector sources to textures, cached by content key, with every
//! byte accounted (section 19 "Memory and GPU": the render cache category of
//! the lease).
//!
//! - Coverage scenes use exact area coverage in `r32float` atlas pages,
//!   then composite their paint in float at working precision.
//! - SVG images draw at output resolution through the float GPU painter,
//!   or resvg for documents it cannot draw. Playback frames reproject a
//!   persistent source raster instead. No editor window is involved.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pm_protocol::color::Precision;
use pm_vector::kurbo::{Affine, BezPath, Cap, Join, PathEl};
use pm_vector::render::scene::{
    AtlasLayout, CoverageItem, CoverageScene, FillRule, ItemStyle, OutlineScratch, PixelRect,
};

use crate::coverage::{AreaCoverage, Batch, InputScratch, TransformedItem};

use super::VectorSource;

struct Entry {
    texture: wgpu::Texture,
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

#[derive(Default)]
struct ScenePage {
    inputs: InputScratch,
    values: Option<wgpu::Buffer>,
    value_capacity: u64,
    texture: Option<wgpu::Texture>,
    view: Option<wgpu::TextureView>,
    width: u32,
    height: u32,
}

impl ScenePage {
    fn ensure(&mut self, device: &wgpu::Device, width: u32, height: u32, bytes: u64) -> Result<(), String> {
        let limits = device.limits();
        let value_limit = limits.max_buffer_size.min(limits.max_storage_buffer_binding_size);
        if bytes > value_limit {
            return Err(format!("coverage values need {bytes} bytes, binding limit {value_limit}"));
        }
        if width > limits.max_texture_dimension_2d || height > limits.max_texture_dimension_2d {
            return Err(format!("coverage page {width}x{height} exceeds texture limit {}", limits.max_texture_dimension_2d));
        }
        if bytes > self.value_capacity {
            self.value_capacity = crate::coverage::bounded_capacity(bytes, 16, value_limit)?;
            self.values = Some(crate::coverage::buffer(device, self.value_capacity));
        }
        if width > self.width || height > self.height {
            let grown = (self.width.max(width), self.height.max(height));
            let grown_bytes = u64::from(grown.0) * u64::from(grown.1) * 4;
            let room = MAX_SCENE_SCRATCH_BYTES.saturating_sub(
                self.inputs.capacity_bytes().saturating_add(self.value_capacity)
            );
            // Reuse the larger rectangle while it fits the scratch cap.
            // Crossing two narrow dimensions must not allocate a huge square.
            (self.width, self.height) = if grown_bytes <= room { grown } else { (width, height) };
            let texture = device.create_texture(&crate::coverage::page_descriptor(
                self.width,
                self.height,
            ));
            self.view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            self.texture = Some(texture);
        }
        Ok(())
    }

    fn bytes(&self) -> u64 {
        self.inputs.capacity_bytes()
            + self.value_capacity
            + u64::from(self.width) * u64::from(self.height) * 4
    }
}

/// Scratch is kept across vector misses but never grows without a bound.
const MAX_SCENE_SCRATCH_BYTES: u64 = 48 * 1024 * 1024;

struct OutlineEntry {
    path: Arc<BezPath>,
    used: u64,
    bytes: u64,
}

struct SceneOutline {
    path: Arc<BezPath>,
    transform: Affine,
}

/// Exactly keyed, bounded outlines with translation removed before polygon
/// resolution. Paint is deliberately absent from the key.
#[derive(Default)]
struct SceneOutlines {
    entries: HashMap<Vec<u64>, OutlineEntry>,
    /// Outlines [`SceneOutlines::prepare`] made for the misses it was given,
    /// until [`SceneOutlines::outline`] takes them into the cache.
    prepared: HashMap<Vec<u64>, BezPath>,
    tick: u64,
    bytes: u64,
}

impl SceneOutlines {
    const MAX_BYTES: u64 = 32 * 1024 * 1024;
    const MAX_ENTRIES: usize = 512;

    fn bytes(&self) -> u64 {
        self.bytes
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn outline(&mut self, item: &CoverageItem, scratch: &mut OutlineScratch) -> SceneOutline {
        self.tick += 1;
        let [a, b, c, d, tx, ty] = item.transform.as_coeffs();
        let Some(scale) = upper_outline_scale([a, b, c, d]) else {
            return SceneOutline {
                path: Arc::new(item.device_outline_with(scratch)),
                transform: Affine::IDENTITY,
            };
        };
        let key = outline_key(item, scale);
        let placement = Affine::new([a / scale, b / scale, c / scale, d / scale, tx, ty]);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = self.tick;
            return SceneOutline { path: Arc::clone(&entry.path), transform: placement };
        }
        let canonical = Arc::new(match self.prepared.remove(&key) {
            Some(outline) => outline,
            None => item.device_outline_at_scale(scale, scratch),
        });
        let bytes = (key.capacity() * size_of::<u64>()
            + canonical.elements().len().max(4).next_power_of_two() * size_of::<PathEl>()
            + size_of::<Arc<BezPath>>()) as u64;
        if bytes <= Self::MAX_BYTES {
            while self.bytes + bytes > Self::MAX_BYTES
                || self.entries.len() >= Self::MAX_ENTRIES
            {
                let Some(oldest) = self.entries.values().map(|entry| entry.used).min()
                else {
                    break;
                };
                let mut removed_bytes = None;
                self.entries.retain(|_, entry| {
                    if removed_bytes.is_none() && entry.used == oldest {
                        removed_bytes = Some(entry.bytes);
                        false
                    } else {
                        true
                    }
                });
                self.bytes -= removed_bytes.expect("oldest outline entry exists");
            }
            self.bytes += bytes;
            self.entries.insert(key, OutlineEntry {
                path: Arc::clone(&canonical),
                used: self.tick,
                bytes,
            });
        }
        SceneOutline { path: canonical, transform: placement }
    }
}

impl SceneOutlines {
    /// Makes the outlines `items` miss on several threads at once (a scene
    /// new to the cache), each exactly as [`SceneOutlines::outline`] makes it
    /// alone; `outline` takes them into the cache when it asks for each, so
    /// the cache holds and evicts what it does without this. A few misses
    /// are left to `outline`; what an earlier call made and `outline` never
    /// asked for is dropped.
    fn prepare<'a>(&mut self, items: impl IntoIterator<Item = &'a CoverageItem>) {
        self.prepared.clear();
        let mut missing: Vec<(Vec<u64>, &CoverageItem, f64)> = Vec::new();
        for item in items {
            let [a, b, c, d, _, _] = item.transform.as_coeffs();
            let Some(scale) = upper_outline_scale([a, b, c, d]) else {
                continue;
            };
            let key = outline_key(item, scale);
            if !self.entries.contains_key(&key) && !missing.iter().any(|(k, _, _)| *k == key) {
                missing.push((key, item, scale));
            }
        }
        if missing.len() < PREPARE_IN_PARALLEL {
            return;
        }
        let made = crate::parallel::map(missing.len(), |i| {
            let (_, item, scale) = missing[i];
            item.device_outline_at_scale(scale, &mut OutlineScratch::default())
        });
        for ((key, _, _), outline) in missing.into_iter().zip(made) {
            self.prepared.insert(key, outline);
        }
    }
}

/// Outline misses [`SceneOutlines::prepare`] makes on several threads.
const PREPARE_IN_PARALLEL: usize = 4;

/// A power-of-two upper bound for the largest singular value of the 2x2
/// linear transform. Padding covers rounded sums and square roots at a
/// power-of-two boundary; unsupported extremes use the exact path.
fn upper_outline_scale([a, b, c, d]: [f64; 4]) -> Option<f64> {
    let (u, v, cross) = (a * a + b * b, c * c + d * d, a * c + b * d);
    let stretch = libm::sqrt((u + v + libm::hypot(u - v, 2.0 * cross)) * 0.5);
    if !stretch.is_finite() || !(1.0 / 4096.0..=4096.0).contains(&stretch) {
        return None;
    }
    let upper = stretch * (1.0 + 16.0 * f64::EPSILON);
    let bits = upper.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    let needs_next = (bits & ((1u64 << 52) - 1)) != 0;
    let power = exponent + i32::from(needs_next);
    Some(f64::from_bits(((power + 1023) as u64) << 52))
}

fn outline_key(item: &CoverageItem, scale: f64) -> Vec<u64> {
    let mut key = Vec::with_capacity(item.path.elements().len() * 7 + 16);
    key.push(item.path.elements().len() as u64);
    let point = |key: &mut Vec<u64>, p: pm_vector::kurbo::Point| {
        key.push(p.x.to_bits());
        key.push(p.y.to_bits());
    };
    for el in item.path.elements() {
        match *el {
            PathEl::MoveTo(p) => { key.push(0); point(&mut key, p); }
            PathEl::LineTo(p) => { key.push(1); point(&mut key, p); }
            PathEl::QuadTo(p, q) => { key.push(2); point(&mut key, p); point(&mut key, q); }
            PathEl::CurveTo(p, q, r) => { key.push(3); point(&mut key, p); point(&mut key, q); point(&mut key, r); }
            PathEl::ClosePath => key.push(4),
        }
    }
    key.push(scale.to_bits());
    match &item.style {
        ItemStyle::Fill(FillRule::NonZero) => key.push(5),
        ItemStyle::Fill(FillRule::EvenOdd) => key.push(6),
        ItemStyle::Stroke(stroke) => {
            key.push(7);
            key.push(stroke.width.to_bits());
            key.push(match stroke.join { Join::Bevel => 0, Join::Miter => 1, Join::Round => 2 });
            key.push(stroke.miter_limit.to_bits());
            for cap in [stroke.start_cap, stroke.end_cap] {
                key.push(match cap { Cap::Butt => 0, Cap::Square => 1, Cap::Round => 2 });
            }
            key.push(stroke.dash_pattern.len() as u64);
            key.extend(stroke.dash_pattern.iter().map(|v| v.to_bits()));
            key.push(stroke.dash_offset.to_bits());
        }
    }
    key
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

fn paint(i: Out) -> vec4<f32> {
    let cov = textureLoad(coverage, vec2<i32>(floor(i.cell)), 0).r;
    return i.color * cov;
}
@fragment fn fs(i: Out) -> @location(0) vec4<f32> { return paint(i); }
// RGBA16F targets store the nearest half of the painted value
// (`crate::nearest_half`); overlapping items round their blend in hardware.
@fragment fn fs_half(i: Out) -> @location(0) vec4<f32> { return pm_scene_half_color(paint(i)); }
"#;

impl Paint {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm vector paint"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{PAINT_WGSL}", crate::nearest_half::NEAREST_HALF_WGSL).into(),
            ),
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
                    entry_point: Some(if format == wgpu::TextureFormat::Rgba16Float {
                        "fs_half"
                    } else {
                        "fs"
                    }),
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
    scene_batch: Batch,
    outline_scratch: OutlineScratch,
    scene_outlines: SceneOutlines,
    scene_pages: Vec<ScenePage>,
    paint_data: Vec<f32>,
    paint_buffer: Option<wgpu::Buffer>,
    paint_capacity: u64,
    svg: Option<super::svg_float::SvgPainter>,
    svg_sample: Option<super::svg_sample::SvgSample>,
    svg_fallback_reason: Option<String>,
    svg_fallback_reasons: HashMap<([u8; 32], u8), String>,
    /// The SVG images of the frame drawn by the declared fallback.
    svg_fallbacks: Vec<(std::path::PathBuf, String)>,
    svg_unpaintable_rasters: HashSet<[u8; 32]>,
    svg_raster_hits: u64,
    svg_raster_misses: u64,
}

fn record_fallback(
    list: &mut Vec<(std::path::PathBuf, String)>,
    path: &std::path::Path,
    reason: &str,
) {
    if !list.iter().any(|(p, r)| p == path && r == reason) {
        list.push((path.to_owned(), reason.to_owned()));
    }
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
            scene_batch: Batch::default(),
            outline_scratch: OutlineScratch::default(),
            scene_outlines: SceneOutlines::default(),
            scene_pages: Vec::new(),
            paint_data: Vec::new(),
            paint_buffer: None,
            paint_capacity: 0,
            svg: None,
            svg_sample: None,
            svg_fallback_reason: None,
            svg_fallback_reasons: HashMap::new(),
            svg_fallbacks: Vec::new(),
            svg_unpaintable_rasters: HashSet::new(),
            svg_raster_hits: 0,
            svg_raster_misses: 0,
        }
    }

    /// Bytes retained by cached textures and reusable scene resources.
    pub fn bytes(&self) -> u64 {
        self.bytes + self.scene_scratch_bytes()
    }

    /// Why the most recent SVG miss used resvg. This is observable at runtime
    /// without changing the texture resolve contract.
    pub fn svg_fallback_reason(&self) -> Option<&str> {
        self.svg_fallback_reason.as_deref()
    }

    /// The SVG images (file and reason) drawn by the declared resvg
    /// fallback since the frame began, each once.
    pub fn take_svg_fallbacks(&mut self) -> Vec<(std::path::PathBuf, String)> {
        std::mem::take(&mut self.svg_fallbacks)
    }

    /// Playback source raster hits and misses (reuse across translation and
    /// rotation).
    pub fn svg_raster_cache_counts(&self) -> (u64, u64) {
        (self.svg_raster_hits, self.svg_raster_misses)
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
        if self.scene_scratch_bytes() > max_bytes {
            self.clear_scene_scratch();
        }
        self.evict(
            self.max_bytes.saturating_sub(self.scene_scratch_bytes()),
            self.max_entries,
            false,
        );
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
        self.svg_fallbacks.clear();
        self.evict(
            self.max_bytes.saturating_sub(self.scene_scratch_bytes()),
            self.max_entries,
            false,
        );
    }

    /// Evicts least recently used textures until at most `budget` bytes
    /// stay (lease pressure, between frames: every entry may go).
    pub fn trim(&mut self, budget: u64) {
        if self.scene_scratch_bytes() > budget {
            self.clear_scene_scratch();
        }
        self.evict(budget.saturating_sub(self.scene_scratch_bytes()), self.max_entries, true);
    }

    fn scene_scratch_bytes(&self) -> u64 {
        self.scene_batch.capacity_bytes()
            + self.outline_scratch.retained_bytes()
            + self.scene_outlines.bytes()
            + (self.paint_data.capacity() * size_of::<f32>()) as u64
            + self.paint_capacity
            + self.scene_pages.iter().map(ScenePage::bytes).sum::<u64>()
    }

    fn clear_scene_scratch(&mut self) {
        self.scene_batch = Batch::default();
        self.outline_scratch = OutlineScratch::default();
        self.scene_outlines.clear();
        self.scene_pages.clear();
        self.scene_pages.shrink_to_fit();
        self.paint_data = Vec::new();
        self.paint_buffer = None;
        self.paint_capacity = 0;
    }

    /// Evicts least recently used entries until at most `bytes` and
    /// `entries` stay; the current frame's entries stay unless `all`.
    fn evict(&mut self, bytes: u64, entries: usize, all: bool) {
        while self.bytes > bytes || self.entries.len() > entries {
            let Some(k) = self
                .entries
                .iter()
                .filter(|(_, e)| all || e.frame != self.frame)
                .min_by_key(|(k, e)| (e.used, **k))
                .map(|(k, _)| *k)
            else {
                break;
            };
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
        let key = (source.key(), precision as u8);
        if let Some(e) = self.entries.get_mut(&key) {
            e.used = self.tick;
            e.frame = self.frame;
            if let VectorSource::SvgRaster { path, .. } = source {
                self.svg_fallback_reason = self.svg_fallback_reasons.get(&key).cloned();
                if let Some(reason) = &self.svg_fallback_reason {
                    record_fallback(&mut self.svg_fallbacks, path, reason);
                }
            }
            pm_perf::bump(pm_perf::Counter::VectorCacheHits);
            return Ok(e.view.clone());
        }
        pm_perf::bump(pm_perf::Counter::VectorCacheMisses);
        let (texture, bytes) = match source {
            VectorSource::Scene { scene, rect } => {
                self.render_scene(device, queue, scene, *rect, precision)?
            }
            VectorSource::SvgRaster {
                path,
                stamp,
                transform,
                rect,
                clip,
                playback,
            } => {
                self.svg_fallback_reasons
                    .retain(|entry_key, _| self.entries.contains_key(entry_key));
                let (doc, _) = super::svg::document(path)?;
                let transform = Affine::new(*transform);
                let clip = clip.map(|[x, y]| pm_vector::kurbo::Point::new(x, y));
                let (reason, counter) = if let Some(feature) = doc.unsupported_feature() {
                    (
                        format!("SVG {feature} is drawn by resvg"),
                        pm_perf::Counter::SvgFallbackFeature,
                    )
                } else if !device.features().contains(wgpu::Features::FLOAT32_BLENDABLE) {
                    (
                        "GPU lacks FLOAT32_BLENDABLE".to_owned(),
                        pm_perf::Counter::SvgFallbackDevice,
                    )
                } else {
                    if *playback
                        && let Some((texture, bytes)) = self.svg_playback(
                            device, queue, &doc, path, *stamp, transform, *rect, &clip, precision,
                        )?
                    {
                        self.svg_fallback_reason = None;
                        return Ok(self.insert(key, texture, bytes));
                    }
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("pm svg float"),
                        });
                    let painter = self.svg.get_or_insert_with(|| {
                        super::svg_float::SvgPainter::new(device)
                            .expect("FLOAT32_BLENDABLE was checked")
                    });
                    if let Some((texture, bytes)) = painter.render(
                        device, queue, &mut encoder, &doc, transform, *rect, &clip, precision,
                    )? {
                        queue.submit([encoder.finish()]);
                        self.svg_fallback_reason = None;
                        return Ok(self.insert(key, texture, bytes));
                    }
                    (
                        painter
                            .fallback_reason()
                            .unwrap_or("float SVG painter declined")
                            .to_owned(),
                        pm_perf::Counter::SvgFallbackLimit,
                    )
                };
                pm_perf::bump(counter);
                record_fallback(&mut self.svg_fallbacks, path, &reason);
                // The declared fallback: resvg at output resolution, with the
                // reason observable at runtime and restored on a cache hit.
                self.svg_fallback_reason = Some(reason.clone());
                self.svg_fallback_reasons.insert(key, reason);
                let b = doc.raster_device(transform, *rect, Some(&clip));
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

    /// A playback frame from the persistent source raster: the raster is
    /// looked up (or painted once) by content, scale bucket and crop, then
    /// reprojected at `transform`. `None` paints the frame directly.
    #[allow(clippy::too_many_arguments)]
    fn svg_playback(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        doc: &pm_vector::render::svg::SvgDocument,
        path: &std::path::Path,
        stamp: super::svg::Stamp,
        transform: Affine,
        rect: PixelRect,
        clip: &[pm_vector::kurbo::Point; 4],
        precision: Precision,
    ) -> Result<Option<(wgpu::Texture, u64)>, String> {
        let Some(plan) =
            super::svg_sample::plan_with_clip(doc, path, stamp, transform, clip, &device.limits())
        else {
            return Ok(None);
        };
        let raster_key = (plan.key, u8::MAX);
        let source = if let Some(entry) = self.entries.get_mut(&raster_key) {
            entry.used = self.tick;
            entry.frame = self.frame;
            self.svg_raster_hits += 1;
            pm_perf::bump(pm_perf::Counter::SvgRasterHits);
            entry.texture.clone()
        } else if self.svg_unpaintable_rasters.contains(&plan.key) {
            return Ok(None);
        } else {
            self.svg_raster_misses += 1;
            pm_perf::bump(pm_perf::Counter::SvgRasterMisses);
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm svg source raster"),
            });
            let painter = self.svg.get_or_insert_with(|| {
                super::svg_float::SvgPainter::new(device).expect("FLOAT32_BLENDABLE was checked")
            });
            let Some((texture, bytes)) = painter.render(
                device,
                queue,
                &mut encoder,
                doc,
                Affine::scale(plan.scale),
                plan.rect,
                &plan.source_clip,
                super::svg_sample::SOURCE_PRECISION,
            )?
            else {
                // A raster beyond the painter's budget stays beyond it on
                // the next moving frame; remember it (bounded).
                if self.svg_unpaintable_rasters.len() >= self.max_entries {
                    self.svg_unpaintable_rasters.clear();
                }
                self.svg_unpaintable_rasters.insert(plan.key);
                return Ok(None);
            };
            queue.submit([encoder.finish()]);
            self.insert(raster_key, texture.clone(), bytes);
            texture
        };
        let sampler = self
            .svg_sample
            .get_or_insert_with(|| super::svg_sample::SvgSample::new(device));
        Ok(sampler.render(device, queue, &source, &plan, transform, rect, clip, precision))
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
            self.max_bytes
                .saturating_sub(bytes)
                .saturating_sub(self.scene_scratch_bytes()),
            self.max_entries.saturating_sub(1),
            false,
        );
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                texture,
                view: view.clone(),
                bytes,
                used: self.tick,
                frame: self.frame,
            },
        );
        view
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
        let limits = device.limits();
        let value_limit = limits.max_buffer_size.min(limits.max_storage_buffer_binding_size);
        let max_side = limits.max_texture_dimension_2d
            .min(8192)
            .min((value_limit / 4).isqrt().min(u64::from(u32::MAX)) as u32);
        if max_side < 16 {
            return Err(format!("coverage page limit {max_side} is below minimum tile size"));
        }
        if rect.width > limits.max_texture_dimension_2d || rect.height > limits.max_texture_dimension_2d {
            return Err(format!("vector output {}x{} exceeds texture limit {}", rect.width, rect.height, limits.max_texture_dimension_2d));
        }
        let layout = AtlasLayout::pack(scene, rect, max_side);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pm vector"),
        });
        let coverage = self
            .coverage
            .get_or_insert_with(|| AreaCoverage::new(device));
        // Shapes new to the outline cache are outlined on several threads.
        self.scene_outlines
            .prepare(layout.cells.iter().map(|cell| &scene.items[cell.item]));
        let mut outlines: Vec<Option<SceneOutline>> =
            std::iter::repeat_with(|| None).take(scene.items.len()).collect();
        for cell in &layout.cells {
            if outlines[cell.item].is_none() {
                outlines[cell.item] = Some(
                    self.scene_outlines
                        .outline(&scene.items[cell.item], &mut self.outline_scratch),
                );
            }
        }
        self.scene_pages
            .resize_with(layout.pages.len(), ScenePage::default);
        for (i, &(w, h)) in layout.pages.iter().enumerate() {
            let items: Vec<TransformedItem<'_>> = layout
                .cells
                .iter()
                .filter(|c| c.page == i)
                .map(|c| {
                    let outline = outlines[c.item].as_ref().expect("item outline");
                    TransformedItem {
                    path: &outline.path,
                    transform: outline.transform,
                    bounds: c.device,
                    at: [c.x, c.y],
                    }
                })
                .collect();
            self.scene_batch.reset_transformed(&items);
            self.scene_batch.fits_gpu_limits(&limits)?;
            let page = &mut self.scene_pages[i];
            page.ensure(device, w, h, self.scene_batch.bytes())?;
            coverage.encode_pooled(
                device,
                queue,
                &mut encoder,
                &self.scene_batch,
                crate::coverage::CoverageOutput {
                    values: page.values.as_ref().expect("coverage values"),
                    page: page.view.as_ref(),
                },
                &mut page.inputs,
            )?;
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
        // Uniform globals and every instance's paint data are one upload.
        // The first 16 bytes are the uniform; the vertex slice starts there.
        self.paint_data.clear();
        self.paint_data.reserve(4 + layout.cells.len() * 10);
        self.paint_data
            .extend_from_slice(&[rect.width as f32, rect.height as f32, 0.0, 0.0]);
        for c in &layout.cells {
            let item = &scene.items[c.item];
            self.paint_data.extend_from_slice(&[
                (c.device.x - rect.x) as f32,
                (c.device.y - rect.y) as f32,
                c.device.width as f32,
                c.device.height as f32,
                c.x as f32,
                c.y as f32,
            ]);
            self.paint_data.extend_from_slice(&item.premultiplied());
        }
        let bytes = crate::composite::gaussian::f32_bytes(&self.paint_data);
        if bytes.len() as u64 > limits.max_buffer_size {
            return Err(format!("paint data needs {} bytes, buffer limit {}", bytes.len(), limits.max_buffer_size));
        }
        if bytes.len() as u64 > self.paint_capacity {
            let needed = bytes.len() as u64;
            self.paint_capacity = crate::coverage::bounded_capacity(needed, 256, limits.max_buffer_size)?;
            self.paint_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pm vector pooled paint data"),
                size: self.paint_capacity,
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let paint_buffer = self.paint_buffer.as_ref().expect("paint buffer");
        queue.write_buffer(paint_buffer, 0, bytes);
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
            pass.set_vertex_buffer(0, paint_buffer.slice(16..));
            // Items draw in scene order; consecutive items on one page share
            // a draw.
            let mut i = 0;
            let cells = &layout.cells;
            let mut binds: Vec<Option<wgpu::BindGroup>> =
                (0..layout.pages.len()).map(|_| None).collect();
            while i < cells.len() {
                let page = cells[i].page;
                let mut j = i;
                while j < cells.len() && cells[j].page == page {
                    j += 1;
                }
                let bind = binds[page].get_or_insert_with(|| {
                    let view = self.scene_pages[page]
                        .view
                        .as_ref()
                        .expect("coverage page view");
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pm vector paint"),
                        layout: &paint.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: paint_buffer,
                                    offset: 0,
                                    size: std::num::NonZeroU64::new(16),
                                }),
                            },
                        ],
                    })
                });
                pass.set_bind_group(0, &*bind, &[]);
                pass.draw(0..4, i as u32..j as u32);
                i = j;
            }
        }
        queue.submit([encoder.finish()]);
        if self.scene_scratch_bytes() > MAX_SCENE_SCRATCH_BYTES.min(self.max_bytes) {
            self.clear_scene_scratch();
        }
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

#[cfg(test)]
mod scene_pool_tests {
    use super::*;
    use pm_vector::kurbo::{Affine, BezPath};
    use pm_vector::render::scene::{CoverageItem, FillRule, ItemStyle};

    fn scene(x: f64, y: f64, size: f64) -> CoverageScene {
        let mut path = BezPath::new();
        path.move_to((x, y));
        path.line_to((x + size, y));
        path.line_to((x + size, y + size));
        path.line_to((x, y + size));
        path.close_path();
        CoverageScene {
            items: vec![CoverageItem {
                path,
                transform: Affine::IDENTITY,
                style: ItemStyle::Fill(FillRule::NonZero),
                color: [0.8, 0.3, 0.2, 0.7],
            }],
        }
    }

    #[test]
    fn outline_key_tracks_geometry_not_paint_or_translation() {
        let mut item = scene(2.25, 3.5, 30.0).items.remove(0);
        let key = |item: &CoverageItem| {
            let [a, b, c, d, _, _] = item.transform.as_coeffs();
            outline_key(item, upper_outline_scale([a, b, c, d]).unwrap())
        };
        let original = key(&item);
        item.color = [0.1, 0.9, 0.4, 0.2];
        item.transform = Affine::translate((17.0, -23.0));
        assert_eq!(key(&item), original);
        item.transform = Affine::scale(1.125) * item.transform;
        assert_eq!(key(&item), original);
        item.transform = Affine::scale(2.25);
        assert_ne!(key(&item), original);
        item.transform = Affine::IDENTITY;
        item.path.line_to((33.0, 33.0));
        assert_ne!(key(&item), original);
    }

    #[test]
    fn upper_outline_scale_bounds_rotation_and_skew() {
        for angle in [0.0, 0.17, 0.71, 1.4, 2.9] {
            let affine = Affine::rotate(angle) * Affine::new([1.25, 0.0, 0.6, 0.75, 0.0, 0.0]);
            let [a, b, c, d, _, _] = affine.as_coeffs();
            let bucket = upper_outline_scale([a, b, c, d]).unwrap();
            for theta in 0..360 {
                let t = f64::from(theta).to_radians();
                let length = libm::hypot(a * t.cos() + c * t.sin(), b * t.cos() + d * t.sin());
                assert!(length <= bucket);
            }
        }
    }

    #[test]
    fn pooled_page_rejects_limits_before_creating_resources() {
        let gpu = pm_gpu::Gpu::new("scene page preflight").expect("GPU");
        let limits = gpu.device.limits();
        let mut page = ScenePage::default();
        assert!(page.ensure(&gpu.device, 1, 1, limits.max_storage_buffer_binding_size + 1).is_err());
        assert!(page.ensure(&gpu.device, limits.max_texture_dimension_2d + 1, 1, 16).is_err());
        assert_eq!(page.bytes(), 0);
    }

    #[test]
    fn outline_cache_hit_shares_geometry_across_integer_translation() {
        let mut item = scene(2.25, 3.5, 30.0).items.remove(0);
        let mut cache = SceneOutlines::default();
        let mut scratch = OutlineScratch::default();
        let first = cache.outline(&item, &mut scratch);
        item.transform = Affine::translate((17.0, -23.0));
        let moved = cache.outline(&item, &mut scratch);
        assert!(Arc::ptr_eq(&first.path, &moved.path));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.tick, 2);
        assert_eq!(moved.transform * &*moved.path, item.device_outline_with(&mut scratch));
    }

    /// Outlines made on several threads are the outlines `outline` makes
    /// alone, and the cache ends as it does without preparing.
    #[test]
    fn prepared_outlines_are_what_outline_makes() {
        let items: Vec<CoverageItem> = (0..10)
            .map(|i| {
                let mut item = scene(1.5 + i as f64 * 0.7, 2.0 + i as f64 * 0.3, 20.0 + i as f64).items.remove(0);
                item.transform = Affine::rotate(i as f64 * 0.41) * item.transform;
                // Overlapping contours (resolved to one nonzero boundary).
                item.path.move_to((8.0, 8.0));
                item.path.quad_to((40.0, 4.0), (30.0, 34.0));
                item.path.line_to((6.0, 30.0));
                item.path.close_path();
                item
            })
            .collect();
        let mut scratch = OutlineScratch::default();
        let mut alone = SceneOutlines::default();
        let want: Vec<BezPath> = items.iter().map(|item| {
            let o = alone.outline(item, &mut scratch);
            o.transform * &*o.path
        }).collect();
        let mut prepared = SceneOutlines::default();
        prepared.prepare(items.iter().rev());
        assert!(!prepared.prepared.is_empty());
        for (item, want) in items.iter().zip(&want) {
            let o = prepared.outline(item, &mut scratch);
            assert_eq!(&(o.transform * &*o.path), want);
        }
        assert!(prepared.prepared.is_empty());
        assert_eq!((prepared.entries.len(), prepared.bytes, prepared.tick), (alone.entries.len(), alone.bytes, alone.tick));
    }

    #[test]
    fn outline_cache_evicts_oldest_entry_at_cap() {
        let mut cache = SceneOutlines::default();
        for n in 0..SceneOutlines::MAX_ENTRIES {
            cache.entries.insert(
                vec![n as u64],
                OutlineEntry {
                    path: Arc::new(BezPath::new()),
                    used: n as u64,
                    bytes: 64,
                },
            );
            cache.bytes += 64;
        }
        cache.tick = SceneOutlines::MAX_ENTRIES as u64;
        let item = scene(2.25, 3.5, 30.0).items.remove(0);
        cache.outline(&item, &mut OutlineScratch::default());
        assert_eq!(cache.entries.len(), SceneOutlines::MAX_ENTRIES);
        assert!(!cache.entries.contains_key(&vec![0]));
        assert!(cache.bytes <= SceneOutlines::MAX_BYTES);
    }

    #[test]
    fn pooled_scene_inputs_preserve_prior_submissions_and_trim() {
        let gpu = pm_gpu::Gpu::new("scene pool test").expect("GPU");
        let rect = PixelRect::new(0, 0, 64, 64);
        let a = scene(2.25, 3.5, 30.0);
        let b = scene(19.75, 18.25, 21.0);
        let mut pooled = VectorCache::new();
        // Both render calls submit before either result is read. Reusing one
        // input buffer without queue ordering would overwrite A with B.
        let (out_a, _) = pooled
            .render_scene(&gpu.device, &gpu.queue, &a, rect, Precision::Float16)
            .expect("A renders");
        let (out_b, _) = pooled
            .render_scene(&gpu.device, &gpu.queue, &b, rect, Precision::Float16)
            .expect("B renders");
        let mut fresh_a = VectorCache::new();
        let (want_a, _) = fresh_a
            .render_scene(&gpu.device, &gpu.queue, &a, rect, Precision::Float16)
            .expect("A fresh renders");
        let mut fresh_b = VectorCache::new();
        let (want_b, _) = fresh_b
            .render_scene(&gpu.device, &gpu.queue, &b, rect, Precision::Float16)
            .expect("B fresh renders");
        let read = |texture| pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, texture).unwrap();
        assert_eq!(read(&out_a), read(&want_a));
        assert_eq!(read(&out_b), read(&want_b));
        assert!(pooled.scene_scratch_bytes() > 0);
        assert!(pooled.scene_scratch_bytes() <= MAX_SCENE_SCRATCH_BYTES);
        assert_eq!(pooled.bytes(), pooled.scene_scratch_bytes());
        pooled.trim(0);
        assert_eq!(pooled.scene_scratch_bytes(), 0);
        assert_eq!(pooled.bytes(), 0);
    }
}
