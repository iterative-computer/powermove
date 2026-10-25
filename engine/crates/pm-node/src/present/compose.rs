//! A document frame for the viewer (Phase 2 tracer bullet; plan sections 8
//! and 18): the composition the viewer shows, drawn by `pm-render` where the
//! viewer's zoom and pan put it, into the presenter's working texture.
//!
//! The viewer state's size is the stage in device pixels; zoom is CSS
//! pixels per composition pixel and pan CSS pixels from the centered
//! position, so the composition rectangle is the one the legacy viewer
//! places its canvas in (`view::composition_rect` for the test source). It
//! is snapped to whole device pixels, so at Full resolution every
//! composition pixel lands on one frame pixel and the frame is sampled
//! 1:1. Only the visible part is rendered (`pm-render`'s region of
//! interest), at the preview resolution, then placed: composited over
//! opaque black inside the rectangle (today's WebGL canvas is opaque) and
//! transparent around it, where the viewer's stage shows through.

use std::collections::HashMap;
use std::sync::Arc;

use pm_gpu::Target;
use pm_gpu::bridge_out::display::{WORKING_FORMAT, WorkingTexture};
use pm_protocol::color::Precision;
use pm_protocol::common::{Mat3, PixelRect, Rect, Size, Ticks};
use pm_protocol::frame::{OverlayGeometry, OverlayLayer, PreviewResolution, ViewerState};
use pm_render::effects::Effects;
use pm_render::{AssetResolver, LayerFailure, RenderError, RenderRequest, Renderer};

/// Where the composition goes in a viewer frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Where output pixel (0, 0)'s top-left corner lands, in frame pixels.
    pub x: f64,
    pub y: f64,
    /// Frame pixels per output pixel, per axis (1 / the preview resolution,
    /// or today's canvas box over its backing size).
    pub fx: f64,
    pub fy: f64,
    /// The output: the whole composition at the render scale, today's
    /// canvas, or (a view larger than the largest output without a
    /// canvas) the visible part of the composition.
    pub output: Size,
    /// Output pixels per composition pixel when it is not the output over
    /// the composition (today's crop, or a visible part); `None`: derived
    /// from the output and composition sizes.
    pub scale: Option<(f64, f64)>,
    /// Output pixel (0, 0) in scaled composition pixels
    /// ([`RenderRequest::origin`](pm_render::RenderRequest::origin)): the
    /// crop's origin at today's scale, unrounded, as today's
    /// `scaledWorld` puts it (`crop.x × sx`). `None`: the composition's
    /// origin.
    pub origin: Option<(f64, f64)>,
    /// The visible part of the output (`None`: nothing is visible).
    pub region: Option<PixelRect>,
    /// The frame-pixel rectangle the composition covers.
    pub comp: Rect,
    /// The frame pixels the placement writes (today's snapped canvas box
    /// within the frame); `None`: the region's rectangle.
    pub clip: Option<PixelRect>,
    /// Today's canvas (decision V1): the region is stored as today's 8-bit
    /// canvas and scaled into its box with Chromium's bilinear sampling.
    pub canvas: bool,
}

impl Placement {
    /// The frame-pixel rectangle the composition covers.
    pub fn rect(&self) -> Rect {
        self.comp
    }

    /// Where the rendered region goes in the frame.
    pub fn region_rect(&self) -> Option<Rect> {
        let r = self.region?;
        Some(Rect {
            x: self.x + f64::from(r.x) * self.fx,
            y: self.y + f64::from(r.y) * self.fy,
            width: f64::from(r.width) * self.fx,
            height: f64::from(r.height) * self.fy,
        })
    }

    /// An output-pixel point in frame pixels.
    pub fn to_frame(&self, p: [f64; 2]) -> [f64; 2] {
        [self.x + p[0] * self.fx, self.y + p[1] * self.fy]
    }
}

/// The output scale of a preview resolution (Automatic draws at Full until
/// the quality ladder lands with proxies, section 14).
pub fn resolution_scale(resolution: PreviewResolution) -> f64 {
    match resolution {
        PreviewResolution::Automatic | PreviewResolution::Full => 1.0,
        PreviewResolution::Half => 0.5,
        PreviewResolution::Quarter => 0.25,
    }
}

