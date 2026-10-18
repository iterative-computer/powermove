//! The headless render API: (document, composition, time, request) →
//! texture or readback. One [`Renderer`] per engine (interactive or job);
//! it keeps compiled pipelines, the target pool, media caches, and the
//! evaluator's caches across frames.

use std::sync::Arc;

use pm_eval::{Comp, Evaluator};
use pm_gpu::{Target, TargetPool, working_format};
use pm_model::Document;
use pm_protocol::color::Precision;
use pm_protocol::common::Ticks;

use crate::assets::AssetResolver;
use crate::capability::{self, Feature, Unsupported};
use crate::composite::{ColorSettings, EffectHost};
use crate::effects::Effects;
use crate::error::{LayerFailure, RenderError};
use crate::exec::{Exec, Region};
use crate::expr::PmExprEngine;
use crate::media::MediaCache;
use crate::pipelines::Pipelines;
use crate::plan::{self, Background, CompPlan};
use crate::request::RenderRequest;
use crate::scene3d::Scene3dRenderer;

/// A rendered frame.
pub struct Frame {
    /// Premultiplied working values (Standard: E-encoded, Rec.709
    /// primaries), the request's precision, the region's size.
    pub color: Target,
    /// The depth and object-id surface (`rgba16float`), when requested.
    pub depth_id: Option<Target>,
    /// Top-level layer ids by id surface index (id − 1).
    pub legend: Vec<String>,
    /// Layers that failed and drew nothing.
    pub failures: Vec<LayerFailure>,
    /// The composition time rendered, in seconds.
    pub seconds: f64,
}

/// One layer's text raster ([`Renderer::text_raster`]).
pub struct TextRaster {
    /// Premultiplied working values of the plan's crop, top-down.
    pub pixels: Vec<f32>,
    /// The whole canvas in device pixels (`plan.crop` is a part of it).
    pub canvas: (u32, u32),
    /// The crop's quad in layer pixels and its anchor.
    pub box_size: (f64, f64),
    pub anchor: (f64, f64),
    pub plan: crate::text::TextPlan,
}

/// A pass-through candidate ([`Renderer::pass_through`]).
#[derive(Debug, Clone, PartialEq)]
pub struct PassThrough {
    pub path: std::path::PathBuf,
    /// Source time in seconds (the frame is selected by section 5 rule 7).
    pub seconds: f64,
    pub facts: pm_color::chroma::FrameFacts,
}

/// What a render into a caller's target reports.
pub struct Rendered {
    pub legend: Vec<String>,
    pub failures: Vec<LayerFailure>,
    pub seconds: f64,
    /// Where each top-level layer was drawn: its content quad in output
    /// pixels, top-down, corners in the order (0,0), (1,0), (1,1), (0,1)
    /// of the layer's unit quad (the viewer's overlay geometry, section 18
    /// item 6). A layer drawn as several quads (a flattened group) reports
    /// each of them.
    pub quads: Vec<LayerQuad>,
}

/// One drawn quad of a top-level layer ([`Rendered::quads`]).
#[derive(Debug, Clone, PartialEq)]
pub struct LayerQuad {
    pub layer: String,
    pub corners: [[f64; 2]; 4],
}

impl LayerQuad {
    /// The quads of a plan's top-level draws, in output pixels top-down.
    /// Plan matrices map the unit quad in the frame's own (top-down)
    /// composition space scaled to output pixels.
    pub fn of_plan(plan: &CompPlan) -> Vec<LayerQuad> {
        let mut out = Vec::new();
        for draw in &plan.draws {
            quads_of(draw, &mut out);
        }
        out
    }
}

fn quads_of(draw: &plan::Draw, out: &mut Vec<LayerQuad>) {
    match &draw.content {
        // A 2.5D layer: its projected quad.
        plan::Content::Placed(p) if p.plane.is_some() => {
            out.push(LayerQuad {
                layer: draw.layer.clone(),
                corners: crate::space25d::corners(p.plane.as_ref().expect("checked")),
            });
        }
        // A motion-blurred layer: where its samples were drawn.
        plan::Content::Texture {
            source: plan::Source::Accumulate(acc),
            ..
        } => {
            let start = out.len();
            for d in &acc.draws {
                quads_of(d, out);
            }
            for q in &mut out[start..] {
                q.layer.clone_from(&draw.layer);
            }
        }
        _ => {
            let m = draw.matrix;
            let at = |x: f64, y: f64| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
            out.push(LayerQuad {
                layer: draw.layer.clone(),
                corners: [at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)],
            });
        }
    }
}

/// Commands recorded around each frame's own commands, in the frame's
/// command buffer: the job engine's GPU timestamps (contract 13), which must
/// not count the time the GPU waits for the frame to be encoded.
pub trait CommandBracket: Send + Sync {
    /// Recorded first.
    fn begin(&self, encoder: &mut wgpu::CommandEncoder);
    /// Recorded last; `output` is the frame's target, written by then.
    fn end(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView);
}

