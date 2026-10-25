//! Executes a plan on the GPU: sources first, nested targets depth first,
//! then one render pass per target with every draw blended in stack order.

use pm_gpu::{Target, TargetPool};
use pm_protocol::color::{BlendDomain, Precision, WorkingEncoding};

use crate::capability::Unsupported;
use crate::effects::Effects;
use crate::effects::watchdog::FrameTimer;
use crate::error::LayerFailure;
use crate::media::{MediaCache, MediaFailure};
use crate::pipelines::{FillUniform, PassBlend, Pipelines, QuadUniform, UNIFORM_STRIDE, bytes_of};
use crate::plan::{Background, CompPlan, Content, Draw, Source};
use crate::sampling::gpu::DRAW_UNIFORM_SIZE;
use crate::scene3d::Scene3dRenderer;

pub use crate::composite::region::Region;

/// A draw with its source resolved to textures.
pub(crate) struct Resolved {
    pub matrix: [f64; 6],
    pub view: Option<wgpu::TextureView>,
    /// Linear view depth of a 3D layer (`r32float`).
    pub depth: Option<wgpu::TextureView>,
    pub alpha: f64,
    /// Solid premultiplied color.
    pub solid: Option<[f64; 4]>,
    /// The texture is a target in GL row order (`u_fromFbo`).
    pub from_fbo: bool,
    /// The texture holds premultiplied E values whatever the working
    /// encoding (the 3D pass output, section 9 "3D layers").
    pub encoded: bool,
    /// A linear 3D source: exposure, display background, weighted samples.
    pub scene: Option<(f32, Option<[f32; 3]>, bool)>,
    pub uv: [f64; 4],
    pub layer: String,
    /// A 2.5D plane: replaces `matrix` (unit quad to target pixels,
    /// column-major 3×3).
    pub plane: Option<[f64; 9]>,
    /// Raster sampling policy (`crate::sampling`).
    pub level2: Option<crate::sampling::Sampling>,
    /// The sampled source, with mips only when the draw can read them.
    pub pyramid: Option<crate::sampling::gpu::Pyramid>,
}

impl Resolved {
    /// Whether this draw can read beyond mip zero. Projective draws retain
    /// the full pyramid because their Jacobian changes across the quad.
    fn mip_levels(&self, texture: &wgpu::Texture, sampling: crate::sampling::Sampling) -> u32 {
        if self.plane.is_some() {
            return texture.width().max(texture.height()).ilog2() + 1;
        }
        crate::sampling::gpu::mip_levels(&self.matrix, self.uv, texture, sampling)
    }

    /// The quad's `mat3` uniform: the plane, or the affine matrix.
    pub fn uniform_matrix(&self) -> [[f32; 4]; 3] {
        match &self.plane {
            Some(p) => [
                [p[0] as f32, p[1] as f32, p[2] as f32, 0.0],
                [p[3] as f32, p[4] as f32, p[5] as f32, 0.0],
                [p[6] as f32, p[7] as f32, p[8] as f32, 0.0],
            ],
            None => m3(&self.matrix),
        }
    }
}

/// A frame's draw uniforms, kept frame after frame: each pass takes a
/// 256-byte-aligned range of the first chunk with room, so passes share
/// chunks (and their quad bind group) instead of one buffer each. A new
/// chunk is the next power of two of the pass that needed it, so a frame
/// never allocates more than one buffer per pass did (creating a buffer per
/// pass per frame was 8% of a viewer frame at 500 layers, docs/engine/perf.md).
/// `queue.write_buffer` lands before the next submission's commands, after
/// the earlier submissions that read the same bytes.
#[derive(Default)]
pub(crate) struct UniformPool {
    chunks: Vec<UniformChunk>,
}

struct UniformChunk {
    buffer: wgpu::Buffer,
    used: u64,
}

impl UniformPool {
    /// A frame starts: drop the chunks the last frame left unused and hand
    /// the rest out from their start again.
    pub fn reset(&mut self) {
        self.chunks.retain(|chunk| chunk.used > 0);
        for chunk in &mut self.chunks {
            chunk.used = 0;
        }
    }