/// The placement of a `width` × `height` composition in a viewer frame:
/// today's canvas when the viewer sends it ([`canvas_placement`]), else the
/// composition at zoom × DPR × the preview resolution.
pub fn placement(state: &ViewerState, width: f64, height: f64) -> Placement {
    if let Some(canvas) = state.canvas.as_ref() {
        return canvas_placement(state, canvas, width, height);
    }
    let dpr = if state.device_pixel_ratio > 0.0 {
        state.device_pixel_ratio
    } else {
        1.0
    };
    let zoom = if state.zoom > 0.0 { state.zoom } else { 1.0 };
    let resolution = resolution_scale(state.resolution);
    let frame_w = f64::from(state.size.width);
    let frame_h = f64::from(state.size.height);
    let full_w = (width * zoom * dpr).max(1.0);
    let full_h = (height * zoom * dpr).max(1.0);
    let x = ((frame_w - full_w) / 2.0 + state.pan[0] * dpr).round();
    let y = ((frame_h - full_h) / 2.0 + state.pan[1] * dpr).round();
    let max = f64::from(pm_protocol::frame::MAX_OUTPUT_DIMENSION);
    // The whole composition at the render scale; beyond the largest
    // output, only the visible part is drawn, from its own origin.
    let (full_out_w, full_out_h) = (
        (full_w * resolution).round().max(1.0),
        (full_h * resolution).round().max(1.0),
    );
    let span = |origin: f64, frame: f64, out: f64| -> Option<(f64, f64)> {
        let a = ((0.0 - origin) * resolution).floor().clamp(0.0, out);
        let b = ((frame - origin) * resolution).ceil().clamp(0.0, out);
        (b > a).then_some((a, b - a))
    };
    let visible = match (span(x, frame_w, full_out_w), span(y, frame_h, full_out_h)) {
        (Some(h), Some(v)) => Some((h, v)),
        _ => None,
    };
    let comp = Rect {
        x,
        y,
        width: full_out_w / resolution,
        height: full_out_h / resolution,
    };
    let base = Placement {
        x,
        y,
        fx: 1.0 / resolution,
        fy: 1.0 / resolution,
        output: Size::new(full_out_w.min(max) as u32, full_out_h.min(max) as u32),
        scale: None,
        origin: None,
        region: None,
        comp,
        clip: None,
        canvas: false,
    };
    let Some(((rx, rw), (ry, rh))) = visible else {
        return base;
    };
    if full_out_w <= max && full_out_h <= max {
        return Placement {
            region: Some(PixelRect {
                x: rx as u32,
                y: ry as u32,
                width: rw as u32,
                height: rh as u32,
            }),
            ..base
        };
    }
    // The visible part only, at the same scale and on the same pixel grid
    // (its origin is a whole output pixel).
    Placement {
        x: x + rx / resolution,
        y: y + ry / resolution,
        output: Size::new(rw as u32, rh as u32),
        scale: Some((full_out_w / width, full_out_h / height)),
        origin: Some((rx, ry)),
        region: Some(PixelRect {
            x: 0,
            y: 0,
            width: rw as u32,
            height: rh as u32,
        }),
        ..base
    }
}

/// Today's viewer canvas (decision V1): the output is the canvas today's
/// viewer draws (its backing size, at today's `outputScale`: the canvas
/// over the part of the composition it holds), and it covers the canvas's
/// CSS box snapped to whole device pixels as Blink snaps a replaced
/// element's content box (each edge rounded).
///
/// A crop's output starts at the crop (decision V1b): composition pixel
/// `x` lands at output pixel `x · sx − crop.x · sx`, the origin unrounded,
/// as today's `scaledWorld` puts it with a preview viewport. The output is
/// the canvas itself, so a crop never needs more than today's canvas
/// (at most the largest output), at today's full scale.
pub fn canvas_placement(
    state: &ViewerState,
    canvas: &pm_protocol::frame::ViewerCanvas,
    width: f64,
    height: f64,
) -> Placement {
    let dpr = if state.device_pixel_ratio > 0.0 {
        state.device_pixel_ratio
    } else {
        1.0
    };
    let snap = |v: f64| (v * dpr).round();
    let (x0, x1) = (snap(canvas.css.x), snap(canvas.css.x + canvas.css.width));
    let (y0, y1) = (snap(canvas.css.y), snap(canvas.css.y + canvas.css.height));
    let (w, h) = (f64::from(canvas.width), f64::from(canvas.height));
    let (scale, origin) = canvas_render_grid(canvas, width, height);
    let (sx, sy) = scale.unwrap_or((w / width, h / height));
    let (ox, oy) = origin.unwrap_or((0.0, 0.0));
    let output = Size::new(canvas.width, canvas.height);
    let fx = (x1 - x0) / w;
    let fy = (y1 - y0) / h;
    let comp = Rect {
        x: x0 - ox * fx,
        y: y0 - oy * fy,
        width: width * sx * fx,
        height: height * sy * fy,
    };
    // The frame pixels inside the box, and the canvas texels their bilinear
    // samples read (`u = (i + 0.5 − x0) / fx − 0.5` and the texel after it).
    let span = |lo: f64, hi: f64, frame: u32, f: f64, n: f64| -> Option<((u32, u32), (u32, u32))> {
        let a = lo.max(0.0);
        let b = hi.min(f64::from(frame));
        if !(b > a && f > 0.0) {
            return None;
        }
        let u0 = ((a + 0.5 - lo) / f - 0.5).floor().clamp(0.0, n);
        let u1 = (((b - 0.5 - lo) / f - 0.5).floor() + 2.0).clamp(0.0, n);
        (u1 > u0).then_some(((a as u32, (b - a) as u32), (u0 as u32, (u1 - u0) as u32)))
    };
    let (region, clip) = match (
        span(x0, x1, state.size.width, fx, w),
        span(y0, y1, state.size.height, fy, h),
    ) {
        (Some(((cx, cw), (rx, rw))), Some(((cy, ch), (ry, rh)))) => (
            Some(PixelRect {
                x: rx,
                y: ry,
                width: rw,
                height: rh,
            }),
            Some(PixelRect {
                x: cx,
                y: cy,
                width: cw,
                height: ch,
            }),
        ),
        _ => (None, None),
    };
    Placement {
        x: x0,
        y: y0,
        fx,
        fy,
        output,
        scale,
        origin,
        region,
        comp,
        clip,
        canvas: true,
    }
}