/// The motion blur and resampler a request asks of its root composition.
fn build_options(
    comp: &Comp<'_>,
    request: &RenderRequest,
    shader_fallback: bool,
) -> plan::BuildOptions {
    plan::BuildOptions {
        blur: crate::motion::resolve(comp, request.motion_blur),
        sampling: request.sampling,
        shader_fallback,
        video_end: request.video_end,
    }
}

/// The renderer.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Pipelines,
    media: MediaCache,
    pool: TargetPool,
    uniforms: crate::exec::UniformPool,
    scene3d: Scene3dRenderer,
    /// The 3D pass's planning options (mesh definitions; the view override
    /// comes with each request).
    options3d: crate::scene3d::Options3d,
    vector: crate::vector::cache::VectorCache,
    pyramids: crate::sampling::gpu::PyramidCache,
    evaluator: Evaluator,
    bracket: Option<Arc<dyn CommandBracket>>,
    /// Effect, transition, and layer definitions and their programs
    /// (`crate::effects`, contract 10): the renderer's own effect host.
    effects: Effects,
    /// Another effect host in place of the renderer's own (tests).
    effect_host: Option<Arc<dyn EffectHost>>,
    /// Shader-layer programs and the decision 19 fallback runner's state
    /// (`crate::shader_layers`).
    shaders: crate::shader_layers::exec::ShaderState,
    /// Mesh definitions given with [`Renderer::set_mesh_definitions`]; the
    /// definition set's mesh snapshots are added to them.
    explicit_meshes: std::collections::HashMap<String, crate::scene3d::legacy::MeshDefinition>,
    text: crate::text::TextContext,
    text_gpu: crate::text::gpu::TextGpu,
}

impl Renderer {
    /// A renderer on a device. The device needs
    /// `pm_gpu::device::required_features()`; 32-bit precision also needs
    /// `FLOAT32_FILTERABLE` and `FLOAT32_BLENDABLE` (`pm_gpu::Gpu` requests
    /// them when present).
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Renderer {
        let pipelines = Pipelines::new(&device);
        let media = MediaCache::new(&device);
        let text_gpu = crate::text::gpu::TextGpu::new(&device);
        Renderer {
            text: crate::text::TextContext::default(),
            text_gpu,
            pipelines,
            media,
            pool: TargetPool::new(),
            uniforms: crate::exec::UniformPool::default(),
            scene3d: Scene3dRenderer::default(),
            options3d: crate::scene3d::Options3d::default(),
            vector: crate::vector::cache::VectorCache::new(),
            pyramids: crate::sampling::gpu::PyramidCache::new(),
            evaluator: Evaluator::new().with_expressions(Arc::new(PmExprEngine)),
            device,
            queue,
            bracket: None,
            effects: Effects::new(format!("{}; wgpu 30", std::env::consts::OS)),
            effect_host: None,
            shaders: Default::default(),
            explicit_meshes: Default::default(),
        }
    }

    /// Runs layer effects and adjustment layers through `host` instead of
    /// the renderer's definitions; `None` goes back to them.
    pub fn set_effect_host(&mut self, host: Option<Arc<dyn EffectHost>>) {
        self.effect_host = host;
    }

    /// Replaces the effect, transition, and layer definitions with an
    /// embedded set (an export job's, a collected project's, `pm-cli`'s;
    /// contract 10) and translates what changed. Returns per-definition
    /// failures. Without definitions, layers with effects or transitions
    /// are routed.
    pub fn set_definitions(
        &mut self,
        set: &pm_protocol::definitions::DefinitionSetSnapshot,
    ) -> Result<Vec<pm_protocol::error::EngineError>, pm_protocol::definitions::UpdateError> {
        let out = self.effects.set_snapshot(set);
        self.refresh_meshes();
        out
    }

    /// Applies a kernel update (`pushDefinitions`); the failures are
    /// `DefinitionsApplied.failures`. In force for every later frame.
    pub fn apply_definitions(
        &mut self,
        update: &pm_protocol::definitions::DefinitionsUpdate,
    ) -> Result<Vec<pm_protocol::error::EngineError>, pm_protocol::definitions::UpdateError> {
        let out = self.effects.apply(update);
        self.refresh_meshes();
        out
    }

    /// Forgets every definition: effects and transitions are routed again.
    pub fn clear_definitions(&mut self) {
        self.effects.clear();
        self.refresh_meshes();
    }

    /// The mesh layer definitions the 3D pass plans with: the built-in OBJ
    /// model layer, [`Renderer::set_mesh_definitions`], and every mesh
    /// snapshot of the definition set (contract 10: `renderer.assetField`
    /// and the parameter defaults), the set's winning over the others.
    fn refresh_meshes(&mut self) {
        let mut all = crate::scene3d::legacy::builtin_mesh_definitions();
        all.extend(self.explicit_meshes.clone());
        all.extend(crate::shader_layers::mesh_definitions(&self.effects));
        self.options3d.mesh_definitions = Arc::new(all);
    }

