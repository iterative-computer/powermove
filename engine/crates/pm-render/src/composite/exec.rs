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
//! A node that reads nothing of the stack beneath it (normal blending, no
//! transition, no effect reading the backdrop: [`draws_over_the_stack`])
//! does not split the pass: its own passes are recorded first and its image
//! joins the accumulator pass after it, so several such nodes and the plain
//! draws between them go into the accumulator in one pass, in plan order
//! ([`DEFERRED_IMAGES`], [`DEFERRED_BYTES`]). The accumulator receives the
//! same draws in the same order with the same blending; a pass split or
//! joined over the same attachment stores the same bits.
//!
//! Effects run over the plan's region grown by the largest effect padding.
//! When every node writes pointwise and no node samples the stack, a
//! viewport accumulator can cut back the unused attachment edges while
//! keeping the raster origin. Otherwise the accumulator shares the padded
//! effect domain. A node that fails is reported and leaves the accumulator
//! as it was (section 8, isolation).

use pm_gpu::Target;
use pm_protocol::color::{BlendMode, WorkingEncoding};

use super::effects::{Covered, EffectContext, Padding, PreparedEffects, PreparedTransition};
use super::gpu::{
    FLAG_BLEND_IN_E, FLAG_FLIP, FLAG_LINEAR, FLAG_MASKS, FLAG_MATTE, FLAG_MATTE_INVERTED,
    FLAG_MATTE_LUMA, FLAG_PATH_MASKS, FLAG_SRGB8, FLAG_TONE_MAP, Op, OpUniform,
};
use super::region::Region;
use super::{AdjustmentNode, LayerNode, Masks, Matte, Node};
use crate::error::LayerFailure;
use crate::exec::{Exec, Resolved};
use crate::plan::{CompPlan, Content};

use crate::effects::exec::EffectOutputBounds;

struct EffectOutput {
    target: Target,
    bounds: Option<EffectOutputBounds>,
}

/// The physical attachment rows covered by a two-texel reconstruction halo.
fn sample_bounds(
    bounds: EffectOutputBounds,
    covered: Region,
    rect: Region,
    full: (u32, u32),
) -> Option<[f32; 4]> {
    // Bound the coordinate error of the full-quad normalized mapping.
    if full.0.max(full.1) > 16_384 {
        return None;
    }
    let (x, y, w, h) = bounds.0;
    if w == 0 || h == 0 {
        return Some([0.0; 4]);
    }
    let (dx, dy) = covered.texel_offset(rect, full.1);
    Some([
        (x.saturating_sub(2) as i32 + dx).clamp(0, rect.width as i32) as f32,
        (y.saturating_sub(2) as i32 + dy).clamp(0, rect.height as i32) as f32,
        ((x + w + 2).min(covered.width) as i32 + dx).clamp(0, rect.width as i32) as f32,
        ((y + h + 2).min(covered.height) as i32 + dy).clamp(0, rect.height as i32) as f32,
    ])
}

/// The grid a padded pass aligns to (`Region::align`): an effect's
/// reduced-resolution passes down to 1/16 keep the whole frame's texels.
const EFFECT_GRID: u32 = 16;

/// Layer images one accumulator pass draws at most: past it the pass is
/// recorded before the next node ([`Exec::render_comp_nodes`]).
const DEFERRED_IMAGES: usize = 8;

/// The bytes of layer images and group passes held for accumulator passes
/// not yet recorded, at every level of the frame together (a nested
/// composition's nodes defer too): the scrub-borrow bound, 128 MiB. Three
/// 2880 × 1620 half-float images (37.3 MB each), eight at 1920 × 1080.
const DEFERRED_BYTES: u64 = 128 * 1024 * 1024;