/// The unscaled backing canvas grid shared by live composition and pinned
/// viewer snapshots. CSS placement only happens after this render.
pub(crate) fn canvas_render_grid(
    canvas: &pm_protocol::frame::ViewerCanvas,
    width: f64,
    height: f64,
) -> (Option<(f64, f64)>, Option<(f64, f64)>) {
    let whole = canvas.crop.x == 0.0
        && canvas.crop.y == 0.0
        && canvas.crop.width == width
        && canvas.crop.height == height;
    if whole {
        return (None, None);
    }
    let sx = f64::from(canvas.width) / canvas.crop.width;
    let sy = f64::from(canvas.height) / canvas.crop.height;
    (
        Some((sx, sy)),
        Some((canvas.crop.x * sx, canvas.crop.y * sy)),
    )
}

/// What a composed frame reports.
#[derive(Debug, Clone)]
pub struct Composed {
    pub scene3d_meta: Vec<pm_protocol::frame::Scene3dMeta>,
    pub overlay: OverlayGeometry,
    pub chrome_legend: Vec<String>,
    pub chrome_glyphs: Vec<pm_protocol::frame::ChromeGlyph>,
    pub chrome_selection: Option<pm_protocol::frame::ChromeSelection>,
    pub failures: Vec<LayerFailure>,
    /// Render and placement time on the CPU (µs), the GPU work submitted.
    pub cpu_us: i64,
    /// Video layers drawn with a stand-in while their exact frame decodes
    /// (scrub renders, docs/engine/scrub.md): the source frame each shows.
    pub stand_ins: Vec<usize>,
}

const BLIT_WGSL: &str = r#"
struct U { dst: vec4f };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let q = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(q * 2.0 - 1.0, 0.0, 1.0);
}

// Over opaque black, as today's opaque WebGL canvas shows a composition.
@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let uv = (pos.xy - u.dst.xy) / u.dst.zw;
    let c = textureSampleLevel(src, samp, uv, 0.0);
    return vec4f(c.rgb, 1.0);
}
"#;

/// Today's WebGL canvas holds 8 bits per channel (opaque: the composition
/// over black). The region is stored that way before it is scaled, so the
/// upscale filters the values today's compositor would have filtered.
const CANVAS_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let q = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(q * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let c = textureLoad(src, vec2i(pos.xy), 0);
    return vec4f(c.rgb, 1.0);
}
"#;

/// Today's canvas format.
const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Renders documents for the presenter. One per GPU device.
pub struct Composer {
    renderer: Renderer,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    scratch: Option<Target>,
    /// The region as today's 8-bit canvas (decision V1).
    canvas_scratch: Option<Target>,
    canvas_pipeline: wgpu::RenderPipeline,
    canvas_layout: wgpu::BindGroupLayout,
    /// The viewer is playing: today's playing motion-blur count and the
    /// playback resampler (`set_playing`).
    playing: bool,
    /// Whether viewer frames reuse kept layer draws (on; `compose_bench`
    /// turns it off to compare).
    draw_reuse: bool,
    /// The view (hashed) whose definition set is in the renderer, and the
    /// identity and version of the set it was loaded from.
    definitions: Option<(u64, Option<(usize, u64)>)>,
    /// Every other view's definition set, kept while another view draws:
    /// each view keeps its own programs and watchdog state, so views
    /// sharing the device never reset each other's.
    parked: HashMap<u64, (Effects, Option<(usize, u64)>)>,
}

fn owner_hash(owner: impl std::hash::Hash) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(owner)
}