    /// Whether a decision 19 fallback runner (a WebGL2 context in the
    /// renderer) draws programs the native pipeline cannot translate. With
    /// one, such layers plan as fallback requests and a frame without their
    /// pixels fails with [`RenderError::ShaderFallbackPending`]; without
    /// one (the default, every headless host) they route as
    /// `Feature::ShaderFallback`.
    pub fn set_shader_fallback_runner(&mut self, attached: bool) {
        self.shaders.fallback_runner = attached;
        if !attached {
            self.shaders.fallback.clear();
        }
    }

    /// Stores the runner's pixels for a request key
    /// (`crate::shader_layers::FallbackRequest::key`).
    pub fn put_fallback_pixels(
        &mut self,
        key: String,
        pixels: crate::shader_layers::FallbackPixels,
    ) {
        self.shaders.fallback.insert(key, pixels);
    }

    /// What the shader UI is told about the shader and extension layers of
    /// `composition` and the compositions it nests, at the document's
    /// revision (`crate::shader_layers::reports`): shader layers whose code
    /// does not compile and extension layers whose definition is missing.
    pub fn layer_reports(
        &self,
        document: &pm_model::Document,
        composition: &str,
    ) -> Vec<pm_protocol::error::EngineError> {
        let meshes = self.options3d.mesh_definitions.clone();
        crate::shader_layers::layer_reports(
            document,
            composition,
            effect_host(&self.effect_host, &self.effects).and_then(|e| e.layer_definitions()),
            &|id| meshes.contains_key(id),
        )
    }

    /// Shader-layer programs built and fallback pixels held.
    pub fn shader_state(&self) -> &crate::shader_layers::exec::ShaderState {
        &self.shaders
    }

    /// The definitions in force (what an export job embeds).
    pub fn definitions(&self) -> pm_protocol::definitions::DefinitionSetSnapshot {
        self.effects.snapshot()
    }

    pub fn effects(&self) -> &Effects {
        &self.effects
    }

    pub fn effects_mut(&mut self) -> &mut Effects {
        &mut self.effects
    }

    /// Swaps in another definition set (one per view of a shared renderer:
    /// each keeps its programs and watchdog state); returns the previous.
    pub fn swap_effects(&mut self, effects: &mut Effects) {
        std::mem::swap(&mut self.effects, effects);
    }

    /// Runs `f` with an effect context over the renderer's own definitions,
    /// then submits and gives the intermediates back.
    fn with_gpu<T>(
        &mut self,
        label: &str,
        precision: Precision,
        full: (u32, u32),
        f: impl FnOnce(&mut crate::effects::exec::Gpu<'_>) -> T,
    ) -> (T, Vec<Unsupported>) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        let mut taken = Vec::new();
        let mut unsupported = Vec::new();
        let out = {
            let mut cx = crate::composite::effects::EffectContext {
                device: &self.device,
                queue: &self.queue,
                encoder: &mut encoder,
                pool: &mut self.pool,
                precision,
                working: pm_protocol::color::WorkingEncoding::Standard,
                full,
                taken: &mut taken,
                native: Some(crate::effects::exec::Native {
                    effects: &mut self.effects,
                    timer: None,
                    linear: &self.pipelines.linear,
                    unsupported: &mut unsupported,
                }),
            };
            let mut gpu = crate::effects::exec::Gpu::of(&mut cx).expect("native context");
            f(&mut gpu)
        };
        self.queue.submit([encoder.finish()]);
        for t in taken {
            self.pool.give(t);
        }
        (out, unsupported)
    }

    /// Runs effect pass nodes on `input` as the compositor's `runEffects`
    /// does (harnesses and the shader parity tests): every buffer the size
    /// of `input`, in GL row order, E-encoded. `region` places the buffer in
    /// a level of `full` pixels (`effectViewportCoordinates`); `backdrop` is
    /// `u_backdrop` (an empty unit without one). Returns a new working
    /// target.
    #[allow(clippy::too_many_arguments)]
    pub fn run_effects(
        &mut self,
        input: &Target,
        backdrop: Option<&Target>,
        steps: &[crate::effects::plan::EffectStep],
        time: f64,
        region: Option<(u32, u32)>,
        full: (u32, u32),
        precision: Precision,
    ) -> Result<Target, RenderError> {
        let (w, h) = (input.width(), input.height());
        let region = region.map(|(x, y)| Region {
            x,
            y,
            width: w,
            height: h,
        });
        let coordinates = crate::effects::exec::effect_coordinates(region, full, (w, h));
        let (out, unsupported) = self.with_gpu("pm effects", precision, full, |gpu| {
            gpu.chain(
                "",
                steps,
                time,
                input,
                backdrop,
                coordinates,
                pm_protocol::color::WorkingEncoding::Standard,
            )
            .unwrap_or_else(|| gpu.copy_of(input))
        });
        if !unsupported.is_empty() {
            return Err(RenderError::Unsupported(unsupported));
        }
        Ok(out)
    }