thread_local! {
    /// [`DEFERRED_BYTES`]'s count: what every [`Pending`] being recorded on
    /// this thread holds (a frame records on one thread; nested levels
    /// recurse on it).
    static DEFERRED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// What the next accumulator pass of a node loop draws.
#[derive(Default)]
struct Pending {
    /// Plain draws and normal-blend layer images, in plan order.
    draws: Vec<Resolved>,
    /// The pool targets those draws read (scoped layer images and group
    /// passes), given back once the pass is recorded.
    targets: Vec<Target>,
    /// Their bytes, counted in [`DEFERRED`].
    bytes: u64,
    /// Layer images among `draws`.
    images: usize,
}

impl Pending {
    fn hold(&mut self, target: Target) {
        let bytes = target.bytes();
        self.bytes += bytes;
        DEFERRED.with(|d| d.set(d.get() + bytes));
        self.targets.push(target);
    }

    /// The targets, no longer counted (the pass reading them is recorded).
    fn take_targets(&mut self) -> Vec<Target> {
        let bytes = std::mem::take(&mut self.bytes);
        DEFERRED.with(|d| d.set(d.get().saturating_sub(bytes)));
        std::mem::take(&mut self.targets)
    }
}

impl Drop for Pending {
    /// A loop left early (a panic) stops counting what it held.
    fn drop(&mut self) {
        let bytes = std::mem::take(&mut self.bytes);
        DEFERRED.with(|d| d.set(d.get().saturating_sub(bytes)));
    }
}

/// Whether a layer node reads nothing of the stack beneath it: its image
/// goes over the stack with a plain premultiplied source-over draw
/// (`drawFbo`), it has no transition (which reads the stack and the stack
/// with the layer), and no effect of it reads the backdrop. Its content,
/// masks, matte (the matte source renders alone), and effects read its own
/// targets only. Its passes can then be recorded before the stack beneath
/// it is drawn into the accumulator, and its image drawn by the same pass.
fn draws_over_the_stack(n: &LayerNode) -> bool {
    n.blend == BlendMode::Normal
        && n.transition.is_none()
        && !n.effects.as_ref().is_some_and(|e| e.backdrop)
}

/// The padding a node's effects need around the region. In a final frame
/// an effect draws over the whole frame: a program's pixels follow its
/// target's size (interpolated coordinates round with it, `u_texel` and
/// `v_st` scale with it, and a program may pack reduced-resolution tiles
/// into its target), so only the whole frame's passes produce the whole
/// frame's bits. A viewer's crop keeps the declared spread.
fn node_padding(node: &Node, final_frame: bool) -> Padding {
    let of = |e: &PreparedEffects| {
        if final_frame {
            Padding::Full
        } else {
            e.padding
        }
    };
    match node {
        // A transition reads the whole frame (its program sees the full
        // level, as today's).
        Node::Layer(l) if l.transition.is_some() => Padding::Full,
        Node::Layer(l) => l.effects.as_ref().map_or(Padding::NONE, of),
        Node::Adjustment(a) => of(&a.effects),
    }
}

fn full_quad(full: (u32, u32)) -> [[f32; 4]; 3] {
    [
        [full.0 as f32, 0.0, 0.0, 0.0],
        [0.0, full.1 as f32, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]
}

/// No intermediate mask/effect target owns the layer image.
fn unmasked(n: &LayerNode) -> bool {
    n.masks.is_none() && n.matte.is_none() && n.effects.is_none() && n.transition.is_none()
}

/// A quad whose pixels have unit area coverage even with analytic solid
/// edges. Other quads retain their layer target so fractional and rotated
/// edge coverage is applied before the blend.
fn full_pixel_solid(matrix: [f64; 6]) -> bool {
    let [a, b, c, d, x, y] = matrix;
    b == 0.0
        && c == 0.0
        && [a, d, x, y].into_iter().all(f64::is_finite)
        && [x, x + a, y, y + d]
            .into_iter()
            .all(|edge| edge == edge.round())
}

/// Conservative device bounds, including the sampler's analytic edge support.
/// Effects and transitions retain the complete pass: their programs may generate
/// pixels outside the input or read the backdrop at arbitrary coordinates.
fn layer_rect(n: &LayerNode, rect: Region, full: (u32, u32)) -> Option<Region> {
    if n.effects.is_some() || n.transition.is_some() {
        return Some(rect);
    }
    let Ok(bounds) = placed_bounds(n) else {
        return Some(rect);
    };
    let b = bounds?;
    let x0 = (b[0].max(f64::from(rect.x)).max(0.0) as u32).min(full.0);
    let y0 = (b[1].max(f64::from(rect.y)).max(0.0) as u32).min(full.1);
    let x1 = (b[2].min(f64::from(rect.x + rect.width)).max(0.0) as u32).min(full.0);
    let y1 = (b[3].min(f64::from(rect.y + rect.height)).max(0.0) as u32).min(full.1);
    (x1 > x0 && y1 > y0).then(|| Region {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// The union of a layer's content placements in top-down device pixels
/// (with the sampler's analytic edge support): `Ok(None)` when nothing is
/// placed, `Err(())` when the layer holds a composite, whose bounds are
/// not known here. A group boundary counts as its whole quad.
fn placed_bounds(n: &LayerNode) -> Result<Option<[f64; 4]>, ()> {
    draws_bounds(&n.content, None)
}

/// [`placed_bounds`], looking through group boundaries drawn 1:1
/// ([`group_bounds`]): where a layer's own target can be non-zero.
fn content_bounds(n: &LayerNode) -> Result<Option<[f64; 4]>, ()> {
    draws_bounds(&n.content, Some(0))
}

/// How many group boundaries deep [`content_bounds`] looks; deeper nests
/// count as unknown.
const GROUP_DEPTH_MAX: u32 = 32;

/// The union of `draws`' bounds ([`draw_bounds`]). `groups`: the nesting
/// depth when group boundaries are looked through, `None` when each counts
/// as its whole quad.
fn draws_bounds(draws: &[crate::plan::Draw], groups: Option<u32>) -> Result<Option<[f64; 4]>, ()> {
    let mut bounds: Option<[f64; 4]> = None;
    for d in draws {
        let placed = match (&d.content, groups) {
            (
                Content::Texture {
                    source: crate::plan::Source::Comp(child),
                    uv,
                    alpha,
                },
                Some(depth),
            ) => group_bounds(d, child, *uv, *alpha, depth)?,
            _ => draw_bounds(d)?,
        };
        let Some(p) = placed else {
            continue;
        };
        bounds = Some(match bounds {
            None => p,
            Some(b) => [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[2]),
                b[3].max(p[3]),
            ],
        });
    }
    Ok(bounds)
}

/// One draw's placement bounds (with the sampler's analytic edge
/// support): `Ok(None)` when it places nothing, `Err(())` for a composite.
fn draw_bounds(d: &crate::plan::Draw) -> Result<Option<[f64; 4]>, ()> {
    let (plane, uv) = match &d.content {
        Content::Placed(p) => {
            let uv = match &p.content {
                Content::Texture { uv, .. } => *uv,
                _ => [0.0, 0.0, 1.0, 1.0],
            };
            (p.plane, uv)
        }
        Content::Composite(_) => return Err(()),
        Content::Texture { uv, .. } => (None, *uv),
        Content::Solid(_) => (None, [0.0, 0.0, 1.0, 1.0]),
    };
    let matrix = plane.unwrap_or_else(|| crate::sampling::gpu::affine3(&d.matrix));
    Ok(crate::sampling::gpu::placement(matrix, uv).map(|p| p.bounds))
}

/// Where a draw of a group boundary's target can be non-zero.
///
/// A group boundary (`CompPlan::group`) drawn through the unit quad scaled
/// to its own size (`[w, 0, 0, h, 0, 0]`), the whole UV window, and an
/// alpha in `0..=1` (an f32 `+0 × alpha` stays zero) shares its parent's
/// pixel space 1:1: its members render
/// into its target (over the parent's region, `Exec::resolve_content`) at
/// the parent's device coordinates, and the draw samples texel `p` at pixel
/// `p`. A transparent background leaves the target `+0` outside its
/// members' bounds, and a bilinear sample there reads texels within one
/// pixel of it: the members' bounds (recursively), out to whole pixels
/// and grown by one, within the quad's own bounds. `Err(())` when a member
/// is a composite (its bounds are not known here) or the nesting is deeper
/// than [`GROUP_DEPTH_MAX`]; any other draw of a composition is its quad.
fn group_bounds(
    d: &crate::plan::Draw,
    child: &crate::plan::CompPlan,
    uv: [f64; 4],
    alpha: f64,
    depth: u32,
) -> Result<Option<[f64; 4]>, ()> {
    let Some(quad) = draw_bounds(d)? else {
        return Ok(None);
    };
    let one_to_one = child.group
        && matches!(child.background, crate::plan::Background::Transparent)
        && uv == [0.0, 0.0, 1.0, 1.0]
        && (0.0..=1.0).contains(&alpha)
        && d.matrix
            == [
                f64::from(child.width),
                0.0,
                0.0,
                f64::from(child.height),
                0.0,
                0.0,
            ];
    if !one_to_one {
        return Ok(Some(quad));
    }
    if depth >= GROUP_DEPTH_MAX {
        return Err(());
    }
    let Some(inner) = draws_bounds(&child.draws, Some(depth + 1))? else {
        // No member placed: the target is its clear.
        return Ok(None);
    };
    if !inner.iter().all(|v| v.is_finite()) {
        return Ok(Some(quad));
    }
    let b = [
        (inner[0].floor() - 1.0).max(quad[0]),
        (inner[1].floor() - 1.0).max(quad[1]),
        (inner[2].ceil() + 1.0).min(quad[2]),
        (inner[3].ceil() + 1.0).min(quad[3]),
    ];
    // Members wholly outside the quad: nothing is drawn.
    Ok((b[2] > b[0] && b[3] > b[1]).then_some(b))
}

/// Where a layer's effect input (its content, then its masks, which only
/// multiply it) can be non-zero, in the texels of a target covering `rect`
/// (x, GL row from the bottom of `rect`, width, height): the whole pixels
/// its placements touch, one more on every side. Outside, the input is
/// exactly transparent. `None` when that is not known (a composite).
fn effect_input_bounds(n: &LayerNode, rect: Region) -> Option<(u32, u32, u32, u32)> {
    let Some(b) = content_bounds(n).ok()? else {
        // Nothing placed: transparent everywhere.
        return Some((0, 0, 0, 0));
    };
    if !b.iter().all(|v| v.is_finite()) {
        return None;
    }
    let clamp = |v: f64, lo: u32, hi: u32| v.clamp(f64::from(lo), f64::from(hi)) as u32;
    let (left, right) = (rect.x, rect.x + rect.width);
    let (top, bottom) = (rect.y, rect.y + rect.height);
    let x0 = clamp(b[0].floor() - 1.0, left, right);
    let x1 = clamp(b[2].ceil() + 1.0, left, right);
    let y0 = clamp(b[1].floor() - 1.0, top, bottom);
    let y1 = clamp(b[3].ceil() + 1.0, top, bottom);
    if x1 <= x0 || y1 <= y0 {
        return Some((0, 0, 0, 0));
    }
    // Targets are in GL row order: row 0 is the bottom of `rect`.
    Some((x0 - left, bottom - y1, x1 - x0, y1 - y0))
}

impl Exec<'_> {
    fn op_uniform(&self, full: (u32, u32)) -> OpUniform {
        let mut flags = 0;
        if self.working == WorkingEncoding::LinearLight {
            flags |= FLAG_LINEAR;
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

    /// A working target covering `rect` that the node loop gives back
    /// itself once nothing reads it ([`Self::release`]).
    fn take_scoped(&mut self, rect: Region) -> Target {
        self.pool
            .take_working(self.device, rect.width, rect.height, self.precision)
    }

    /// Gives back a [`Self::take_scoped`] target whose last read is
    /// recorded: later work in this frame may draw into it (commands run in
    /// the order they are recorded). Without [`Exec::recycle`] (the depth
    /// and id surface, which reads the frame's images afterwards, or a
    /// reference render), it stays until the frame is
    /// submitted. The pool keeps it for this frame's later takes whatever
    /// its free budget (`TargetPool::recycle`): every working target a
    /// later pass draws into is cleared first or wholly overwritten.
    pub(crate) fn release(&mut self, target: Target) {
        if self.recycle {
            self.pool.recycle(target);
        } else {
            self.taken.push(target);
        }
    }

    /// A scoped target when nothing reads this frame's images after it
    /// ([`Exec::recycle`]), else one given back after the frame is
    /// submitted; and whether it is scoped ([`Self::release`] it).
    fn take_node(&mut self, rect: Region) -> (Target, bool) {
        if self.recycle {
            (self.take_scoped(rect), true)
        } else {
            (self.take_rect(rect), false)
        }
    }

    /// Takes a chain's result back out of `taken`, where
    /// `EffectPasses::record` leaves it, so that the node gives it back
    /// itself once its last read is recorded: `true` when it was there
    /// (and is now the caller's to [`Self::release`]). Never without
    /// [`Exec::recycle`].
    fn adopt(&mut self, target: &Target) -> bool {
        if !self.recycle {
            return false;
        }
        match self.taken.iter().rposition(|t| t.texture == target.texture) {
            Some(i) => {
                // `remove`, not `swap_remove`: motion-blur samples give back
                // `taken[mark..]` by position.
                self.taken.remove(i);
                true
            }
            None => false,
        }
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
            &mut self.effects.uniforms,
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

    /// Decode before the sampler sees the source or builds its mips.
    pub(crate) fn decode_sampled_source(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
    ) -> Target {
        let full = (source.width(), source.height());
        let rect = Region::full(full.0, full.1);
        let target = self.take_rect(rect);
        let u = self.op_uniform(full);
        self.run_op(
            encoder,
            Op::DecodeSource,
            &u,
            source,
            None,
            &target,
            rect,
            full,
        );
        target
    }

    /// The 3D output boundary, before the layer's masks and effect chain.
    pub(crate) fn scene_output(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        exposure: f32,
        background: Option<[f32; 3]>,
        weighted: bool,
    ) -> Target {
        let full = (source.width(), source.height());
        let rect = Region::full(full.0, full.1);
        let target = self.take_rect(rect);
        let mut u = self.op_uniform(full);
        u.f[0] = exposure;
        u.f[1] = f32::from(weighted);
        if let Some(rgb) = background {
            u.g[0] = [rgb[0], rgb[1], rgb[2], 1.0];
        }
        self.run_op(
            encoder,
            Op::SceneOutput,
            &u,
            source,
            None,
            &target,
            rect,
            full,
        );
        target
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
                Content::Composite(n) => Some(node_padding(n, self.final_frame)),
                _ => None,
            })
            .fold(Padding::NONE, Padding::max);
        let mut effect_rect = want.grow(padding, full);
        if padding > Padding::NONE {
            effect_rect = effect_rect.align(EFFECT_GRID, full);
        }
        // Keep each effect on its original domain. The accumulator only
        // needs to cover the visible destination of the source-over draws.
        // Keep its GL pixel origin too: shifting the viewport can round the
        // rasterizer's interpolants differently around sharp edges.
        let rect = if effect_rect != want
            && !self.final_frame
            && self.recycle
            && !matches!(plan.background, crate::plan::Background::Gradient(_))
            && plan.draws.iter().all(|draw| match &draw.content {
                Content::Composite(node) => {
                    matches!(node.as_ref(), Node::Layer(n) if draws_over_the_stack(n))
                }
                _ => true,
            }) {
            Region {
                x: effect_rect.x,
                y: want.y,
                width: want.x + want.width - effect_rect.x,
                height: effect_rect.y + effect_rect.height - want.y,
            }
            .align(EFFECT_GRID, full)
        } else {
            effect_rect
        };
        let source_region = (effect_rect != Region::full(full.0, full.1)).then_some(effect_rect);
        let pass_region = (rect != Region::full(full.0, full.1)).then_some(rect);
        // Bound how many effect images can remain live before a flush.
        let image_bytes = u64::from(effect_rect.width)
            * u64::from(effect_rect.height)
            * u64::from(
                pm_gpu::working_format(self.precision)
                    .block_copy_size(None)
                    .unwrap_or(16),
            );
        // The accumulator: the caller's target until a node writes a new one.
        // `owned`: the loop took it (`take_scoped`) and gives it back once a
        // node replaced it, as today's compositor frees an accumulator after
        // its last use. Layer images go back after the flush that draws them
        // (`pending_targets`), except with the depth and id surface, which
        // reads them after the frame (`Exec::recycle`).
        let mut in_target = rect == want;
        let mut owned = !in_target;
        let mut acc = if in_target {
            target.clone()
        } else {
            self.take_scoped(rect)
        };
        let mut pending = Pending::default();
        let mut cleared = false;
        let mut ids: Vec<Resolved> = Vec::new();
        let mut remaining = plan.draws.as_slice();
        while !remaining.is_empty() {
            let run_len = remaining
                .iter()
                .take_while(|draw| !matches!(draw.content, Content::Composite(_)))
                .count();
            let (run, rest) = remaining.split_at(run_len);
            if !run.is_empty() {
                // Resolve adjacent draws together so their missing vector
                // scenes can share a coverage pass. Nested targets stay
                // live until the pending draws that read them are recorded.
                let (resolved, groups) =
                    self.resolve_draws_owned(encoder, plan, run, source_region);
                pending.draws.extend(resolved);
                for target in groups {
                    pending.hold(target);
                }
            }
            let Some((draw, rest)) = rest.split_first() else {
                break;
            };
            remaining = rest;
            let Content::Composite(node) = &draw.content else {
                unreachable!("the run ends at a compositor node");
            };
            // A frame drawn in slices that is giving way records no further
            // node: each would run its whole effect chain for nothing.
            if self.effects.slices.as_ref().is_some_and(|s| s.stopped()) {
                break;
            }
            // Everything beneath the node goes into the accumulator first,
            // unless nothing in the node reads it: its passes are then
            // recorded first, its image joins `pending` after the draws
            // beneath it, and one later pass draws them all in plan order.
            // Never with the depth and id surface or in a reference render
            // (`Exec::recycle`): one pass per node, as before. Held images
            // stay within `DEFERRED_IMAGES` and, at every level together,
            // `DEFERRED_BYTES`.
            let deferred = self.recycle
                && matches!(node.as_ref(), Node::Layer(n) if draws_over_the_stack(n))
                && pending.images < DEFERRED_IMAGES
                && DEFERRED.with(|d| d.get()) + image_bytes <= DEFERRED_BYTES;
            if !deferred {
                self.flush_pending(
                    encoder,
                    plan,
                    &mut pending,
                    pass_region,
                    &acc,
                    &mut cleared,
                    &mut ids,
                );
            }
            match node.as_ref() {
                Node::Layer(n) => {
                    let Some(covered) = layer_rect(n, effect_rect, full) else {
                        continue;
                    };
                    if !self.depth_id
                        && unmasked(n)
                        && n.blend != BlendMode::Normal
                        && let [draw] = n.content.as_slice()
                        && let Content::Solid(solid) = &draw.content
                        && solid.radius <= 0.0
                        && full_pixel_solid(draw.matrix)
                    {
                        self.blend_solid_region(
                            encoder,
                            draw.matrix,
                            solid.color,
                            covered,
                            &acc,
                            rect,
                            full,
                            n.blend,
                            n.alpha,
                        );
                        continue;
                    }
                    // `res_owned`: the image is ours to give back once the
                    // pass reading it is recorded (else it is in `taken`).
                    // A deferred node is never given the accumulator: it
                    // reads no backdrop (`draws_over_the_stack`), and the
                    // stack beneath it is not drawn yet.
                    let backdrop = (!deferred).then_some(&acc);
                    let Some((res, depth, res_owned, output_bounds)) =
                        self.layer_node(encoder, plan, n, covered, backdrop)
                    else {
                        continue;
                    };
                    // The finished layer covers the pass rectangle.
                    let image = Resolved {
                        matrix: [
                            f64::from(covered.width),
                            0.0,
                            0.0,
                            f64::from(covered.height),
                            f64::from(covered.x),
                            f64::from(covered.y),
                        ],
                        view: Some(res.view.clone()),
                        depth,
                        alpha: n.alpha,
                        solid: None,
                        from_fbo: true,
                        encoded: false,
                        scene: None,
                        uv: [0.0, 0.0, 1.0, 1.0],
                        layer: n.layer.clone(),
                        plane: None,
                        level2: None,
                        pyramid: None,
                        transient_source: self.pool.playback_retention(),
                        sample_bounds: output_bounds
                            .filter(|_| {
                                n.blend == BlendMode::Normal
                                    && n.transition.is_none()
                                    && !self.depth_id
                                    && res.format() == wgpu::TextureFormat::Rgba16Float
                                    && (n.alpha as f32).is_finite()
                                    && (n.alpha as f32).is_sign_positive()
                                    && self.pipelines.effect_output_bounds
                            })
                            .and_then(|bounds| sample_bounds(bounds, covered, rect, full)),
                    };
                    if let Some(transition) = &n.transition {
                        // `withLayer`: the stack with the layer, then the
                        // transition from the stack beneath to it.
                        let with_layer = if n.blend == BlendMode::Normal {
                            let w = self.take_scoped(rect);
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
                            let out = self.take_scoped(rect);
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
                        // The pass reading the layer image is recorded; the
                        // transition reads the two stacks only.
                        if res_owned {
                            self.release(res);
                        }
                        // A transition's result is its own target (in
                        // `taken`), never one of its inputs.
                        let (next, next_owned) = match self.run_transition(
                            encoder,
                            transition,
                            &acc,
                            &with_layer,
                            rect,
                            full,
                            plan.view,
                        ) {
                            Ok(Some(t)) => {
                                self.release(with_layer);
                                (t, false)
                            }
                            Ok(None) => (with_layer, true),
                            Err(message) => {
                                // The layer fails; the stack stays as it was.
                                self.release(with_layer);
                                self.fail_layer(plan, &n.layer, message);
                                continue;
                            }
                        };
                        let old = std::mem::replace(&mut acc, next);
                        if std::mem::replace(&mut owned, next_owned) {
                            self.release(old);
                        }
                        in_target = false;
                        ids.push(image);
                    } else if n.blend == BlendMode::Normal {
                        // `drawFbo(res, W, H, compositeAlpha)`: given back
                        // after the flush that draws it, masked or not.
                        pending.draws.push(image);
                        pending.images += 1;
                        if res_owned {
                            pending.hold(res);
                        }
                    } else {
                        self.blend_region(
                            encoder, &res, covered, &acc, rect, full, n.blend, n.alpha,
                        );
                        ids.push(image);
                        if res_owned {
                            self.release(res);
                        }
                    }
                }
                Node::Adjustment(n) => {
                    if let Some((out, out_owned)) =
                        self.adjustment_node(encoder, plan, n, rect, &acc)
                    {
                        // The old accumulator's last reads (the chain and
                        // the composite) are recorded.
                        let old = std::mem::replace(&mut acc, out);
                        if std::mem::replace(&mut owned, out_owned) {
                            self.release(old);
                        }
                        in_target = false;
                    }
                }
            }
        }
        self.flush_pending(
            encoder,
            plan,
            &mut pending,
            pass_region,
            &acc,
            &mut cleared,
            &mut ids,
        );
        // The pass ran over `rect` in `acc`; the caller's target is `want`.
        if !in_target {
            let mut u = self.op_uniform(full);
            let (dx, dy) = want.texel_offset(rect, full.1);
            u.off = [dx, dy, 0, 0];
            self.run_op(encoder, Op::Copy, &u, &acc, None, target, want, full);
        }
        if owned {
            self.release(acc);
        }
        ids
    }

    /// Draws `pending` into the accumulator in one pass (cleared to the
    /// plan's background first when nothing was drawn into it yet), then
    /// gives back the layer images and group passes it read.
    #[allow(clippy::too_many_arguments)]
    fn flush_pending(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        pending: &mut Pending,
        pass_region: Option<Region>,
        acc: &Target,
        cleared: &mut bool,
        ids: &mut Vec<Resolved>,
    ) {
        self.draw_pass(encoder, plan, &pending.draws, pass_region, acc, !*cleared);
        *cleared = true;
        ids.append(&mut pending.draws);
        pending.images = 0;
        for t in pending.take_targets() {
            self.release(t);
        }
    }

    /// Snapshot only the destination pixels read by the blend, then write
    /// them back in place. Pixels outside the scissor retain their exact bits.
    #[allow(clippy::too_many_arguments)]
    fn blend_region(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        covered: Region,
        acc: &Target,
        rect: Region,
        full: (u32, u32),
        blend: BlendMode,
        alpha: f64,
    ) {
        let (destination, x, y) =
            self.snapshot_blend_destination(encoder, covered, acc, rect, full);
        let mut u = self.composite_uniform(full, blend, alpha);
        u.off = [-x, -y, -x, -y];
        self.pipelines.ops.run_with_coverage(
            self.device,
            &mut self.effects.uniforms,
            encoder,
            Op::Composite,
            &u,
            &source.view,
            Some(&destination.view),
            None,
            &acc.view,
            acc.format(),
            Self::viewport(rect, full),
            Some([x as u32, y as u32, covered.width, covered.height]),
        );
        self.release(destination);
    }

    #[allow(clippy::too_many_arguments)]
    fn blend_solid_region(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        matrix: [f64; 6],
        color: [f64; 4],
        covered: Region,
        acc: &Target,
        rect: Region,
        full: (u32, u32),
        blend: BlendMode,
        alpha: f64,
    ) {
        let (destination, x, y) =
            self.snapshot_blend_destination(encoder, covered, acc, rect, full);
        let mut u = self.composite_uniform(full, blend, alpha);
        u.m = [
            [matrix[0] as f32, matrix[1] as f32, 0.0, 0.0],
            [matrix[2] as f32, matrix[3] as f32, 0.0, 0.0],
            [matrix[4] as f32, matrix[5] as f32, 1.0, 0.0],
        ];
        u.g[0] = super::color::premultiplied_to_working(color, self.working).map(|c| c as f32);
        u.off = [0, 0, -x, -y];
        self.pipelines.ops.run_with_coverage(
            self.device,
            &mut self.effects.uniforms,
            encoder,
            Op::SolidComposite,
            &u,
            &destination.view,
            Some(&destination.view),
            None,
            &acc.view,
            acc.format(),
            Self::viewport(rect, full),
            Some([x as u32, y as u32, covered.width, covered.height]),
        );
        self.release(destination);
    }

    fn snapshot_blend_destination(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        covered: Region,
        acc: &Target,
        rect: Region,
        full: (u32, u32),
    ) -> (Target, i32, i32) {
        let destination = self.take_scoped(covered);
        let (x, y) = covered.texel_offset(rect, full.1);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                origin: wgpu::Origin3d {
                    x: x as u32,
                    y: y as u32,
                    z: 0,
                },
                ..acc.texture.as_image_copy()
            },
            destination.texture.as_image_copy(),
            wgpu::Extent3d {
                width: covered.width,
                height: covered.height,
                depth_or_array_layers: 1,
            },
        );
        (destination, x, y)
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

    /// Masks and the matte in f32, with one working-format store. `scoped`:
    /// the output is the caller's to [`Self::release`] (else in `taken`).
    #[allow(clippy::too_many_arguments)]
    fn mask_and_matte(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        input: &Target,
        masks: Option<&Masks>,
        matte: Option<(&Matte, &Target)>,
        rect: Region,
        full: (u32, u32),
        scoped: bool,
    ) -> Target {
        let mut u = self.op_uniform(full);
        let mut coverage = None;
        if let Some(m) = masks {
            if let Some(path) = &m.path {
                u.i[1] |= FLAG_PATH_MASKS;
                let texture = self
                    .pipelines
                    .ops
                    .path_masks
                    .coverage(self.device, encoder, path);
                coverage = Some(texture.create_view(&Default::default()));
            } else {
                u.i[1] |= FLAG_MASKS;
                let (g, q) = m.masks.uniforms();
                for i in 0..8 {
                    u.g[i] = [g[i * 4], g[i * 4 + 1], g[i * 4 + 2], g[i * 4 + 3]];
                    u.q[i] = [q[i * 4], q[i * 4 + 1], q[i * 4 + 2], q[i * 4 + 3]];
                }
                u.i[2] = m.masks.masks.len() as i32;
                u.i[3] = i32::from(m.masks.has_add);
            }
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
        let out = if scoped {
            self.take_scoped(rect)
        } else {
            self.take_rect(rect)
        };
        self.pipelines.ops.run_with_coverage(
            self.device,
            &mut self.effects.uniforms,
            encoder,
            Op::Layer,
            &u,
            &input.view,
            matte.map(|(_, t)| &t.view),
            coverage.as_ref(),
            &out.view,
            out.format(),
            Self::viewport(rect, full),
            None,
        );
        out
    }

    /// `content`: where `input` can be non-zero ([`effect_input_bounds`]).
    #[allow(clippy::too_many_arguments)]
    fn run_effects(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        effects: &PreparedEffects,
        input: &Target,
        backdrop: Option<&Target>,
        rect: Region,
        full: (u32, u32),
        view: Option<crate::plan::OutputView>,
        content: Option<(u32, u32, u32, u32)>,
    ) -> Result<Option<EffectOutput>, String> {
        let mut ready = vec![];
        for producer in effects.passes.input_plans() {
            if ready
                .iter()
                .any(|r: &crate::effects::resources::Ready| r.request == producer.request)
            {
                continue;
            }
            let target = self.take_rect(Region::full(producer.plan.width, producer.plan.height));
            self.render_comp_nodes(encoder, &producer.plan, None, &target);
            ready.push(crate::effects::resources::Ready {
                source_extent: producer.source_extent,
                request: producer.request.clone(),
                target,
                owner: producer.owner,
                origin: producer.plan.view.map_or([0., 0.], |v| v.origin),
            });
        }
        for asset in effects.passes.vector_assets() {
            let target = self.take_rect(Region::full(asset.width, asset.height));
            let plan = crate::plan::CompPlan {
                composition: "brush".into(),
                group: true,
                width: asset.width,
                height: asset.height,
                background: crate::plan::Background::Transparent,
                view: None,
                draws: vec![crate::plan::Draw {
                    layer: "brush".into(),
                    matrix: [asset.width as f64, 0., 0., asset.height as f64, 0., 0.],
                    content: crate::plan::Content::Texture {
                        source: crate::plan::Source::Vector(Box::new(
                            asset.vector.as_ref().unwrap().clone(),
                        )),
                        uv: [0., 0., 1., 1.],
                        alpha: 1.,
                    },
                }],
            };
            self.render_comp_nodes(encoder, &plan, None, &target);
            ready.push(crate::effects::resources::Ready {
                source_extent: None,
                request: asset.request(),
                target,
                owner: crate::effects::plan::Owner {
                    valid: true,
                    uv: [1., 0., 0., 0., 1., 0., 0., 0., 1.],
                    size: [asset.width as f64, asset.height as f64],
                    bounds: [0., 0., asset.width as f64, asset.height as f64],
                    pixel_scale: [1., 1.],
                },
                origin: [0., 0.],
            });
        }
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
                view,
                taken: &mut taken,
                recycle: self.recycle,
                native: Some(crate::effects::exec::Native {
                    inputs: &ready,
                    effects: self.effects,
                    timer: self.timer.as_mut(),
                    linear: &self.pipelines.linear,
                    unsupported: &mut self.unsupported,
                    content,
                    output_bounds: None,
                }),
            };
            let result = effects.passes.record(
                &mut cx,
                Covered {
                    target: input,
                    rect,
                },
                backdrop
                    .filter(|_| effects.backdrop)
                    .map(|t| Covered { target: t, rect }),
            );
            let bounds = cx.native.and_then(|native| native.output_bounds);
            result.map(|target| target.map(|target| EffectOutput { target, bounds }))
        };
        // The chain's targets, the returned one included, are in `taken`.
        self.taken.extend(taken);
        result
    }

    /// `runTransition` over the pass rectangle (the whole frame).
    #[allow(clippy::too_many_arguments)]
    fn run_transition(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        transition: &PreparedTransition,
        before: &Target,
        with_layer: &Target,
        rect: Region,
        full: (u32, u32),
        view: Option<crate::plan::OutputView>,
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
                view,
                taken: &mut taken,
                recycle: self.recycle,
                native: Some(crate::effects::exec::Native {
                    inputs: &[],
                    effects: self.effects,
                    timer: self.timer.as_mut(),
                    linear: &self.pipelines.linear,
                    unsupported: &mut self.unsupported,
                    content: None,
                    output_bounds: None,
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

    /// A layer node: content, masks, effects, matte. `backdrop`: the stack
    /// beneath it, for effects reading it (`None` for a node whose passes
    /// are recorded before that stack is drawn, which reads none). `None`
    /// when the layer failed (reported) and draws nothing. The finished
    /// layer image, its depth, and whether the image is the caller's to
    /// [`Self::release`] once its last read is recorded (else it is in
    /// `taken`). Every target
    /// before it goes back as soon as the pass reading it is recorded
    /// ([`Exec::recycle`]).
    fn layer_node(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        n: &LayerNode,
        rect: Region,
        backdrop: Option<&Target>,
    ) -> Option<(
        Target,
        Option<wgpu::TextureView>,
        bool,
        Option<EffectOutputBounds>,
    )> {
        let full = (plan.width, plan.height);
        let pass_region = (rect != Region::full(full.0, full.1)).then_some(rect);
        // `owned`: `res` (here the content) is ours to release.
        let (content, mut owned) = if unmasked(n) {
            (self.take_scoped(rect), true)
        } else {
            self.take_node(rect)
        };
        // Scoped masks, mattes, and effect results (else in `taken`).
        let scoped = self.recycle;
        let depth = if self.pipelines.direct_group_target
            && !self.depth_id
            && pass_region.is_none()
            && plan.view.is_none()
            && n.blend == BlendMode::Normal
            && n.alpha == 1.0
            && let [draw] = n.content.as_slice()
            && let crate::plan::Content::Texture {
                source: crate::plan::Source::Comp(child),
                uv,
                alpha,
            } = &draw.content
            && child.group
            && (child.width, child.height) == full
            && child.view == plan.view
            && *uv == [0.0, 0.0, 1.0, 1.0]
            && *alpha == 1.0
            && draw.matrix == [f64::from(full.0), 0.0, 0.0, f64::from(full.1), 0.0, 0.0]
        {
            self.pipelines
                .direct_group_draws
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.render_comp(encoder, child, None, &content);
            None
        } else {
            let (resolved, groups) =
                self.resolve_draws_owned(encoder, plan, &n.content, pass_region);
            // A 3D layer keeps its linear view depth over the whole output.
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
                view: plan.view,
            };
            self.draw_pass(encoder, &layer_plan, &resolved, pass_region, &content, true);
            for target in groups {
                self.release(target);
            }
            depth
        };
        let matte = n.matte.as_ref().map(|m| {
            let (t, _) = self.take_node(rect);
            self.render_comp(encoder, &m.source, pass_region, &t);
            (m, t)
        });
        let mut res = content;
        let mut output_bounds = None;
        match &n.effects {
            None => {
                if n.masks.is_some() || matte.is_some() {
                    let masked = self.mask_and_matte(
                        encoder,
                        &res,
                        n.masks.as_ref(),
                        matte.as_ref().map(|(m, t)| (*m, t)),
                        rect,
                        full,
                        scoped,
                    );
                    self.replace_res(&mut res, &mut owned, masked, scoped);
                }
            }
            Some(effects) => {
                if n.masks.is_some() {
                    let masked = self.mask_and_matte(
                        encoder,
                        &res,
                        n.masks.as_ref(),
                        None,
                        rect,
                        full,
                        scoped,
                    );
                    self.replace_res(&mut res, &mut owned, masked, scoped);
                }
                let content = effect_input_bounds(n, rect);
                match self.run_effects(
                    encoder, effects, &res, backdrop, rect, full, plan.view, content,
                ) {
                    Ok(Some(EffectOutput { target: t, bounds })) => {
                        output_bounds = bounds;
                        // The chain read its input (and `u_orig`, the same
                        // target) in passes recorded by now.
                        let adopted = self.adopt(&t);
                        self.replace_res(&mut res, &mut owned, t, adopted);
                    }
                    Ok(None) => {}
                    Err(message) => {
                        self.fail_layer(plan, &n.layer, message);
                        if owned {
                            self.release(res);
                        }
                        if let Some((_, t)) = matte
                            && scoped
                        {
                            self.release(t);
                        }
                        return None;
                    }
                }
                if let Some((m, t)) = &matte {
                    output_bounds = None;
                    let matted =
                        self.mask_and_matte(encoder, &res, None, Some((m, t)), rect, full, scoped);
                    self.replace_res(&mut res, &mut owned, matted, scoped);
                }
            }
        }
        // The matte's last read (`mask_and_matte`) is recorded.
        if let Some((_, t)) = matte
            && scoped
        {
            self.release(t);
        }
        Some((res, depth, owned, output_bounds))
    }

    /// `res` becomes `next` (whose ownership is `next_owned`); the old one,
    /// whose last read is recorded, goes back when it was the node's.
    fn replace_res(&mut self, res: &mut Target, owned: &mut bool, next: Target, next_owned: bool) {
        let old = std::mem::replace(res, next);
        let old_owned = std::mem::replace(owned, next_owned);
        // A result is a new target, never its input; were it the same
        // texture, it would stay, still the node's.
        if old.texture == res.texture {
            *owned = old_owned || next_owned;
        } else if old_owned {
            self.release(old);
        }
    }

    /// An adjustment node: the accumulator through the effects, masked, and
    /// composited over the accumulator. `None` leaves it unchanged. The new
    /// accumulator, and whether it is the caller's to [`Self::release`]
    /// once replaced (else in `taken`). The effect result and its masked
    /// copy go back once the composite reading them is recorded; the old
    /// accumulator is the caller's.
    fn adjustment_node(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        n: &AdjustmentNode,
        rect: Region,
        acc: &Target,
    ) -> Option<(Target, bool)> {
        let full = (plan.width, plan.height);
        let adjusted = match self.run_effects(
            encoder,
            &n.effects,
            acc,
            Some(acc),
            rect,
            full,
            plan.view,
            None,
        ) {
            Ok(Some(output)) => output.target,
            // A missing or failed effect program leaves the composition
            // unchanged.
            Ok(None) => return None,
            Err(message) => {
                self.fail_layer(plan, &n.layer, message);
                return None;
            }
        };
        let scoped = self.recycle;
        // The chain read the accumulator (input, `u_orig`, and backdrop) in
        // passes recorded by now; the accumulator stays the caller's.
        let mut owned = self.adopt(&adjusted);
        let mut contribution = adjusted;
        if let Some(m) = &n.masks {
            let masked =
                self.mask_and_matte(encoder, &contribution, Some(m), None, rect, full, scoped);
            self.replace_res(&mut contribution, &mut owned, masked, scoped);
        }
        let (out, out_owned) = self.take_node(rect);
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
        if owned {
            self.release(contribution);
        }
        Some((out, out_owned))
    }
}

impl Exec<'_> {
    /// Copies the finished frame (GL row order) into the caller's top-down
    /// target, encoding linear light with E (section 9: "E⁻¹ on input,
    /// linear blending, E on output"). Frames leave the renderer as
    /// premultiplied E values in every working encoding.
    pub(crate) fn finish_color(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        target: &Target,
        flip: bool,
        tone: Option<crate::request::OutputTone>,
    ) {
        if self.working == WorkingEncoding::Standard
            && target.format() != wgpu::TextureFormat::Rgba8Unorm
            && flip
            && tone.is_none()
        {
            self.flip(encoder, source, target);
            return;
        }
        let size = (target.width(), target.height());
        let mut u = self.op_uniform(size);
        if flip {
            u.i[1] |= FLAG_FLIP;
        }
        if target.format() == wgpu::TextureFormat::Rgba8Unorm {
            u.i[1] |= FLAG_SRGB8;
        }
        if let Some(tone) = tone {
            u.i[1] |= FLAG_TONE_MAP;
            u.f[0] = tone.source_peak;
            u.f[1] = tone.target_peak;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Background, Draw, Solid, Source};

    const W: u32 = 256;
    const H: u32 = 144;
    const FULL: [f64; 6] = [W as f64, 0.0, 0.0, H as f64, 0.0, 0.0];

    #[test]
    fn effect_sample_bounds_translate_gl_rows_and_preserve_empty_outputs() {
        let covered = Region {
            x: 30,
            y: 50,
            width: 40,
            height: 20,
        };
        let acc = Region {
            x: 20,
            y: 40,
            width: 100,
            height: 80,
        };
        assert_eq!(
            sample_bounds(EffectOutputBounds((3, 4, 10, 8)), covered, acc, (200, 200)),
            Some([11.0, 52.0, 25.0, 64.0])
        );
        assert_eq!(
            sample_bounds(EffectOutputBounds((0, 0, 0, 0)), covered, acc, (200, 200)),
            Some([0.0; 4])
        );
    }

    fn solid(w: f64, h: f64, x: f64, y: f64) -> Draw {
        Draw {
            layer: "s".into(),
            matrix: [w, 0.0, 0.0, h, x, y],
            content: Content::Solid(Solid {
                color: [1.0, 0.5, 0.25, 1.0],
                size: [w, h],
                radius: 0.0,
            }),
        }
    }

    fn plan(group: bool, draws: Vec<Draw>) -> CompPlan {
        CompPlan {
            composition: "c".into(),
            group,
            width: W,
            height: H,
            background: Background::Transparent,
            draws,
            view: None,
        }
    }

    /// A draw of `child` through the full quad (a boundary group's draw).
    fn comp(child: CompPlan, alpha: f64) -> Draw {
        Draw {
            layer: "g".into(),
            matrix: FULL,
            content: Content::Texture {
                source: Source::Comp(Box::new(child)),
                uv: [0.0, 0.0, 1.0, 1.0],
                alpha,
            },
        }
    }

    fn node(content: Vec<Draw>) -> LayerNode {
        LayerNode {
            layer: "n".into(),
            content,
            masks: None,
            effects: None,
            matte: None,
            blend: BlendMode::Normal,
            alpha: 1.0,
            transition: None,
        }
    }

    fn full() -> Region {
        Region::full(W, H)
    }

    /// The full quad's own placement (one pixel of edge support).
    const QUAD: [f64; 4] = [-1.0, -1.0, W as f64 + 1.0, H as f64 + 1.0];

    /// A group boundary's members, recursively: their union, out to whole
    /// pixels and one more per level, within the quad. Without looking
    /// through groups (`layer_rect`'s bounds) it is the whole quad.
    #[test]
    fn group_bounds_are_the_union_of_their_members() {
        // [29, 39, 51, 51] and [99, 59, 111, 71] (each grown by one).
        let group = comp(
            plan(
                true,
                vec![solid(20., 10., 30., 40.), solid(10., 10., 100., 60.)],
            ),
            1.0,
        );
        let n = node(vec![group]);
        assert_eq!(placed_bounds(&n), Ok(Some(QUAD)));
        assert_eq!(content_bounds(&n), Ok(Some([28., 38., 112., 72.])));
        // Texels: [27, 113) × [37, 73), rows from the bottom.
        assert_eq!(
            effect_input_bounds(&n, full()),
            Some((27, H - 73, 113 - 27, 73 - 37))
        );
        // A region: relative to its corner, clamped to it.
        let rect = Region {
            x: 40,
            y: 20,
            width: 100,
            height: 40,
        };
        assert_eq!(effect_input_bounds(&n, rect), Some((0, 0, 73, 23)));
        // Any finite alpha (a boundary at half opacity), and fractional
        // placements round outwards.
        let half = comp(plan(true, vec![solid(10.5, 10.25, 30.25, 40.5)]), 0.5);
        // Solid: [29, 39, 42, 52]; the group: one more.
        assert_eq!(
            content_bounds(&node(vec![half])),
            Ok(Some([28., 38., 43., 53.]))
        );
    }

    /// Nested boundaries add a pixel per level; a leaf beside the group
    /// joins the union.
    #[test]
    fn nested_groups_are_looked_through() {
        let inner = comp(plan(true, vec![solid(10., 10., 100., 60.)]), 0.5);
        let outer = comp(plan(true, vec![solid(20., 10., 30., 40.), inner]), 1.0);
        // Inner: [99, 59, 111, 71] → [98, 58, 112, 72]; with the solid
        // [29, 39, 51, 51]: [29, 39, 112, 72] → [28, 38, 113, 73].
        assert_eq!(
            content_bounds(&node(vec![outer.clone()])),
            Ok(Some([28., 38., 113., 73.]))
        );
        let leaf = solid(4., 4., 200., 10.);
        // Leaf [199, 9, 205, 15].
        assert_eq!(
            content_bounds(&node(vec![outer, leaf])),
            Ok(Some([28., 9., 205., 73.]))
        );
        // Too deep: unknown.
        let mut deep = comp(plan(true, vec![solid(4., 4., 10., 10.)]), 1.0);
        for _ in 0..GROUP_DEPTH_MAX {
            deep = comp(plan(true, vec![deep]), 1.0);
        }
        assert_eq!(content_bounds(&node(vec![deep.clone()])), Err(()));
        assert_eq!(effect_input_bounds(&node(vec![deep]), full()), None);
        let mut shallow = comp(plan(true, vec![solid(4., 4., 10., 10.)]), 1.0);
        for _ in 1..GROUP_DEPTH_MAX {
            shallow = comp(plan(true, vec![shallow]), 1.0);
        }
        assert!(content_bounds(&node(vec![shallow])).unwrap().is_some());
    }

    /// Only a group boundary drawn 1:1 with a transparent clear is looked
    /// through; any other draw of a composition counts as its quad, and a
    /// member through its own target is unknown.
    #[test]
    fn other_compositions_count_as_their_quad() {
        let members = || vec![solid(20., 10., 30., 40.)];
        let bounds = |d: Draw| content_bounds(&node(vec![d]));
        // A precomp (sampled through its layer's transform).
        assert_eq!(bounds(comp(plan(false, members()), 1.0)), Ok(Some(QUAD)));
        // A filled background.
        let mut filled = plan(true, members());
        filled.background = Background::Solid([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(bounds(comp(filled, 1.0)), Ok(Some(QUAD)));
        // Not 1:1: moved, scaled, or through a UV window.
        for matrix in [
            [W as f64, 0.0, 0.0, H as f64, 1.0, 0.0],
            [W as f64 / 2.0, 0.0, 0.0, H as f64 / 2.0, 0.0, 0.0],
        ] {
            let mut d = comp(plan(true, members()), 1.0);
            d.matrix = matrix;
            let quad = crate::sampling::gpu::placement(
                crate::sampling::gpu::affine3(&matrix),
                [0.0, 0.0, 1.0, 1.0],
            )
            .unwrap()
            .bounds;
            assert_eq!(bounds(d), Ok(Some(quad)), "{matrix:?}");
        }
        let mut window = comp(plan(true, members()), 1.0);
        if let Content::Texture { uv, .. } = &mut window.content {
            *uv = [0.0, 0.0, 0.5, 1.0];
        }
        assert!(bounds(window).unwrap().unwrap()[2] > 100.0);
        // An alpha outside `0..=1` (`NaN × 0`, and `1e300` as f32 is `inf`).
        for alpha in [f64::NAN, f64::INFINITY, 1e300, 2.0, -1.0] {
            assert_eq!(bounds(comp(plan(true, members()), alpha)), Ok(Some(QUAD)));
        }
        // A member through its own target.
        let member = Draw {
            layer: "m".into(),
            matrix: FULL,
            content: Content::Composite(Box::new(Node::Layer(node(members())))),
        };
        let group = comp(plan(true, vec![solid(4., 4., 0., 0.), member]), 1.0);
        assert_eq!(bounds(group.clone()), Err(()));
        assert_eq!(effect_input_bounds(&node(vec![group]), full()), None);
    }

    /// Members partly or wholly off the frame, and groups that place
    /// nothing.
    #[test]
    fn group_bounds_clamp_to_the_frame() {
        let bounds =
            |members: Vec<Draw>| content_bounds(&node(vec![comp(plan(true, members), 1.0)]));
        // [-11, 19, 21, 31] → [-12, 18, 22, 32], within the quad.
        let partly = vec![solid(30., 10., -10., 20.)];
        assert_eq!(bounds(partly.clone()), Ok(Some([-1., 18., 22., 32.])));
        assert_eq!(
            effect_input_bounds(&node(vec![comp(plan(true, partly), 1.0)]), full()),
            Some((0, H - 33, 23, 33 - 17))
        );
        // Wholly off the frame, or nothing placed: transparent.
        for members in [
            vec![solid(30., 10., -500., 20.)],
            vec![solid(30., 10., 20., 900.)],
            vec![],
            vec![comp(plan(true, vec![]), 1.0)],
            // A singular placement draws nothing.
            vec![solid(0., 10., 20., 20.)],
        ] {
            assert_eq!(bounds(members.clone()), Ok(None), "{members:?}");
            assert_eq!(
                effect_input_bounds(&node(vec![comp(plan(true, members), 1.0)]), full()),
                Some((0, 0, 0, 0))
            );
        }
    }

    /// Scissored effect passes of group boundaries render exactly the bits
    /// of whole passes, through the pinned extension stand-in
    /// (`effects::bounds::tests::DOT_MATRIX`) and a built-in blur, in final
    /// output and viewer frames at several densities; and the group's
    /// passes are scissored at all (before looking through groups they
    /// shaded the whole frame).
    #[test]
    fn scissored_group_effects_are_bit_identical() {
        use crate::effects::bounds::tests::{DOT_MATRIX, dot_matrix_definition};
        use crate::{MapResolver, OutputKind, RenderRequest, Renderer};
        use pm_protocol::color::Precision;
        use pm_protocol::common::{Size, Ticks};
        use serde_json::{Value, json};
        let ch = |v: Value| json!({ "v": v, "kf": [], "expr": null });
        let layer = |id: &str, ty: &str, d: Value, x: f64, y: f64, group: Option<&str>| {
            json!({
                "id": id, "type": ty, "name": id, "from": 0, "dur": 2, "on": true,
                "lock": false, "shy": false, "collapsed": true, "threeD": false,
                "solo": false, "color": "#4C8DFF", "blend": "normal", "mblur": false,
                "parent": null, "group": group, "matteSource": null,
                "p": {
                    "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)),
                    "position.x": ch(json!(x)), "position.y": ch(json!(y)),
                    "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
                    "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
                },
                "fx": [], "transitionIn": null, "transitionOut": null, "masks": [], "d": d,
                "locked_intent": {}
            })
        };
        let solid = |id: &str, color: &str, w: u32, h: u32, x: f64, y: f64, group: &str| {
            layer(
                id,
                "solid",
                json!({"w": w, "h": h, "color": color}),
                x,
                y,
                Some(group),
            )
        };
        let dots = |pitch: f64| {
            json!({"id": "dm", "type": "dot-matrix-resolve", "on": true,
                "p": {"pitch": ch(json!(pitch)), "detail": ch(json!(2)), "resolve": ch(json!(30))}})
        };
        let blur = json!({"id": "b", "type": "blur", "version": 2, "on": true, "p": {
            "amount": ch(json!(4)), "dimensions": ch(json!(0)), "repeatEdgePixels": ch(json!(false))}});
        let document = |fx: Value| {
            let mut g = layer("g", "group", json!({}), 0., 0., None);
            g["fx"] = fx;
            // A nested boundary (half opacity), a member partly off the
            // frame, and a rotated one.
            let mut h = layer("h", "group", json!({}), 0., 0., Some("g"));
            h["p"]["opacity"] = ch(json!(50));
            let mut r = solid("r", "#40A0F0", 20, 12, 150., 90., "g");
            r["p"]["rotation"] = ch(json!(30));
            let layers = vec![
                g,
                solid("a", "#F0C040", 24, 16, 60., 40., "g"),
                h,
                solid("b", "#E04080", 16, 16, 110., 30., "h"),
                solid("c", "#FFFFFF", 30, 10, -12., 120., "g"),
                r,
            ];
            let doc = json!({
                "id": "P", "name": "p", "w": W, "h": H, "fps": 30, "dur": 2, "bg": "#000000",
                "backgroundFill": {"type": "none", "angle": 0,
                    "stops": [{"id": "s1", "color": "#000000", "position": 0}]},
                "compId": "P-root", "compName": "p", "markers": [], "work": [0, 2],
                "params": {}, "revision": 0, "edits": [], "created": 0, "shutter": 0.5,
                "layers": layers, "assets": {}, "comps": {}
            });
            pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
        };
        let mut r = Renderer::headless().unwrap();
        let render = |r: &mut Renderer,
                      doc: &pm_model::Document,
                      kind: OutputKind,
                      scale: f64,
                      precision: Precision| {
            let failures = r
                .set_document_effects(doc, &[dot_matrix_definition(DOT_MATRIX)])
                .unwrap();
            assert!(failures.is_empty(), "{failures:?}");
            let mut req = RenderRequest::at_scale(Size::new(W, H), scale, precision);
            req.viewer = kind == OutputKind::Preview;
            req.output_kind = kind;
            req.background = false;
            let frame = r
                .render(doc, "P-root", Ticks::ZERO, &req, &MapResolver::default())
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            r.read_rgba(&frame)
                .unwrap()
                .into_iter()
                .map(f32::to_bits)
                .collect::<Vec<u32>>()
        };
        // Without effects, for telling that each chain ran.
        let plain = render(
            &mut r,
            &document(json!([])),
            OutputKind::Final,
            1.0,
            Precision::Float16,
        );
        for (what, fx) in [
            ("dot matrix pitch 4", json!([dots(4.)])),
            ("dot matrix pitch 1", json!([dots(1.)])),
            ("dot matrix pitch 30", json!([dots(30.)])),
            ("blur, dot matrix", json!([blur.clone(), dots(6.)])),
            ("dot matrix, blur", json!([dots(6.), blur.clone()])),
        ] {
            let doc = document(fx);
            for kind in [OutputKind::Final, OutputKind::Preview] {
                for scale in [1.0, 0.75, 1.5] {
                    for precision in [Precision::Float16, Precision::Float32] {
                        let what = format!("{what} {kind:?} ×{scale} {precision:?}");
                        r.set_effect_scissor(false);
                        let scope = pm_perf::Scope::begin();
                        let whole = render(&mut r, &doc, kind, scale, precision);
                        let off = scope.finish().get(pm_perf::Counter::EffectScissorHits);
                        assert_eq!(off, 0, "{what}: scissored with the scissor off");
                        r.set_effect_scissor(true);
                        let scope = pm_perf::Scope::begin();
                        let kept = render(&mut r, &doc, kind, scale, precision);
                        let hits = scope.finish().get(pm_perf::Counter::EffectScissorHits);
                        assert_eq!(whole.len(), kept.len(), "{what}");
                        if let Some(i) = whole.iter().zip(&kept).position(|(a, b)| a != b) {
                            panic!(
                                "{what}: pixel {} channel {} differs: whole {:?} scissored {:?}",
                                i / 4,
                                i % 4,
                                f32::from_bits(whole[i]),
                                f32::from_bits(kept[i])
                            );
                        }
                        assert!(whole.iter().any(|b| *b != 0), "{what}: drew nothing");
                        if (kind, scale, precision) == (OutputKind::Final, 1.0, Precision::Float16)
                        {
                            assert_ne!(whole, plain, "{what}: the effects did not run");
                        }
                        assert!(hits > 0, "{what}: the group's passes were not scissored");
                    }
                }
            }
        }
    }

    /// Only a normal-blend node without a transition (or an effect reading
    /// the backdrop, `tests/composite_batch.rs`) is batched; masks and a
    /// matte read the node's own targets.
    #[test]
    fn only_nodes_reading_nothing_beneath_them_are_batched() {
        let mut n = node(vec![solid(20., 10., 30., 40.)]);
        assert!(draws_over_the_stack(&n));
        for blend in [BlendMode::Screen, BlendMode::Multiply, BlendMode::Add] {
            n.blend = blend;
            assert!(!draws_over_the_stack(&n), "{blend:?} reads the stack");
        }
    }

    /// Three 2880 × 1620 half-float images fit the bound and a fourth does
    /// not; eight 1920 × 1080 ones fit.
    #[test]
    fn the_byte_bound_holds_three_retina_images_and_eight_hd_ones() {
        let image = |w: u64, h: u64| w * h * 8;
        assert!(3 * image(2880, 1620) <= DEFERRED_BYTES);
        assert!(4 * image(2880, 1620) > DEFERRED_BYTES);
        assert!(8 * image(1920, 1080) <= DEFERRED_BYTES);
        assert_eq!(DEFERRED_IMAGES, 8);
    }

    /// What every node loop on the thread holds is counted together (a
    /// nested loop sees the bytes of the one around it), and stops being
    /// counted once its pass is recorded or the loop is left.
    #[test]
    fn deferred_bytes_count_every_level_and_go_back() {
        let gpu = pm_gpu::Gpu::new("deferred bytes test").unwrap();
        let target = |w: u32, h: u32| {
            Target::wrap(gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("deferred bytes test"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }))
        };
        let held = || DEFERRED.with(|d| d.get());
        let before = held();
        let mut outer = Pending::default();
        outer.hold(target(64, 32));
        assert_eq!(held() - before, 64 * 32 * 8);
        {
            let mut inner = Pending::default();
            inner.hold(target(32, 32));
            assert_eq!(held() - before, (64 * 32 + 32 * 32) * 8, "both levels");
            let given = inner.take_targets();
            assert_eq!(given.len(), 1);
            assert_eq!(held() - before, 64 * 32 * 8, "the inner pass is recorded");
            // Left while holding one (a panic unwinding the loop).
            inner.hold(target(16, 16));
        }
        assert_eq!(held() - before, 64 * 32 * 8);
        assert_eq!(outer.take_targets().len(), 1);
        outer.hold(target(8, 8));
        drop(outer);
        assert_eq!(held(), before);
    }
}
