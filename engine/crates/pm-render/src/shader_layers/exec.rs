//! Draws a [`ShaderSource`]: the compositor's `grab(targetW, targetH)`,
//! `clear(0, 0, 0, 0)`, blending off, the quad through `PM.VERT`, and the
//! uniform calls in issue order, in GL row order.

use std::collections::HashMap;
use std::sync::Arc;

use pm_glsl::{Failure, Key};
use pm_gpu::Target;
use pm_protocol::definitions::DefinitionKind;

use super::fallback::{FallbackPixels, FallbackRequest, FallbackStore};
use super::plan::{ProgramRef, ShaderSource};
use crate::effects::program::{Call, GpuProgram};

/// GPU programs of shader-layer code kept by a renderer (by source key),
/// and the fallback runner's state.
#[derive(Default)]
pub struct ShaderState {
    programs: HashMap<Key, (GpuProgram, u64)>,
    tick: u64,
    /// Whether a fallback runner draws programs the native pipeline cannot
    /// (the interactive engine with a viewer); see [`super::fallback`].
    pub fallback_runner: bool,
    /// Pixels the runner uploaded.
    pub fallback: FallbackStore,
    /// GPU copies of runner pixels by request key, with the pixels they
    /// were made from and when they were last drawn: a fallback layer that
    /// does not change (a still frame, a static layer during playback) is
    /// not uploaded again for every frame.
    uploads: HashMap<String, (Arc<FallbackPixels>, Target, u64)>,
    uploaded: u64,
    /// Requests asked for and not answered: their program (`program_id`).
    pub(crate) waiting: HashMap<String, String>,
    /// Programs the runner could not draw, by `program_id`, and whether
    /// the failure was reported.
    pub(crate) failed: HashMap<String, (super::FallbackFailure, bool)>,
    /// Tests: the device refuses every shader-layer pipeline, as Metal
    /// would a translation it cannot compile.
    #[cfg(test)]
    pub(crate) refuse_pipelines: bool,
}

/// Programs kept before the least recently used is dropped (today's
/// `shaderUniformDefs` bound).
const PROGRAMS: usize = 256;

/// Runner pixel textures kept on the GPU (several fallback layers in one
/// frame stay resident; animated ones replace the oldest).
const UPLOADS: usize = 8;

impl ShaderState {
    /// Programs built.
    pub fn programs(&self) -> usize {
        self.programs.len()
    }

    /// Runner pixel uploads made so far (one per request drawn, not per
    /// frame drawing it).
    pub fn fallback_uploads(&self) -> u64 {
        self.uploaded
    }

    /// Forgets the runner's pixels and their GPU copies (the runner was
    /// detached).
    pub fn clear_fallback(&mut self) {
        self.fallback.clear();
        self.uploads.clear();
    }

    /// The runner's pixels for `request` on the GPU, uploaded once per
    /// pixels stored; `None` until they arrive.
    fn fallback_target(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: &FallbackRequest,
    ) -> Option<Target> {
        let pixels = self.fallback.get(&request.key)?;
        if !pixels.fits(request) {
            return None;
        }
        self.tick += 1;
        let tick = self.tick;
        if let Some((held, target, t)) = self.uploads.get_mut(&request.key)
            && Arc::ptr_eq(held, &pixels)
        {
            *t = tick;
            return Some(target.clone());
        }
        let target = upload(device, queue, &pixels, request.color_depth);
        self.uploaded += 1;
        self.uploads.remove(&request.key);
        if self.uploads.len() >= UPLOADS {
            let oldest = self
                .uploads
                .iter()
                .min_by_key(|(_, (_, _, t))| *t)
                .map(|(k, _)| k.clone())
                .expect("not empty");
            self.uploads.remove(&oldest);
        }
        self.uploads
            .insert(request.key.clone(), (pixels, target.clone(), tick));
        Some(target)
    }