    /// `size` bytes of this frame's uniforms: a chunk and the range's start.
    fn take(&mut self, device: &wgpu::Device, size: u64) -> (wgpu::Buffer, u64) {
        let span = size.next_multiple_of(UNIFORM_STRIDE);
        if let Some(chunk) = self
            .chunks
            .iter_mut()
            .find(|chunk| chunk.used + size <= chunk.buffer.size())
        {
            let at = chunk.used;
            chunk.used += span;
            return (chunk.buffer.clone(), at);
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm quads"),
            size: size.next_power_of_two().max(UNIFORM_STRIDE),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.chunks.push(UniformChunk {
            buffer: buffer.clone(),
            used: span,
        });
        (buffer, 0)
    }
}

pub(crate) struct Exec<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub pipelines: &'a Pipelines,
    pub uniforms: &'a mut UniformPool,
    /// Bind groups kept across frames (`crate::binds`).
    pub binds: &'a mut crate::binds::BindCache,
    pub media: &'a mut MediaCache,
    pub pool: &'a mut TargetPool,
    pub scene3d: &'a mut Scene3dRenderer,
    /// Effect and transition definitions and programs.
    pub effects: &'a mut Effects,
    /// This frame's watchdog timestamps (devices with `TIMESTAMP_QUERY`).
    pub timer: Option<FrameTimer>,
    pub vector: &'a mut crate::vector::cache::VectorCache,
    /// Shader-layer programs and the fallback runner's pixels.
    pub shaders: &'a mut crate::shader_layers::exec::ShaderState,
    /// Fallback runner requests whose pixels have not arrived.
    pub fallback_pending: Vec<std::sync::Arc<crate::shader_layers::FallbackRequest>>,
    pub text: &'a mut crate::text::gpu::TextGpu,
    pub precision: Precision,
    pub working: WorkingEncoding,
    /// Where screen, overlay, and soft light run with linear blending on.
    pub blend_domain: BlendDomain,
    pub export: bool,
    /// The frame also renders the depth and id surface (3D layers then
    /// produce their linear view depth).
    pub depth_id: bool,
    /// Pool targets to give back after the frame is submitted.
    pub taken: Vec<Target>,
    /// Mip pyramids of level 2 stills, kept across frames.
    pub pyramids: &'a mut crate::sampling::gpu::PyramidCache,
    pub failures: Vec<LayerFailure>,
    pub unsupported: Vec<Unsupported>,
    /// Nested compositions of this frame's plan whose plans are equal
    /// ([`comp_twins`]): (a later one, the first), by plan address.
    pub twins: Vec<(usize, usize)>,
    /// Targets of the first of such compositions this frame rendered
    /// (`resolve_content`): a composition drawn twice with the same plan (a
    /// precomp used by two layers at one time) samples one target. An entry
    /// lives as long as its target is in `taken` (a motion-blur sample gives
    /// its targets back early and drops its entries).
    pub comps: Vec<RenderedComp>,
}

/// A nested composition's target this frame, by its plan's address.
pub(crate) struct RenderedComp {
    first: usize,
    revision: u64,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// The nested compositions (not group passes) of `plan` whose plans equal an
/// earlier one's: (its address, the earlier one's). The addresses are
/// identities within the plan, which outlives the frame's `Exec`; they are
/// never read through.
pub(crate) fn comp_twins(plan: &CompPlan) -> Vec<(usize, usize)> {
    fn content<'p>(c: &'p Content, out: &mut Vec<&'p CompPlan>) {
        match c {
            Content::Placed(p) => content(&p.content, out),
            Content::Texture {
                source: Source::Comp(child),
                ..
            } => {
                if !child.group {
                    out.push(child);
                }
                walk(child, out);
            }
            Content::Texture {
                source: Source::Accumulate(child),
                ..
            } => walk(child, out),
            _ => {}
        }
    }
    fn walk<'p>(plan: &'p CompPlan, out: &mut Vec<&'p CompPlan>) {
        for d in &plan.draws {
            content(&d.content, out);
        }
    }
    let mut comps = Vec::new();
    walk(plan, &mut comps);
    let mut twins = Vec::new();
    for (j, later) in comps.iter().enumerate() {
        if let Some(first) = comps[..j].iter().find(|first| ***first == **later) {
            twins.push((plan_address(later), plan_address(first)));
        }
    }
    twins
}

fn plan_address(plan: &CompPlan) -> usize {
    std::ptr::from_ref(plan) as usize
}

/// `m3(M)`: a 2×3 matrix as the column-major mat3 uniform, in f32.
fn m3(m: &[f64; 6]) -> [[f32; 4]; 3] {
    [
        [m[0] as f32, m[1] as f32, 0.0, 0.0],
        [m[2] as f32, m[3] as f32, 0.0, 0.0],
        [m[4] as f32, m[5] as f32, 1.0, 0.0],
    ]
}

/// A deterministic session instance per layer (FNV-1a of its id).
pub(crate) fn instance_of(layer: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in layer.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A region of interest renders the full frame's geometry through a
/// viewport placed so the region lands at the target's origin. Targets are in
/// GL row order: the region's bottom row is full row `H − y − h`.
/// The pipeline and source group a draw pass last bound.
#[derive(Default)]
struct Bound<'p> {
    pipeline: Option<&'p wgpu::RenderPipeline>,
    source: Option<&'p wgpu::BindGroup>,
}

impl<'p> Bound<'p> {
    fn pipeline(&mut self, pass: &mut wgpu::RenderPass<'_>, pipeline: &'p wgpu::RenderPipeline) {
        if self.pipeline != Some(pipeline) {
            pass.set_pipeline(pipeline);
            self.pipeline = Some(pipeline);
            self.source = None;
        }
    }

