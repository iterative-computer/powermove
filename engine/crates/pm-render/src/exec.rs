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
    pub uv: [f64; 4],
    pub layer: String,
    /// A 2.5D plane: replaces `matrix` (unit quad to target pixels,
    /// column-major 3×3).
    pub plane: Option<[f64; 9]>,
    /// Level 2 raster content (`crate::sampling`).
    pub level2: Option<crate::sampling::Sampling>,
    /// The level 2 source's mip pyramid.
    pub pyramid: Option<crate::sampling::gpu::Pyramid>,
}

impl Resolved {
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

/// The quad uniform buffers of a renderer's passes, reused frame after frame
/// (the n-th pass of a frame takes the n-th buffer): creating one per pass
/// per frame was 8% of a viewer frame at 500 layers (docs/engine/perf.md).
/// `queue.write_buffer` lands before the next submission's commands, after
/// the earlier submissions that read the buffer.
#[derive(Default)]
pub(crate) struct UniformPool {
    buffers: Vec<wgpu::Buffer>,
    next: usize,
}

impl UniformPool {
    /// A frame starts: hand the buffers out from the first again.
    pub fn reset(&mut self) {
        self.next = 0;
    }

    /// A buffer of at least `size` bytes for this frame's next pass.
    fn take(&mut self, device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        let i = self.next;
        self.next += 1;
        if let Some(b) = self.buffers.get(i).filter(|b| b.size() >= size) {
            return b.clone();
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm quads"),
            size: size.next_power_of_two().max(4096),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if i < self.buffers.len() {
            self.buffers[i] = buffer.clone();
        } else {
            self.buffers.push(buffer.clone());
        }
        buffer
    }
}

pub(crate) struct Exec<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub pipelines: &'a Pipelines,
    pub uniforms: &'a mut UniformPool,
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
    /// What a level 1 SVG raster miss draws (`crate::vector::svg_rasters`).
    pub svg_policy: crate::vector::svg_rasters::SvgMissPolicy,
    /// The frame also renders the depth and id surface (3D layers then
    /// produce their linear view depth).
    pub depth_id: bool,
    /// Pool targets to give back after the frame is submitted.
    pub taken: Vec<Target>,
    /// Mip pyramids of level 2 stills, kept across frames.
    pub pyramids: &'a mut crate::sampling::gpu::PyramidCache,
    pub failures: Vec<LayerFailure>,
    pub unsupported: Vec<Unsupported>,
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
fn instance_of(layer: &str) -> u64 {
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
            Content::Solid(c) => {
                r.solid = Some(crate::composite::color::premultiplied_to_working(
                    *c,
                    self.working,
                ))
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
                        let (w, h) = match child_region {
                            Some(g) => (g.width, g.height),
                            None => (child.width, child.height),
                        };
                        let target = self.pool.take_working(self.device, w, h, self.precision);
                        let inner = if matches!(source, Source::Accumulate(_)) {
                            self.render_accumulation(encoder, child, child_region, &target);
                            Vec::new()
                        } else {
                            self.render_comp(encoder, child, child_region, &target)
                        };
                        r.view = Some(target.view.clone());
                        self.prepare_level2(encoder, r, &target.texture, false);
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
                                self.prepare_level2(encoder, r, &t.texture, true);
                            }
                            Err(e) => self.fail(plan, draw, e),
                        }
                    }
                    Source::Video {
                        level2,
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
                            !*level2,
                        ) {
                            Ok(Some(t)) => {
                                r.view = Some(t.view.clone());
                                self.prepare_level2(encoder, r, &t.texture, false);
                            }
                            Ok(None) => {}
                            Err(e) => self.fail(plan, draw, e),
                        }
                    }
                    Source::Vector(v) => {
                        if let crate::vector::VectorSource::SvgImage { key, asset, path } = &**v {
                            use crate::vector::cache::SvgResolved;
                            // Chromium's raster of today's bucket (decision
                            // S1); a miss draws the content's latest bucket
                            // in preview, nothing otherwise.
                            match self.vector.resolve_svg(
                                self.device,
                                self.queue,
                                key,
                                asset,
                                path,
                                self.svg_policy,
                                (&plan.composition, &draw.layer),
                            ) {
                                Ok(SvgResolved::Drawn(view) | SvgResolved::Substitute(view)) => {
                                    self.prepare_level2(encoder, r, view.texture(), false);
                                    r.view = Some(view);
                                }
                                Ok(SvgResolved::Missing) => {}
                                Err(message) => {
                                    self.fail(plan, draw, MediaFailure::Failed(message))
                                }
                            }
                        } else {
                            match self
                                .vector
                                .resolve(self.device, self.queue, v, self.precision)
                            {
                                Ok(view) => {
                                    // Level 2 content on a plane is resampled
                                    // (`crate::space25d::local`).
                                    self.prepare_level2(encoder, r, view.texture(), false);
                                    r.view = Some(view);
                                }
                                Err(message) => {
                                    self.fail(plan, draw, MediaFailure::Failed(message))
                                }
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
                        ) {
                            Ok(view) => {
                                self.prepare_level2(encoder, r, view.texture(), false);
                                r.view = Some(view);
                            }
                            Err(message) => self.fail(plan, draw, MediaFailure::Failed(message)),
                        }
                    }
                    Source::Blender(blender) => {
                        r.view = Some(crate::blender::gpu::upload(
                            self.device,
                            self.queue,
                            &blender.surface,
                            self.precision,
                            blender.compatibility,
                        ));
                        // Premultiplied E like the native 3D output: linear
                        // light decodes it (section 9).
                        r.encoded = true;
                    }
                    Source::Shader(shader) => match self.shader_source(encoder, shader) {
                        Ok((target, pooled)) => {
                            // Level 2 shader output is resampled.
                            self.prepare_level2(encoder, r, &target.texture, false);
                            // Legacy GLSL targets are in GL row order
                            // and hold E-encoded values.
                            r.view = Some(target.view.clone());
                            r.from_fbo = true;
                            r.encoded = true;
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
                        if !self.scene3d.prepare_svg_maps(
                            self.device,
                            self.queue,
                            encoder,
                            object,
                            &mut self.vector.svg,
                            &plan.composition,
                        ) {
                            return;
                        }
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
                                r.view = Some(out.color.view.clone());
                                r.from_fbo = true;
                                // E-encoded: linear light decodes it.
                                r.encoded = true;
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

    /// The mip pyramid a level 2 draw samples (`crate::sampling`).
    fn prepare_level2(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        r: &mut Resolved,
        texture: &wgpu::Texture,
        still: bool,
    ) {
        if r.level2.is_some() {
            r.pyramid = Some(self.pipelines.resample.pyramid(
                self.device,
                encoder,
                texture,
                still,
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
        let level2: Vec<Option<(wgpu::BindGroup, [f64; 6])>> = resolved
            .iter()
            .map(|r| {
                let (Some(sampling), Some(pyramid)) = (r.level2, r.pyramid.as_ref()) else {
                    return None;
                };
                let matrix = r
                    .plane
                    .unwrap_or_else(|| crate::sampling::gpu::affine3(&r.matrix));
                let placement = crate::sampling::gpu::placement(matrix, r.uv)?;
                let bounds = placement.bounds_matrix(fw, fh)?;
                let origin = region.map_or((0, 0), |g| (g.x, g.y));
                let u = placement.uniform(
                    r.uv,
                    r.alpha,
                    r.from_fbo,
                    sampling,
                    origin,
                    target.height(),
                    pyramid,
                );
                Some((
                    self.pipelines
                        .resample
                        .bind(self.device, self.queue, pyramid, &u),
                    bounds,
                ))
            })
            .collect();
        // Uniforms: one slot per draw plus the background quad.
        let slots = resolved.len() + 1;
        let mut data = vec![0u8; slots * UNIFORM_STRIDE as usize];
        let quad = |m: [f64; 6], uv: [f64; 4], color: [f64; 4]| QuadUniform {
            m: m3(&m),
            res: [fw as f32, fh as f32, 0.0, 0.0],
            uv: uv.map(|v| v as f32),
            color: color.map(|v| v as f32),
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
            if let Some((_, bounds)) = &level2[i] {
                u = crate::sampling::gpu::bounds_quad(*bounds, fw, fh);
            }
            let at = (i + 1) * UNIFORM_STRIDE as usize;
            data[at..at + size_of::<QuadUniform>()].copy_from_slice(bytes_of(&u));
        }
        let buffer = self.uniforms.take(self.device, data.len() as u64);
        self.queue.write_buffer(&buffer, 0, &data);
        let quad_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm quads"),
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
                r.view.as_ref().map(|view| {
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pm texture"),
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
            pass.set_bind_group(0, &quad_bind, &[0]);
            pass.set_bind_group(1, fill, &[]);
            pass.draw(0..4, 0..1);
        }
        for (i, r) in resolved.iter().enumerate() {
            let offset = ((i + 1) as u64 * UNIFORM_STRIDE) as u32;
            if r.level2.is_some() && r.solid.is_none() {
                // Level 2: the resampler, or nothing when no part of the
                // content is in front of the camera and in the frame.
                if let Some((bind, _)) = &level2[i] {
                    pass.set_pipeline(self.pipelines.resample.pipeline(format, blend));
                    pass.set_bind_group(0, &quad_bind, &[offset]);
                    pass.set_bind_group(1, bind, &[]);
                    pass.draw(0..4, 0..1);
                }
            } else if r.solid.is_some() {
                pass.set_pipeline(self.pipelines.solid_with(format, blend));
                pass.set_bind_group(0, &quad_bind, &[offset]);
                pass.draw(0..4, 0..1);
            } else if let Some(bind) = &texture_binds[i] {
                pass.set_pipeline(self.pipelines.draw_with(format, blend));
                pass.set_bind_group(0, &quad_bind, &[offset]);
                pass.set_bind_group(1, bind, &[]);
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
                    size: wgpu::BufferSize::new(size_of::<QuadUniform>() as u64),
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
    pub fn flip(&self, encoder: &mut wgpu::CommandEncoder, source: &Target, target: &Target) {
        let (w, h) = (target.width(), target.height());
        let u = QuadUniform {
            m: m3(&[f64::from(w), 0.0, 0.0, f64::from(h), 0.0, 0.0]),
            res: [w as f32, h as f32, 0.0, 0.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [0.0; 4],
        };
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm flip"),
            size: UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, bytes_of(&u));
        let quad = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm flip"),
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
            label: Some("pm flip source"),
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
        pass.set_bind_group(0, &quad, &[0]);
        pass.set_bind_group(1, &tex, &[]);
        pass.draw(0..4, 0..1);
    }
}
