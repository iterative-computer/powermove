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
/// for a buffer covering `region` of a level of `full` pixels. With a
/// `view` (an output origin, today's preview viewport) the coordinates are
/// the composition's at today's scale: the resolution is the whole
/// composition's, and the origin is the view's plus the region's.
pub(crate) fn effect_coordinates(
    region: Option<Region>,
    full: (u32, u32),
    buffer: (u32, u32),
    view: Option<crate::plan::OutputView>,
) -> ([f64; 4], [f64; 2], [f64; 2]) {
    let at = region.map_or([0.0, 0.0], |r| [f64::from(r.x), f64::from(r.y)]);
    let (resolution, origin) = match view {
        Some(v) => (v.resolution, [v.origin[0] + at[0], v.origin[1] + at[1]]),
        None => ([f64::from(full.0.max(1)), f64::from(full.1.max(1))], at),
    };
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

/// A target's pixels in GL row order, preserving the requested working depth.
pub(crate) fn read_half(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    color_depth: u8,
) -> Result<FallbackPixels, String> {
    let raw =
        pm_gpu::readback::read_raw(device, queue, &target.texture).map_err(|e| e.to_string())?;
    let code8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round_ties_even() as u16;
    let rgba32f = if color_depth == 32 {
        Some(match target.format() {
            wgpu::TextureFormat::Rgba32Float => crate::shader_layers::fallback::float_bits(&raw),
            wgpu::TextureFormat::Rgba16Float => crate::shader_layers::fallback::halves(&raw)
                .into_iter()
                .map(|h| pm_gpu::readback::half_to_f32(h).to_bits())
                .collect(),
            wgpu::TextureFormat::Rgba8Unorm => raw
                .iter()
                .map(|b| (f32::from(*b) / 255.0).to_bits())
                .collect(),
            other => return Err(format!("format {other:?}")),
        })
    } else {
        None
    };
    let rgba16f: Vec<u16> = if color_depth == 32 {
        Vec::new()
    } else {
        match target.format() {
            wgpu::TextureFormat::Rgba8Unorm => raw
                .iter()
                .map(|b| {
                    if color_depth == 8 {
                        u16::from(*b)
                    } else {
                        pm_gpu::readback::f32_to_half(f32::from(*b) / 255.0)
                    }
                })
                .collect(),
            wgpu::TextureFormat::Rgba16Float => crate::shader_layers::fallback::halves(&raw)
                .into_iter()
                .map(|h| {
                    if color_depth == 8 {
                        code8(pm_gpu::readback::half_to_f32(h))
                    } else {
                        h
                    }
                })
                .collect(),
            wgpu::TextureFormat::Rgba32Float => raw
                .chunks_exact(4)
                .map(|c| {
                    let value = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    if color_depth == 8 {
                        code8(value)
                    } else {
                        pm_gpu::readback::f32_to_half(value)
                    }
                })
                .collect(),
            other => return Err(format!("format {other:?}")),
        }
    };
    Ok(FallbackPixels {
        width: target.width(),
        height: target.height(),
        rgba16f,
        rgba32f,
    })
}

/// Writes the runner's pixels into a working target (before the
/// rest of the frame's commands run: they are submitted after it).
fn write_half(queue: &wgpu::Queue, target: &Target, pixels: &FallbackPixels) {
    let (bytes, texel): (Vec<u8>, u32) = match target.format() {
        wgpu::TextureFormat::Rgba8Unorm => (
            pixels
                .rgba16f
                .iter()
                .map(|code| u8::try_from(*code).expect("RGBA8 fallback code"))
                .collect(),
            4,
        ),
        wgpu::TextureFormat::Rgba32Float => (
            pixels.rgba32f.as_ref().map_or_else(
                || {
                    pixels
                        .rgba16f
                        .iter()
                        .flat_map(|h| pm_gpu::readback::half_to_f32(*h).to_le_bytes())
                        .collect()
                },
                |bits| crate::shader_layers::fallback::float_bytes(bits),
            ),
            16,
        ),
        _ => (
            pixels.rgba32f.as_ref().map_or_else(
                || crate::shader_layers::fallback::half_bytes(&pixels.rgba16f),
                |bits| {
                    crate::shader_layers::fallback::half_bytes(
                        &bits
                            .iter()
                            .map(|v| pm_gpu::readback::f32_to_half(f32::from_bits(*v)))
                            .collect::<Vec<_>>(),
                    )
                },
            ),
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
    pub inputs: &'a [super::resources::Ready],
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
    inputs: Vec<super::resources::Ready>,
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
            inputs: native.inputs.to_vec(),
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
        let mut ready = self.inputs.clone();
        for r in &mut ready {
            r.target = self.encode_e(&r.target, working);
        }
        self.inputs = ready;
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
        let mut asset_targets = Vec::new();
        for (name, asset) in &step.assets {
            if asset.vector.is_some() {
                let ready = self
                    .inputs
                    .iter()
                    .find(|r| r.request == asset.request())
                    .expect("brush vector producer prepared");
                asset_targets.push((name, ready.target.clone()));
                continue;
            }
            let target = Target::new(
                device,
                "pm effect asset lease",
                asset.width,
                asset.height,
                wgpu::TextureFormat::Rgba32Float,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            let data = asset
                .lut
                .as_ref()
                .map_or(asset.rgba.as_slice(), |c| c.rgba.as_slice());
            let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.queue.write_texture(
                target.texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(asset.width * 16),
                    rows_per_image: Some(asset.height),
                },
                target.texture.size(),
            );
            asset_targets.push((name, target));
        }
        let mut outputs: Vec<Target> = Vec::new();
        let passes = if step.temporal {
            step.references.len() as u8
        } else {
            step.passes
        };
        for pass in 0..passes {
            let (ow, oh) = if pass == 0 {
                step.spatial
                    .and_then(|s| s.grid)
                    .map_or((w, h), |g| (g[0], g[1]))
            } else {
                (w, h)
            };
            let mut calls: Vec<(String, Call)> = vec![("u_tex".into(), Call::I(0))];
            if step.keep_orig {
                calls.push(("u_orig".into(), Call::I(1)));
            }
            if step.backdrop {
                calls.push(("u_backdrop".into(), Call::I(2)));
            }
            calls.push(("u_m".into(), Call::M3(full_quad(ow, oh))));
            calls.push(("u_res".into(), f(&[f64::from(ow), f64::from(oh)])));
            calls.push(("u_uv".into(), f(&uv)));
            calls.push(("u_coordRes".into(), f(&resolution)));
            calls.push(("u_pxOrigin".into(), f(&origin)));
            calls.push((
                "u_texel".into(),
                f(&[1.0 / f64::from(w), 1.0 / f64::from(h)]),
            ));
            calls.push(("u_time".into(), f(&[time])));
            calls.push(("u_pass".into(), Call::I(i32::from(pass))));
            if let Some(owner) = &step.owner {
                calls.push(("u_layerUV".into(), Call::M3(owner.uv)));
                calls.push(("u_layerRes".into(), f(&owner.size)));
                calls.push(("u_pixelScale".into(), f(&owner.pixel_scale)));
                calls.push(("u_sourceRes".into(), f(&[f64::from(w), f64::from(h)])));
                calls.push((
                    "u_sampleLod".into(),
                    f(&[step.spatial.map_or(0., |s| s.lod as f64)]),
                ));
            }
            if let Some(grid) = step.spatial.and_then(|s| s.grid) {
                calls.push(("u_gridRes".into(), f(&[grid[0] as f64, grid[1] as f64])));
            }
            let program = self
                .effects
                .program(device, DefinitionKind::Effect, &step.definition, format)
                .expect("built above");
            param_calls(program, &step.params, &mut calls);
            let mut previous = outputs.last().unwrap_or(input).clone();
            if step.spatial.is_some_and(|s| s.lod > 0)
                && !(pass > 0 && step.spatial.is_some_and(|s| s.grid.is_some()))
            {
                let mut pyramid = Target::with_mips(
                    device,
                    "pm spatial effect mips",
                    previous.width(),
                    previous.height(),
                    format,
                    pm_gpu::target::working_usage(),
                );
                self.encoder.copy_texture_to_texture(
                    previous.texture.as_image_copy(),
                    pyramid.texture.as_image_copy(),
                    previous.texture.size(),
                );
                let mips = self
                    .effects
                    .mips
                    .get_or_insert_with(|| crate::sampling::gpu::Resampler::new(device));
                mips.fill_mips(device, self.encoder, &pyramid.texture);
                pyramid.view = pyramid.texture.create_view(&Default::default());
                self.taken.push(pyramid.clone());
                previous = pyramid;
            }
            let mut units = vec![
                Some(&previous),
                step.keep_orig.then_some(input),
                backdrop.filter(|_| step.backdrop),
            ];
            let ready = self.inputs.clone();
            for (index, reference) in step.references.iter().enumerate() {
                if step.temporal && index != pass as usize {
                    continue;
                }
                let resource = ready
                    .iter()
                    .find(|r| r.request == reference.request)
                    .expect("producer prepared before effect");
                let name = &reference.name;
                calls.push((format!("u_{name}"), Call::I(units.len() as i32)));
                calls.push((format!("u_has_{name}"), f(&[1.])));
                calls.push((format!("u_{name}Res"), f(&resource.owner.size)));
                calls.push((format!("u_{name}UV"), Call::M3(resource.owner.uv)));
                let mut mapping = super::resources::from_uv(&resource.owner);
                mapping[6] -= resource.origin[0] as f32;
                mapping[7] -= resource.origin[1] as f32;
                let ratio = [
                    w as f32 / resource.target.width() as f32,
                    h as f32 / resource.target.height() as f32,
                ];
                for column in 0..3 {
                    mapping[column * 3] *= ratio[0];
                    mapping[column * 3 + 1] *= ratio[1];
                }
                calls.push((format!("u_{name}FromUV"), Call::M3(mapping)));
                if step.temporal {
                    let owner = step.owner.as_ref().expect("temporal owner prepared");
                    calls.push(("u_temporalOwnerUV".into(), Call::M3(owner.uv)));
                    calls.push(("u_temporalBounds".into(), f(&owner.bounds)));
                    calls.push(("u_temporalPriorBounds".into(), f(&resource.owner.bounds)));
                    calls.push(("u_temporalRes".into(), f(&[w as f64, h as f64])));
                    calls.push(("u_temporalIndex".into(), f(&[index as f64])));
                    calls.push(("u_temporalCount".into(), f(&[step.references.len() as f64])));
                    calls.push(("u_temporalValid".into(), f(&[1.])));
                }
                units.push(Some(&resource.target));
            }
            for ((name, asset), (_, target)) in step.assets.iter().zip(&asset_targets) {
                calls.push((format!("u_{name}"), Call::I(units.len() as i32)));
                calls.push((format!("u_has_{name}"), f(&[1.])));
                calls.push((
                    format!("u_{name}Res"),
                    f(&[asset.width as f64, asset.height as f64]),
                ));
                if let Some(lut) = &asset.lut {
                    calls.push(("u_lutSize".into(), f(&[lut.size as f64])));
                    calls.push(("u_lutDomainMin".into(), f(&lut.domain_min.map(f64::from))));
                    calls.push(("u_lutDomainMax".into(), f(&lut.domain_max.map(f64::from))));
                } else {
                    let mapping = if asset.brush {
                        let sx = w as f32 / asset.width as f32;
                        let sy = h as f32 / asset.height as f32;
                        [
                            w as f32 * sx,
                            0.,
                            0.,
                            0.,
                            h as f32 * sy,
                            0.,
                            origin[0] as f32 * sx,
                            origin[1] as f32 * sy,
                            1.,
                        ]
                    } else {
                        full_quad(w, h)
                    };
                    calls.push((format!("u_{name}FromUV"), Call::M3(mapping)));
                }
                units.push(Some(target));
            }
            let out = self.take(ow, oh);
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
                if let Some(owner) = &step.owner {
                    calls.push(("u_layerUV".into(), Call::M3(owner.uv)));
                    calls.push(("u_layerRes".into(), f(&owner.size)));
                    calls.push(("u_pixelScale".into(), f(&owner.pixel_scale)));
                    calls.push(("u_sourceRes".into(), f(&[f64::from(w), f64::from(h)])));
                    calls.push(("u_sampleLod".into(), f(&[0.])));
                }
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
    #[allow(clippy::too_many_arguments)]
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
        let (w, h) = (textures[0].width(), textures[0].height());
        if let Some(why) = self.effects.fallback.failure(&program, w, h).cloned() {
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
            match read_half(
                device,
                queue,
                t,
                crate::shader_layers::fallback::color_depth(self.precision),
            ) {
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
        let request = crate::shader_layers::fallback::at_color_depth(
            &request,
            crate::shader_layers::fallback::color_depth(self.precision),
        );
        if let Some(code) = self
            .effects
            .fallback
            .result(&request.key)
            .and_then(|result| result.invalid_rgba8_code(&request))
        {
            let message = format!(
                "{id}: the fallback runner returned invalid RGBA8 channel code {code} (expected 0..=255)"
            );
            if self.effects.watchdog.policy() == super::watchdog::Policy::Job {
                self.effects.fallback.fatal.get_or_insert(message.clone());
            }
            self.effects.watchdog.report(EngineError::new(
                ErrorCode::InvalidMessage,
                span_scope(&span),
                message,
            ));
            return None;
        }
        if let Some(result) = self.effects.fallback.result(&request.key)
            && result.fits(&request)
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
            "tooLarge" => (
                ErrorCode::BudgetExceeded,
                format!(
                    "{name} needs the WebGL fallback runner, and its {} is past this GPU's WebGL limit; it is skipped at this size.",
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
#[derive(Clone)]
pub(crate) struct Chain {
    pub inputs: Vec<super::resources::Producer>,
    pub layer: String,
    pub steps: Vec<EffectStep>,
    /// `u_time`: the level's time in seconds.
    pub time: f64,
    /// The steps' program sources (a hot reload changes the key).
    pub sources: Vec<Option<pm_glsl::Key>>,
}

impl EffectPasses for Chain {
    fn vector_assets(&self) -> Vec<Arc<super::resources::Asset>> {
        self.steps
            .iter()
            .flat_map(|s| {
                s.assets
                    .iter()
                    .filter(|(_, a)| a.vector.is_some())
                    .map(|(_, a)| a.clone())
            })
            .collect()
    }
    fn input_requests(&self) -> Vec<super::resources::Reference> {
        self.steps
            .iter()
            .flat_map(|s| s.references.clone())
            .collect()
    }
    fn input_plans(&self) -> &[super::resources::Producer] {
        &self.inputs
    }
    fn with_input_plans(
        &self,
        plans: Vec<super::resources::Producer>,
    ) -> Option<Arc<dyn EffectPasses>> {
        let mut chain = self.clone();
        chain.inputs = plans;
        Some(Arc::new(chain))
    }
    fn key(&self) -> Vec<u8> {
        format!(
            "effects|{}|{:?}|{:?}|{:?}",
            self.layer,
            self.time.to_bits(),
            self.steps,
            (&self.sources, &self.inputs)
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
        let coordinates = effect_coordinates(region, full, (rect.width, rect.height), cx.view);
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
        let (uv, origin, res) = effect_coordinates(None, (1920, 1080), (1920, 1080), None);
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
        let (uv, origin, res) = effect_coordinates(Some(r), (1920, 1080), (960, 540), None);
        assert_eq!(uv, [0.25, 0.25, 0.5, 0.5]);
        assert_eq!(origin, [480.0, 270.0]);
        assert_eq!(res, [1920.0, 1080.0]);
        // Today's preview viewport: the level is a 1000 × 600 canvas of a
        // 1920 × 1080 composition at scale 4, starting at (3000.5, 1200);
        // a buffer at (100, 50) of it.
        let view = crate::plan::OutputView {
            origin: [3000.5, 1200.0],
            resolution: [7680.0, 4320.0],
        };
        let r = Region {
            x: 100,
            y: 50,
            width: 768,
            height: 432,
        };
        let (uv, origin, res) = effect_coordinates(Some(r), (1000, 600), (768, 432), Some(view));
        assert_eq!(origin, [3100.5, 1250.0]);
        assert_eq!(res, [7680.0, 4320.0]);
        assert_eq!(uv, [3100.5 / 7680.0, 1250.0 / 4320.0, 0.1, 0.1]);
        // The whole level (no region): the view's origin.
        let (_, origin, _) = effect_coordinates(None, (1000, 600), (1000, 600), Some(view));
        assert_eq!(origin, [3000.5, 1200.0]);
    }
}
