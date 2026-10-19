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
use pm_render::{LayerFailure, RenderError, RenderRequest, Renderer};

use super::document::MediaMap;

/// Where the composition goes in a viewer frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The composition's top-left in frame pixels (whole pixels).
    pub x: f64,
    pub y: f64,
    /// Preview resolution: output pixels per frame pixel (1, 1/2, 1/4).
    pub resolution: f64,
    /// The whole composition at the render scale, in output pixels.
    pub output: Size,
    /// The visible part of the output (`None`: nothing is visible).
    pub region: Option<PixelRect>,
}

impl Placement {
    /// The frame-pixel rectangle the composition covers.
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: f64::from(self.output.width) / self.resolution,
            height: f64::from(self.output.height) / self.resolution,
        }
    }

    /// Where the rendered region goes in the frame.
    pub fn region_rect(&self) -> Option<Rect> {
        let r = self.region?;
        Some(Rect {
            x: self.x + f64::from(r.x) / self.resolution,
            y: self.y + f64::from(r.y) / self.resolution,
            width: f64::from(r.width) / self.resolution,
            height: f64::from(r.height) / self.resolution,
        })
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

/// The placement of a `width` × `height` composition in a viewer frame.
pub fn placement(state: &ViewerState, width: f64, height: f64) -> Placement {
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
    let output = Size::new(
        (full_w * resolution).round().clamp(1.0, max) as u32,
        (full_h * resolution).round().clamp(1.0, max) as u32,
    );
    let span = |origin: f64, frame: f64, out: u32| -> Option<(u32, u32)> {
        let a = ((0.0 - origin) * resolution)
            .floor()
            .clamp(0.0, f64::from(out));
        let b = ((frame - origin) * resolution)
            .ceil()
            .clamp(0.0, f64::from(out));
        (b > a).then_some((a as u32, (b - a) as u32))
    };
    let region = match (
        span(x, frame_w, output.width),
        span(y, frame_h, output.height),
    ) {
        (Some((rx, rw)), Some((ry, rh))) => Some(PixelRect {
            x: rx,
            y: ry,
            width: rw,
            height: rh,
        }),
        _ => None,
    };
    Placement {
        x,
        y,
        resolution,
        output,
        region,
    }
}

/// What a composed frame reports.
#[derive(Debug, Clone)]
pub struct Composed {
    pub overlay: OverlayGeometry,
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

/// Renders documents for the presenter. One per GPU device.
pub struct Composer {
    renderer: Renderer,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    scratch: Option<Target>,
    /// The region's depth and object-id surface (`compose_with_depth`).
    depth_scratch: Option<Target>,
    /// The viewer is playing: today's playing motion-blur count and the
    /// playback resampler (`set_playing`).
    playing: bool,
    /// The view (hashed) whose definition set is in the renderer, and the
    /// identity and version of the set it was loaded from.
    definitions: Option<(u64, Option<(usize, u64)>)>,
    /// Every other view's definition set, kept while another view draws:
    /// each view keeps its own programs and watchdog state, so views
    /// sharing the device never reset each other's.
    parked: HashMap<u64, (Effects, Option<(usize, u64)>)>,
    /// How the next frames treat Blender compositions (`set_blender`).
    blender: pm_render::blender::BlenderRender,
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
            depth_scratch: None,
            definitions: None,
            parked: HashMap::new(),
            blender: pm_render::blender::BlenderRender::Native,
            playing: false,
        }
    }

    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
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

    /// Whether the next frames are playback frames (timed) or paused ones.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Blender in the next frames: native shading, or the paused viewer's
    /// Blender previews (`super::blender`).
    pub fn set_blender(&mut self, blender: pm_render::blender::BlenderRender) {
        self.blender = blender;
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
        let x0 = dst.x.max(0.0).floor() as u32;
        let y0 = dst.y.max(0.0).floor() as u32;
        let x1 = ((dst.x + dst.width).ceil().max(0.0) as u32).min(size.width);
        let y1 = ((dst.y + dst.height).ceil().max(0.0) as u32).min(size.height);
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
        let sampler = if exact || place.resolution == 1.0 {
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

    /// Draws `composition` of `doc` at `time` into `working` for `state`.
    /// On an error the working texture is left as it was.
    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        &mut self,
        doc: &pm_model::Document,
        composition: &str,
        media: &MediaMap,
        time: Ticks,
        state: &ViewerState,
        working: &WorkingTexture,
    ) -> Result<Composed, RenderError> {
        self.compose_with_depth(doc, composition, media, time, state, working, None)
    }

    /// [`Self::compose`], also writing the frame's depth and object-id
    /// surface (r depth, g/b id − 1, a coverage; plan section 8) into
    /// `depth`, a `rgbaf16` texture of the working texture's size, placed
    /// like the color (never filtered: ids must stay exact).
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_depth(
        &mut self,
        doc: &pm_model::Document,
        composition: &str,
        media: &MediaMap,
        time: Ticks,
        state: &ViewerState,
        working: &WorkingTexture,
        depth: Option<&wgpu::TextureView>,
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
        if let Some(region) = place.region {
            let request = RenderRequest {
                output_size: place.output,
                scale: None,
                region: Some(region),
                precision: Precision::Float16,
                depth_id: depth.is_some(),
                background: true,
                export: false,
                // The viewer's override: editor camera, Wireframe and
                // X-ray looks, Local View, and the shading Blender uses.
                view: state.view.clone(),
                blender: self.blender.clone(),
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
            let reuse = self
                .scratch
                .as_ref()
                .is_some_and(|t| t.width() == region.width && t.height() == region.height);
            if !reuse {
                self.scratch = Some(Target::new(
                    &device,
                    "viewer region",
                    region.width,
                    region.height,
                    WORKING_FORMAT,
                    pm_gpu::target::working_usage(),
                ));
            }
            let scratch = self.scratch.as_ref().expect("made above");
            if depth.is_some()
                && !self
                    .depth_scratch
                    .as_ref()
                    .is_some_and(|t| t.width() == region.width && t.height() == region.height)
            {
                self.depth_scratch = Some(Target::new(
                    &device,
                    "viewer depth and id",
                    region.width,
                    region.height,
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                ));
            }
            let depth_target = depth.and(self.depth_scratch.as_ref());
            let rendered = self.renderer.render_into(
                doc,
                composition,
                time,
                &request,
                media,
                scratch,
                depth_target,
            )?;
            failures = rendered.failures;
            quads = rendered.quads;
            stand_ins = rendered.stand_ins;
        }
        // Place the region (or only clear when nothing is visible).
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewer placement"),
        });
        self.place(
            &mut encoder,
            &device,
            &queue,
            &working.view,
            working.size,
            &place,
            self.scratch.as_ref(),
            false,
        );
        if let Some(view) = depth {
            self.place(
                &mut encoder,
                &device,
                &queue,
                view,
                working.size,
                &place,
                self.depth_scratch.as_ref(),
                true,
            );
        }
        queue.submit([encoder.finish()]);
        Ok(Composed {
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
    let to_frame = |p: [f64; 2]| {
        [
            place.x + p[0] / place.resolution,
            place.y + p[1] / place.resolution,
        ]
    };
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
