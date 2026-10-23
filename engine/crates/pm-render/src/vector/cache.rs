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

/// What [`VectorCache::prefetch`] did for a frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrefetchStats {
    /// Rasters drawn ahead and cached.
    pub drawn: u32,
    /// Wall time of the parallel rasterization (µs).
    pub raster_us: u64,
    /// The slowest raster: its time (µs) and pixels.
    pub slowest_us: u64,
    pub slowest_px: u64,
    /// Of `drawn`, the rasters drawn ahead of the frame (`prefetch_ahead`).
    pub ahead: u32,
}

/// Worker threads for [`VectorCache::prefetch`]: half the cores, at most 4.
/// Each worker's exact canvas emulation holds a multisampled float target
/// while it draws (up to 64 bytes a pixel): more workers multiply that
/// transient memory for little more speed.
/// `PM_VECTOR_PREFETCH_WORKERS` (1 to 8) overrides it for measurements.
fn prefetch_workers() -> usize {
    static WORKERS: std::sync::LazyLock<usize> = std::sync::LazyLock::new(|| {
        std::env::var("PM_VECTOR_PREFETCH_WORKERS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| (1..=8).contains(n))
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| (n.get() / 2).clamp(1, 4))
                    .unwrap_or(2)
            })
    });
    *WORKERS
}

/// Bytes of level 1 rasters drawn ahead (`VectorCache::prefetch_ahead`)
/// and not yet taken by a frame; past it nothing more is drawn ahead.
pub const AHEAD_MAX_BYTES: usize = 128 << 20;
/// Frames a raster drawn ahead waits to be taken before it is dropped
/// (more than a paused view warms ahead of its playhead).
pub const AHEAD_KEEP_FRAMES: u64 = 64;

type RasterKey = ([u8; 32], u8);
type Drawn = Result<(u32, u32, Vec<u8>), String>;

/// Level 1 rasters drawn ahead of the frames that will draw them, on
/// threads of their own: a playing view plans the frames it will draw next
/// (`Renderer::compat_needs`) and their shape and path rasters are drawn
/// here while it draws the current one. A frame takes the ones done, waits
/// for the ones being drawn, and draws the rest itself (`prefetch`).
#[derive(Default)]
struct Ahead {
    state: std::sync::Mutex<AheadState>,
    done: std::sync::Condvar,
}

#[derive(Default)]
struct AheadState {
    /// Done, with the cache's frame count when it was asked for.
    ready: HashMap<RasterKey, (Drawn, u64)>,
    drawing: std::collections::HashSet<RasterKey>,
    queue: std::collections::VecDeque<(RasterKey, VectorSource, u64)>,
    bytes: usize,
    workers: usize,
    /// Stops the workers (the cache was dropped).
    closed: bool,
    /// Rasters drawn ahead, and those a frame took or waited for.
    drawn: u64,
    taken: u64,
    waited: u64,
}

impl Ahead {
    fn worker(self: std::sync::Arc<Self>) {
        loop {
            let (key, source, asked) = {
                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if st.closed {
                        st.workers -= 1;
                        return;
                    }
                    if let Some(job) = st.queue.pop_front() {
                        break job;
                    }
                    let (next, timeout) = self
                        .done
                        .wait_timeout(st, std::time::Duration::from_secs(10))
                        .unwrap_or_else(|e| e.into_inner());
                    st = next;
                    // Idle a while: the thread ends (a paused view needs none).
                    if timeout.timed_out() && st.queue.is_empty() {
                        st.workers -= 1;
                        return;
                    }
                }
            };
            let drawn = raster_level1(&source);
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.drawing.remove(&key);
            if let Ok((_, _, rgba)) = &drawn {
                st.bytes += rgba.len();
            }
            st.drawn += 1;
            st.ready.insert(key, (drawn, asked));
            drop(st);
            self.done.notify_all();
        }
    }
}

/// A level 1 shape or paths raster: width, height and premultiplied
/// `Rgba8Unorm` rows top-down.
fn raster_level1(source: &VectorSource) -> Result<(u32, u32, Vec<u8>), String> {
    match source {
        VectorSource::Shape {
            content,
            colors,
            scale,
            crop,
        } => {
            let b = pm_vector::canvas2d::shape::raster_shape_window(content, colors, *scale, *crop)
                .map_err(|e| e.to_string())?;
            Ok((b.width, b.height, b.rgba))
        }
        VectorSource::Paths {
            nodes,
            colors,
            scale,
            crop,
        } => {
            let layer = PathLayer::new(nodes);
            let (b, _) = pm_vector::canvas2d::shape::raster_paths_window(&layer, colors, *scale, *crop)
                .map_err(|e| e.to_string())?;
            Ok((b.width, b.height, b.rgba))
        }
        _ => Err("not a level 1 raster".into()),
    }
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
    /// Level 1 rasters drawn ahead (`prefetch_ahead`).
    ahead: std::sync::Arc<Ahead>,
}