    /// Runs a transition node from `from` to `to` (`runTransition`).
    pub fn run_transition(
        &mut self,
        from: &Target,
        to: &Target,
        step: &crate::effects::plan::TransitionStep,
        time: f64,
        precision: Precision,
    ) -> Result<Target, RenderError> {
        let mut step = step.clone();
        step.edge_in = true;
        let full = (from.width(), from.height());
        let (out, unsupported) = self.with_gpu("pm transition", precision, full, |gpu| {
            gpu.run_transition("", &step, from, to, time)
        });
        match out {
            Some(t) if unsupported.is_empty() => Ok(t),
            _ => Err(RenderError::Unsupported(unsupported)),
        }
    }

    /// Reads finished watchdog timestamps (`wait`: every submitted frame)
    /// and returns the overruns reported since the last call.
    pub fn watchdog_reports(&mut self, wait: bool) -> Vec<pm_protocol::error::EngineError> {
        self.effects
            .watchdog
            .collect(&self.device, &self.queue, wait);
        self.effects.watchdog.take_reports()
    }

    /// Records `bracket` around every following frame's commands (`None`:
    /// nothing).
    pub fn set_bracket(&mut self, bracket: Option<Arc<dyn CommandBracket>>) {
        self.bracket = bracket;
    }

    /// The capability check for this renderer: `unsupported_with_media`,
    /// with text and captions drawn when fonts are configured.
    pub fn unsupported_with_media(
        &self,
        document: &Document,
        composition: &str,
        assets: &dyn AssetResolver,
    ) -> Vec<Unsupported> {
        if self.text.has_fonts() {
            capability::unsupported_with_fonts(document, composition, Some(assets))
        } else {
            capability::unsupported_with_media(document, composition, assets)
        }
    }

    /// The fonts text and captions layers lay out with (`None`: those
    /// layers are unsupported and route to the legacy renderer). Dropping
    /// or replacing them clears the glyph and raster caches.
    pub fn set_text_fonts(&mut self, fonts: Option<Box<dyn crate::text::TextFonts>>) {
        self.text.set_fonts(fonts);
        self.text_gpu.clear();
    }

    /// Text layout and rendering state (profiles, the captions switch).
    pub fn text(&mut self) -> &mut crate::text::TextContext {
        &mut self.text
    }

    /// The GPU side of text rendering (coverage source, cache, counters).
    pub fn text_gpu(&mut self) -> &mut crate::text::gpu::TextGpu {
        &mut self.text_gpu
    }

    /// `PM.captionsHidden` for the following frames: an export without
    /// burned-in captions (`captionsBurn: false`) draws no captions layer.
    pub fn set_captions_hidden(&mut self, hidden: bool) {
        self.text.set_captions_hidden(hidden);
    }