    fn program(
        &mut self,
        device: &wgpu::Device,
        key: Key,
        prepared: &super::translate::Translated,
        format: wgpu::TextureFormat,
    ) -> Result<&mut GpuProgram, Failure> {
        self.tick += 1;
        let tick = self.tick;
        if !self.programs.contains_key(&key) {
            let prepared = prepared.as_ref().as_ref().map_err(|f| f.clone())?;
            let program = prepared
                .finish()
                .and_then(|p| GpuProgram::new(device, p, "pm shader layer"))?;
            if self.programs.len() >= PROGRAMS {
                let oldest = self
                    .programs
                    .iter()
                    .min_by_key(|(_, (_, t))| *t)
                    .map(|(k, _)| *k)
                    .expect("not empty");
                self.programs.remove(&oldest);
            }
            self.programs.insert(key, (program, tick));
        }
        let entry = self.programs.get_mut(&key).expect("inserted above");
        entry.1 = tick;
        #[cfg(test)]
        if self.refuse_pipelines {
            self.programs.remove(&key);
            return Err(Failure::new(
                pm_glsl::FailureStage::NagaValidate,
                "the device refused the pipeline (test)",
            ));
        }
        if let Err(f) = entry.0.pipeline(device, format) {
            self.programs.remove(&key);
            return Err(f);
        }
        Ok(&mut self.programs.get_mut(&key).expect("kept").0)
    }
}

/// Why a source could not be drawn.
pub(crate) enum Missed {
    /// The watchdog turned the program off (an overrun or a runaway): the
    /// layer draws nothing, as a program that does not compile, and the
    /// watchdog's report names it.
    Disabled,
    /// The hang guard holds the program (a frame that drew it may be
    /// running away): the frame leaves it out and is not presented.
    Held,
    /// The program WebGL draws failed in the native pipeline at pipeline
    /// creation (the Metal compiler): it needs the fallback runner.
    NeedsFallback(String),
    /// The runner's pixels for this request have not arrived.
    Pending(Arc<FallbackRequest>),
}

/// One draw of a legacy program into `out` (cleared to transparent,
/// blending off) with the compositor's calls; samplers read `units` or the
/// empty unit (0, 0, 0, 1).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_program(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    program: &mut GpuProgram,
    calls: &[(String, Call)],
    units: &[Option<&wgpu::TextureView>],
    empty: &wgpu::TextureView,
    linear: &wgpu::Sampler,
    out: &Target,
) {
    let format = out.format();
    let buffer = |block: &pm_glsl::reflect::Block| {
        let bytes = GpuProgram::block_bytes(block, calls);
        let b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm shader layer uniforms"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&b, 0, &bytes);
        b
    };
    let fragment_buffer = program.fragment_block.as_ref().map(buffer);
    let vertex_buffer = program.vertex_block.as_ref().map(buffer);
    let views: Vec<&wgpu::TextureView> = program
        .sampler_units(calls)
        .into_iter()
        .map(|unit| units.get(unit as usize).copied().flatten().unwrap_or(empty))
        .collect();
    let mut entries = Vec::new();
    if let (Some(block), Some(b)) = (&program.fragment_block, &fragment_buffer) {
        entries.push(wgpu::BindGroupEntry {
            binding: block.binding,
            resource: b.as_entire_binding(),
        });
    }
    for ((_, binding), view) in program.samplers.iter().zip(&views) {
        entries.push(wgpu::BindGroupEntry {
            binding: *binding,
            resource: wgpu::BindingResource::TextureView(view),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: binding + pm_glsl::SAMPLER_BINDING_OFFSET,
            resource: wgpu::BindingResource::Sampler(linear),
        });
    }
    let fragment = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm shader layer fragment"),
        layout: &program.fragment_layout,
        entries: &entries,
    });
    let mut vertex_entries = Vec::new();
    if let (Some(block), Some(b)) = (&program.vertex_block, &vertex_buffer) {
        vertex_entries.push(wgpu::BindGroupEntry {
            binding: block.binding,
            resource: b.as_entire_binding(),
        });
    }
    let vertex = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm shader layer vertex"),
        layout: &program.vertex_layout,
        entries: &vertex_entries,
    });
    let pipeline = program
        .pipeline(device, format)
        .expect("built by the caller")
        .clone();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pm shader layer"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &out.view,
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
    pass.set_pipeline(&pipeline);
    pass.set_bind_group(0, &fragment, &[]);
    pass.set_bind_group(1, &vertex, &[]);
    pass.draw(0..4, 0..1);
}

