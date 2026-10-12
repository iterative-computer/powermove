//! The three Vello renderers on the same scenes:
//! - `vello` 0.11: GPU compute, analytic area coverage (`AaConfig::Area`);
//! - `vello_gpu` 0.3: strips prepared on the CPU, coverage and paint on the
//!   GPU (render pipeline);
//! - `vello_cpu` 0.3: the CPU renderer (the plan's CPU fallback).
//!
//! All three write 8-bit RGBA targets (Vello 0.11 and `vello_gpu` accept only
//! `Rgba8Unorm`), so "coverage with float paint" means: render coverage, then
//! paint in float in the engine's own pass. White content measures the
//! coverage directly; its quantization step is 1/255.

use crate::image::Image;
use crate::prepare::{Draw, FaceData, GlyphRun, Prepared};
use crate::scene::Color;
use kurbo::{Affine, Cap, Join, Stroke};
use peniko::{Blob, Fill, FontData};
use s4_layout::font::FaceId;
use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: String,
}

impl Gpu {
    pub fn new() -> Gpu {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))
            .expect("Metal adapter");
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("s4"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("device");
        Gpu { device, queue, adapter: format!("{} ({:?})", info.name, info.backend) }
    }

    fn target(&self, w: u32, h: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("s4 target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: usage | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn read(&self, tex: &wgpu::Texture, premultiplied: bool) -> Image {
        let (w, h) = (tex.width(), tex.height());
        let row = (w * 4).next_multiple_of(256);
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: row as u64 * h as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = buf.slice(..).get_mapped_range().expect("mapped");
        let mut bytes = Vec::with_capacity((w * h * 4) as usize);
        for r in mapped.chunks_exact(row as usize) {
            bytes.extend_from_slice(&r[..(w * 4) as usize]);
        }
        drop(mapped);
        buf.unmap();
        Image::from_rgba8(w, h, &bytes, premultiplied)
    }

    fn wait(&self) {
        self.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
}

/// Font data and normalized coordinates shared by the three renderers.
#[derive(Default)]
pub struct Fonts {
    data: HashMap<FaceId, FontData>,
    coords: HashMap<(FaceId, Vec<(u32, u32)>), Arc<Vec<i16>>>,
    upems: HashMap<FaceId, u16>,
    pub tune: Tune,
}

/// Text tuning toward the macOS look, applied by the engine on top of plain
/// outline coverage (fitted by `s4-render fit`, validated by `compare`):
/// - stem darkening like CoreGraphics font smoothing: the outline expanded
///   by min(base + k × size, max) px per side (`kurbo::expand_path` through
///   each renderer's `font_embolden`), bitmap glyphs excepted;
/// - the ink moved up by min(dy_k × expansion, dy_max) px (CoreGraphics
///   smoothing grows glyphs upward: the measured centroid shift is about
///   -0.3 px from 24 px up);
/// - glyph origin snapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct Tune {
    pub base: f64,
    pub k: f64,
    pub max: f64,
    pub dy_k: f64,
    pub dy_max: f64,
    pub snap: Snap,
    /// Sizes above this draw as plain paths (Skia renders glyphs larger than
    /// its 256 px atlas limit as paths, without CoreGraphics smoothing).
    pub path_above: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub enum Snap {
    /// Exact layout positions (CoreText positions glyphs continuously).
    #[default]
    Exact,
    /// x to 1/4 px, y rounded to whole pixels (Skia's subpixel positioning
    /// for horizontal text, as Canvas2D draws it).
    QuarterRound,
}

impl Tune {
    /// Matches CoreText with font smoothing (the macOS look).
    pub const COREGRAPHICS: Tune = Tune { base: 0.0, k: 0.0175, max: 0.34, dy_k: 1.0, dy_max: 0.30, snap: Snap::Exact, path_above: None };
    /// Matches today's Canvas2D (Chromium's Skia over CoreText masks).
    pub const CANVAS2D: Tune = Tune { base: 0.11, k: 0.0075, max: 0.46, dy_k: 0.65, dy_max: 0.30, snap: Snap::QuarterRound, path_above: Some(256.0) };
    /// A constant expansion and offset (for fitting).
    pub fn fixed(amount: f64, dy: f64, snap: Snap) -> Tune {
        Tune { base: amount, k: 0.0, max: amount, dy_k: 0.0, dy_max: -dy, snap, path_above: None }
    }

    /// Snapping only (stroke passes: glyph outlines stroked as paths, which
    /// font smoothing does not touch).
    pub fn outline_only(&self) -> Tune {
        Tune { snap: self.snap, ..Tune::default() }
    }

    pub fn amount(&self, size: f32, bitmap: bool) -> f64 {
        if bitmap || self.path_above.is_some_and(|p| size > p) { 0.0 } else { (self.base + self.k * size as f64).min(self.max) }
    }

    fn dy(&self, size: f32, bitmap: bool) -> f64 {
        if bitmap || self.path_above.is_some_and(|p| size > p) {
            0.0
        } else if self.dy_k == 0.0 {
            -self.dy_max
        } else {
            -(self.dy_k * self.amount(size, false)).min(self.dy_max)
        }
    }

    pub fn position(&self, x: f64, y: f64, size: f32, bitmap: bool) -> (f32, f32) {
        let (x, y) = match self.snap {
            Snap::Exact => (x, y),
            Snap::QuarterRound => ((x * 4.0 + 0.5).floor() / 4.0, (y + 0.5).floor()),
        };
        (x as f32, (y + self.dy(size, bitmap)) as f32)
    }
}

impl Fonts {
    fn upem(&mut self, faces: &BTreeMap<FaceId, FaceData>, face: FaceId) -> u16 {
        *self.upems.entry(face).or_insert_with(|| {
            let f = &faces[&face];
            skrifa::FontRef::from_index(&f.data, f.index).expect("font").head().map(|h| h.units_per_em()).unwrap_or(1000)
        })
    }

    fn font(&mut self, faces: &BTreeMap<FaceId, FaceData>, face: FaceId) -> FontData {
        self.data
            .entry(face)
            .or_insert_with(|| {
                let f = &faces[&face];
                FontData::new(Blob::new(f.data.clone()), f.index)
            })
            .clone()
    }

    fn normalized(&mut self, faces: &BTreeMap<FaceId, FaceData>, run: &GlyphRun) -> Arc<Vec<i16>> {
        let key = (run.face, run.coords.iter().map(|(t, v)| (*t, v.to_bits())).collect::<Vec<_>>());
        self.coords
            .entry(key)
            .or_insert_with(|| {
                if run.coords.is_empty() {
                    return Arc::new(Vec::new());
                }
                let f = &faces[&run.face];
                let font = skrifa::FontRef::from_index(&f.data, f.index).expect("font");
                let loc = font.axes().location(run.coords.iter().map(|(t, v)| (skrifa::Tag::from_u32(*t), *v)));
                Arc::new(loc.coords().iter().map(|c| c.to_bits()).collect())
            })
            .clone()
    }
}

/// glifo caches unhinted fill outlines at units-per-em size and scales them
/// when drawing, so its expansion is in font units (callers convert).
fn glifo_embolden(a: f64) -> glifo::FontEmbolden {
    glifo::FontEmbolden::new(kurbo::Diagonal2::new(a, a))
}

fn color(c: &Color) -> peniko::Color {
    peniko::Color::new(*c)
}

fn stroke_style(width: f64, round: bool) -> Stroke {
    Stroke::new(width).with_join(if round { Join::Round } else { Join::Miter }).with_caps(Cap::Butt)
}

/// Text stroke as captions/render.ts sets it: 2 × stroke, round joins, miter limit 2.
fn text_stroke(stroke: f64) -> Stroke {
    Stroke::new(stroke * 2.0).with_join(Join::Round).with_miter_limit(2.0)
}

/// Timings of one frame: CPU time to build the scene, and wall time until
/// the GPU finished (CPU-only renderers report the same value twice).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTime {
    pub encode: Duration,
    pub total: Duration,
}