    /// A renderer on its own device (`pm_gpu::Gpu::new`).
    pub fn headless() -> Result<Renderer, RenderError> {
        let gpu = pm_gpu::Gpu::new("pm-render")?;
        Ok(Renderer::new(gpu.device, gpu.queue))
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Bytes of intermediate targets held by the pool and of the kept mip
    /// pyramids of level 2 stills.
    pub fn pooled_bytes(&self) -> u64 {
        self.pool.bytes() + self.pyramids.bytes()
    }

    /// Keeps translated 3D programs (amendment 11's disk program cache) and
    /// translated GLSL (`dir/glsl`, plan section 11) in `dir` across
    /// processes; the engine host passes its shader cache folder.
    pub fn set_program_cache_dir(&mut self, dir: Option<std::path::PathBuf>) {
        // Translated GLSL (`pm_glsl::Cache`: source hash and device) in its
        // own folder; definition sets made before keep their cache.
        self.effects
            .set_cache_dir(dir.as_ref().map(|d| d.join("glsl")));
        // Shader layers share the folder (entries are content-addressed).
        crate::shader_layers::translate::set_cache_dir(dir.as_ref().map(|d| d.join("glsl")));
        self.scene3d.set_program_cache(dir);
    }

    /// GPU meshes and material maps the 3D pass keeps between frames
    /// (released once no 3D frame draws them).
    pub fn scene3d_resident(&self) -> crate::scene3d::Resident3d {
        self.scene3d.resident()
    }

    /// Definition snapshots of mesh layers (`renderer.kind === 'mesh'`) by
    /// definition id, so legacy OBJ mesh layers render natively. The
    /// built-in OBJ model layer is always known; a snapshot for its id
    /// replaces it.
    pub fn set_mesh_definitions(
        &mut self,
        definitions: std::collections::HashMap<String, crate::scene3d::legacy::MeshDefinition>,
    ) {
        self.explicit_meshes = definitions;
        self.refresh_meshes();
    }

    /// 3D layers render without MSAA when `false` (only for the parity
    /// lane's classification of MSAA edge pixels, section 25; output always
    /// uses today's 4 samples).
    pub fn set_3d_msaa(&mut self, on: bool) {
        self.scene3d
            .set_samples(if on { crate::scene3d::SAMPLES } else { 1 });
    }

    /// Frees idle intermediate targets and cached vector textures so that
    /// together they hold at most `bytes` (lease pressure): the pool trims
    /// first, and the vector cache gets what the pool leaves. The text
    /// raster cache is kept apart from the pool and trims to `bytes` on its
    /// own (`text_gpu().cached_bytes()`).
    pub fn trim(&mut self, bytes: u64) {
        self.pool.trim(bytes);
        if self.pool.bytes() + self.pyramids.bytes() > bytes {
            self.pyramids.clear();
        }
        self.vector
            .trim(bytes.saturating_sub(self.pool.bytes() + self.pyramids.bytes()));
        self.text_gpu.trim(bytes);
    }

    /// The vector cache's standing bound, enforced while frames render
    /// (default: today's raster cache, 192 MiB and 4096 entries).
    pub fn set_vector_cache_budget(&mut self, max_bytes: u64, max_entries: usize) {
        self.vector.set_budget(max_bytes, max_entries);
    }

    /// Cached vector textures.
    pub fn vector_cache_len(&self) -> usize {
        self.vector.len()
    }

    /// Bytes of cached vector textures (shape rasters and level 2 scenes):
    /// the render cache category of the lease.
    pub fn vector_cache_bytes(&self) -> u64 {
        self.vector.bytes()
    }

    /// Selects the level 2 coverage backend (Vello on the GPU with the
    /// `vello_cpu` fallback, or `vello_cpu` always).
    pub fn set_vector_backend(&mut self, backend: crate::vector::cache::CoverageBackend) {
        self.vector.set_backend(backend);
    }

    /// The backend that rendered the last level 2 scene.
    pub fn last_vector_backend(&self) -> Option<&'static str> {
        self.vector.last_backend
    }

    fn supports_precision(&self, precision: Precision) -> bool {
        precision == Precision::Float16
            || self
                .device
                .features()
                .contains(wgpu::Features::FLOAT32_FILTERABLE | wgpu::Features::FLOAT32_BLENDABLE)
    }

    /// Renders `composition` at `time` into a new texture.
    pub fn render(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        assets: &dyn AssetResolver,
    ) -> Result<Frame, RenderError> {
        let size = request.target_size();
        let precision = match document.root().map(project_settings) {
            Some(Ok(c)) => effective_precision(request.precision, c.precision),
            _ => request.precision,
        };
        let target = Target::new(
            &self.device,
            "pm frame",
            size.width,
            size.height,
            working_format(precision),
            pm_gpu::target::working_usage(),
        );
        let depth_id = request.depth_id.then(|| {
            Target::new(
                &self.device,
                "pm depth and id",
                size.width,
                size.height,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
            )
        });
        let r = self.render_into(
            document,
            composition,
            time,
            request,
            assets,
            &target,
            depth_id.as_ref(),
        )?;
        Ok(Frame {
            color: target,
            depth_id,
            legend: r.legend,
            failures: r.failures,
            seconds: r.seconds,
        })
    }