/// Uploads runner pixels at their working depth, in GL row order.
pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixels: &FallbackPixels,
    color_depth: u8,
) -> Target {
    let rgba8 = color_depth == 8;
    let target = Target::new(
        device,
        "pm shader fallback pixels",
        pixels.width,
        pixels.height,
        match color_depth {
            8 => wgpu::TextureFormat::Rgba8Unorm,
            32 => wgpu::TextureFormat::Rgba32Float,
            _ => wgpu::TextureFormat::Rgba16Float,
        },
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let bytes = if rgba8 {
        pixels
            .rgba16f
            .iter()
            .map(|code| u8::try_from(*code).expect("RGBA8 fallback code"))
            .collect()
    } else if color_depth == 32 {
        super::fallback::float_bytes(pixels.rgba32f.as_ref().expect("RGBA32F fallback bits"))
    } else {
        super::fallback::half_bytes(&pixels.rgba16f)
    };
    queue.write_texture(
        target.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(
                pixels.width
                    * match color_depth {
                        8 => 4,
                        32 => 16,
                        _ => 8,
                    },
            ),
            rows_per_image: Some(pixels.height),
        },
        wgpu::Extent3d {
            width: pixels.width,
            height: pixels.height,
            depth_or_array_layers: 1,
        },
    );
    target
}

