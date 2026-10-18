//! Records effect pass and transition nodes (`runEffects`: 1 to 8
//! ping-pong passes per effect, `u_orig`, `u_backdrop` = the accumulated
//! stack beneath the layer, blending off; `runTransition`). The compositor
//! (`crate::composite`) decides where they run: a layer's chain after its
//! masks and before its matte, an adjustment layer's on the accumulator,
//! and a transition after the layer's composite. Every buffer covers the
//! pass rectangle, in GL row order, at the frame's precision.
//!
//! Legacy GLSL receives E-encoded values (section 11, "Color domain"): with
//! linear blending on, the chain's inputs are encoded with E first and its
//! output decoded after.

use std::sync::Arc;

use pm_gpu::{Target, TargetPool};
use pm_protocol::color::{Precision, WorkingEncoding};
use pm_protocol::definitions::DefinitionKind;

use super::plan::{Binding, EffectStep, Param, TransitionStep};
use super::program::{Call, GpuProgram, active};
use super::watchdog::{FrameTimer, Span, Subject};
use super::{Effects, SimpleOp};
use crate::capability::{Feature, Unsupported};
use crate::composite::effects::{Covered, EffectContext, EffectPasses, TransitionPasses};
use crate::exec::Region;
use crate::shader_layers::{FallbackPixels, FallbackUnit};
use pm_protocol::error::{EngineError, ErrorCode, ErrorScope};

/// `fullQuad(w, h)` as the column-major mat3 uniform.
fn full_quad(w: u32, h: u32) -> [f32; 9] {
    [w as f32, 0.0, 0.0, 0.0, h as f32, 0.0, 0.0, 0.0, 1.0]
}

/// `effectViewportCoordinates(W, H)`: `u_uv`, `u_pxOrigin`, `u_coordRes`
/// for a buffer covering `region` of a level of `full` pixels.
pub(crate) fn effect_coordinates(
    region: Option<Region>,
    full: (u32, u32),
    buffer: (u32, u32),
) -> ([f64; 4], [f64; 2], [f64; 2]) {
    let resolution = [f64::from(full.0.max(1)), f64::from(full.1.max(1))];
    let origin = region.map_or([0.0, 0.0], |r| [f64::from(r.x), f64::from(r.y)]);
    let uv = [
        origin[0] / resolution[0],
        origin[1] / resolution[1],
        f64::from(buffer.0) / resolution[0],
        f64::from(buffer.1) / resolution[1],
    ];
    (uv, origin, resolution)
}

fn f(values: &[f64]) -> Call {
    Call::F(values.iter().map(|v| *v as f32).collect())
}

/// `setParam`: the named uniform when the program has it active (GL's
/// `getUniformLocation` is program-wide: either stage), else the positional
/// alias.
fn param_calls(program: &GpuProgram, params: &[Param], calls: &mut Vec<(String, Call)>) {
    for (index, p) in params.iter().enumerate() {
        let named = format!("u_{}", p.key);
        let name = if active(program.fragment_block.as_ref(), &named)
            || active(program.vertex_block.as_ref(), &named)
        {
            named
        } else {
            format!("{}{index}", if p.color { "u_c" } else { "u_p" })
        };
        let call = match p.value {
            Binding::Float(v) => f(&[v]),
            Binding::Color(rgb) => f(&rgb),
        };
        calls.push((name, call));
    }
}

/// `setParam` for the fallback runner, which applies GL's rule itself:
/// each parameter by name when that uniform is active in the linked
/// program, else through its positional alias (`name|alias`, split by
/// `shader_layers::fallback::call`).
fn runner_param_calls(params: &[Param], calls: &mut Vec<(String, Call)>) {
    for (index, p) in params.iter().enumerate() {
        let name = format!("u_{}|{}{index}", p.key, if p.color { "u_c" } else { "u_p" });
        let call = match p.value {
            Binding::Float(v) => f(&[v]),
            Binding::Color(rgb) => f(&rgb),
        };
        calls.push((name, call));
    }
}