impl Composer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Composer {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewer placement"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer placement"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewer placement"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewer placement"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: WORKING_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let canvas_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewer canvas"),
            source: wgpu::ShaderSource::Wgsl(CANVAS_WGSL.into()),
        });
        let canvas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer canvas"),
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
        let canvas_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("viewer canvas"),
                bind_group_layouts: &[Some(&canvas_layout)],
                immediate_size: 0,
            });
        let canvas_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewer canvas"),
            layout: Some(&canvas_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &canvas_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &canvas_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: CANVAS_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer placement"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("viewer placement"),
                mag_filter: filter,
                min_filter: filter,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            })
        };
        Composer {
            renderer: Renderer::new(device.clone(), queue.clone()),
            pipeline,
            layout,
            uniforms,
            linear: sampler(wgpu::FilterMode::Linear),
            nearest: sampler(wgpu::FilterMode::Nearest),
            scratch: None,
            canvas_scratch: None,
            canvas_pipeline,
            canvas_layout,
            definitions: None,
            parked: HashMap::new(),
            playing: false,
            draw_reuse: true,
        }
    }

    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    /// Adopts a definition set translated before this device and renderer
    /// existed. Runtime parity belongs to the renderer; prepared sets only
    /// carry CPU translation and their per-owner watchdog state.
    pub fn park_prepared_definitions(
        &mut self,
        owner: impl std::hash::Hash,
        mut effects: Effects,
        loaded: (usize, u64),
    ) {
        let parity = &self.renderer.effects().parity;
        if effects.parity.environment() != parity.environment()
            || effects.parity.directory() != parity.directory()
        {
            effects.parity.configure(
                parity.environment().map(str::to_owned),
                parity.directory().map(std::path::Path::to_path_buf),
            );
        }
        self.parked
            .insert(owner_hash(owner), (effects, Some(loaded)));
    }

    /// Polls all owners before their CPU definition state leaves this device.
    /// An owner may have been parked while its last frame was in flight.
    pub fn definitions_quiescent(&mut self) -> bool {
        let device = self.renderer.device().clone();
        let queue = self.renderer.queue().clone();
        self.renderer
            .effects_mut()
            .watchdog
            .collect(&device, &queue, false);
        for (effects, _) in self.parked.values_mut() {
            effects.watchdog.collect(&device, &queue, false);
        }
        self.renderer.effects().watchdog.frames_in_flight() == 0
            && self
                .parked
                .values()
                .all(|(effects, _)| effects.watchdog.frames_in_flight() == 0)
    }

    /// Moves one owner's translated definitions out of a retiring composer.
    /// Programs and query objects are discarded; source preparation,
    /// watchdog judgments and parity decisions survive until reattachment.
    pub fn take_prepared_definitions(
        &mut self,
        owner: impl std::hash::Hash,
    ) -> Option<(Effects, (usize, u64))> {
        let owner = owner_hash(owner);
        let (mut effects, loaded) = if self.definitions.map(|d| d.0) == Some(owner) {
            let (_, loaded) = self.definitions.take().expect("owner above");
            let mut empty = self.renderer.effects().sibling();
            self.renderer.swap_effects(&mut empty);
            (empty, loaded)
        } else {
            self.parked.remove(&owner)?
        };
        if !effects.release_device() {
            self.parked.insert(owner, (effects, loaded));
            return None;
        }
        loaded.map(|loaded| (effects, loaded))
    }

    /// Puts `owner`'s definition set in the renderer (parking the one it
    /// held) and brings it up to `definitions` when that set changed: a
    /// changed set translates only definitions whose source changed, and
    /// keeps the programs and watchdog records of the rest (an effect the
    /// watchdog turned off stays off). Returns the translation failures
    /// when it loaded a changed set.
    pub fn use_definitions(
        &mut self,
        owner: impl std::hash::Hash,
        definitions: Option<&(Arc<pm_protocol::definitions::DefinitionSetSnapshot>, u64)>,
        registered: &[(
            pm_protocol::definitions::DefinitionKind,
            pm_protocol::common::DefinitionId,
        )],
    ) -> Option<Vec<pm_protocol::error::EngineError>> {
        let owner = owner_hash(owner);
        let version = definitions.map(|(set, version)| (Arc::as_ptr(set) as usize, *version));
        if self.definitions.map(|d| d.0) != Some(owner) {
            if let Some((previous, loaded)) = self.definitions.take() {
                let mut out = self.renderer.effects().sibling();
                self.renderer.swap_effects(&mut out);
                self.parked.insert(previous, (out, loaded));
            }
            let (mut set, loaded) = self
                .parked
                .remove(&owner)
                .unwrap_or_else(|| (self.renderer.effects().sibling(), None));
            self.renderer.swap_effects(&mut set);
            self.definitions = Some((owner, loaded));
        }
        // Registered again (a register or hot reload of the id, even with
        // the same source): the watchdog starts over for it.
        for (kind, id) in registered {
            self.renderer.effects_mut().watchdog.forget(*kind, id);
        }
        let current = self.definitions.as_mut().expect("set above");
        if current.1 == version {
            return None;
        }
        current.1 = version;
        match definitions {
            Some((set, _)) => Some(self.renderer.set_definitions(set).unwrap_or_default()),
            None => {
                self.renderer.clear_definitions();
                Some(Vec::new())
            }
        }
    }

    /// Drops `owner`'s definition set (its session closed).
    pub fn forget_definitions(&mut self, owner: impl std::hash::Hash) {
        let owner = owner_hash(owner);
        self.parked.remove(&owner);
        if self.definitions.map(|d| d.0) == Some(owner) {
            self.definitions = None;
            let mut empty = self.renderer.effects().sibling();
            self.renderer.swap_effects(&mut empty);
        }
    }

    /// Whether viewer frames reuse the draws of layers nothing changed
    /// since the last frame (`pm_render::reuse`; on by default).
    pub fn set_draw_reuse(&mut self, enabled: bool) {
        self.draw_reuse = enabled;
        if !enabled {
            self.renderer.clear_draw_reuse();
        }
    }

    /// Whether the next frames are playback frames (timed) or paused ones.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Places the rendered region (`source`) where `place` puts it in a
    /// `size` target (`view`), cleared to transparent around it. `exact`:
    /// always nearest sampling (depth and ids must not be filtered).
    #[allow(clippy::too_many_arguments)]
    fn place(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: Size,
        place: &Placement,
        source: Option<&Target>,
        exact: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewer placement"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let (Some(dst), Some(source)) = (place.region_rect(), source) else {
            return;
        };
        if place.region.is_none() {
            return;
        }
        let (x0, y0, x1, y1) = match place.clip {
            // Today's snapped canvas box: the bilinear margin texels outside
            // it are never written.
            Some(c) => (
                c.x.min(size.width),
                c.y.min(size.height),
                (c.x + c.width).min(size.width),
                (c.y + c.height).min(size.height),
            ),
            None => (
                dst.x.max(0.0).floor() as u32,
                dst.y.max(0.0).floor() as u32,
                ((dst.x + dst.width).ceil().max(0.0) as u32).min(size.width),
                ((dst.y + dst.height).ceil().max(0.0) as u32).min(size.height),
            ),
        };
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let u = [
            dst.x as f32,
            dst.y as f32,
            dst.width as f32,
            dst.height as f32,
        ];
        let mut bytes = [0u8; 16];
        for (i, v) in u.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        // Both placements of a frame write the same rectangle.
        queue.write_buffer(&self.uniforms, 0, &bytes);
        let source_view = source.texture.create_view(&Default::default());
        // Chromium scales a canvas into its CSS box bilinearly (no mipmaps,
        // clamped at the canvas's edges); depth and ids are never filtered.
        let sampler = if exact || (!place.canvas && place.fx == 1.0 && place.fy == 1.0) {
            &self.nearest
        } else {
            &self.linear
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewer placement"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
        pass.draw(0..3, 0..1);
    }

    /// Stores the rendered region as today's 8-bit opaque canvas (decision
    /// V1): the target is taken out of `canvas_scratch` for this frame's
    /// placement and kept there again after it.
    fn store_canvas(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
    ) -> Option<Target> {
        let source = self.scratch.as_ref()?;
        let (w, h) = (source.texture.width(), source.texture.height());
        let canvas = match self.canvas_scratch.take() {
            Some(t) if t.texture.width() == w && t.texture.height() == h => t,
            _ => Target::new(
                device,
                "viewer canvas",
                w,
                h,
                CANVAS_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        };
        let source_view = source.texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewer canvas"),
            layout: &self.canvas_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source_view),
            }],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewer canvas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &canvas.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.canvas_pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        Some(canvas)
    }

    /// Draws `composition` of `doc` at `time` into `working` for `state`.
    /// On an error the working texture is left as it was.
    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        &mut self,
        doc: &pm_model::Document,
        composition: &str,
        media: &dyn AssetResolver,
        time: Ticks,
        state: &ViewerState,
        working: &WorkingTexture,
    ) -> Result<Composed, RenderError> {
        self.compose_with_depth(doc, composition, media, time, state, working, None)
    }

    /// [`Self::compose`], also writing the paired `chromePlanesV1` depth
    /// and ID/selection surfaces at the stage's device-pixel size.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_depth(
        &mut self,
        doc: &pm_model::Document,
        composition: &str,
        media: &dyn AssetResolver,
        time: Ticks,
        state: &ViewerState,
        working: &WorkingTexture,
        depth: Option<(&wgpu::TextureView, &wgpu::TextureView)>,
    ) -> Result<Composed, RenderError> {
        let start = super::render::now_us();
        let comp = pm_eval::Comp::resolve(doc, composition)
            .map_err(|_| RenderError::Composition(composition.to_owned()))?;
        let number = |key: &str| {
            let v = pm_eval::node::to_number(comp.member(key));
            if v.is_finite() && v > 0.0 { v } else { 1.0 }
        };
        let (cw, ch) = (number("w"), number("h"));
        let place = placement(state, cw, ch);
        let device = self.renderer.device().clone();
        let queue = self.renderer.queue().clone();
        let mut failures = Vec::new();
        let mut quads = Vec::new();
        let mut stand_ins = Vec::new();
        let mut scene3d_meta = Vec::new();
        if let Some(region) = place.region {
            let request = RenderRequest {
                output_size: place.output,
                scale: place.scale,
                origin: place.origin,
                region: Some(region),
                preview_canvas: place.canvas,
                precision: Precision::Float16,
                depth_id: false,
                background: true,
                export: false,
                // The viewer's override: editor camera, Wireframe and
                // X-ray looks, Local View, and shading.
                view: state.view.clone(),
                // Today's viewer: 6 samples while playing, 12 when paused;
                // the playback resampler while playing.
                motion_blur: pm_render::motion::MotionBlur::Kind(if self.playing {
                    pm_protocol::frame::MotionBlurKind::PreviewPlaying
                } else {
                    pm_protocol::frame::MotionBlurKind::PreviewPaused
                }),
                sampling: if self.playing {
                    pm_render::sampling::Sampling::Playback
                } else {
                    pm_render::sampling::Sampling::Final
                },
                video_end: pm_render::VideoEnd::Preview,
            };
            let format = self.renderer.device_target_format(doc, &request);
            let reuse = self.scratch.as_ref().is_some_and(|t| {
                t.width() == region.width && t.height() == region.height && t.format() == format
            });
            if !reuse {
                self.scratch = Some(Target::new(
                    &device,
                    "viewer region",
                    region.width,
                    region.height,
                    format,
                    pm_gpu::target::working_usage(),
                ));
            }
            let scratch = self.scratch.as_ref().expect("made above");
            // The viewer's documents live as long as their view and change
            // in place: frames of a paused time reuse the draws of layers
            // nothing changed (docs/engine/perf.md, "Draw reuse").
            self.renderer.set_draw_reuse(self.draw_reuse);
            let rendered =
                self.renderer
                    .render_into(doc, composition, time, &request, media, scratch, None);
            self.renderer.set_draw_reuse(false);
            let rendered = rendered?;
            scene3d_meta = rendered.scene3d_meta;
            failures = rendered.failures;
            quads = rendered.quads;
            stand_ins = rendered.stand_ins;
        }
        // Place the region (or only clear when nothing is visible).
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewer placement"),
        });
        let canvas = if place.canvas && place.region.is_some() {
            self.store_canvas(&mut encoder, &device)
        } else {
            None
        };
        self.place(
            &mut encoder,
            &device,
            &queue,
            &working.view,
            working.size,
            &place,
            canvas.as_ref().or(self.scratch.as_ref()),
            false,
        );
        queue.submit([encoder.finish()]);
        if let Some(canvas) = canvas {
            self.canvas_scratch = Some(canvas);
        }
        let (chrome_legend, chrome_glyphs, chrome_selection) =
            if let Some((depth, coverage)) = depth {
                // Chrome follows today's fractional CSS composition frame.
                // Color placement snaps its backing buffer independently.
                let width = cw * state.zoom * state.device_pixel_ratio;
                let height = ch * state.zoom * state.device_pixel_ratio;
                let x = (f64::from(state.size.width) - width) / 2.0
                    + state.pan[0] * state.device_pixel_ratio;
                let y = (f64::from(state.size.height) - height) / 2.0
                    + state.pan[1] * state.device_pixel_ratio;
                let (legend, glyphs, selection) = self.renderer.render_viewport_chrome(
                    doc,
                    composition,
                    time,
                    media,
                    state.view.as_ref(),
                    state.chrome_selection.as_ref(),
                    (state.size.width, state.size.height),
                    [x, y, width, height],
                    depth,
                    coverage,
                )?;
                (legend, glyphs, Some(selection))
            } else {
                (vec![], vec![], None)
            };
        Ok(Composed {
            scene3d_meta,
            chrome_legend,
            chrome_glyphs,
            chrome_selection,
            overlay: overlay(&place, cw, ch, &quads),
            failures,
            cpu_us: super::render::now_us() - start,
            stand_ins,
        })
    }
}