impl crate::exec::Exec<'_> {
    fn reject_invalid_fallback_code(&mut self, request: &FallbackRequest, layer: &str) -> bool {
        let Some(code) = self
            .shaders
            .fallback
            .get(&request.key)
            .and_then(|pixels| pixels.invalid_rgba8_code(request))
        else {
            return false;
        };
        let message = format!(
            "Layer {layer}: the fallback runner returned invalid RGBA8 channel code {code} (expected 0..=255)"
        );
        if self.effects.watchdog.policy() == crate::effects::watchdog::Policy::Job {
            self.effects.fallback.fatal.get_or_insert(message.clone());
        }
        self.effects
            .watchdog
            .report(pm_protocol::error::EngineError::new(
                pm_protocol::error::ErrorCode::InvalidMessage,
                pm_protocol::error::ErrorScope::Layer {
                    layer: layer.into(),
                },
                message,
            ));
        true
    }

    /// Draws a shader source into a new working target (GL row order,
    /// E-encoded legacy output), and whether the target is the pool's.
    pub(crate) fn shader_source(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &ShaderSource,
    ) -> Result<(Target, bool), Missed> {
        if let ProgramRef::Fallback(request) = &source.program {
            let request = super::fallback::at_color_depth(
                request,
                crate::shader_layers::fallback::color_depth(self.precision),
            );
            // A program the runner could not draw draws nothing, as one
            // WebGL refuses, and is reported once (a draw past the
            // runner's deadline by the watchdog).
            let program = super::fallback::program_id(&request);
            // Its last request failed for a transient reason (the runner's
            // context was lost): asked for again once the backoff passed,
            // drawing nothing until then, reported once.
            // A job's host already tried it again in place: the job stops.
            if self.effects.watchdog.policy() == crate::effects::watchdog::Policy::Job
                && let Some(retry) = self.effects.fallback.retries.last(&program)
            {
                let why = retry.failure.clone();
                self.effects.fallback.fatal.get_or_insert_with(|| {
                    format!(
                        "Layer {} needs the WebGL fallback runner, which could not draw it ({}: {}).",
                        source.layer, why.reason, why.message
                    )
                });
                return Err(Missed::Disabled);
            }
            let now = self.effects.watchdog.now();
            if let Some(crate::effects::fallback::RetryState::Wait { failure, report }) =
                self.effects.fallback.retries.state(&program, now)
            {
                if report {
                    self.effects.watchdog.report(pm_protocol::error::EngineError::new(
                        pm_protocol::error::ErrorCode::NotHeadlessRenderable,
                        pm_protocol::error::ErrorScope::Layer {
                            layer: source.layer.as_str().into(),
                        },
                        crate::effects::truncate(format!(
                            "Layer {} could not be drawn by the WebGL fallback runner for now ({}: {}); it draws nothing until the runner draws it again.",
                            source.layer, failure.reason, failure.message
                        )),
                    ));
                }
                return Err(Missed::Disabled);
            }
            if let Some((why, reported)) = self.shaders.failed.get_mut(&program) {
                let first = !std::mem::replace(reported, true);
                let why = why.clone();
                let span = crate::effects::watchdog::Span {
                    subject: crate::effects::watchdog::Subject::Definition {
                        kind: DefinitionKind::Layer,
                        id: format!("fallback:{program}").as_str().into(),
                    },
                    layer: source.layer.as_str().into(),
                    instance: None,
                    builtin: false,
                };
                if why.reason == "watchdog" {
                    self.effects.watchdog.runner_runaway(
                        &span,
                        &format!("the shader of layer {}", source.layer),
                        format!(
                            "The shader of layer {} did not finish drawing in the WebGL fallback runner ({}), so it is turned off until its code changes.",
                            source.layer, why.message
                        ),
                    );
                } else if self.effects.watchdog.policy() == crate::effects::watchdog::Policy::Job
                    && why.reason != "compile"
                {
                    self.effects.fallback.fatal.get_or_insert_with(|| {
                        format!(
                            "Layer {} needs the WebGL fallback runner, which could not draw it ({}: {}).",
                            source.layer, why.reason, why.message
                        )
                    });
                } else if first {
                    self.effects.watchdog.report(
                        pm_protocol::error::EngineError::new(
                            if why.reason == "unavailable" {
                                pm_protocol::error::ErrorCode::NotHeadlessRenderable
                            } else {
                                pm_protocol::error::ErrorCode::ShaderCompile
                            },
                            pm_protocol::error::ErrorScope::Layer {
                                layer: source.layer.as_str().into(),
                            },
                            crate::effects::truncate(format!(
                                "Layer {} could not be drawn by the WebGL fallback runner ({}: {}); it draws nothing.",
                                source.layer, why.reason, why.message
                            )),
                        ),
                    );
                }
                return Err(Missed::Disabled);
            }
            if self.reject_invalid_fallback_code(&request, &source.layer) {
                return Err(Missed::Disabled);
            }
            return match self
                .shaders
                .fallback_target(self.device, self.queue, &request)
            {
                Some(target) => Ok((target, false)),
                None => Err(Missed::Pending(Arc::new(request))),
            };
        }
        // The watchdog's subject: the code's source key, or the fragment
        // layer's definition.
        let span = match &source.program {
            ProgramRef::Code { key, .. } => {
                crate::effects::watchdog::Span::code(*key, &source.layer)
            }
            ProgramRef::Definition { id, .. } => crate::effects::watchdog::Span::definition(
                DefinitionKind::Layer,
                id,
                &source.layer,
                None,
                self.effects
                    .get(DefinitionKind::Layer, id)
                    .is_some_and(|d| {
                        d.origin == pm_protocol::definitions::DefinitionOrigin::BuiltIn
                    }),
            ),
            ProgramRef::Fallback(_) => unreachable!("handled above"),
        };
        let disabled = match &span.subject {
            crate::effects::watchdog::Subject::Code(key) => {
                self.effects.watchdog.is_code_disabled(key)
            }
            crate::effects::watchdog::Subject::Definition { kind, id } => {
                self.effects.watchdog.is_disabled(*kind, id.as_str())
            }
        };
        if disabled {
            return Err(Missed::Disabled);
        }
        if !span.builtin && self.effects.watchdog.hold(&span.subject) {
            return Err(Missed::Held);
        }
        let out = self
            .pool
            .take_working(self.device, source.width, source.height, self.precision);
        let format = out.format();
        let empty = self.effects.dummy(self.device, self.queue, format);
        let linear = &self.pipelines.linear;
        let device = self.device;
        let queue = self.queue;
        let built = match &source.program {
            ProgramRef::Code {
                key,
                prepared,
                source: code,
            } => match self.shaders.program(device, *key, prepared, format) {
                Ok(_) => Ok(()),
                Err(f) => {
                    // Later plans route it to the runner (or, without one,
                    // to `Feature::ShaderFallback`).
                    super::translate::mark_failed(*key, f.clone());
                    Err((f, code.to_string(), "shader layer".to_owned()))
                }
            },
            ProgramRef::Definition { id, .. } => {
                match self
                    .effects
                    .program(device, DefinitionKind::Layer, id, format)
                {
                    Ok(_) => Ok(()),
                    // The definition is now `Status::NeedsFallback`.
                    Err(f) => {
                        let code = self
                            .effects
                            .get(DefinitionKind::Layer, id)
                            .and_then(|d| d.glsl.as_ref())
                            .map(|g| g.source.clone())
                            .unwrap_or_default();
                        Err((f, code, id.clone()))
                    }
                }
            }
            ProgramRef::Fallback(_) => unreachable!("handled above"),
        };
        // Built: fetched again below (a borrow returned from one match arm
        // would hold `self` in the other).
        if let Err((failure, code, what)) = built {
            self.pool.give(out);
            if !self.shaders.fallback_runner {
                return Err(Missed::NeedsFallback(what));
            }
            // The device refused the translation on first contact: the
            // runner draws it from this frame on, as the plan would
            // have asked had the failure been known
            // (`super::plan::program_plan`, the same request key).
            let request = Arc::new(super::fallback::at_color_depth(
                &super::fallback::request(
                    source.layer.clone(),
                    code,
                    source.width,
                    source.height,
                    &source.calls,
                    &failure,
                ),
                crate::shader_layers::fallback::color_depth(self.precision),
            ));
            if self.reject_invalid_fallback_code(&request, &source.layer) {
                return Err(Missed::Disabled);
            }
            return match self
                .shaders
                .fallback_target(self.device, self.queue, &request)
            {
                Some(target) => Ok((target, false)),
                None => Err(Missed::Pending(request)),
            };
        }
        let program = match &source.program {
            ProgramRef::Code { key, prepared, .. } => self
                .shaders
                .program(device, *key, prepared, format)
                .expect("built above"),
            ProgramRef::Definition { id, .. } => self
                .effects
                .program(device, DefinitionKind::Layer, id, format)
                .expect("built above"),
            ProgramRef::Fallback(_) => unreachable!("handled above"),
        };
        let queries = self.timer.as_mut().and_then(|t| t.begin(span));
        if let (Some(timer), Some((b, _))) = (self.timer.as_ref(), queries) {
            timer.mark(device, encoder, &out.view, b);
        }
        draw_program(
            device,
            queue,
            encoder,
            program,
            &source.calls,
            &[],
            &empty.view,
            linear,
            &out,
        );
        if let (Some(timer), Some((_, e))) = (self.timer.as_ref(), queries) {
            timer.mark(device, encoder, &out.view, e);
        }
        Ok((out, true))
    }
}