/// A target's pixels as binary16 RGBA in its row order (GL order for
/// every pm-render target).
fn read_half(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
) -> Result<FallbackPixels, String> {
    let raw =
        pm_gpu::readback::read_raw(device, queue, &target.texture).map_err(|e| e.to_string())?;
    let rgba16f: Vec<u16> = match target.format() {
        wgpu::TextureFormat::Rgba16Float => crate::shader_layers::fallback::halves(&raw),
        wgpu::TextureFormat::Rgba32Float => raw
            .chunks_exact(4)
            .map(|c| pm_gpu::readback::f32_to_half(f32::from_le_bytes([c[0], c[1], c[2], c[3]])))
            .collect(),
        other => return Err(format!("format {other:?}")),
    };
    Ok(FallbackPixels {
        width: target.width(),
        height: target.height(),
        rgba16f,
    })
}

/// Writes the runner's binary16 pixels into a working target (before the
/// rest of the frame's commands run: they are submitted after it).
fn write_half(queue: &wgpu::Queue, target: &Target, pixels: &FallbackPixels) {
    let (bytes, texel): (Vec<u8>, u32) = match target.format() {
        wgpu::TextureFormat::Rgba32Float => (
            pixels
                .rgba16f
                .iter()
                .flat_map(|h| pm_gpu::readback::half_to_f32(*h).to_le_bytes())
                .collect(),
            16,
        ),
        _ => (
            crate::shader_layers::fallback::half_bytes(&pixels.rgba16f),
            8,
        ),
    };
    queue.write_texture(
        target.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(pixels.width * texel),
            rows_per_image: Some(pixels.height),
        },
        wgpu::Extent3d {
            width: pixels.width,
            height: pixels.height,
            depth_or_array_layers: 1,
        },
    );
}

fn span_name(span: &Span) -> String {
    match &span.subject {
        Subject::Definition { id, .. } => id.as_str().to_owned(),
        Subject::Code(_) => format!("The shader of layer {}", span.layer.as_str()),
    }
}

fn span_scope(span: &Span) -> ErrorScope {
    match (&span.subject, &span.instance) {
        (Subject::Definition { id, .. }, Some(effect)) => ErrorScope::Effect {
            layer: span.layer.clone(),
            effect: effect.clone(),
            definition: id.clone(),
        },
        (Subject::Definition { id, .. }, None) => ErrorScope::Definition {
            definition: id.clone(),
        },
        _ => ErrorScope::Layer {
            layer: span.layer.clone(),
        },
    }
}

/// The renderer's own effect state, lent to the chains it prepared.
pub(crate) struct Native<'a> {
    pub effects: &'a mut Effects,
    pub timer: Option<&'a mut FrameTimer>,
    pub linear: &'a wgpu::Sampler,
    /// A program WebGL draws that the engine cannot build routes the frame.
    pub unsupported: &'a mut Vec<Unsupported>,
}

/// Everything a chain records with.
pub(crate) struct Gpu<'c> {
    device: &'c wgpu::Device,
    queue: &'c wgpu::Queue,
    encoder: &'c mut wgpu::CommandEncoder,
    pool: &'c mut TargetPool,
    precision: Precision,
    /// Intermediates, given back after the frame is submitted.
    taken: &'c mut Vec<Target>,
    effects: &'c mut Effects,
    timer: Option<&'c mut FrameTimer>,
    linear: &'c wgpu::Sampler,
    unsupported: &'c mut Vec<Unsupported>,
}

