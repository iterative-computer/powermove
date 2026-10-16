//! Executes staged layers (`renderProject`'s layer-buffer path): the
//! layer's pixels into its own buffer (`lf`), the effect chain
//! (`runEffects`: 1 to 8 ping-pong passes per effect, `u_orig`,
//! `u_backdrop` = the accumulated lower stack, blending off), and the
//! composite onto the stack, through a transition (`runTransition`) when one
//! is active. Every buffer is the size of the level (composition-sized),
//! in GL row order, at the frame's precision.
//!
//! A level holding staged layers is drawn in runs: the draws before a
//! staged layer go into the target in one render pass (the first clears
//! and fills the background, later ones load), so the accumulated stack a
//! staged layer reads as its backdrop is exactly today's `acc`.

use pm_gpu::Target;
use pm_protocol::common::{DefinitionId, EffectInstanceId, LayerId};
use pm_protocol::definitions::DefinitionKind;

use super::plan::{Binding, EffectStep, Param, Stage, TransitionStep};
use super::program::{Call, GpuProgram, active};
use super::watchdog::Span;
use crate::capability::{Feature, Unsupported};
use crate::exec::{Exec, Region, Resolved};
use crate::pipelines::{QuadUniform, UNIFORM_STRIDE, bytes_of};
use crate::plan::{CompPlan, Content, Draw};

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

