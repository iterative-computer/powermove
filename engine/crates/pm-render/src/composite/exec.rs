//! Records a plan holding compositing nodes (`super::Node`).
//!
//! The target's render pass is split at each node. Plain draws between
//! nodes go into one pass over the accumulator, as without nodes. A layer
//! node renders its content into its own target, applies masks, effects,
//! and its matte, and then either joins the next pass as one more
//! premultiplied source-over draw (normal blending, today's `drawFbo`) or
//! is composited with its blend mode into a new accumulator
//! (`FRAG_COMPOSITE`). An adjustment node runs its effects on the
//! accumulator and composites the masked result over it.
//!
//! A pass runs over the plan's region grown by the largest effect padding
//! of its nodes (clamped to the frame); the region is cut from it at the
//! end. A node that fails is reported and leaves the accumulator as it was
//! (section 8, isolation).

use pm_gpu::Target;
use pm_protocol::color::{BlendMode, Precision, WorkingEncoding};

use super::effects::{Covered, EffectContext, Padding, PreparedEffects, PreparedTransition};
use super::gpu::{
    FLAG_BLEND_IN_E, FLAG_HALF, FLAG_LINEAR, FLAG_MASKS, FLAG_MATTE, FLAG_MATTE_INVERTED,
    FLAG_MATTE_LUMA, Op, OpUniform,
};
use super::region::Region;
use super::{AdjustmentNode, LayerNode, Masks, Matte, Node};
use crate::error::LayerFailure;
use crate::exec::{Exec, Resolved};
use crate::plan::{CompPlan, Content};

/// The padding a node's effects need around the region.
fn node_padding(node: &Node) -> Padding {
    match node {
        // A transition reads the whole frame (its program sees the full
        // level, as today's).
        Node::Layer(l) if l.transition.is_some() => Padding::Full,
        Node::Layer(l) => l.effects.as_ref().map_or(Padding::NONE, |e| e.padding),
        Node::Adjustment(a) => a.effects.padding,
    }
}