impl<'c> Gpu<'c> {
    pub(crate) fn of(cx: &'c mut EffectContext<'_>) -> Option<Gpu<'c>> {
        let native = cx.native.as_mut()?;
        Some(Gpu {
            device: cx.device,
            queue: cx.queue,
            encoder: &mut *cx.encoder,
            pool: &mut *cx.pool,
            precision: cx.precision,
            taken: &mut *cx.taken,
            effects: &mut *native.effects,
            timer: native.timer.as_deref_mut(),
            linear: native.linear,
            unsupported: &mut *native.unsupported,
        })
    }

    /// Whether a definition is a built-in's (the hang guard keeps those).
    fn builtin(&self, kind: DefinitionKind, id: &str) -> bool {
        self.effects
            .get(kind, id)
            .is_some_and(|d| d.origin == pm_protocol::definitions::DefinitionOrigin::BuiltIn)
    }

    /// A program the hang guard holds (a frame that drew it may be running
    /// away): left out, and the frame is not presented.
    fn held(&mut self, kind: DefinitionKind, id: &str) -> bool {
        self.effects
            .watchdog
            .hold(&super::watchdog::Subject::Definition {
                kind,
                id: id.into(),
            })
    }

    fn take(&mut self, w: u32, h: u32) -> Target {
        self.pool.take_working(self.device, w, h, self.precision)
    }

    /// An exact copy of `source` in a new working target.
    pub(crate) fn copy_of(&mut self, source: &Target) -> Target {
        let out = self.take(source.width(), source.height());
        self.encoder.copy_texture_to_texture(
            source.texture.as_image_copy(),
            out.texture.as_image_copy(),
            wgpu::Extent3d {
                width: out.width(),
                height: out.height(),
                depth_or_array_layers: 1,
            },
        );
        out
    }

    /// `input` in E (a new target), or `input` itself in Standard.
    fn encode_e(&mut self, input: &Target, working: WorkingEncoding) -> Target {
        if working == WorkingEncoding::Standard {
            return input.clone();
        }
        let out = self.take(input.width(), input.height());
        self.simple_pass(SimpleOp::EncodeE, input, &out);
        self.taken.push(out.clone());
        out
    }

    /// `result` (in E) back in the working encoding.
    fn decode_e(&mut self, result: Target, working: WorkingEncoding) -> Target {
        if working == WorkingEncoding::Standard {
            return result;
        }
        let out = self.take(result.width(), result.height());
        self.simple_pass(SimpleOp::DecodeE, &result, &out);
        self.taken.push(result);
        out
    }

    /// The chain on `input`: the last output, or `None` when no effect ran.
    /// Intermediates go to `taken`; the result does not.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn chain(
        &mut self,
        layer: &str,
        steps: &[EffectStep],
        time: f64,
        input: &Target,
        backdrop: Option<&Target>,
        coordinates: ([f64; 4], [f64; 2], [f64; 2]),
        working: WorkingEncoding,
    ) -> Option<Target> {
        let input = self.encode_e(input, working);
        let backdrop = backdrop.map(|b| self.encode_e(b, working));
        let mut current: Option<Target> = None;
        for step in steps {
            let from = current.as_ref().unwrap_or(&input).clone();
            if let Some(next) =
                self.run_effect(layer, step, &from, backdrop.as_ref(), time, coordinates)
                && let Some(previous) = current.replace(next)
            {
                self.taken.push(previous);
            }
        }
        current.map(|r| self.decode_e(r, working))
    }

    fn fallback(&mut self, layer: &str, definition: &str) {
        self.unsupported.push(Unsupported {
            composition: None,
            layer: Some(layer.to_owned()),
            feature: Feature::ShaderFallback(definition.to_owned()),
        });
    }

    /// `runEffects` for one effect: its passes from `input`; the output, or
    /// `None` when the effect is skipped.
    pub(crate) fn run_effect(
        &mut self,
        layer: &str,
        step: &EffectStep,
        input: &Target,
        backdrop: Option<&Target>,
        time: f64,
        (uv, origin, resolution): ([f64; 4], [f64; 2], [f64; 2]),
    ) -> Option<Target> {
        let format = input.format();
        let (w, h) = (input.width(), input.height());
        let device = self.device;
        if let Err(failure) =
            self.effects
                .program(device, DefinitionKind::Effect, &step.definition, format)
        {
            // WebGL draws it and the engine cannot: the fallback runner
            // draws it when one is attached, else the frame routes.
            if !self.effects.fallback.runner {
                self.fallback(layer, &step.definition);
                return None;
            }
            let mut result = self.runner_effect(
                layer,
                step,
                input,
                backdrop,
                time,
                failure,
                (uv, origin, resolution),
            )?;
            if step.clamp {
                let clamped = self.take(w, h);
                self.simple_pass(SimpleOp::Clamp, &result, &clamped);
                self.taken.push(std::mem::replace(&mut result, clamped));
            }
            return Some(result);
        }
        let builtin = self.builtin(DefinitionKind::Effect, &step.definition);
        if !builtin && self.held(DefinitionKind::Effect, &step.definition) {
            return None;
        }
        let span = self.timer.as_deref_mut().and_then(|t| {
            t.begin(Span::definition(
                DefinitionKind::Effect,
                &step.definition,
                layer,
                Some(&step.instance),
                builtin,
            ))
        });
        if let (Some(timer), Some((b, _))) = (self.timer.as_deref(), span) {
            timer.mark(device, self.encoder, &input.view, b);
        }
        let mut outputs: Vec<Target> = Vec::new();
        for pass in 0..step.passes {
            let mut calls: Vec<(String, Call)> = vec![("u_tex".into(), Call::I(0))];
            if step.keep_orig {
                calls.push(("u_orig".into(), Call::I(1)));
            }
            if step.backdrop {
                calls.push(("u_backdrop".into(), Call::I(2)));
            }
            calls.push(("u_m".into(), Call::M3(full_quad(w, h))));
            calls.push(("u_res".into(), f(&[f64::from(w), f64::from(h)])));
            calls.push(("u_uv".into(), f(&uv)));
            calls.push(("u_coordRes".into(), f(&resolution)));
            calls.push(("u_pxOrigin".into(), f(&origin)));
            calls.push((
                "u_texel".into(),
                f(&[1.0 / f64::from(w), 1.0 / f64::from(h)]),
            ));
            calls.push(("u_time".into(), f(&[time])));
            calls.push(("u_pass".into(), Call::I(i32::from(pass))));
            let program = self
                .effects
                .program(device, DefinitionKind::Effect, &step.definition, format)
                .expect("built above");
            param_calls(program, &step.params, &mut calls);
            let previous = outputs.last().unwrap_or(input).clone();
            let units: [Option<&Target>; 3] = [
                Some(&previous),
                step.keep_orig.then_some(input),
                backdrop.filter(|_| step.backdrop),
            ];
            let out = self.take(w, h);
            self.legacy_pass(
                DefinitionKind::Effect,
                &step.definition,
                &calls,
                &units,
                &out,
            );
            outputs.push(out);
        }
        if let (Some(timer), Some((_, e)), Some(last)) =
            (self.timer.as_deref(), span, outputs.last())
        {
            timer.mark(device, self.encoder, &last.view, e);
        }
        let mut result = outputs.pop().expect("at least one pass");
        self.taken.extend(outputs);
        if step.clamp {
            let clamped = self.take(w, h);
            self.simple_pass(SimpleOp::Clamp, &result, &clamped);
            self.taken.push(std::mem::replace(&mut result, clamped));
        }
        Some(result)
    }

    /// `runTransition` from `before` and `with_layer` (both in E): the
    /// output, or `None` when the program cannot run.
    pub(crate) fn run_transition(
        &mut self,
        layer: &str,
        step: &TransitionStep,
        before: &Target,
        with_layer: &Target,
        time: f64,
    ) -> Option<Target> {
        let format = before.format();
        let (w, h) = (before.width(), before.height());
        let device = self.device;
        if let Err(failure) =
            self.effects
                .program(device, DefinitionKind::Transition, &step.definition, format)
        {
            if !self.effects.fallback.runner {
                self.fallback(layer, &step.definition);
                return None;
            }
            return self.runner_transition(layer, step, before, with_layer, time, failure);
        }
        let builtin = self.builtin(DefinitionKind::Transition, &step.definition);
        if !builtin && self.held(DefinitionKind::Transition, &step.definition) {
            return None;
        }
        let span = self.timer.as_deref_mut().and_then(|t| {
            t.begin(Span::definition(
                DefinitionKind::Transition,
                &step.definition,
                layer,
                None,
                builtin,
            ))
        });
        let (from, to) = if step.edge_in {
            (before, with_layer)
        } else {
            (with_layer, before)
        };
        let mut calls: Vec<(String, Call)> = vec![
            ("u_from".into(), Call::I(0)),
            ("u_to".into(), Call::I(1)),
            ("u_m".into(), Call::M3(full_quad(w, h))),
            ("u_res".into(), f(&[f64::from(w), f64::from(h)])),
            ("u_uv".into(), f(&[0.0, 0.0, 1.0, 1.0])),
            (
                "u_texel".into(),
                f(&[1.0 / f64::from(w), 1.0 / f64::from(h)]),
            ),
            ("u_time".into(), f(&[time])),
            ("u_prog".into(), f(&[step.progress])),
        ];
        let program = self
            .effects
            .program(device, DefinitionKind::Transition, &step.definition, format)
            .expect("built above");
        param_calls(program, &step.params, &mut calls);
        let out = self.take(w, h);
        if let (Some(timer), Some((b, _))) = (self.timer.as_deref(), span) {
            timer.mark(device, self.encoder, &from.view, b);
        }
        self.legacy_pass(
            DefinitionKind::Transition,
            &step.definition,
            &calls,
            &[Some(from), Some(to)],
            &out,
        );
        if let (Some(timer), Some((_, e))) = (self.timer.as_deref(), span) {
            timer.mark(device, self.encoder, &out.view, e);
        }
        Some(out)
    }

    /// An effect only the fallback runner draws (`super::fallback`): its
    /// passes with the compositor's calls, `u_tex` chained through them,
    /// `u_orig` the input, `u_backdrop` the stack beneath.
    #[allow(clippy::too_many_arguments)]
    fn runner_effect(
        &mut self,
        layer: &str,
        step: &EffectStep,
        input: &Target,
        backdrop: Option<&Target>,
        time: f64,
        failure: pm_glsl::Failure,
        (uv, origin, resolution): ([f64; 4], [f64; 2], [f64; 2]),
    ) -> Option<Target> {
        let (w, h) = (input.width(), input.height());
        let passes: Vec<Vec<(String, Call)>> = (0..step.passes.max(1))
            .map(|pass| {
                let mut calls: Vec<(String, Call)> = vec![("u_tex".into(), Call::I(0))];
                if step.keep_orig {
                    calls.push(("u_orig".into(), Call::I(1)));
                }
                if step.backdrop {
                    calls.push(("u_backdrop".into(), Call::I(2)));
                }
                calls.push(("u_m".into(), Call::M3(full_quad(w, h))));
                calls.push(("u_res".into(), f(&[f64::from(w), f64::from(h)])));
                calls.push(("u_uv".into(), f(&uv)));
                calls.push(("u_coordRes".into(), f(&resolution)));
                calls.push(("u_pxOrigin".into(), f(&origin)));
                calls.push((
                    "u_texel".into(),
                    f(&[1.0 / f64::from(w), 1.0 / f64::from(h)]),
                ));
                calls.push(("u_time".into(), f(&[time])));
                calls.push(("u_pass".into(), Call::I(i32::from(pass))));
                runner_param_calls(&step.params, &mut calls);
                calls
            })
            .collect();
        let backdrop = backdrop.filter(|_| step.backdrop);
        let mut units = vec![FallbackUnit {
            unit: 0,
            texture: 0,
            chain: true,
        }];
        if step.keep_orig {
            units.push(FallbackUnit {
                unit: 1,
                texture: 0,
                chain: false,
            });
        }
        let mut textures = vec![input];
        if let Some(b) = backdrop {
            units.push(FallbackUnit {
                unit: 2,
                texture: 1,
                chain: false,
            });
            textures.push(b);
        }
        let span = Span::definition(
            DefinitionKind::Effect,
            &step.definition,
            layer,
            Some(&step.instance),
            self.builtin(DefinitionKind::Effect, &step.definition),
        );
        self.runner_step(span, passes, units, &textures, failure)
    }

    /// A transition only the fallback runner draws: `u_from` and `u_to`
    /// the stack without and with the layer.
    fn runner_transition(
        &mut self,
        layer: &str,
        step: &TransitionStep,
        before: &Target,
        with_layer: &Target,
        time: f64,
        failure: pm_glsl::Failure,
    ) -> Option<Target> {
        let (w, h) = (before.width(), before.height());
        let (from, to) = if step.edge_in {
            (before, with_layer)
        } else {
            (with_layer, before)
        };
        let mut calls: Vec<(String, Call)> = vec![
            ("u_from".into(), Call::I(0)),
            ("u_to".into(), Call::I(1)),
            ("u_m".into(), Call::M3(full_quad(w, h))),
            ("u_res".into(), f(&[f64::from(w), f64::from(h)])),
            ("u_uv".into(), f(&[0.0, 0.0, 1.0, 1.0])),
            (
                "u_texel".into(),
                f(&[1.0 / f64::from(w), 1.0 / f64::from(h)]),
            ),
            ("u_time".into(), f(&[time])),
            ("u_prog".into(), f(&[step.progress])),
        ];
        runner_param_calls(&step.params, &mut calls);
        let units = vec![
            FallbackUnit {
                unit: 0,
                texture: 0,
                chain: false,
            },
            FallbackUnit {
                unit: 1,
                texture: 1,
                chain: false,
            },
        ];
        let span = Span::definition(
            DefinitionKind::Transition,
            &step.definition,
            layer,
            None,
            self.builtin(DefinitionKind::Transition, &step.definition),
        );
        self.runner_step(span, vec![calls], units, &[from, to], failure)
    }

    /// The runner's pixels for a step, written into a working target, or
    /// `None` when the step is left out: its request waits for the runner
    /// (the frame is not presented), the hang guard holds it, or the runner
    /// could not draw its program (skipped and reported, as WebGL skips a
    /// program it refuses).
    fn runner_step(
        &mut self,
        span: Span,
        passes: Vec<Vec<(String, Call)>>,
        units: Vec<FallbackUnit>,
        textures: &[&Target],
        failure: pm_glsl::Failure,
    ) -> Option<Target> {
        let Subject::Definition { kind, id } = &span.subject else {
            return None;
        };
        let (kind, id) = (*kind, id.as_str().to_owned());
        if self.effects.watchdog.is_disabled(kind, &id)
            || (!span.builtin && self.effects.watchdog.hold(&span.subject))
        {
            return None;
        }
        let source = self
            .effects
            .get(kind, &id)
            .and_then(|d| d.glsl.as_ref())
            .map(|g| g.source.clone())
            .unwrap_or_default();
        let program = super::fallback::program_id_of(pm_glsl::VERTEX, &source);
        if let Some(why) = self.effects.fallback.failure(&program).cloned() {
            self.runner_failed(&span, &program, &why);
            return None;
        }
        // Its last request failed for a transient reason (the runner's
        // context was lost): asked for again once the backoff passed, drawn
        // without it until then and reported once.
        // A job's host already tried it again in place: the job stops.
        if self.effects.watchdog.policy() == super::watchdog::Policy::Job
            && let Some(retry) = self.effects.fallback.retries.last(&program)
        {
            let why = retry.failure.clone();
            self.runner_failed(&span, &program, &why);
            return None;
        }
        let now = self.effects.watchdog.now();
        if let Some(super::fallback::RetryState::Wait { failure, report }) =
            self.effects.fallback.retries.state(&program, now)
        {
            if report {
                let name = span_name(&span);
                self.effects.watchdog.report(EngineError::new(
                    ErrorCode::NotHeadlessRenderable,
                    span_scope(&span),
                    crate::effects::truncate(format!(
                        "{name} could not be drawn by the WebGL fallback runner for now ({}: {}); it is skipped until the runner draws it again.",
                        failure.reason, failure.message
                    )),
                ));
            }
            return None;
        }
        // A request already waits in this frame: what this step reads may
        // depend on it, so it is asked for once that one is answered.
        if self.effects.fallback.blocked {
            return None;
        }
        let (w, h) = (textures[0].width(), textures[0].height());
        // The frame so far runs now, so the textures hold this frame's
        // pixels when they are read.
        let device = self.device;
        let queue = self.queue;
        let done = std::mem::replace(
            self.encoder,
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm frame (after a fallback readback)"),
            }),
        );
        queue.submit([done.finish()]);
        let mut pixels = Vec::with_capacity(textures.len());
        for t in textures {
            match read_half(device, queue, t) {
                Ok(p) => pixels.push(Arc::new(p)),
                Err(e) => {
                    self.effects.watchdog.report(EngineError::new(
                        ErrorCode::Internal,
                        span_scope(&span),
                        format!("{id}: the fallback runner's input could not be read ({e})"),
                    ));
                    return None;
                }
            }
        }
        let request = crate::shader_layers::fallback::request_with_textures(
            span.layer.as_str().to_owned(),
            source,
            w,
            h,
            &passes,
            units,
            &pixels.iter().map(|p| p.as_ref()).collect::<Vec<_>>(),
            &failure,
        );
        if let Some(result) = self.effects.fallback.result(&request.key)
            && result.width == w
            && result.height == h
        {
            let out = self.take(w, h);
            write_half(queue, &out, &result);
            return Some(out);
        }
        self.effects.fallback.wait(Arc::new(request), pixels);
        None
    }

    /// The runner could not draw a step's program: reported once; a draw
    /// past the runner's deadline is a runaway (the watchdog turns it off,
    /// or stops a job); a job cannot deliver without a program WebGL draws
    /// today, so any other failure but WebGL refusing it stops the job.
    fn runner_failed(
        &mut self,
        span: &Span,
        program: &str,
        why: &crate::shader_layers::FallbackFailure,
    ) {
        let name = span_name(span);
        let job = self.effects.watchdog.policy() == super::watchdog::Policy::Job;
        if why.reason == "watchdog" {
            self.effects.watchdog.runner_runaway(
                span,
                &name,
                format!(
                    "{name} did not finish drawing in the WebGL fallback runner ({}), so it is turned off until it is registered or reloaded again.",
                    why.message
                ),
            );
            return;
        }
        if job && why.reason != "compile" {
            self.effects.fallback.fatal.get_or_insert_with(|| {
                format!(
                    "{name} needs the WebGL fallback runner, which could not draw it ({}: {}).",
                    why.reason, why.message
                )
            });
            return;
        }
        if !self.effects.fallback.report_once(program) {
            return;
        }
        let (code, message) = match why.reason.as_str() {
            "unavailable" => (
                ErrorCode::NotHeadlessRenderable,
                format!(
                    "{name} needs the WebGL fallback runner, which is not available here ({}).",
                    why.message
                ),
            ),
            reason => (
                ErrorCode::ShaderCompile,
                format!(
                    "{name} could not be drawn by the WebGL fallback runner ({reason}: {}); it is skipped.",
                    why.message
                ),
            ),
        };
        self.effects.watchdog.report(EngineError::new(
            code,
            span_scope(span),
            crate::effects::truncate(message),
        ));
    }

    /// One draw of a legacy program into `out` (cleared to transparent,
    /// blending off), with the compositor's uniform calls and texture
    /// units.
    pub(crate) fn legacy_pass(
        &mut self,
        kind: DefinitionKind,
        definition: &str,
        calls: &[(String, Call)],
        units: &[Option<&Target>],
        out: &Target,
    ) {
        let device = self.device;
        let queue = self.queue;
        let format = out.format();
        let dummy = self.effects.dummy(device, queue, format);
        let linear = self.linear;
        let program = self
            .effects
            .program(device, kind, definition, format)
            .expect("checked by the caller");
        let buffer = |block: &pm_glsl::reflect::Block| {
            let bytes = GpuProgram::block_bytes(block, calls);
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pm legacy uniforms"),
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
            .map(|unit| {
                units
                    .get(unit as usize)
                    .copied()
                    .flatten()
                    .map_or(&dummy.view, |t| &t.view)
            })
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
            label: Some("pm legacy fragment"),
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
            label: Some("pm legacy vertex"),
            layout: &program.vertex_layout,
            entries: &vertex_entries,
        });
        let pipeline = program
            .pipeline(device, format)
            .expect("built above")
            .clone();
        let mut pass = begin(
            self.encoder,
            out,
            "pm legacy glsl",
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &fragment, &[]);
        pass.set_bind_group(1, &vertex, &[]);
        pass.draw(0..4, 0..1);
    }

    /// A per-pixel pass of `source` into `out`: `clamp: true` (0..1), or
    /// the E encode and decode around legacy programs.
    fn simple_pass(&mut self, op: SimpleOp, source: &Target, out: &Target) {
        let pipeline = self
            .effects
            .simple_pipeline(self.device, op, out.format())
            .clone();
        let layout = pipeline.get_bind_group_layout(0);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm effect pixel pass"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source.view),
            }],
        });
        let mut pass = begin(
            self.encoder,
            out,
            "pm effect pixel pass",
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..4, 0..1);
    }
}