pub struct VelloClassic {
    renderer: vello::Renderer,
    pub fonts: Fonts,
    target: Option<wgpu::Texture>,
}

impl VelloClassic {
    pub fn new(gpu: &Gpu) -> Self {
        let renderer = vello::Renderer::new(&gpu.device, vello::RendererOptions { use_cpu: false, antialiasing_support: vello::AaSupport::area_only(), num_init_threads: None, pipeline_cache: None })
            .expect("vello renderer");
        VelloClassic { renderer, fonts: Fonts::default(), target: None }
    }

    fn encode(&mut self, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> vello::Scene {
        let mut s = vello::Scene::new();
        for draw in &scene.draws {
            match draw {
                Draw::Text { runs, color: c, stroke, stroke_color } => {
                    for pass in [true, false] {
                        if pass && *stroke <= 0.0 {
                            continue;
                        }
                        for run in runs {
                            let font = self.fonts.font(faces, run.face);
                            let coords = self.fonts.normalized(faces, run);
                            let tune = if pass { self.fonts.tune.outline_only() } else { self.fonts.tune };
                            let bitmap = faces[&run.face].bitmap;
                            let glyphs = run.glyphs.iter().map(|g| {
                                let (x, y) = tune.position(g.x, g.y, run.size, bitmap);
                                vello::Glyph { id: g.id, x, y }
                            });
                            let a = tune.amount(run.size, bitmap);
                            let b = s
                                .draw_glyphs(&font)
                                .font_size(run.size)
                                .normalized_coords(&coords)
                                .hint(false)
                                .font_embolden(vello::FontEmbolden::new(kurbo::Diagonal2::new(a, a)));
                            if pass {
                                b.brush(color(stroke_color)).draw(&text_stroke(*stroke), glyphs);
                            } else {
                                b.brush(color(c)).draw(Fill::NonZero, glyphs);
                            }
                        }
                    }
                }
                Draw::Path { path, color: c, even_odd, stroke, round_join } => {
                    if *stroke > 0.0 {
                        s.stroke(&stroke_style(*stroke, *round_join), Affine::IDENTITY, color(c), None, path);
                    } else {
                        s.fill(if *even_odd { Fill::EvenOdd } else { Fill::NonZero }, Affine::IDENTITY, color(c), None, path);
                    }
                }
            }
        }
        s
    }

    pub fn frame(&mut self, gpu: &Gpu, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> FrameTime {
        let started = Instant::now();
        let s = self.encode(scene, faces);
        let encode = started.elapsed();
        let (w, h) = (scene.width, scene.height);
        if self.target.as_ref().is_none_or(|t| t.width() != w || t.height() != h) {
            self.target = Some(gpu.target(w, h, wgpu::TextureUsages::STORAGE_BINDING));
        }
        let view = self.target.as_ref().unwrap().create_view(&Default::default());
        self.renderer
            .render_to_texture(&gpu.device, &gpu.queue, &s, &view, &vello::RenderParams { base_color: peniko::Color::TRANSPARENT, width: w, height: h, antialiasing_method: vello::AaConfig::Area })
            .expect("vello render");
        gpu.wait();
        FrameTime { encode, total: started.elapsed() }
    }

    pub fn render(&mut self, gpu: &Gpu, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> Image {
        self.frame(gpu, scene, faces);
        // Vello's fine stage stores straight (unpremultiplied) color.
        gpu.read(self.target.as_ref().unwrap(), false)
    }
}

pub struct VelloGpu {
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    pub fonts: Fonts,
    size: (u16, u16),
    target: Option<(wgpu::Texture, wgpu::TextureView)>,
    scene: vello_gpu::Scene,
}

impl VelloGpu {
    pub fn new(gpu: &Gpu, w: u16, h: u16) -> Self {
        let (renderer, resources) = vello_gpu::Renderer::new(&gpu.device, &vello_gpu::RenderTargetConfig { format: wgpu::TextureFormat::Rgba8Unorm, width: w, height: h });
        VelloGpu { renderer, resources, fonts: Fonts::default(), size: (w, h), target: None, scene: vello_gpu::Scene::new(w, h) }
    }

    pub fn frame(&mut self, gpu: &Gpu, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> FrameTime {
        let (w, h) = (scene.width as u16, scene.height as u16);
        if self.size != (w, h) {
            let tune = self.fonts.tune;
            *self = VelloGpu::new(gpu, w, h);
            self.fonts.tune = tune;
        }
        let started = Instant::now();
        let s = &mut self.scene;
        s.reset();
        for draw in &scene.draws {
            match draw {
                Draw::Text { runs, color: c, stroke, stroke_color } => {
                    for pass in [true, false] {
                        if pass && *stroke <= 0.0 {
                            continue;
                        }
                        if pass {
                            s.set_stroke(text_stroke(*stroke));
                            s.set_paint(color(stroke_color));
                        } else {
                            s.set_paint(color(c));
                        }
                        for run in runs {
                            let font = self.fonts.font(faces, run.face);
                            let coords = self.fonts.normalized(faces, run);
                            let tune = if pass { self.fonts.tune.outline_only() } else { self.fonts.tune };
                            let bitmap = faces[&run.face].bitmap;
                            let glyphs = run.glyphs.iter().map(move |g| {
                                let (x, y) = tune.position(g.x, g.y, run.size, bitmap);
                                vello_cpu::Glyph { id: g.id, x, y }
                            });
                            // vello_gpu draws bitmap glyphs only through its glyph atlas
                            // (direct bitmap paints panic: "pixmap image sources are not
                            // supported"); outlines are drawn directly.
                            let b = s.glyph_run(&mut self.resources, &font).font_size(run.size).normalized_coords(&coords).hint(false).atlas_cache(faces[&run.face].bitmap)
                                .font_embolden(glifo_embolden(tune.amount(run.size, bitmap) * self.fonts.upem(faces, run.face) as f64 / run.size as f64));
                            let r = if pass { b.stroke_glyphs(glyphs) } else { b.fill_glyphs(glyphs) };
                            r.expect("vello_gpu glyphs");
                        }
                    }
                }
                Draw::Path { path, color: c, even_odd, stroke, round_join } => {
                    s.set_paint(color(c));
                    if *stroke > 0.0 {
                        s.set_stroke(stroke_style(*stroke, *round_join));
                        s.stroke_path(path);
                    } else {
                        s.set_fill_rule(if *even_odd { Fill::EvenOdd } else { Fill::NonZero });
                        s.fill_path(path);
                    }
                }
            }
        }
        if self.target.is_none() {
            let t = gpu.target(w as u32, h as u32, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let v = t.create_view(&Default::default());
            self.target = Some((t, v));
        }
        let encode = started.elapsed();
        let size = vello_gpu::RenderSize { width: w, height: h };
        let depth = vello_gpu::Renderer::create_depth_texture_view(&gpu.device, &size);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.renderer
            .render(
                &self.scene,
                &mut self.resources,
                &gpu.device,
                &gpu.queue,
                &mut enc,
                &size,
                &self.target.as_ref().unwrap().1,
                Some(&depth),
                &vello_gpu::TextureBindings::new(),
                vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()),
            )
            .expect("vello_gpu render");
        gpu.queue.submit([enc.finish()]);
        gpu.wait();
        FrameTime { encode, total: started.elapsed() }
    }

    pub fn render(&mut self, gpu: &Gpu, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> Image {
        self.frame(gpu, scene, faces);
        gpu.read(&self.target.as_ref().unwrap().0, true)
    }
}

/// Four worker threads (the spike's machine-load limit).
fn cpu_context(w: u16, h: u16) -> vello_cpu::RenderContext {
    vello_cpu::RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: 4, ..Default::default() })
}

pub struct VelloCpu {
    ctx: vello_cpu::RenderContext,
    resources: vello_cpu::Resources,
    pub fonts: Fonts,
    pixmap: vello_cpu::Pixmap,
}

impl VelloCpu {
    pub fn new(w: u16, h: u16) -> Self {
        VelloCpu { ctx: cpu_context(w, h), resources: vello_cpu::Resources::new(), fonts: Fonts::default(), pixmap: vello_cpu::Pixmap::new(w, h) }
    }

    pub fn frame(&mut self, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> FrameTime {
        let (w, h) = (scene.width as u16, scene.height as u16);
        if (self.ctx.width(), self.ctx.height()) != (w, h) {
            self.ctx = cpu_context(w, h);
            self.pixmap = vello_cpu::Pixmap::new(w, h);
        }
        let started = Instant::now();
        let s = &mut self.ctx;
        s.reset();
        for draw in &scene.draws {
            match draw {
                Draw::Text { runs, color: c, stroke, stroke_color } => {
                    for pass in [true, false] {
                        if pass && *stroke <= 0.0 {
                            continue;
                        }
                        if pass {
                            s.set_stroke(text_stroke(*stroke));
                            s.set_paint(color(stroke_color));
                        } else {
                            s.set_paint(color(c));
                        }
                        for run in runs {
                            let font = self.fonts.font(faces, run.face);
                            let coords = self.fonts.normalized(faces, run);
                            let tune = if pass { self.fonts.tune.outline_only() } else { self.fonts.tune };
                            let bitmap = faces[&run.face].bitmap;
                            let glyphs = run.glyphs.iter().map(move |g| {
                                let (x, y) = tune.position(g.x, g.y, run.size, bitmap);
                                vello_cpu::Glyph { id: g.id, x, y }
                            });
                            let b = s.glyph_run(&mut self.resources, &font).font_size(run.size).normalized_coords(&coords).hint(false)
                                .font_embolden(glifo_embolden(tune.amount(run.size, bitmap) * self.fonts.upem(faces, run.face) as f64 / run.size as f64));
                            let r = if pass { b.stroke_glyphs(glyphs) } else { b.fill_glyphs(glyphs) };
                            r.expect("vello_cpu glyphs");
                        }
                    }
                }
                Draw::Path { path, color: c, even_odd, stroke, round_join } => {
                    s.set_paint(color(c));
                    if *stroke > 0.0 {
                        s.set_stroke(stroke_style(*stroke, *round_join));
                        s.stroke_path(path);
                    } else {
                        s.set_fill_rule(if *even_odd { Fill::EvenOdd } else { Fill::NonZero });
                        s.fill_path(path);
                    }
                }
            }
        }
        s.flush();
        let encode = started.elapsed();
        // The pixmap is overwritten (cleared) by each render.
        for p in self.pixmap.data_mut() {
            *p = vello_cpu::color::PremulRgba8 { r: 0, g: 0, b: 0, a: 0 };
        }
        s.render(&mut self.pixmap, &mut self.resources);
        FrameTime { encode, total: started.elapsed() }
    }

    pub fn render(&mut self, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> Image {
        self.frame(scene, faces);
        let bytes: Vec<u8> = self.pixmap.data().iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
        Image::from_rgba8(scene.width, scene.height, &bytes, true)
    }
}