impl Drop for VectorCache {
    fn drop(&mut self) {
        let mut st = self.ahead.state.lock().unwrap_or_else(|e| e.into_inner());
        st.closed = true;
        st.queue.clear();
        drop(st);
        self.ahead.done.notify_all();
    }
}

/// What [`VectorCache::prefetch_ahead`] has done so far (stats).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AheadStats {
    /// Rasters drawn ahead, and those a frame then took (or waited for).
    pub drawn: u64,
    pub taken: u64,
    pub waited: u64,
    /// Bytes held for frames that have not come yet.
    pub bytes: u64,
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
            ahead: std::sync::Arc::default(),
        }
    }

    /// Draws the level 1 shape and path rasters of a frame that will be
    /// drawn soon (its plan's sources) on threads of their own, for
    /// [`Self::prefetch`] to take when that frame comes: the same function
    /// of the same source, under the same key, so the frame's pixels are
    /// the ones it would draw itself. Nothing is drawn for a source already
    /// cached, done or being drawn, and nothing past [`AHEAD_MAX_BYTES`]
    /// held for frames that have not come.
    pub fn prefetch_ahead(&mut self, sources: &[&VectorSource], precision: Precision) {
        let mut st = self.ahead.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.bytes >= AHEAD_MAX_BYTES {
            return;
        }
        let mut queued = false;
        for source in sources {
            if !matches!(source, VectorSource::Shape { .. } | VectorSource::Paths { .. }) {
                continue;
            }
            let key = (source.key(), precision as u8);
            if self.entries.contains_key(&key)
                || st.ready.contains_key(&key)
                || st.drawing.contains(&key)
            {
                continue;
            }
            st.drawing.insert(key);
            st.queue.push_back((key, (*source).clone(), self.frame));
            queued = true;
        }
        if !queued {
            return;
        }
        let want = prefetch_workers().min(st.queue.len() + st.workers);
        while st.workers < want {
            let ahead = self.ahead.clone();
            let spawned = std::thread::Builder::new()
                .name("pm-vector-ahead".into())
                .spawn(move || ahead.worker());
            if spawned.is_err() {
                break;
            }
            st.workers += 1;
        }
        drop(st);
        self.ahead.done.notify_all();
    }

    /// Rasters drawn ahead so far (stats).
    pub fn ahead_stats(&self) -> AheadStats {
        let st = self.ahead.state.lock().unwrap_or_else(|e| e.into_inner());
        AheadStats {
            drawn: st.drawn,
            taken: st.taken,
            waited: st.waited,
            bytes: st.bytes as u64,
        }
    }

    /// Drops what was drawn ahead and not taken (a seek, a stop: those
    /// frames will not come), and what is still queued.
    pub fn forget_ahead(&mut self) {
        let mut st = self.ahead.state.lock().unwrap_or_else(|e| e.into_inner());
        let queued: Vec<RasterKey> = st.queue.drain(..).map(|(k, _, _)| k).collect();
        for k in queued {
            st.drawing.remove(&k);
        }
        st.ready.clear();
        st.bytes = 0;
    }

    /// The rasters among `keys` drawn ahead, taken; waits for those being
    /// drawn ahead (drawing them again here would only race the worker).
    fn take_ahead(&mut self, keys: &[RasterKey]) -> HashMap<RasterKey, Drawn> {
        let mut out = HashMap::new();
        let mut st = self.ahead.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.ready.is_empty() && st.drawing.is_empty() {
            return out;
        }
        let mut waited = false;
        loop {
            for key in keys {
                if let Some((drawn, _)) = st.ready.remove(key) {
                    if let Ok((_, _, rgba)) = &drawn {
                        st.bytes = st.bytes.saturating_sub(rgba.len());
                    }
                    st.taken += 1;
                    out.insert(*key, drawn);
                }
            }
            // Queued but not started: the frame draws those itself.
            let queued: Vec<RasterKey> = st
                .queue
                .iter()
                .filter(|(k, _, _)| keys.contains(k))
                .map(|(k, _, _)| *k)
                .collect();
            if !queued.is_empty() {
                st.queue.retain(|(k, _, _)| !queued.contains(k));
                for k in &queued {
                    st.drawing.remove(k);
                }
            }
            if !keys.iter().any(|k| st.drawing.contains(k)) {
                break;
            }
            waited = true;
            st = self.ahead.done.wait(st).unwrap_or_else(|e| e.into_inner());
        }
        if waited {
            st.waited += 1;
        }
        out
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
        // Rasters drawn ahead for frames that passed without taking them
        // (planned before a skip, a layer whose answer changed its source).
        {
            let frame = self.frame;
            let mut st = self.ahead.state.lock().unwrap_or_else(|e| e.into_inner());
            if !st.ready.is_empty() {
                let mut freed = 0usize;
                st.ready.retain(|_, (drawn, asked)| {
                    let keep = *asked + AHEAD_KEEP_FRAMES >= frame;
                    if !keep && let Ok((_, _, rgba)) = drawn {
                        freed += rgba.len();
                    }
                    keep
                });
                st.bytes = st.bytes.saturating_sub(freed);
            }
        }
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

    /// Rasterizes a frame's level 1 shape and path misses on worker threads
    /// before the frame records, and caches them as [`Self::resolve`] would
    /// (same keys, bytes, order and pixels: the rasterizers are pure CPU
    /// functions of the source). Today's compositor rasterized these on the
    /// GPU canvas; the exact CPU emulation of it is the slow part of a
    /// frame with many animated shapes, and a frame's sources are
    /// independent. A source that fails is left to `resolve`, which reports
    /// the error in order.
    pub fn prefetch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sources: &[&VectorSource],
        precision: Precision,
    ) -> PrefetchStats {
        let mut stats = PrefetchStats::default();
        let mut misses: Vec<(([u8; 32], u8), &VectorSource)> = Vec::new();
        for source in sources {
            if !matches!(source, VectorSource::Shape { .. } | VectorSource::Paths { .. }) {
                continue;
            }
            // Keyed as `resolve` keys them, by the frame's precision.
            let key = (source.key(), precision as u8);
            if self.entries.contains_key(&key) || misses.iter().any(|(k, _)| *k == key) {
                continue;
            }
            misses.push((key, source));
        }
        // Drawn ahead (a playing view's next frames): uploaded as the frame
        // would upload its own.
        let keys: Vec<RasterKey> = misses.iter().map(|(k, _)| *k).collect();
        let mut ahead = self.take_ahead(&keys);
        if !ahead.is_empty() {
            for (key, _) in &misses {
                let Some(Ok((width, height, rgba))) = ahead.remove(key) else {
                    continue;
                };
                self.tick += 1;
                let texture = upload_rgba8(device, queue, width, height, &rgba);
                self.insert(*key, texture, u64::from(width) * u64::from(height) * 4);
                stats.drawn += 1;
                stats.ahead += 1;
            }
            // A failed one is left to `resolve`, which reports it in order.
            misses.retain(|(k, _)| !self.entries.contains_key(k) && !ahead.contains_key(k));
        }
        if misses.len() < 2 {
            // One raster gains nothing from a thread: `resolve` draws it.
            return stats;
        }
        let workers = prefetch_workers().min(misses.len());
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut drawn: Vec<Option<(u32, u32, Vec<u8>)>> = Vec::new();
        drawn.resize_with(misses.len(), || None);
        let started = std::time::Instant::now();
        let mut slowest = (0u64, 0u64);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    scope.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some((_, source)) = misses.get(i) else {
                                break;
                            };
                            let began = std::time::Instant::now();
                            let raster = raster_level1(source);
                            out.push((i, raster, began.elapsed().as_micros() as u64));
                        }
                        out
                    })
                })
                .collect();
            for handle in handles {
                if let Ok(results) = handle.join() {
                    for (i, result, us) in results {
                        if us > slowest.0 {
                            let px = result.as_ref().map_or(0, |(w, h, _)| u64::from(*w) * u64::from(*h));
                            slowest = (us, px);
                        }
                        drawn[i] = result.ok();
                    }
                }
            }
        });
        stats.raster_us = started.elapsed().as_micros() as u64;
        (stats.slowest_us, stats.slowest_px) = slowest;
        for ((key, _), result) in misses.into_iter().zip(drawn) {
            let Some((width, height, rgba)) = result else {
                continue;
            };
            self.tick += 1;
            let texture = upload_rgba8(device, queue, width, height, &rgba);
            self.insert(key, texture, u64::from(width) * u64::from(height) * 4);
            stats.drawn += 1;
        }
        stats
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
            return Ok(e.view.clone());
        }
        let (texture, bytes) = match source {
            VectorSource::Shape { .. } | VectorSource::Paths { .. } => {
                let (width, height, rgba) = raster_level1(source)?;
                (
                    upload_rgba8(device, queue, width, height, &rgba),
                    u64::from(width) * u64::from(height) * 4,
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
            return Ok(SvgResolved::Drawn(view));
        }
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