    fn source(&mut self, pass: &mut wgpu::RenderPass<'_>, group: &'p wgpu::BindGroup) {
        if self.source != Some(group) {
            pass.set_bind_group(1, group, &[]);
            self.source = Some(group);
        }
    }
}

fn region_viewport(
    pass: &mut wgpu::RenderPass<'_>,
    region: Option<Region>,
    full_width: u32,
    full_height: u32,
    target_height: u32,
) {
    if let Some(g) = region {
        let below = full_height - g.y - target_height;
        pass.set_viewport(
            -(g.x as f32),
            -(below as f32),
            full_width as f32,
            full_height as f32,
            0.0,
            1.0,
        );
    }
}

impl Exec<'_> {
    fn fail(&mut self, plan: &CompPlan, draw: &Draw, error: MediaFailure) {
        match error {
            MediaFailure::Unsupported(feature) => self.unsupported.push(Unsupported {
                composition: Some(plan.composition.clone()),
                layer: Some(draw.layer.clone()),
                feature,
            }),
            MediaFailure::Failed(message) => self.failures.push(LayerFailure {
                composition: plan.composition.clone(),
                layer: draw.layer.clone(),
                message,
            }),
        }
    }

    /// Resolves the sources of `draws` (of `plan`), recording any passes
    /// they need.
    pub(crate) fn resolve_draws(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        draws: &[Draw],
        region: Option<Region>,
    ) -> Vec<Resolved> {
        let mut out = Vec::with_capacity(draws.len());
        for draw in draws {
            // Drawn by `composite::exec`, never resolved as a quad.
            if matches!(draw.content, Content::Composite(_)) {
                continue;
            }
            let mut r = Resolved {
                matrix: draw.matrix,
                view: None,
                depth: None,
                alpha: 1.0,
                solid: None,
                from_fbo: false,
                encoded: false,
                scene: None,
                uv: [0.0, 0.0, 1.0, 1.0],
                layer: draw.layer.clone(),
                plane: None,
                level2: None,
                pyramid: None,
            };
            self.resolve_content(encoder, plan, draw, &draw.content, region, &mut r);
            out.push(r);
        }
        out
    }

    fn resolve_content(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        draw: &Draw,
        content: &Content,
        region: Option<Region>,
        r: &mut Resolved,
    ) {
        match content {
            Content::Solid(s) => {
                r.solid = Some(crate::composite::color::premultiplied_to_working(
                    s.color,
                    self.working,
                ));
                // `fs_solid` reads the box and radius from the UV slot.
                r.uv = [s.size[0], s.size[1], s.radius, 0.0];
            }
            // Drawn by `composite::exec` (skipped by `resolve_draws`).
            Content::Composite(_) => {}
            Content::Placed(p) => {
                r.plane = p.plane;
                r.level2 = p.level2;
                self.resolve_content(encoder, plan, draw, &p.content, region, r);
            }
            Content::Texture { source, uv, alpha } => {
                r.uv = *uv;
                r.alpha = *alpha;
                match source {
                    Source::Comp(child) | Source::Accumulate(child) => {
                        // A group boundary (and a motion-blur accumulation)
                        // shares its parent's pixel space: it renders only
                        // the region, drawn 1:1.
                        let group = child.group;
                        let child_region = if group { region } else { None };
                        // The same composition planned the same way earlier
                        // in this frame: its target holds exactly the pixels
                        // rendering it again would draw (never a group pass:
                        // `comp_twins` lists nested compositions only).
                        let accumulate = matches!(source, Source::Accumulate(_));
                        let address = plan_address(child);
                        if !accumulate
                            && !self.depth_id
                            && let Some(&(_, first)) =
                                self.twins.iter().find(|(later, _)| *later == address)
                            && let Some(kept) = self.comps.iter().find(|c| c.first == first)
                        {
                            let texture = kept.texture.clone();
                            let revision = kept.revision;
                            r.view = Some(kept.view.clone());
                            self.prepare_level2(encoder, r, &texture, Some(revision));
                            r.from_fbo = true;
                            return;
                        }
                        let (w, h) = match child_region {
                            Some(g) => (g.width, g.height),
                            None => (child.width, child.height),
                        };
                        let target = self.pool.take_working(self.device, w, h, self.precision);
                        // What the render reported: a render that failed a
                        // layer, met something unsupported, or waits for
                        // fallback pixels is not given to a second use.
                        let reported = (
                            self.failures.len(),
                            self.unsupported.len(),
                            self.fallback_pending.len(),
                            self.effects.fallback.pending.len(),
                        );
                        let inner = if accumulate {
                            self.render_accumulation(encoder, child, child_region, &target);
                            Vec::new()
                        } else {
                            self.render_comp(encoder, child, child_region, &target)
                        };
                        let revision = self.pyramids.changed();
                        if !accumulate
                            && !self.depth_id
                            && !group
                            && self.twins.iter().any(|(_, first)| *first == address)
                            && reported
                                == (
                                    self.failures.len(),
                                    self.unsupported.len(),
                                    self.fallback_pending.len(),
                                    self.effects.fallback.pending.len(),
                                )
                        {
                            self.comps.push(RenderedComp {
                                first: address,
                                revision,
                                texture: target.texture.clone(),
                                view: target.view.clone(),
                            });
                        }
                        r.view = Some(target.view.clone());
                        self.prepare_level2(encoder, r, &target.texture, Some(revision));
                        r.from_fbo = true;
                        // The depth and id surface sees 3D inside the
                        // composition: its own surface's depth (GL row
                        // order, like the color) stands in for a 3D
                        // layer's depth texture.
                        if self.depth_id && inner.iter().any(|d| d.depth.is_some()) {
                            let ids = self.pool.take(
                                self.device,
                                w,
                                h,
                                wgpu::TextureFormat::Rgba16Float,
                                wgpu::TextureUsages::RENDER_ATTACHMENT
                                    | wgpu::TextureUsages::TEXTURE_BINDING,
                            );
                            self.render_ids(
                                encoder,
                                &inner,
                                child_region,
                                (child.width, child.height),
                                &ids,
                            );
                            r.depth = Some(ids.view.clone());
                            self.taken.push(ids);
                        }
                        if let (true, Some(g)) = (group, region) {
                            r.matrix = [
                                f64::from(g.width),
                                0.0,
                                0.0,
                                f64::from(g.height),
                                f64::from(g.x),
                                f64::from(g.y),
                            ];
                        }
                        self.taken.push(target);
                    }
                    Source::Image {
                        path,
                        interpret,
                        level2,
                        ..
                    } => {
                        match self.media.image(
                            self.device,
                            self.queue,
                            self.pipelines,
                            encoder,
                            path,
                            self.working,
                            self.precision,
                            interpret.as_ref(),
                            !*level2,
                        ) {
                            Ok(t) => {
                                r.view = Some(t.view.clone());
                                self.prepare_level2(encoder, r, &t.texture, Some(0));
                            }
                            Err(e) => self.fail(plan, draw, e),
                        }
                    }
                    Source::Video {
                        sequence,
                        path,
                        time,
                        interpret,
                        composition_rate,
                        ..
                    } => {
                        match self.media.video(
                            self.device,
                            self.queue,
                            self.pipelines,
                            encoder,
                            path,
                            instance_of(&draw.layer),
                            *time,
                            self.export,
                            self.working,
                            self.precision,
                            interpret.as_ref(),
                            *composition_rate,
                            *sequence,
                        ) {
                            Ok(Some(t)) => {
                                r.view = Some(t.view.clone());
                                let revision = self.media.texture_revision(&t.texture);
                                self.prepare_level2(encoder, r, &t.texture, revision);
                            }
                            Ok(None) => {}
                            Err(e) => self.fail(plan, draw, e),
                        }
                    }
                    Source::Vector(v) => {
                        match self
                            .vector
                            .resolve(self.device, self.queue, v, self.precision)
                        {
                            Ok(view) => {
                                // Level 2 content on a plane is resampled
                                // (`crate::space25d::local`).
                                self.prepare_level2(encoder, r, view.texture(), None);
                                r.view = Some(view);
                            }
                            Err(message) => {
                                self.fail(plan, draw, MediaFailure::Failed(message))
                            }
                        }
                    }
                    Source::Text(text) => {
                        match self.text.render(
                            self.device,
                            self.queue,
                            encoder,
                            self.pool,
                            &mut self.taken,
                            text,
                            self.precision,
                            self.working,
                        ) {
                            Ok(view) => {
                                self.prepare_level2(encoder, r, view.texture(), None);
                                r.view = Some(view);
                            }
                            Err(message) => self.fail(plan, draw, MediaFailure::Failed(message)),
                        }
                    }
                    Source::Compat(compat) => {
                        // A canvas's backing store: premultiplied E values,
                        // top row first, like a text raster (linear light
                        // decodes them).
                        r.view = Some(crate::compat::upload(
                            self.device,
                            self.queue,
                            &compat.0,
                            self.precision,
                        ));
                        r.encoded = true;
                    }
                    Source::Shader(shader) => match self.shader_source(encoder, shader) {
                        Ok((target, pooled)) => {
                            // Legacy programs evaluate their authored formula in E.
                            // Decode the texels before linear-light reconstruction.
                            if self.working == WorkingEncoding::LinearLight && r.level2.is_some() {
                                let linear = self.decode_sampled_source(encoder, &target);
                                self.prepare_level2(encoder, r, &linear.texture, None);
                                r.view = Some(linear.view.clone());
                            } else {
                                self.prepare_level2(encoder, r, &target.texture, None);
                                r.view = Some(target.view.clone());
                                r.encoded = true;
                            }
                            r.from_fbo = true;
                            // Uploaded fallback pixels are not the
                            // pool's; wgpu keeps them alive until the
                            // frame's commands are done.
                            if pooled {
                                self.taken.push(target);
                            }
                        }
                        // Turned off by the watchdog: nothing drawn; its
                        // report names the layer.
                        Err(crate::shader_layers::exec::Missed::Disabled) => {}
                        // Held by the hang guard: the frame is not presented.
                        Err(crate::shader_layers::exec::Missed::Held) => {}
                        Err(crate::shader_layers::exec::Missed::NeedsFallback(what)) => self.fail(
                            plan,
                            draw,
                            MediaFailure::Unsupported(crate::capability::Feature::ShaderFallback(
                                what,
                            )),
                        ),
                        Err(crate::shader_layers::exec::Missed::Pending(request)) => {
                            self.effects.fallback.blocked = true;
                            if !self.fallback_pending.iter().any(|p| p.key == request.key) {
                                self.fallback_pending.push(request);
                            }
                        }
                    },
                    Source::Object3d(object) => {
                        self.scene3d
                            .prepare_svg_maps(self.device, self.queue, encoder, object);
                        match self.scene3d.render_at_precision(
                            self.device,
                            self.queue,
                            encoder,
                            self.pool,
                            object,
                            self.depth_id,
                            self.precision,
                        ) {
                            Ok(out) => {
                                // The pass renders in GL row order.
                                match out.color_space {
                                    crate::scene3d::OutputColor3d::Encoded => {
                                        if self.working == WorkingEncoding::LinearLight
                                            && r.level2.is_some()
                                        {
                                            let linear =
                                                self.decode_sampled_source(encoder, &out.color);
                                            self.prepare_level2(encoder, r, &linear.texture, None);
                                            r.view = Some(linear.view.clone());
                                        } else {
                                            self.prepare_level2(
                                                encoder,
                                                r,
                                                &out.color.texture,
                                                None,
                                            );
                                            r.view = Some(out.color.view.clone());
                                            r.encoded = true;
                                        }
                                    }
                                    crate::scene3d::OutputColor3d::LinearScene {
                                        exposure,
                                        background,
                                        weighted,
                                    } => {
                                        if r.level2.is_some() {
                                            let converted = self.scene_output(
                                                encoder, &out.color, exposure, background,
                                                weighted,
                                            );
                                            self.prepare_level2(
                                                encoder,
                                                r,
                                                &converted.texture,
                                                None,
                                            );
                                            r.view = Some(converted.view.clone());
                                        } else {
                                            r.view = Some(out.color.view.clone());
                                            r.scene = Some((exposure, background, weighted));
                                        }
                                    }
                                }
                                r.from_fbo = true;
                                r.depth = out.depth.as_ref().map(|d| d.view.clone());
                                self.taken.push(out.color);
                                if let Some(d) = out.depth {
                                    self.taken.push(d);
                                }
                            }
                            Err(crate::scene3d::Error3d::Unsupported(feature)) => {
                                self.fail(plan, draw, MediaFailure::Unsupported(feature))
                            }
                            Err(crate::scene3d::Error3d::Failed(message)) => {
                                self.fail(plan, draw, MediaFailure::Failed(message))
                            }
                        }
                    }
                }
            }
        }
    }

    /// The source and optional mip pyramid a raster draw samples (`crate::sampling`).
    fn prepare_level2(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        r: &mut Resolved,
        texture: &wgpu::Texture,
        revision: Option<u64>,
    ) {
        if let Some(sampling) = r.level2 {
            let levels = r.mip_levels(texture, sampling);
            let view = r.view.as_ref().filter(|v| v.texture() == texture);
            r.pyramid = Some(self.pipelines.resample.pyramid_for(
                self.device,
                encoder,
                texture,
                view,
                levels,
                revision,
                self.pyramids,
            ));
        }
    }

    /// Renders `plan` into `target` (the region's size when `region` is set).
    pub fn render_comp(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        region: Option<Region>,
        target: &Target,
    ) -> Vec<Resolved> {
        if crate::composite::has_nodes(plan) {
            return self.render_comp_nodes(encoder, plan, region, target);
        }
        self.render_comp_with(encoder, plan, region, target, PassBlend::Over)
    }

    /// Motion-blur samples (`crate::motion`) accumulated additively
    /// (`drawSample(true)`: `ONE, ONE`) into a cleared target, one pass per
    /// sample: each sample's sources (nested compositions, group passes)
    /// go back to the pool once its pass is recorded, so a blurred precomp
    /// holds one sample's targets at a time, as today's `free(sample)`.
    pub fn render_accumulation(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        region: Option<Region>,
        target: &Target,
    ) {
        if plan.draws.is_empty() {
            self.render_draws(
                encoder,
                plan,
                &[],
                region,
                target,
                PassBlend::Additive,
                true,
            );
            return;
        }
        for (i, draw) in plan.draws.iter().enumerate() {
            let mark = self.taken.len();
            let comps = self.comps.len();
            self.render_draws(
                encoder,
                plan,
                std::slice::from_ref(draw),
                region,
                target,
                PassBlend::Additive,
                i == 0,
            );
            let done: Vec<Target> = self.taken.drain(mark..).collect();
            for t in done {
                self.pool.give(t);
            }
            // Compositions rendered for this sample lived in those targets,
            // which the pool hands out again: they are no longer there to
            // share.
            self.comps.truncate(comps);
        }
    }

    /// [`Self::render_comp`] of a plan without compositing nodes, with the
    /// pass's blending.
    pub fn render_comp_with(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        region: Option<Region>,
        target: &Target,
        blend: PassBlend,
    ) -> Vec<Resolved> {
        self.render_draws(encoder, plan, &plan.draws, region, target, blend, true)
    }

    /// Resolves `draws` of `plan` and records them into `target` in one
    /// pass: cleared to the plan's background first (`clear`), or drawn
    /// over what it holds.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_draws(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        draws: &[Draw],
        region: Option<Region>,
        target: &Target,
        blend: PassBlend,
        clear: bool,
    ) -> Vec<Resolved> {
        let resolved = self.resolve_draws(encoder, plan, draws, region);
        self.draw_pass_with(encoder, plan, &resolved, region, target, clear, blend);
        resolved
    }

    /// Records one render pass of `resolved` into `target`: cleared to the
    /// plan's background first when `clear`, else drawn over its contents.
    pub(crate) fn draw_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        resolved: &[Resolved],
        region: Option<Region>,
        target: &Target,
        clear: bool,
    ) {
        self.draw_pass_with(
            encoder,
            plan,
            resolved,
            region,
            target,
            clear,
            PassBlend::Over,
        );
    }

    /// [`Self::draw_pass`] with the pass's blending.
    #[allow(clippy::too_many_arguments)]
    fn draw_pass_with(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        resolved: &[Resolved],
        region: Option<Region>,
        target: &Target,
        clear: bool,
        blend: PassBlend,
    ) {
        let (ox, oy) = region.map_or((0.0, 0.0), |g| (f64::from(g.x), f64::from(g.y)));
        let (th, format) = (target.height(), target.format());
        // Draws keep full-frame coordinates; a region of interest moves the
        // viewport instead (`region_viewport`), so the rasterizer sees the
        // same vertex positions as in the full frame and every pixel of the
        // region equals the full frame's.
        let (fw, fh) = (plan.width, plan.height);
        // Level 2 draws: the content polygon, its bounds, and the
        // resampler's bind group (`crate::sampling`).
        let mut sample_uniforms = Vec::new();
        let level2: Vec<Option<usize>> = resolved
            .iter()
            .map(|r| {
                let (Some(sampling), Some(pyramid)) = (r.level2, r.pyramid.as_ref()) else {
                    return None;
                };
                let matrix = r
                    .plane
                    .unwrap_or_else(|| crate::sampling::gpu::affine3(&r.matrix));
                let placement = crate::sampling::gpu::placement(matrix, r.uv)?;
                let origin = region.map_or((0, 0), |g| (g.x, g.y));
                let u = placement.uniform(
                    r.uv,
                    r.alpha,
                    r.from_fbo,
                    sampling,
                    origin,
                    target.height(),
                    (fw, fh),
                    pyramid,
                )?;
                let index = sample_uniforms.len();
                sample_uniforms.push(u);
                Some(index)
            })
            .collect();
        // Uniforms: one slot per draw plus the background quad.
        let slots = resolved.len() + 1;
        let mut data = vec![0u8; slots * UNIFORM_STRIDE as usize];
        // A region's origin in the full frame's GL rows (`region_viewport`):
        // solids compute coverage from full-frame pixel positions.
        let region_origin = region.map_or([0.0, 0.0], |g| {
            [g.x as f32, (fh - g.y - th) as f32]
        });
        let quad = |m: [f64; 6], uv: [f64; 4], color: [f64; 4]| QuadUniform {
            m: m3(&m),
            res: [fw as f32, fh as f32, region_origin[0], region_origin[1]],
            uv: uv.map(|v| v as f32),
            color: color.map(|v| v as f32),
            scene: [0.0; 4],
        };
        let background = quad(
            [f64::from(fw), 0.0, 0.0, f64::from(fh), 0.0, 0.0],
            [0.0, 0.0, 1.0, 1.0],
            [0.0; 4],
        );
        data[..size_of::<QuadUniform>()].copy_from_slice(bytes_of(&background));
        for (i, r) in resolved.iter().enumerate() {
            let decode = r.encoded && self.working == WorkingEncoding::LinearLight;
            let color = r.solid.unwrap_or([
                r.alpha,
                if r.from_fbo { 1.0 } else { 0.0 },
                if decode { 1.0 } else { 0.0 },
                0.0,
            ]);
            let mut u = quad(r.matrix, r.uv, color);
            u.m = r.uniform_matrix();
            if let Some((exposure, background, weighted)) = r.scene {
                u.color[2] = if self.working == WorkingEncoding::LinearLight {
                    2.0
                } else {
                    3.0
                };
                u.color[3] = f32::from(background.is_some()) + 2.0 * f32::from(weighted);
                let bg = background.unwrap_or([0.0; 3]);
                u.scene = [bg[0], bg[1], bg[2], exposure];
            }
            let at = (i + 1) * UNIFORM_STRIDE as usize;
            if let Some(sample) = &level2[i] {
                let raw = bytes_of(&sample_uniforms[*sample]);
                data[at..at + raw.len()].copy_from_slice(raw);
            } else {
                data[at..at + size_of::<QuadUniform>()].copy_from_slice(bytes_of(&u));
            }
        }
        let (buffer, base) = self.uniforms.take(self.device, data.len() as u64);
        self.queue.write_buffer(&buffer, base, &data);
        let base = base as u32;
        let quad_bind = self
            .pipelines
            .resample
            .bind_uniform(self.device, self.binds, &buffer);
        let sample_binds: Vec<Option<wgpu::BindGroup>> = resolved
            .iter()
            .zip(&level2)
            .map(|(r, u)| {
                u.as_ref()
                    .and(r.pyramid.as_ref())
                    .map(|p| self.pipelines.resample.bind(self.device, self.binds, p))
            })
            .collect();
        let fill_bind = match &plan.background {
            Background::Gradient(g) if clear => {
                let mut u = FillUniform::default();
                for (i, (rgb, pos)) in g.stops.iter().take(8).enumerate() {
                    u.stops[i] = [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, *pos as f32];
                }
                for i in g.stops.len().min(8)..8 {
                    u.stops[i] = [0.0, 0.0, 0.0, 1.0];
                }
                u.params = [
                    g.stops.len().min(8) as f32,
                    if g.radial { 2.0 } else { 1.0 },
                    g.angle_degrees as f32,
                    th as f32,
                ];
                // `backgroundView` with a preview viewport: the gradient
                // spans the composition at today's scale, from the level's
                // output origin (its y from the top, as today's `u_view`
                // adds it to GL's bottom-up `v_st`).
                u.view = match plan.view {
                    Some(v) => [
                        (v.origin[0] + ox) as f32,
                        (v.resolution[1] - v.origin[1] - f64::from(plan.height) + oy) as f32,
                        v.resolution[0] as f32,
                        v.resolution[1] as f32,
                    ],
                    None => [ox as f32, oy as f32, plan.width as f32, plan.height as f32],
                };
                u.flags = [
                    if self.working == WorkingEncoding::LinearLight {
                        1.0
                    } else {
                        0.0
                    },
                    0.0,
                    0.0,
                    0.0,
                ];
                let b = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pm fill"),
                    size: size_of::<FillUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.queue.write_buffer(&b, 0, bytes_of(&u));
                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("pm fill"),
                    layout: &self.pipelines.fill_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 2,
                        resource: b.as_entire_binding(),
                    }],
                }))
            }
            _ => None,
        };
        let texture_binds: Vec<Option<wgpu::BindGroup>> = resolved
            .iter()
            .map(|r| {
                r.view.as_ref().filter(|_| r.level2.is_none()).map(|view| {
                    self.binds.texture(
                        self.device,
                        &self.pipelines.texture_layout,
                        &self.pipelines.linear,
                        view,
                    )
                })
            })
            .collect();
        let load = if clear {
            wgpu::LoadOp::Clear(match &plan.background {
                Background::Transparent => wgpu::Color::TRANSPARENT,
                Background::Solid(c) => {
                    let c = crate::composite::color::premultiplied_to_working(*c, self.working);
                    wgpu::Color {
                        r: c[0],
                        g: c[1],
                        b: c[2],
                        a: c[3],
                    }
                }
                Background::Gradient(_) => wgpu::Color::BLACK,
            })
        } else {
            wgpu::LoadOp::Load
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm composite"),
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
        });
        region_viewport(&mut pass, region, fw, fh, th);
        if let Some(fill) = fill_bind.as_ref() {
            pass.set_pipeline(self.pipelines.background(format));
            pass.set_bind_group(0, &quad_bind, &[base]);
            pass.set_bind_group(1, fill, &[]);
            pass.draw(0..4, 0..1);
        }
        // Consecutive draws of one kind keep their pipeline, and a repeated
        // source keeps its group; a pipeline change rebinds the source.
        let mut bound = Bound::default();
        for (i, r) in resolved.iter().enumerate() {
            let offset = base + ((i + 1) as u64 * UNIFORM_STRIDE) as u32;
            if r.level2.is_some() && r.solid.is_none() {
                // Level 2: the resampler, or nothing when no part of the
                // content is in front of the camera and in the frame.
                if let Some(bind) = &sample_binds[i] {
                    bound.pipeline(&mut pass, self.pipelines.resample.pipeline(format, blend));
                    pass.set_bind_group(0, &quad_bind, &[offset]);
                    bound.source(&mut pass, bind);
                    pass.draw(0..4, 0..1);
                }
            } else if r.solid.is_some() {
                bound.pipeline(
                    &mut pass,
                    if r.uv[2] == 0.0 {
                        self.pipelines.rect_with(format, blend)
                    } else {
                        self.pipelines.solid_with(format, blend)
                    },
                );
                pass.set_bind_group(0, &quad_bind, &[offset]);
                pass.draw(0..4, 0..1);
            } else if let Some(bind) = &texture_binds[i] {
                bound.pipeline(
                    &mut pass,
                    if r.scene.is_some() {
                        self.pipelines.scene_with(format, blend)
                    } else {
                        self.pipelines.draw_with(format, blend)
                    },
                );
                pass.set_bind_group(0, &quad_bind, &[offset]);
                bound.source(&mut pass, bind);
                pass.draw(0..4, 0..1);
            }
        }
        drop(pass);
    }

    /// Records the depth and id surface of the top-level draws.
    pub fn render_ids(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        resolved: &[Resolved],
        region: Option<Region>,
        full: (u32, u32),
        target: &Target,
    ) {
        let th = target.height();
        let mut data = vec![0u8; resolved.len().max(1) * UNIFORM_STRIDE as usize];
        for (i, r) in resolved.iter().enumerate() {
            let id = i as u32;
            let alpha = r.solid.map_or(r.alpha, |c| c[3]);
            let u = QuadUniform {
                m: r.uniform_matrix(),
                res: [full.0 as f32, full.1 as f32, 0.0, 0.0],
                uv: r.uv.map(|v| v as f32),
                color: [
                    alpha as f32,
                    (id & 0x7ff) as f32,
                    (id >> 11) as f32,
                    if r.from_fbo { 1.0 } else { 0.0 },
                ],
                scene: [0.0; 4],
            };
            let at = i * UNIFORM_STRIDE as usize;
            data[at..at + size_of::<QuadUniform>()].copy_from_slice(bytes_of(&u));
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm id quads"),
            size: data.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, &data);
        let quad_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm id quads"),
            layout: &self.pipelines.quad_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(DRAW_UNIFORM_SIZE),
                }),
            }],
        });
        let binds: Vec<(Option<wgpu::BindGroup>, Option<wgpu::BindGroup>)> = resolved
            .iter()
            .map(|r| {
                let tex = r.view.as_ref().map(|view| {
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pm id texture"),
                        layout: &self.pipelines.texture_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.pipelines.linear),
                            },
                        ],
                    })
                });
                let depth = r.depth.as_ref().map(|view| {
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pm id depth"),
                        layout: &self.pipelines.depth_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(view),
                        }],
                    })
                });
                (tex, depth)
            })
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm depth and id"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::INFINITY,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        region_viewport(&mut pass, region, full.0, full.1, th);
        for (i, r) in resolved.iter().enumerate() {
            let offset = (i as u64 * UNIFORM_STRIDE) as u32;
            match (&binds[i], r.solid) {
                (_, Some(_)) => {
                    pass.set_pipeline(&self.pipelines.id_solid);
                    pass.set_bind_group(0, &quad_bind, &[offset]);
                    pass.draw(0..4, 0..1);
                }
                ((Some(tex), Some(depth)), None) => {
                    pass.set_pipeline(&self.pipelines.id_3d);
                    pass.set_bind_group(0, &quad_bind, &[offset]);
                    pass.set_bind_group(1, tex, &[]);
                    pass.set_bind_group(2, depth, &[]);
                    pass.draw(0..4, 0..1);
                }
                ((Some(tex), None), None) => {
                    pass.set_pipeline(&self.pipelines.id_draw);
                    pass.set_bind_group(0, &quad_bind, &[offset]);
                    pass.set_bind_group(1, tex, &[]);
                    pass.draw(0..4, 0..1);
                }
                _ => {}
            }
        }
    }

    /// Copies `source` (GL row order) into `target` top-down, exactly.
    pub fn flip(&mut self, encoder: &mut wgpu::CommandEncoder, source: &Target, target: &Target) {
        let (w, h) = (target.width(), target.height());
        let u = QuadUniform {
            m: m3(&[f64::from(w), 0.0, 0.0, f64::from(h), 0.0, 0.0]),
            res: [w as f32, h as f32, 0.0, 0.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [0.0; 4],
            scene: [0.0; 4],
        };
        let (buffer, base) = self.uniforms.take(self.device, UNIFORM_STRIDE);
        // The flip entries never read the scene-output fields.
        self.queue.write_buffer(
            &buffer,
            base,
            &bytes_of(&u)[..std::mem::offset_of!(QuadUniform, scene)],
        );
        let quad = self.binds.quads(
            self.device,
            &self.pipelines.quad_layout,
            &buffer,
            DRAW_UNIFORM_SIZE,
        );
        let tex = self.binds.texture(
            self.device,
            &self.pipelines.texture_layout,
            &self.pipelines.linear,
            &source.view,
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm flip"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
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
        pass.set_pipeline(self.pipelines.flip(target.format()));
        pass.set_bind_group(0, &quad, &[base as u32]);
        pass.set_bind_group(1, &tex, &[]);
        pass.draw(0..4, 0..1);
    }
}