/// The overlay geometry of a composed frame, in frame pixels: the
/// composition rectangle as the viewport, and each top-level layer's
/// drawn quad (a layer drawn as several quads reports their bounding box).
pub fn overlay(
    place: &Placement,
    cw: f64,
    ch: f64,
    quads: &[pm_render::LayerQuad],
) -> OverlayGeometry {
    let rect = place.rect();
    let sx = rect.width / cw;
    let sy = rect.height / ch;
    // Composition pixels to frame pixels (column-major 3×3).
    let view: Mat3 = [sx, 0.0, 0.0, 0.0, sy, 0.0, rect.x, rect.y, 1.0];
    let to_frame = |p: [f64; 2]| place.to_frame(p);
    let mut layers: Vec<OverlayLayer> = Vec::new();
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for q in quads {
        *counts.entry(q.layer.as_str()).or_default() += 1;
    }
    for q in quads {
        let corners = q.corners.map(to_frame);
        if counts[q.layer.as_str()] > 1 {
            if let Some(existing) = layers.iter_mut().find(|l| l.layer.as_str() == q.layer) {
                let all: Vec<[f64; 2]> = existing
                    .bounds
                    .iter()
                    .chain(corners.iter())
                    .copied()
                    .collect();
                existing.bounds = bounding(&all);
                continue;
            }
            layers.push(OverlayLayer {
                layer: q.layer.as_str().into(),
                world_2d: Some(view),
                world_3d: None,
                bounds: bounding(&corners),
                paths: Vec::new(),
            });
        } else {
            layers.push(OverlayLayer {
                layer: q.layer.as_str().into(),
                world_2d: Some(view),
                world_3d: None,
                bounds: corners,
                paths: Vec::new(),
            });
        }
    }
    OverlayGeometry {
        viewport: rect,
        layers,
        caret: None,
    }
}