fn full_quad(full: (u32, u32)) -> [[f32; 4]; 3] {
    [
        [full.0 as f32, 0.0, 0.0, 0.0],
        [0.0, full.1 as f32, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]
}

impl Exec<'_> {
    fn op_uniform(&self, full: (u32, u32)) -> OpUniform {
        let mut flags = 0;
        if self.working == WorkingEncoding::LinearLight {
            flags |= FLAG_LINEAR;
        }
        if self.precision == Precision::Float16 {
            flags |= FLAG_HALF;
        }
        OpUniform {
            m: full_quad(full),
            res: [full.0 as f32, full.1 as f32, 0.0, 0.0],
            i: [0, flags, 0, 0],
            ..Default::default()
        }
    }

    /// A working target covering `rect`, returned to the pool after the
    /// frame is submitted.
    fn take_rect(&mut self, rect: Region) -> Target {
        let t = self
            .pool
            .take_working(self.device, rect.width, rect.height, self.precision);
        self.taken.push(t.clone());
        t
    }

    fn viewport(rect: Region, full: (u32, u32)) -> Option<[f32; 4]> {
        (rect != Region::full(full.0, full.1)).then(|| rect.viewport(full))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_op(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        op: Op,
        uniform: &OpUniform,
        a: &Target,
        b: Option<&Target>,
        out: &Target,
        rect: Region,
        full: (u32, u32),
    ) {
        self.pipelines.ops.run(
            self.device,
            self.queue,
            encoder,
            op,
            uniform,
            &a.view,
            b.map(|t| &t.view),
            &out.view,
            out.format(),
            Self::viewport(rect, full),
        );
    }

    /// Renders a plan with nodes into `target` (the region's size when
    /// `region` is set). Returns the top-level draws for the depth and id
    /// surface (layer nodes as their finished layer image).
    pub(crate) fn render_comp_nodes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        region: Option<Region>,
        target: &Target,
    ) -> Vec<Resolved> {
        let full = (plan.width, plan.height);
        let want = region.unwrap_or(Region::full(full.0, full.1));
        let padding = plan
            .draws
            .iter()
            .filter_map(|d| match &d.content {
                Content::Composite(n) => Some(node_padding(n)),
                _ => None,
            })
            .fold(Padding::NONE, Padding::max);
        let rect = want.grow(padding, full);
        let pass_region = (rect != Region::full(full.0, full.1)).then_some(rect);
        // The accumulator: the caller's target until a node writes a new one.
        let mut in_target = rect == want;
        let mut acc = if in_target {
            target.clone()
        } else {
            self.take_rect(rect)
        };
        let mut pending: Vec<Resolved> = Vec::new();
        let mut cleared = false;
        let mut ids: Vec<Resolved> = Vec::new();
        for draw in &plan.draws {
            let Content::Composite(node) = &draw.content else {
                let r = self.resolve_draws(encoder, plan, std::slice::from_ref(draw), pass_region);
                pending.extend(r);
                continue;
            };
            // Everything beneath the node goes into the accumulator first.
            self.draw_pass(encoder, plan, &pending, pass_region, &acc, !cleared);
            cleared = true;
            ids.append(&mut pending);
            match node.as_ref() {
                Node::Layer(n) => {
                    let Some((res, depth)) = self.layer_node(encoder, plan, n, rect, &acc) else {
                        continue;
                    };
                    // The finished layer covers the pass rectangle.
                    let image = Resolved {
                        matrix: [
                            f64::from(rect.width),
                            0.0,
                            0.0,
                            f64::from(rect.height),
                            f64::from(rect.x),
                            f64::from(rect.y),
                        ],
                        view: Some(res.view.clone()),
                        depth,
                        alpha: n.alpha,
                        solid: None,
                        from_fbo: true,
                        encoded: false,
                        uv: [0.0, 0.0, 1.0, 1.0],
                        layer: n.layer.clone(),
                    };
                    if let Some(transition) = &n.transition {
                        // `withLayer`: the stack with the layer, then the
                        // transition from the stack beneath to it.
                        let with_layer = if n.blend == BlendMode::Normal {
                            let w = self.take_rect(rect);
                            copy_same(encoder, &acc, &w);
                            self.draw_pass(
                                encoder,
                                plan,
                                std::slice::from_ref(&image),
                                pass_region,
                                &w,
                                false,
                            );
                            w
                        } else {
                            let out = self.take_rect(rect);
                            let u = self.composite_uniform(full, n.blend, n.alpha);
                            self.run_op(
                                encoder,
                                Op::Composite,
                                &u,
                                &res,
                                Some(&acc),
                                &out,
                                rect,
                                full,
                            );
                            out
                        };
                        acc = match self.run_transition(
                            encoder,
                            transition,
                            &acc,
                            &with_layer,
                            rect,
                            full,
                        ) {
                            Ok(Some(t)) => t,
                            Ok(None) => with_layer,
                            Err(message) => {
                                // The layer fails; the stack stays as it was.
                                self.fail_layer(plan, &n.layer, message);
                                continue;
                            }
                        };
                        in_target = false;
                        ids.push(image);
                    } else if n.blend == BlendMode::Normal {
                        // `drawFbo(res, W, H, compositeAlpha)`.
                        pending.push(image);
                    } else {
                        let out = self.take_rect(rect);
                        let u = self.composite_uniform(full, n.blend, n.alpha);
                        self.run_op(
                            encoder,
                            Op::Composite,
                            &u,
                            &res,
                            Some(&acc),
                            &out,
                            rect,
                            full,
                        );
                        acc = out;
                        in_target = false;
                        ids.push(image);
                    }
                }
                Node::Adjustment(n) => {
                    if let Some(out) = self.adjustment_node(encoder, plan, n, rect, &acc) {
                        acc = out;
                        in_target = false;
                    }
                }
            }
        }
        self.draw_pass(encoder, plan, &pending, pass_region, &acc, !cleared);
        ids.append(&mut pending);
        // The pass ran over `rect` in `acc`; the caller's target is `want`.
        if !in_target {
            let mut u = self.op_uniform(full);
            let (dx, dy) = want.texel_offset(rect, full.1);
            u.off = [dx, dy, 0, 0];
            self.run_op(encoder, Op::Copy, &u, &acc, None, target, want, full);
        }
        ids
    }

    /// `FRAG_COMPOSITE`'s uniforms. Every input covers the pass rectangle.
    fn composite_uniform(&self, full: (u32, u32), blend: BlendMode, alpha: f64) -> OpUniform {
        let mut u = self.op_uniform(full);
        u.f = [alpha as f32, 0.0, 0.0, 0.0];
        u.i[0] = pm_color::linear::blend_id(blend);
        if self.working == WorkingEncoding::LinearLight
            && pm_color::linear::blends_in_e(blend, self.working, self.blend_domain)
        {
            u.i[1] |= FLAG_BLEND_IN_E;
        }
        u
    }

    fn fail_layer(&mut self, plan: &CompPlan, layer: &str, message: String) {
        self.failures.push(LayerFailure {
            composition: plan.composition.clone(),
            layer: layer.to_owned(),
            message,
        });
    }

    /// Masks and the matte on `input` (one fused pass; each step's result
    /// rounded to the storage precision as today's separate passes store
    /// it).
    fn mask_and_matte(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        input: &Target,
        masks: Option<&Masks>,
        matte: Option<(&Matte, &Target)>,
        rect: Region,
        full: (u32, u32),
    ) -> Target {
        let mut u = self.op_uniform(full);
        if let Some(m) = masks {
            u.i[1] |= FLAG_MASKS;
            let (g, q) = m.masks.uniforms();
            for i in 0..8 {
                u.g[i] = [g[i * 4], g[i * 4 + 1], g[i * 4 + 2], g[i * 4 + 3]];
                u.q[i] = [q[i * 4], q[i * 4 + 1], q[i * 4 + 2], q[i * 4 + 3]];
            }
            u.i[2] = m.masks.masks.len() as i32;
            u.i[3] = i32::from(m.masks.has_add);
            for c in 0..3 {
                u.inv[c] = [
                    m.inverse[c * 3] as f32,
                    m.inverse[c * 3 + 1] as f32,
                    m.inverse[c * 3 + 2] as f32,
                    0.0,
                ];
            }
        }
        if let Some((m, _)) = matte {
            u.i[1] |= FLAG_MATTE;
            if m.luma {
                u.i[1] |= FLAG_MATTE_LUMA;
            }
            if m.inverted {
                u.i[1] |= FLAG_MATTE_INVERTED;
            }
        }
        let out = self.take_rect(rect);
        self.run_op(
            encoder,
            Op::Layer,
            &u,
            input,
            matte.map(|(_, t)| t),
            &out,
            rect,
            full,
        );
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn run_effects(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        effects: &PreparedEffects,
        input: &Target,
        backdrop: Option<&Target>,
        rect: Region,
        full: (u32, u32),
    ) -> Result<Option<Target>, String> {
        let mut taken = Vec::new();
        let result = {
            let mut cx = EffectContext {
                device: self.device,
                queue: self.queue,
                encoder,
                pool: self.pool,
                precision: self.precision,
                working: self.working,
                full,
                taken: &mut taken,
                native: Some(crate::effects::exec::Native {
                    effects: self.effects,
                    timer: self.timer.as_mut(),
                    linear: &self.pipelines.linear,
                    unsupported: &mut self.unsupported,
                }),
            };
            effects.passes.record(
                &mut cx,
                Covered {
                    target: input,
                    rect,
                },
                backdrop
                    .filter(|_| effects.backdrop)
                    .map(|t| Covered { target: t, rect }),
            )
        };
        // The chain's targets, the returned one included, are in `taken`.
        self.taken.extend(taken);
        result
    }

    /// `runTransition` over the pass rectangle (the whole frame).
    fn run_transition(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        transition: &PreparedTransition,
        before: &Target,
        with_layer: &Target,
        rect: Region,
        full: (u32, u32),
    ) -> Result<Option<Target>, String> {
        let mut taken = Vec::new();
        let result = {
            let mut cx = EffectContext {
                device: self.device,
                queue: self.queue,
                encoder,
                pool: self.pool,
                precision: self.precision,
                working: self.working,
                full,
                taken: &mut taken,
                native: Some(crate::effects::exec::Native {
                    effects: self.effects,
                    timer: self.timer.as_mut(),
                    linear: &self.pipelines.linear,
                    unsupported: &mut self.unsupported,
                }),
            };
            transition.passes.record(
                &mut cx,
                Covered {
                    target: before,
                    rect,
                },
                Covered {
                    target: with_layer,
                    rect,
                },
            )
        };
        self.taken.extend(taken);
        result
    }

    /// A layer node: content, masks, effects, matte. `None` when the layer
    /// failed (reported) and draws nothing.
    fn layer_node(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        n: &LayerNode,
        rect: Region,
        acc: &Target,
    ) -> Option<(Target, Option<wgpu::TextureView>)> {
        let full = (plan.width, plan.height);
        let pass_region = (rect != Region::full(full.0, full.1)).then_some(rect);
        let content = self.take_rect(rect);
        let resolved = self.resolve_draws(encoder, plan, &n.content, pass_region);
        // A 3D layer keeps its linear view depth in the depth and id
        // surface. Its depth covers the full output, as the finished layer
        // image does when the pass is not a region.
        let depth = match resolved.as_slice() {
            [only] if pass_region.is_none() => only.depth.clone(),
            _ => None,
        };
        let layer_plan = CompPlan {
            composition: plan.composition.clone(),
            group: plan.group,
            width: plan.width,
            height: plan.height,
            background: crate::plan::Background::Transparent,
            draws: Vec::new(),
        };
        self.draw_pass(encoder, &layer_plan, &resolved, pass_region, &content, true);
        let matte = n.matte.as_ref().map(|m| {
            let t = self.take_rect(rect);
            self.render_comp(encoder, &m.source, pass_region, &t);
            (m, t)
        });
        let mut res = content;
        match &n.effects {
            None => {
                if n.masks.is_some() || matte.is_some() {
                    res = self.mask_and_matte(
                        encoder,
                        &res,
                        n.masks.as_ref(),
                        matte.as_ref().map(|(m, t)| (*m, t)),
                        rect,
                        full,
                    );
                }
            }
            Some(effects) => {
                if n.masks.is_some() {
                    res = self.mask_and_matte(encoder, &res, n.masks.as_ref(), None, rect, full);
                }
                match self.run_effects(encoder, effects, &res, Some(acc), rect, full) {
                    Ok(Some(t)) => res = t,
                    Ok(None) => {}
                    Err(message) => {
                        self.fail_layer(plan, &n.layer, message);
                        return None;
                    }
                }
                if let Some((m, t)) = &matte {
                    res = self.mask_and_matte(encoder, &res, None, Some((m, t)), rect, full);
                }
            }
        }
        Some((res, depth))
    }

    /// An adjustment node: the accumulator through the effects, masked, and
    /// composited over the accumulator. `None` leaves it unchanged.
    fn adjustment_node(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        n: &AdjustmentNode,
        rect: Region,
        acc: &Target,
    ) -> Option<Target> {
        let full = (plan.width, plan.height);
        let adjusted = match self.run_effects(encoder, &n.effects, acc, Some(acc), rect, full) {
            Ok(Some(t)) => t,
            // A missing or failed effect program leaves the composition
            // unchanged.
            Ok(None) => return None,
            Err(message) => {
                self.fail_layer(plan, &n.layer, message);
                return None;
            }
        };
        let contribution = match &n.masks {
            Some(m) => self.mask_and_matte(encoder, &adjusted, Some(m), None, rect, full),
            None => adjusted,
        };
        let out = self.take_rect(rect);
        let u = self.composite_uniform(full, n.blend, n.alpha);
        self.run_op(
            encoder,
            Op::Composite,
            &u,
            &contribution,
            Some(acc),
            &out,
            rect,
            full,
        );
        Some(out)
    }
}

impl Exec<'_> {
    /// Copies the finished frame (GL row order) into the caller's top-down
    /// target, encoding linear light with E (section 9: "E⁻¹ on input,
    /// linear blending, E on output"). Frames leave the renderer as
    /// premultiplied E values in every working encoding.
    pub(crate) fn flip_color(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        target: &Target,
    ) {
        if self.working == WorkingEncoding::Standard {
            self.flip(encoder, source, target);
            return;
        }
        let size = (target.width(), target.height());
        let u = self.op_uniform(size);
        self.run_op(
            encoder,
            Op::EncodeFlip,
            &u,
            source,
            None,
            target,
            Region::full(size.0, size.1),
            size,
        );
    }
}

/// `copyFbo`: an exact copy between targets of one size and format.
fn copy_same(encoder: &mut wgpu::CommandEncoder, source: &Target, target: &Target) {
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