#[cfg(test)]
mod tests {
    use pm_protocol::color::Precision;
    use pm_protocol::common::{Size, Ticks};
    use pm_protocol::error::ErrorCode;
    use serde_json::json;

    use super::super::FallbackPixels;
    use crate::{MapResolver, RenderError, RenderRequest, Renderer};

    fn document(code: &str) -> pm_model::Document {
        let ch = |v: serde_json::Value| json!({ "v": v, "kf": [], "expr": null });
        let layer = json!({
            "id": "probe", "type": "shader", "name": "probe", "from": 0, "dur": 10, "on": true,
            "lock": false, "threeD": false, "solo": false, "blend": "normal", "mblur": false,
            "parent": null, "group": null, "matteSource": null,
            "p": {
                "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(0)),
                "position.y": ch(json!(0)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
                "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
            },
            "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
            "d": { "code": code, "uniforms": {}, "w": 16, "h": 8 }
        });
        let doc = json!({
            "id": "P", "name": "p", "w": 16, "h": 8, "fps": 30, "dur": 10, "bg": "#000000",
            "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
            "compId": "P-root", "compName": "p", "markers": [], "work": [0, 10], "params": {},
            "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer],
            "assets": {}, "comps": {}
        });
        pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
    }

    fn render(r: &mut Renderer, doc: &pm_model::Document) -> Result<Vec<f32>, RenderError> {
        let mut request = RenderRequest::sized(Size::new(16, 8), Precision::Float16);
        request.background = false;
        let frame = r.render(
            doc,
            "P-root",
            Ticks::ZERO,
            &request,
            &MapResolver::default(),
        )?;
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        Ok(r.read_rgba(&frame).unwrap())
    }

    /// Metal refusing a translated program at pipeline creation (first
    /// contact, not known when the frame was planned): with a runner the
    /// layer asks it for its pixels in the same frame instead of routing
    /// the composition, is reported as not headless-renderable, and its
    /// pixels are uploaded once however many frames draw them.
    #[test]
    fn a_pipeline_the_device_refuses_goes_to_the_runner_on_first_contact() {
        // Unique code: the refusal is recorded for the process.
        let code = "void main(){ fragColor = vec4(0.25, 0.5, 0.75, 1.0) + vec4(uv * 0.0, 0.0, 0.0); } // exec refusal probe";
        let doc = document(code);
        let mut r = Renderer::headless().unwrap();
        r.set_shader_fallback_runner(true);
        r.shaders_mut().refuse_pipelines = true;
        assert!(
            r.layer_reports(&doc, "P-root").is_empty(),
            "compiles before the device sees it"
        );
        let requests = match render(&mut r, &doc) {
            Err(RenderError::ShaderFallbackPending(requests)) => requests,
            other => panic!("expected a fallback request, got {other:?}"),
        };
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.layer, "probe");
        assert_eq!(request.stage, "naga-validate");
        assert_eq!((request.width, request.height), (16, 8));
        // The plan asks for the same request from now on.
        r.shaders_mut().refuse_pipelines = false;
        let again = match render(&mut r, &doc) {
            Err(RenderError::ShaderFallbackPending(requests)) => requests,
            other => panic!("expected the same request, got {other:?}"),
        };
        assert_eq!(again[0].key, request.key);
        // Reported for the Shader panel.
        let reports = r.layer_reports(&doc, "P-root");
        assert_eq!(reports.len(), 1, "{reports:?}");
        assert_eq!(reports[0].code, ErrorCode::NotHeadlessRenderable);
        assert!(
            reports[0].message.contains("headless"),
            "{}",
            reports[0].message
        );
        // The runner's pixels draw, uploaded once for every frame showing them.
        let px: Vec<u16> = (0..16 * 8)
            .flat_map(|_| [0x3800u16, 0x3400, 0x3000, 0x3c00])
            .collect();
        r.put_fallback_pixels(
            request.key.clone(),
            FallbackPixels {
                rgba32f: None,
                width: 16,
                height: 8,
                rgba16f: px,
            },
        );
        for _ in 0..3 {
            let frame = render(&mut r, &doc).unwrap();
            assert!(frame.chunks_exact(4).all(|p| p == [0.5, 0.25, 0.125, 1.0]));
        }
        assert_eq!(r.shader_state().fallback_uploads(), 1);
    }