fn bounding(points: &[[f64; 2]]) -> [[f64; 2]; 4] {
    let x0 = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let x1 = points
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let y0 = points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let y1 = points
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max);
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::color::ViewerColorSpace;
    use pm_protocol::common::Seq;
    use pm_protocol::frame::MediaPreference;

    fn state(
        width: u32,
        height: u32,
        zoom: f64,
        pan: [f64; 2],
        dpr: f64,
        resolution: PreviewResolution,
    ) -> ViewerState {
        ViewerState {
            headless: None,
            composition: "c".into(),
            size: Size::new(width, height),
            device_pixel_ratio: dpr,
            zoom,
            pan,
            resolution,
            media: MediaPreference::Automatic,
            color_space: ViewerColorSpace::Srgb,
            view: None,
            play_every_frame: false,
            edr: false,
            view_seq: Seq::new(1).unwrap(),
            vsync: None,
            depth_id: None,
            chrome_selection: None,
            canvas: None,
            surface_epoch: None,
        }
    }

    #[test]
    fn the_composition_sits_where_the_legacy_viewer_puts_it() {
        // A 1920×1080 composition fit into a 1195×546 CSS stage at DPR 2.
        let zoom = (1195.0 / 1920.0f64).min(546.0 / 1080.0);
        let s = state(2390, 1092, zoom, [0.0, 0.0], 2.0, PreviewResolution::Full);
        let p = placement(&s, 1920.0, 1080.0);
        assert_eq!(p.output, Size::new(1941, 1092));
        // Centered: (2390 − 1941.3) / 2 = 224.3, snapped to a whole pixel.
        assert_eq!((p.x, p.y), (224.0, 0.0));
        assert_eq!(
            p.region,
            Some(PixelRect {
                x: 0,
                y: 0,
                width: 1941,
                height: 1092
            })
        );
        // Half resolution renders half the pixels into the same rectangle.
        let half = placement(&PreviewResolution::Half.into_state(&s), 1920.0, 1080.0);
        assert_eq!(half.output, Size::new(971, 546));
        assert_eq!(half.rect().x, 224.0);
        assert!((half.rect().width - 1942.0).abs() < 1e-9);
    }

    #[test]
    fn zoomed_in_and_panned_renders_only_what_is_visible() {
        let s = state(1000, 500, 2.0, [300.0, -40.0], 1.0, PreviewResolution::Full);
        let p = placement(&s, 1920.0, 1080.0);
        assert_eq!(p.output, Size::new(3840, 2160));
        assert_eq!((p.x, p.y), (-1120.0, -870.0));
        assert_eq!(
            p.region,
            Some(PixelRect {
                x: 1120,
                y: 870,
                width: 1000,
                height: 500
            })
        );
        let r = p.region_rect().unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (0.0, 0.0, 1000.0, 500.0));
        // Panned off screen: nothing is rendered.
        let off = placement(
            &state(1000, 500, 1.0, [5000.0, 0.0], 1.0, PreviewResolution::Full),
            640.0,
            360.0,
        );
        assert_eq!(off.region, None);
    }

    #[test]
    fn todays_cropped_canvas_renders_its_part_at_its_scale() {
        // 500% at DPR 2 with today's crop of (100.3, 50, 150, 90)
        // composition pixels in a 1500 × 900 canvas: scale 10, the output
        // is the canvas, starting at the crop (origin 1003, 500, unrounded).
        let mut s = state(1600, 1000, 5.0, [0.0, 0.0], 2.0, PreviewResolution::Full);
        s.canvas = Some(pm_protocol::frame::ViewerCanvas {
            width: 1500,
            height: 900,
            crop: Rect {
                x: 100.3,
                y: 50.0,
                width: 150.0,
                height: 90.0,
            },
            css: Rect {
                x: 25.0,
                y: -40.0,
                width: 750.0,
                height: 450.0,
            },
        });
        let p = placement(&s, 600.0, 338.0);
        assert_eq!(p.scale, Some((10.0, 10.0)));
        assert_eq!(p.output, Size::new(1500, 900));
        let (ox, oy) = p.origin.unwrap();
        assert!((ox - 1003.0).abs() < 1e-9 && oy == 500.0, "{ox} {oy}");
        assert_eq!((p.fx, p.fy), (1.0, 1.0));
        // Output pixel 0 (the crop's first) lands on the box's left edge.
        assert_eq!(p.to_frame([0.0, 0.0]), [50.0, -80.0]);
        let r = p.region.unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (0, 80, 1500, 820));
        // Composition pixel 0 lands at the box's edge less the origin.
        assert!((p.comp.x - (50.0 - ox)).abs() < 1e-9);
    }

    /// Today's crop at 500% on a 2x display of a 1920 × 1080 composition,
    /// panned to its right edge (19,200 output pixels from the
    /// composition's origin), and a 4K one at 250%: the output is today's
    /// canvas at today's full scale, its origin the crop's.
    #[test]
    fn a_crop_far_from_the_origin_is_drawn_at_todays_scale() {
        for (zoom, width, height, crop_w, crop_h) in [
            (5.0, 1920.0, 1080.0, 211.2, 151.2),
            (8.0, 1920.0, 1080.0, 132.0, 94.5),
            (2.5, 3840.0, 2160.0, 422.4, 302.4),
        ] {
            let mut s = state(1600, 1000, zoom, [0.0, 0.0], 2.0, PreviewResolution::Full);
            let canvas = pm_protocol::frame::ViewerCanvas {
                width: 2112,
                height: 1512,
                crop: Rect {
                    x: width - crop_w,
                    y: 0.0,
                    width: crop_w,
                    height: crop_h,
                },
                css: Rect {
                    x: -128.0,
                    y: -128.0,
                    width: 1056.0,
                    height: 756.0,
                },
            };
            s.canvas = Some(canvas);
            let p = placement(&s, width, height);
            assert!(p.canvas);
            assert_eq!(p.output, Size::new(2112, 1512));
            let (sx, _) = p.scale.unwrap();
            assert!((sx - 2112.0 / crop_w).abs() < 1e-9);
            let (ox, _) = p.origin.unwrap();
            assert!((ox - (width - crop_w) * sx).abs() < 1e-6, "{ox}");
            // The composition's right edge is the box's right edge.
            let right = p.comp.x + p.comp.width;
            assert!((right - (-256.0 + 2112.0)).abs() < 1e-6, "{right}");
            let clip = p.clip.unwrap();
            assert_eq!(
                (clip.x, clip.y, clip.width, clip.height),
                (0, 0, 1600, 1000)
            );
        }
    }

    /// Without a canvas, a view larger than the largest output draws its
    /// visible part only, from that part's origin, at zoom × DPR.
    #[test]
    fn a_view_beyond_the_largest_output_draws_its_visible_part() {
        let s = state(1600, 1000, 8.0, [0.0, 0.0], 2.0, PreviewResolution::Full);
        let p = placement(&s, 1920.0, 1080.0);
        assert_eq!((p.comp.width, p.comp.height), (30720.0, 17280.0));
        assert_eq!(p.output, Size::new(1600, 1000));
        assert_eq!(p.scale, Some((16.0, 16.0)));
        let (ox, oy) = p.origin.unwrap();
        assert_eq!((ox, oy), (14560.0, 8140.0));
        let rr = p.region_rect().unwrap();
        assert_eq!(
            (rr.x, rr.y, rr.width, rr.height),
            (0.0, 0.0, 1600.0, 1000.0)
        );
        // Composition pixel (910, 508.75) is at the frame's origin.
        assert_eq!(p.comp.x, -14560.0);
        // Within the largest output: the whole composition, as before.
        let p = placement(
            &state(1600, 1000, 4.0, [0.0, 0.0], 2.0, PreviewResolution::Full),
            1920.0,
            1080.0,
        );
        assert_eq!(p.origin, None);
        assert_eq!(p.output, Size::new(15360, 8640));
        assert_eq!((p.fx, p.fy), (1.0, 1.0));
    }

    /// A custom viewer's crop that starts before the composition's origin:
    /// a negative origin, the composition where the crop puts it.
    #[test]
    fn a_crop_before_the_origin_starts_before_the_composition() {
        let mut s = state(1600, 1000, 2.0, [0.0, 0.0], 1.0, PreviewResolution::Full);
        let canvas = pm_protocol::frame::ViewerCanvas {
            width: 400,
            height: 200,
            crop: Rect {
                x: -50.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
            css: Rect {
                x: 100.0,
                y: 100.0,
                width: 400.0,
                height: 200.0,
            },
        };
        s.canvas = Some(canvas);
        let p = placement(&s, 640.0, 360.0);
        assert_eq!(p.origin, Some((-100.0, 0.0)));
        assert_eq!(p.output, Size::new(400, 200));
        // Composition x 0 at css 100 + 50 × 2.
        assert_eq!(p.comp.x, 200.0);
        assert_eq!(p.clip.unwrap().x, 100);
    }

    trait IntoState {
        fn into_state(self, s: &ViewerState) -> ViewerState;
    }
    impl IntoState for PreviewResolution {
        fn into_state(self, s: &ViewerState) -> ViewerState {
            ViewerState {
                resolution: self,
                ..s.clone()
            }
        }
    }
}