/// An effect chain prepared by [`Effects`] (the compositor's
/// `PreparedEffects::passes`).
pub(crate) struct Chain {
    pub layer: String,
    pub steps: Vec<EffectStep>,
    /// `u_time`: the level's time in seconds.
    pub time: f64,
    /// The steps' program sources (a hot reload changes the key).
    pub sources: Vec<Option<pm_glsl::Key>>,
}

impl EffectPasses for Chain {
    fn key(&self) -> Vec<u8> {
        format!(
            "effects|{}|{:?}|{:?}|{:?}",
            self.layer,
            self.time.to_bits(),
            self.steps,
            self.sources
        )
        .into_bytes()
    }

    fn record(
        &self,
        cx: &mut EffectContext<'_>,
        input: Covered<'_>,
        backdrop: Option<Covered<'_>>,
    ) -> Result<Option<Target>, String> {
        let full = cx.full;
        let working = cx.working;
        let rect = input.rect;
        let region = (rect != Region::full(full.0, full.1)).then_some(rect);
        let coordinates = effect_coordinates(region, full, (rect.width, rect.height));
        let mut gpu = Gpu::of(cx).ok_or("the effect chain needs the renderer that prepared it")?;
        let out = gpu.chain(
            &self.layer,
            &self.steps,
            self.time,
            input.target,
            backdrop.map(|b| b.target),
            coordinates,
            working,
        );
        if let Some(t) = &out {
            gpu.taken.push(t.clone());
        }
        Ok(out)
    }
}