    /// A lost runner context says nothing about the layer's code: it is
    /// asked for again (at once, then after a backoff, drawing nothing in
    /// between and reported once) and draws when the runner answers, as
    /// today's compositor draws again once its context is restored. A
    /// refusal (`compile`) is final.
    #[test]
    fn a_lost_runner_context_is_retried_and_the_layer_draws_again() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{Duration, Instant};
        let code = "void main(){ fragColor = vec4(0.125, 0.5, 0.75, 1.0) + vec4(uv * 0.0, 0.0, 0.0); } // lost context probe";
        let doc = document(code);
        let mut r = Renderer::headless().unwrap();
        r.set_shader_fallback_runner(true);
        r.shaders_mut().refuse_pipelines = true;
        let base = Instant::now();
        let offset = Arc::new(AtomicU64::new(0));
        let o = offset.clone();
        r.effects_mut().watchdog.set_clock(Some(Arc::new(move || {
            base + Duration::from_millis(o.load(Ordering::Relaxed))
        })));
        let pending = |r: &mut Renderer| match render(r, &doc) {
            Err(RenderError::ShaderFallbackPending(requests)) => requests,
            other => panic!("expected a fallback request, got {other:?}"),
        };
        let lost = || super::super::FallbackFailure {
            reason: "contextLost".into(),
            message: "the WebGL context was lost".into(),
        };
        let first = pending(&mut r);
        r.shaders_mut().refuse_pipelines = false;
        assert_eq!(
            r.put_fallback_failure(&first[0].key, lost()),
            Some(Duration::ZERO)
        );
        let second = pending(&mut r);
        assert_eq!(second[0].key, first[0].key, "asked for again at once");
        assert_eq!(
            r.put_fallback_failure(&second[0].key, lost()),
            Some(Duration::from_millis(250))
        );
        for _ in 0..2 {
            let frame = render(&mut r, &doc).unwrap();
            assert!(frame.iter().all(|v| *v == 0.0), "draws nothing meanwhile");
        }
        let reports = r.watchdog_reports(true);
        assert_eq!(reports.len(), 1, "{reports:?}");
        assert_eq!(reports[0].code, ErrorCode::NotHeadlessRenderable);
        offset.store(250, Ordering::Relaxed);
        let third = pending(&mut r);
        let px: Vec<u16> = (0..16 * 8)
            .flat_map(|_| [0x3800u16, 0x3400, 0x3000, 0x3c00])
            .collect();
        r.put_fallback_pixels(
            third[0].key.clone(),
            FallbackPixels {
                rgba32f: None,
                width: 16,
                height: 8,
                rgba16f: px,
            },
        );
        let frame = render(&mut r, &doc).unwrap();
        assert!(frame.chunks_exact(4).all(|p| p == [0.5, 0.25, 0.125, 1.0]));
        assert!(r.fallback_retry_at().is_none(), "the streak is over");
    }

    /// Without a runner (every headless host) the same refusal fails the
    /// layer as `Feature::ShaderFallback`.
    #[test]
    fn a_pipeline_the_device_refuses_routes_without_a_runner() {
        let doc = document(
            "void main(){ fragColor = vec4(0.5) + vec4(uv * 0.0, 0.0, 0.0); } // headless refusal probe",
        );
        let mut r = Renderer::headless().unwrap();
        r.shaders_mut().refuse_pipelines = true;
        match render(&mut r, &doc) {
            Err(RenderError::Unsupported(list)) => assert!(
                list.iter()
                    .any(|u| matches!(&u.feature, crate::capability::Feature::ShaderFallback(_))),
                "{list:?}"
            ),
            other => panic!("expected routing, got {other:?}"),
        }
    }
}