    /// Renders into a caller's target (the presenter's working texture, an
    /// export buffer): the region's size, a working format
    /// (`rgba16float`, `rgba32float`), drawable. `depth_id` receives the
    /// depth and id surface when given (`rgba16float`, the same size).
    /// On [`RenderError::Unsupported`] the targets are left untouched.
    #[allow(clippy::too_many_arguments)]
    pub fn render_into(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        assets: &dyn AssetResolver,
        target: &Target,
        depth_id: Option<&Target>,
    ) -> Result<Rendered, RenderError> {
        request.validate()?;
        let root = document
            .root()
            .ok_or_else(|| RenderError::Composition(composition.to_owned()))?;
        let color = project_settings(root)?;
        let precision = effective_precision(request.precision, color.precision);
        if !self.supports_precision(precision) {
            return Err(RenderError::Unsupported(vec![Unsupported::request(
                "32-bit precision on a device without rgba32float filtering and blending",
            )]));
        }
        let size = request.target_size();
        if target.width() != size.width || target.height() != size.height {
            return Err(RenderError::Request(format!(
                "target is {}×{}, the request needs {}×{}",
                target.width(),
                target.height(),
                size.width,
                size.height
            )));
        }
        let comp = Comp::resolve(document, composition)
            .map_err(|_| RenderError::Composition(composition.to_owned()))?;
        let open = capability::open_composition_id(root);
        let project = open
            .as_deref()
            .and_then(|id| Comp::resolve(document, id).ok())
            .unwrap_or_else(|| comp.clone());
        let seconds = comp.seconds_at(time.get());
        let (w, h) = (request.output_size.width, request.output_size.height);
        let scale = request
            .scale
            .unwrap_or_else(|| plan::derived_scale(&comp, w, h));
        // Overruns measured on earlier frames disable their effects now.
        self.effects
            .watchdog
            .collect(&self.device, &self.queue, false);
        let (mut plan, build) = plan::build_with(
            &mut self.evaluator,
            &comp,
            &project,
            seconds,
            w,
            h,
            scale,
            assets,
            effect_host(&self.effect_host, &self.effects),
            self.options3d.for_request(request),
            Some(&mut self.text),
            build_options(&comp, request, self.shaders.fallback_runner),
        );
        if !request.background {
            plan.background = Background::Transparent;
        }
        let quads = LayerQuad::of_plan(&plan);
        let mut failures = build.failures;
        if !build.unsupported.is_empty() {
            return Err(RenderError::Unsupported(build.unsupported));
        }
        let region = request.region.map(|r| Region {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        });
        // The previous frame's vector textures become evictable, and the
        // cache returns within its bound; 3D meshes and maps no recent 3D
        // frame drew are released.
        self.vector.begin_frame();
        self.scene3d.begin_frame();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm frame"),
            });
        if let Some(bracket) = &self.bracket {
            bracket.begin(&mut encoder);
        }
        // Only renderers drawing their own definitions time effects.
        let timer = if self.effects.configured() && self.effect_host.is_none() {
            self.effects.watchdog.frame_timer(&self.device)
        } else {
            None
        };
        let (legend, taken, unsupported, exec_failures, timer, pending) = {
            self.uniforms.reset();
            let mut exec = Exec {
                device: &self.device,
                queue: &self.queue,
                pipelines: &self.pipelines,
                uniforms: &mut self.uniforms,
                media: &mut self.media,
                pool: &mut self.pool,
                scene3d: &mut self.scene3d,
                effects: &mut self.effects,
                timer,
                vector: &mut self.vector,
                shaders: &mut self.shaders,
                fallback_pending: Vec::new(),
                text: &mut self.text_gpu,
                precision,
                working: color.working,
                blend_domain: color.blend_domain,
                export: request.export,
                depth_id: depth_id.is_some(),
                taken: Vec::new(),
                pyramids: &mut self.pyramids,
                failures: Vec::new(),
                unsupported: Vec::new(),
            };
            // Frames are drawn in GL row order and flipped once into the
            // caller's top-down target (see shaders/composite.wgsl).
            let gl_rows = exec.pool.take(
                exec.device,
                size.width,
                size.height,
                target.format(),
                pm_gpu::target::working_usage(),
            );
            let resolved = exec.render_comp(&mut encoder, &plan, region, &gl_rows);
            // Something met at exec time is unsupported (media facts the
            // plan could not see): the frame fails, and the caller's
            // targets keep their previous contents. The commands still run,
            // so media caches filled by this encoder stay valid.
            let complete = exec.unsupported.is_empty() && exec.fallback_pending.is_empty();
            if complete {
                exec.flip_color(&mut encoder, &gl_rows, target);
            }
            exec.taken.push(gl_rows);
            if let Some(ids) = depth_id.filter(|_| complete) {
                let id_rows = exec.pool.take(
                    exec.device,
                    size.width,
                    size.height,
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                );
                exec.render_ids(
                    &mut encoder,
                    &resolved,
                    region,
                    (plan.width, plan.height),
                    &id_rows,
                );
                exec.flip(&mut encoder, &id_rows, ids);
                exec.taken.push(id_rows);
            }
            let legend: Vec<String> = resolved.iter().map(|r| r.layer.clone()).collect();
            let timer = exec.timer.take();
            let pending = std::mem::take(&mut exec.fallback_pending);
            (
                legend,
                exec.taken,
                exec.unsupported,
                exec.failures,
                timer,
                pending,
            )
        };
        if let Some(timer) = timer {
            self.effects.watchdog.finish(&mut encoder, timer);
        }
        if let Some(bracket) = &self.bracket {
            bracket.end(&mut encoder, &target.view);
        }
        self.queue.submit([encoder.finish()]);
        self.effects.watchdog.submitted();
        for t in taken {
            self.pool.give(t);
        }
        self.media.flush();
        failures.extend(exec_failures);
        if !unsupported.is_empty() {
            return Err(RenderError::Unsupported(unsupported));
        }
        if !pending.is_empty() {
            return Err(RenderError::ShaderFallbackPending(
                pending.iter().map(|r| (**r).clone()).collect(),
            ));
        }
        Ok(Rendered {
            quads,
            legend,
            failures,
            seconds,
        })
    }

    /// "Interpret Footage" for a video file: read it with `color` (its
    /// primaries, transfer, matrix, siting, alpha) instead of the probe's
    /// reading; `None` restores the probe's.
    pub fn interpret(
        &mut self,
        path: &std::path::Path,
        color: Option<pm_protocol::color::ColorDescription>,
    ) {
        self.media.interpret(path, color);
    }

    /// The color reading the renderer uses for a video file.
    pub fn video_color(
        &mut self,
        path: &std::path::Path,
    ) -> Result<pm_protocol::color::ColorDescription, RenderError> {
        self.media.video_color(path).map_err(|e| match e {
            crate::media::MediaFailure::Unsupported(f) => {
                RenderError::Unsupported(vec![Unsupported {
                    composition: None,
                    layer: None,
                    feature: f,
                }])
            }
            crate::media::MediaFailure::Failed(m) => RenderError::Media(m),
        })
    }

    /// Reads a frame's color back as premultiplied f32 RGBA, top-down.
    pub fn read_rgba(&self, frame: &Frame) -> Result<Vec<f32>, RenderError> {
        Ok(pm_gpu::readback::read_f32(
            &self.device,
            &self.queue,
            &frame.color.texture,
        )?)
    }

    /// Reads the depth and id surface back (r depth, g/b id − 1, a coverage).
    pub fn read_depth_id(&self, frame: &Frame) -> Result<Option<Vec<f32>>, RenderError> {
        frame
            .depth_id
            .as_ref()
            .map(|t| pm_gpu::readback::read_f32(&self.device, &self.queue, &t.texture))
            .transpose()
            .map_err(Into::into)
    }

    /// Section 9's pass-through candidate: when the frame is one video
    /// source drawn 1:1 over the whole output (no transform, crop,
    /// resampling, or opacity), the source, its time, and the frame facts
    /// `pm_color::chroma::pass_through` needs. The exporter then compares
    /// the source's format, range, subsampling, and siting with the output
    /// and copies its Y′CbCr planes when they match. `None` for any other
    /// frame.
    pub fn pass_through(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        assets: &dyn AssetResolver,
    ) -> Result<Option<PassThrough>, RenderError> {
        request.validate()?;
        let (plan, _) = self.plan(document, composition, time, request, assets)?;
        let [draw] = plan.draws.as_slice() else {
            return Ok(None);
        };
        let plan::Content::Texture { source, uv, alpha } = &draw.content else {
            return Ok(None);
        };
        let plan::Source::Video { path, time, .. } = source else {
            return Ok(None);
        };
        let (w, h) = (request.output_size.width, request.output_size.height);
        let size = self
            .media
            .video_size(path)
            .map_err(|e| RenderError::Media(format!("{e:?}")))?;
        let full = [f64::from(w), 0.0, 0.0, f64::from(h), 0.0, 0.0];
        let untransformed = draw.matrix == full
            && *uv == [0.0, 0.0, 1.0, 1.0]
            && size == (w, h)
            && request.region.is_none();
        Ok(Some(PassThrough {
            path: path.clone(),
            seconds: *time,
            facts: pm_color::chroma::FrameFacts {
                single_source: true,
                untransformed,
                effect_free: *alpha == 1.0,
            },
        }))
    }

    /// The canvas raster of one text or captions layer at composition time
    /// `seconds`, drawn at raster scale `scale` (what today's `PM.raster(L,
    /// scale, T)` paints), read back. `None` when the layer draws nothing
    /// (no cue showing, no ink). For parity tests, `pm-cli`, and debugging.
    pub fn text_raster(
        &mut self,
        document: &Document,
        composition: &str,
        layer: &str,
        seconds: f64,
        scale: f64,
        precision: Precision,
    ) -> Result<Option<TextRaster>, RenderError> {
        let comp = Comp::resolve(document, composition)
            .map_err(|_| RenderError::Composition(composition.to_owned()))?;
        let root = document.root().unwrap();
        let open = capability::open_composition_id(root);
        let project = open
            .as_deref()
            .and_then(|id| Comp::resolve(document, id).ok())
            .unwrap_or_else(|| comp.clone());
        if !self.text.has_fonts() {
            return Err(RenderError::Unsupported(vec![Unsupported::request(
                "text without configured fonts",
            )]));
        }
        let comp_id = comp.id().to_string_lossy();
        let mut scope = pm_eval::Scope::new(&mut self.evaluator, &comp, &project);
        let node = scope
            .layers()
            .iter()
            .find(|l| crate::js::layer_id(l) == layer)
            .ok_or_else(|| RenderError::Request(format!("no layer {layer}")))?;
        let ty = crate::js::layer_type(node);
        let quad = match ty.as_str() {
            "text" => self.text.text_layer(&mut scope, node, seconds, scale),
            "captions" => self.text.captions_layer(&mut scope, node, seconds, scale),
            other => return Err(RenderError::Request(format!("layer {layer} is {other}"))),
        };
        // What the layer could not draw is refused here, as `render` refuses
        // it, and never left for a later layer to report.
        let refused: Vec<Unsupported> = self
            .text
            .take_unsupported()
            .into_iter()
            .map(|what| Unsupported::layer(&comp_id, node, Feature::Text(what)))
            .collect();
        let quad = quad.map_err(|e| RenderError::Request(e.to_string()))?;
        if !refused.is_empty() {
            return Err(RenderError::Unsupported(refused));
        }
        let Some(quad) = quad else { return Ok(None) };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm text raster"),
            });
        let mut taken = Vec::new();
        let view = self
            .text_gpu
            .render(
                &self.device,
                &self.queue,
                &mut encoder,
                &mut self.pool,
                &mut taken,
                &quad.plan,
                precision,
            )
            .map_err(RenderError::Media)?;
        self.queue.submit([encoder.finish()]);
        for t in taken {
            self.pool.give(t);
        }
        let pixels = pm_gpu::readback::read_f32(&self.device, &self.queue, view.texture())?;
        Ok(Some(TextRaster {
            pixels,
            canvas: (quad.plan.scene.width, quad.plan.scene.height),
            box_size: (quad.w, quad.h),
            anchor: (quad.ax, quad.ay),
            plan: quad.plan,
        }))
    }

    /// Paints a text scene (the whole scene) through the GPU text path and
    /// reads it back: premultiplied working values, top-down. For the level
    /// 2 gate, review sheets, and tools that build scenes themselves.
    pub fn paint_text_scene(
        &mut self,
        scene: pm_text::render::scene::TextScene,
        precision: Precision,
    ) -> Result<Vec<f32>, RenderError> {
        let plan = crate::text::TextPlan::whole(scene);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm text scene"),
            });
        let mut taken = Vec::new();
        let view = self
            .text_gpu
            .render(
                &self.device,
                &self.queue,
                &mut encoder,
                &mut self.pool,
                &mut taken,
                &plan,
                precision,
            )
            .map_err(RenderError::Media)?;
        self.queue.submit([encoder.finish()]);
        for t in taken {
            self.pool.give(t);
        }
        Ok(pm_gpu::readback::read_f32(
            &self.device,
            &self.queue,
            view.texture(),
        )?)
    }

    /// The plan of a frame without rendering it (tests, `pm-cli parity`).
    pub fn plan(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        assets: &dyn AssetResolver,
    ) -> Result<(CompPlan, Vec<LayerFailure>), RenderError> {
        let comp = Comp::resolve(document, composition)
            .map_err(|_| RenderError::Composition(composition.to_owned()))?;
        let root = document.root().unwrap();
        let open = capability::open_composition_id(root);
        let project = open
            .as_deref()
            .and_then(|id| Comp::resolve(document, id).ok())
            .unwrap_or_else(|| comp.clone());
        let seconds = comp.seconds_at(time.get());
        let (w, h) = (request.output_size.width, request.output_size.height);
        let scale = request
            .scale
            .unwrap_or_else(|| plan::derived_scale(&comp, w, h));
        let (plan, build) = plan::build_with(
            &mut self.evaluator,
            &comp,
            &project,
            seconds,
            w,
            h,
            scale,
            assets,
            effect_host(&self.effect_host, &self.effects),
            self.options3d.for_request(request),
            Some(&mut self.text),
            build_options(&comp, request, self.shaders.fallback_runner),
        );
        if !build.unsupported.is_empty() {
            return Err(RenderError::Unsupported(build.unsupported));
        }
        Ok((plan, build.failures))
    }
}

/// The effect host plans are built with: another host when one is set,
/// else the renderer's definitions once it has any (without either, layers
/// with effects or transitions are routed).
fn effect_host<'a>(
    host: &'a Option<Arc<dyn EffectHost>>,
    effects: &'a Effects,
) -> Option<&'a dyn EffectHost> {
    match host {
        Some(h) => Some(h.as_ref()),
        None => effects.configured().then_some(effects as &dyn EffectHost),
    }
}

/// The project's color settings, or the routing answer for settings this
/// build cannot render.
fn project_settings(root: &pm_model::Node) -> Result<ColorSettings, RenderError> {
    crate::composite::project_color(root)
        .map(|c| ColorSettings::of(&c))
        .map_err(|_| {
            RenderError::Unsupported(vec![Unsupported {
                composition: None,
                layer: None,
                feature: Feature::ProjectColor,
            }])
        })
}

/// The precision intermediates use: the request's, raised to the project's
/// (Settings › Project › Color › Precision is a floor for every render of
/// the project; exports raise it further by amendment 8).
pub fn effective_precision(request: Precision, project: Precision) -> Precision {
    if request == Precision::Float32 || project == Precision::Float32 {
        Precision::Float32
    } else {
        Precision::Float16
    }
}
