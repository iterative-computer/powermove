//! Typed Studio seam. Full-scene transport is shared by all layer outputs;
//! masking, effects, opacity and blending remain ordinary compositor passes.
use pm_gpu::TargetPool;
use pm_studio::{
    extract::{SceneRequest, SceneSource},
    service::{Denoise, RenderedScene, StudioService, Tier},
};
use std::sync::Arc;

pub trait Source: SceneSource {
    /// Authored camera/environment quality; immutable for this request.
    fn final_samples(&self, request: &SceneRequest) -> Result<u32, String>;
    fn splat_layers(
        &self,
        _request: &SceneRequest,
    ) -> Result<Vec<pm_studio::splats::Layer>, String> {
        Ok(vec![])
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub revision: u64,
    pub preview: Tier,
    pub seed: u32,
    pub denoise: Denoise,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            revision: 0,
            preview: Tier::QualityLive,
            seed: 1,
            denoise: Denoise::Guided,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub samples: u32,
    pub traces: u64,
    pub geometry_uploads: u64,
    pub scene_updates: u64,
    pub blas_builds: u64,
}
pub(crate) struct Pass {
    source: Arc<dyn Source>,
    service: Option<StudioService>,
    cached: Option<(Tier, RenderedScene)>,
    traces: u64,
    splats: Option<pm_studio::splats::Rasterizer>,
    splat_cache: Option<(SceneRequest, Vec<pm_studio::splats::Layer>)>,
}
impl Pass {
    pub fn new(source: Arc<dyn Source>) -> Self {
        Self {
            source,
            service: None,
            cached: None,
            traces: 0,
            splats: None,
            splat_cache: None,
        }
    }
    pub fn replace_source(&mut self, source: Arc<dyn Source>) {
        self.source = source;
        self.cached = None;
        self.splat_cache = None;
    }
    pub fn begin_frame(&mut self) {
        if self
            .cached
            .as_ref()
            .is_some_and(|(tier, _)| matches!(tier, Tier::QualityRefine { .. }))
        {
            self.cached = None;
        }
    }
    pub fn counters(&self) -> Counters {
        Counters {
            samples: self.cached.as_ref().map_or(0, |(_, scene)| scene.samples),
            traces: self.traces,
            geometry_uploads: self
                .service
                .as_ref()
                .map_or(0, StudioService::geometry_uploads),
            scene_updates: self
                .service
                .as_ref()
                .map_or(0, StudioService::scene_updates),
            blas_builds: self.service.as_ref().map_or(0, StudioService::blas_builds),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        layer: &str,
        request: &SceneRequest,
        settings: Settings,
        final_output: bool,
        shutters: u32,
    ) -> Result<super::Rendered3d, super::Error3d> {
        if self
            .splat_cache
            .as_ref()
            .is_none_or(|(key, _)| key != request)
        {
            self.splat_cache = Some((request.clone(), self.source.splat_layers(request)?));
        }
        if let Some(splat) = self
            .splat_cache
            .as_ref()
            .unwrap()
            .1
            .iter()
            .find(|s| s.id == layer)
        {
            let color = pool.take(
                device,
                request.width,
                request.height,
                wgpu::TextureFormat::Rgba8Unorm,
                pm_gpu::target::working_usage(),
            );
            self.splats
                .get_or_insert_with(|| pm_studio::splats::Rasterizer::new(device))
                .record(device, queue, encoder, splat, &color.texture)?;
            return Ok(super::Rendered3d {
                color,
                depth: None,
                color_space: super::OutputColor3d::Encoded,
            });
        }
        let tier = if final_output {
            let samples = self.source.final_samples(request)?;
            if samples == 0 {
                return Err("Studio sample count must be positive".into());
            }
            Tier::Final {
                samples: samples.div_ceil(shutters.max(1)).max(4),
                salt: settings
                    .seed
                    .wrapping_add(((i128::from(request.ticks) + 36_750).div_euclid(73_500)) as u32),
                denoise: settings.denoise,
            }
        } else {
            settings.preview
        };
        if self.service.is_none() {
            self.service = Some(StudioService::with_device(
                device.clone(),
                queue.clone(),
                request.width,
                request.height,
            )?);
        }
        if self
            .cached
            .as_ref()
            .is_none_or(|(t, s)| *t != tier || s.request != *request)
        {
            let rendered = self.service.as_mut().unwrap().render_bottom_up(
                self.source.as_ref(),
                request.clone(),
                tier,
            )?;
            self.cached = Some((tier, rendered));
            self.traces += 1;
        }
        let scene = &self.cached.as_ref().unwrap().1;
        let texture = &scene
            .layers
            .iter()
            .find(|l| l.layer == layer)
            .ok_or_else(|| format!("Studio did not produce layer {layer}"))?
            .texture;
        let color = pool.take(
            device,
            texture.width(),
            texture.height(),
            wgpu::TextureFormat::Rgba8Unorm,
            pm_gpu::target::working_usage(),
        );
        encoder.copy_texture_to_texture(
            texture.as_image_copy(),
            color.texture.as_image_copy(),
            texture.size(),
        );
        Ok(super::Rendered3d {
            color,
            depth: None,
            color_space: super::OutputColor3d::Encoded,
        })
    }
}