/// `setParam`: the named uniform when the program has it active, else the
/// positional alias.
fn param_calls(program: &GpuProgram, params: &[Param], calls: &mut Vec<(String, Call)>) {
    for (index, p) in params.iter().enumerate() {
        let named = format!("u_{}", p.key);
        let name = if active(program.fragment_block.as_ref(), &named) {
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

/// Where a level holding staged layers is drawn: a viewport-safe level
/// under a region of interest draws a region grown by its effects'
/// padding (as today's cropped viewer does, with the crop's effect
/// coordinates); any other staged level draws in full and is cropped.
fn work_region(plan: &CompPlan, region: Option<Region>) -> Option<Region> {
    let region = region?;
    let mut padding = 0.0f64;
    for draw in &plan.draws {
        if let Content::Staged(stage) = &draw.content {
            if stage.transition.is_some() || stage.effects.iter().any(|e| !e.viewport_safe) {
                return None;
            }
            padding += stage.effects.iter().map(|e| e.padding).sum::<f64>();
        }
    }
    // The viewer's overscan (128 px) plus the effects' own reach.
    let pad = (128.0 + padding).ceil().min(f64::from(u32::MAX / 4)) as u32;
    let x0 = region.x.saturating_sub(pad);
    let y0 = region.y.saturating_sub(pad);
    let x1 = (region.x + region.width)
        .saturating_add(pad)
        .min(plan.width);
    let y1 = (region.y + region.height)
        .saturating_add(pad)
        .min(plan.height);
    if x0 == 0 && y0 == 0 && x1 == plan.width && y1 == plan.height {
        return None;
    }
    Some(Region {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

impl Exec<'_> {
    /// A level with staged layers into `target` (the region's size).
    pub(crate) fn render_staged(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        region: Option<Region>,
        target: &Target,
    ) -> Vec<Resolved> {
        let work = work_region(plan, region);
        if work == region {
            return self.render_staged_into(encoder, plan, region, target);
        }
        let (w, h) = work.map_or((plan.width, plan.height), |g| (g.width, g.height));
        let scratch = self.pool.take_working(self.device, w, h, self.precision);
        let resolved = self.render_staged_into(encoder, plan, work, &scratch);
        let r = region.expect("a work region differs only under a region");
        let (wx, wy, wh) = work.map_or((0, 0, plan.height), |g| (g.x, g.y, g.height));
        // GL row order: memory row k of a target is picture row (top + height − 1 − k).
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &scratch.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: r.x - wx,
                    y: wy + wh - r.y - r.height,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: r.width,
                height: r.height,
                depth_or_array_layers: 1,
            },
        );
        self.taken.push(scratch);
        resolved
    }

    fn render_staged_into(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        region: Option<Region>,
        target: &Target,
    ) -> Vec<Resolved> {
        let mut resolved = Vec::with_capacity(plan.draws.len());
        let mut start = 0;
        let mut drawn = false;
        for (i, draw) in plan.draws.iter().enumerate() {
            let Content::Staged(stage) = &draw.content else {
                continue;
            };
            resolved.extend(self.render_run(
                encoder,
                plan,
                &plan.draws[start..i],
                region,
                target,
                drawn,
            ));
            drawn = true;
            resolved.push(self.run_stage(encoder, plan, draw, stage, region, target));
            start = i + 1;
        }
        if start < plan.draws.len() {
            resolved.extend(self.render_run(
                encoder,
                plan,
                &plan.draws[start..],
                region,
                target,
                true,
            ));
        }
        resolved
    }

    /// One staged layer onto `target`.
    fn run_stage(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        draw: &Draw,
        stage: &Stage,
        region: Option<Region>,
        target: &Target,
    ) -> Resolved {
        let (w, h) = (target.width(), target.height());
        let lf = self.pool.take_working(self.device, w, h, self.precision);
        self.render_comp(encoder, &stage.content, region, &lf);
        let coordinates = effect_coordinates(region, (plan.width, plan.height), (w, h));
        let mut current = lf;
        for step in &stage.effects {
            if let Some(next) = self.run_effect(
                encoder,
                &draw.layer,
                step,
                &current,
                Some(target),
                stage.time,
                coordinates,
            ) {
                self.taken.push(std::mem::replace(&mut current, next));
            }
        }
        match &stage.transition {
            None => self.composite(encoder, &current, target, stage.alpha),
            Some(t) => {
                // `withLayer = copyFbo(acc)`, the layer composited onto it.
                let with_layer = self.pool.take_working(self.device, w, h, self.precision);
                copy(encoder, target, &with_layer);
                self.composite(encoder, &current, &with_layer, stage.alpha);
                match self.run_transition(encoder, &draw.layer, t, target, &with_layer, stage.time)
                {
                    Some(out) => {
                        copy(encoder, &out, target);
                        self.taken.push(out);
                    }
                    // A program that cannot run: `acc = withLayer`.
                    None => copy(encoder, &with_layer, target),
                }
                self.taken.push(with_layer);
            }
        }
        let matrix = match region {
            Some(g) => [
                f64::from(g.width),
                0.0,
                0.0,
                f64::from(g.height),
                f64::from(g.x),
                f64::from(g.y),
            ],
            None => [
                f64::from(plan.width),
                0.0,
                0.0,
                f64::from(plan.height),
                0.0,
                0.0,
            ],
        };
        let resolved = Resolved {
            matrix,
            view: Some(current.view.clone()),
            depth: None,
            alpha: stage.alpha,
            solid: None,
            from_fbo: true,
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: draw.layer.clone(),
        };
        self.taken.push(current);
        resolved
    }

    /// `drawFbo(res, W, H, alpha)`: premultiplied source-over, 1:1.
    fn composite(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        target: &Target,
        alpha: f64,
    ) {
        let (w, h) = (target.width(), target.height());
        let u = QuadUniform {
            m: [
                [w as f32, 0.0, 0.0, 0.0],
                [0.0, h as f32, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            res: [w as f32, h as f32, 0.0, 0.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [alpha as f32, 1.0, 0.0, 0.0],
        };
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm stage composite"),
            size: UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, bytes_of(&u));
        let quad = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm stage composite"),
            layout: &self.pipelines.quad_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<QuadUniform>() as u64),
                }),
            }],
        });
        let tex = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm stage composite source"),
            layout: &self.pipelines.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.pipelines.linear),
                },
            ],
        });
        let mut pass = begin(encoder, target, "pm stage composite", wgpu::LoadOp::Load);
        pass.set_pipeline(self.pipelines.draw(target.format()));
        pass.set_bind_group(0, &quad, &[0]);
        pass.set_bind_group(1, &tex, &[]);
        pass.draw(0..4, 0..1);
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_effect(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
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
        if self
            .effects
            .program(device, DefinitionKind::Effect, &step.definition, format)
            .is_err()
        {
            // The plan only stages ready programs; a build failure here is
            // engine-only (WebGL draws it): route the frame.
            self.fallback(layer, &step.definition);
            return None;
        }
        let span = self.timer.as_mut().and_then(|t| {
            t.begin(Span {
                kind: DefinitionKind::Effect,
                definition: DefinitionId::from(step.definition.as_str()),
                layer: LayerId::from(layer),
                instance: Some(EffectInstanceId::from(step.instance.as_str())),
            })
        });
        if let (Some(timer), Some((b, _))) = (&self.timer, span) {
            timer.mark(device, encoder, &input.view, b);
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
            let previous = outputs.last().unwrap_or(input);
            let units: [Option<&Target>; 3] = [
                Some(previous),
                step.keep_orig.then_some(input),
                backdrop.filter(|_| step.backdrop),
            ];
            let out = self.pool.take_working(device, w, h, self.precision);
            self.legacy_pass(
                encoder,
                DefinitionKind::Effect,
                &step.definition,
                &calls,
                &units,
                &out,
            );
            outputs.push(out);
        }
        if let (Some(timer), Some((_, e)), Some(last)) = (&self.timer, span, outputs.last()) {
            timer.mark(device, encoder, &last.view, e);
        }
        let mut result = outputs.pop().expect("at least one pass");
        self.taken.extend(outputs);
        if step.clamp {
            let clamped = self.pool.take_working(device, w, h, self.precision);
            self.clamp_pass(encoder, &result, &clamped);
            self.taken.push(std::mem::replace(&mut result, clamped));
        }
        Some(result)
    }

    /// `runTransition`: the output, or `None` when the program cannot run.
    pub(crate) fn run_transition(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        layer: &str,
        step: &TransitionStep,
        before: &Target,
        with_layer: &Target,
        time: f64,
    ) -> Option<Target> {
        let format = before.format();
        let (w, h) = (before.width(), before.height());
        let device = self.device;
        if self
            .effects
            .program(device, DefinitionKind::Transition, &step.definition, format)
            .is_err()
        {
            self.fallback(layer, &step.definition);
            return None;
        }
        let span = self.timer.as_mut().and_then(|t| {
            t.begin(Span {
                kind: DefinitionKind::Transition,
                definition: DefinitionId::from(step.definition.as_str()),
                layer: LayerId::from(layer),
                instance: None,
            })
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
        let out = self.pool.take_working(device, w, h, self.precision);
        if let (Some(timer), Some((b, _))) = (&self.timer, span) {
            timer.mark(device, encoder, &from.view, b);
        }
        self.legacy_pass(
            encoder,
            DefinitionKind::Transition,
            &step.definition,
            &calls,
            &[Some(from), Some(to)],
            &out,
        );
        if let (Some(timer), Some((_, e))) = (&self.timer, span) {
            timer.mark(device, encoder, &out.view, e);
        }
        Some(out)
    }

    /// One draw of a legacy program into `out` (cleared to transparent,
    /// blending off), with the compositor's uniform calls and texture
    /// units.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn legacy_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
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
        let linear = &self.pipelines.linear;
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
            encoder,
            out,
            "pm legacy glsl",
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &fragment, &[]);
        pass.set_bind_group(1, &vertex, &[]);
        pass.draw(0..4, 0..1);
    }

    /// `clamp: true`: the effect's output limited to 0..1.
    fn clamp_pass(&mut self, encoder: &mut wgpu::CommandEncoder, source: &Target, out: &Target) {
        let pipeline = self
            .effects
            .clamp_pipeline(self.device, out.format())
            .clone();
        let layout = pipeline.get_bind_group_layout(0);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm effect clamp"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source.view),
            }],
        });
        let mut pass = begin(
            encoder,
            out,
            "pm effect clamp",
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..4, 0..1);
    }
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

/// `copyFbo`: an exact copy (same size and format).
fn copy(encoder: &mut wgpu::CommandEncoder, source: &Target, target: &Target) {
    encoder.copy_texture_to_texture(
        source.texture.as_image_copy(),
        target.texture.as_image_copy(),
        wgpu::Extent3d {
            width: target.width(),
            height: target.height(),
            depth_or_array_layers: 1,
        },
    );
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