/// A transition prepared by [`Effects`].
pub(crate) struct Transition {
    pub layer: String,
    pub step: TransitionStep,
    pub time: f64,
    pub source: Option<pm_glsl::Key>,
}

impl TransitionPasses for Transition {
    fn key(&self) -> Vec<u8> {
        format!(
            "transition|{}|{:?}|{:?}|{:?}",
            self.layer,
            self.time.to_bits(),
            self.step,
            self.source
        )
        .into_bytes()
    }

    fn record(
        &self,
        cx: &mut EffectContext<'_>,
        before: Covered<'_>,
        with_layer: Covered<'_>,
    ) -> Result<Option<Target>, String> {
        let working = cx.working;
        let mut gpu = Gpu::of(cx).ok_or("the transition needs the renderer that prepared it")?;
        let b = gpu.encode_e(before.target, working);
        let w = gpu.encode_e(with_layer.target, working);
        let Some(out) = gpu.run_transition(&self.layer, &self.step, &b, &w, self.time) else {
            return Ok(None);
        };
        let out = gpu.decode_e(out, working);
        gpu.taken.push(out.clone());
        Ok(Some(out))
    }
}

/// Shares a prepared chain with the compositor.
pub(crate) fn chain_passes(chain: Chain) -> Arc<dyn EffectPasses> {
    Arc::new(chain)
}

/// Shares a prepared transition with the compositor.
pub(crate) fn transition_passes(t: Transition) -> Arc<dyn TransitionPasses> {
    Arc::new(t)
}

fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    target: &Target,
    label: &str,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &target.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_coordinates_follow_the_viewport_rule() {
        let (uv, origin, res) = effect_coordinates(None, (1920, 1080), (1920, 1080));
        assert_eq!(
            (uv, origin, res),
            ([0.0, 0.0, 1.0, 1.0], [0.0, 0.0], [1920.0, 1080.0])
        );
        let r = Region {
            x: 480,
            y: 270,
            width: 960,
            height: 540,
        };
        let (uv, origin, res) = effect_coordinates(Some(r), (1920, 1080), (960, 540));
        assert_eq!(uv, [0.25, 0.25, 0.5, 0.5]);
        assert_eq!(origin, [480.0, 270.0]);
        assert_eq!(res, [1920.0, 1080.0]);
    }
}
