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
        plan.draws
            .iter()
            .map(|draw| {
                let m = draw.matrix;
                let at = |x: f64, y: f64| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
                LayerQuad {
                    layer: draw.layer.clone(),
                    corners: [at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)],
                }
            })
            .collect()
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

/// The renderer.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Pipelines,
    media: MediaCache,
    pool: TargetPool,
    uniforms: crate::exec::UniformPool,
    scene3d: Scene3dRenderer,
    evaluator: Evaluator,
    bracket: Option<Arc<dyn CommandBracket>>,
    /// Prepares and runs layer effects (`composite::EffectHost`).
    effects: Option<Arc<dyn EffectHost>>,
}

impl Renderer {
    /// A renderer on a device. The device needs
    /// `pm_gpu::device::required_features()`; 32-bit precision also needs
    /// `FLOAT32_FILTERABLE` and `FLOAT32_BLENDABLE` (`pm_gpu::Gpu` requests
    /// them when present).
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Renderer {
        let pipelines = Pipelines::new(&device);
        let media = MediaCache::new(&device);
        Renderer {
            pipelines,
            media,
            pool: TargetPool::new(),
            uniforms: crate::exec::UniformPool::default(),
            scene3d: Scene3dRenderer::default(),
            evaluator: Evaluator::new().with_expressions(Arc::new(PmExprEngine)),
            device,
            queue,
            bracket: None,
            effects: None,
        }
    }

    /// Runs layer effects and adjustment layers through `host` (the effects
    /// lane's runner); `None` routes compositions with effects.
    pub fn set_effect_host(&mut self, host: Option<Arc<dyn EffectHost>>) {
        self.effects = host;
    }

    /// Records `bracket` around every following frame's commands (`None`:
    /// nothing).
    pub fn set_bracket(&mut self, bracket: Option<Arc<dyn CommandBracket>>) {
        self.bracket = bracket;
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

    /// Bytes of intermediate targets held by the pool.
    pub fn pooled_bytes(&self) -> u64 {
        self.pool.bytes()
    }

    /// Keeps translated 3D programs in `dir` across processes (amendment
    /// 11's disk program cache); the engine host passes its cache
    /// directory.
    pub fn set_program_cache_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.scene3d.set_program_cache(dir);
    }

    /// 3D layers render without MSAA when `false` (only for the parity
    /// lane's classification of MSAA edge pixels, section 25; output always
    /// uses today's 4 samples).
    pub fn set_3d_msaa(&mut self, on: bool) {
        self.scene3d
            .set_samples(if on { crate::scene3d::SAMPLES } else { 1 });
    }

    /// Frees idle intermediate targets down to `bytes` (lease pressure).
    pub fn trim(&mut self, bytes: u64) {
        self.pool.trim(bytes);
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
        let (mut plan, build) = plan::build_with_effects(
            &mut self.evaluator,
            &comp,
            &project,
            seconds,
            w,
            h,
            scale,
            assets,
            self.effects.as_deref(),
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm frame"),
            });
        if let Some(bracket) = &self.bracket {
            bracket.begin(&mut encoder);
        }
        let (legend, taken, unsupported, exec_failures) = {
            self.uniforms.reset();
            let mut exec = Exec {
                device: &self.device,
                queue: &self.queue,
                pipelines: &self.pipelines,
                uniforms: &mut self.uniforms,
                media: &mut self.media,
                pool: &mut self.pool,
                scene3d: &mut self.scene3d,
                precision,
                working: color.working,
                blend_domain: color.blend_domain,
                export: request.export,
                depth_id: depth_id.is_some(),
                taken: Vec::new(),
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
            let complete = exec.unsupported.is_empty();
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
            (legend, exec.taken, exec.unsupported, exec.failures)
        };
        if let Some(bracket) = &self.bracket {
            bracket.end(&mut encoder, &target.view);
        }
        self.queue.submit([encoder.finish()]);
        for t in taken {
            self.pool.give(t);
        }
        self.media.flush();
        failures.extend(exec_failures);
        if !unsupported.is_empty() {
            return Err(RenderError::Unsupported(unsupported));
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
        let (plan, build) = plan::build_with_effects(
            &mut self.evaluator,
            &comp,
            &project,
            seconds,
            w,
            h,
            scale,
            assets,
            self.effects.as_deref(),
        );
        if !build.unsupported.is_empty() {
            return Err(RenderError::Unsupported(build.unsupported));
        }
        Ok((plan, build.failures))
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
