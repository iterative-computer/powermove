//! Typed Studio seam. Full-scene transport is shared by all layer outputs;
//! masking, effects, opacity and blending remain ordinary compositor passes.
use crate::{
    extract::{SceneRequest, SceneSource},
    service::{RenderedScene, StudioService, Tier},
};
use pm_render::scene3d::provider::{Backend, Source as RenderSource};
pub use pm_render::scene3d::provider::{Counters, Settings};
use std::sync::Arc;

pub trait Source: SceneSource {
    /// Authored camera/environment quality; immutable for this request.
    fn final_samples(&self, request: &SceneRequest) -> Result<u32, String>;
    fn splat_layers(&self, _request: &SceneRequest) -> Result<Vec<crate::splats::Layer>, String> {
        Ok(vec![])
    }
}
struct Pass {
    source: Arc<dyn Source>,
    service: Option<StudioService>,
    cached: Option<(Tier, RenderedScene)>,
    traces: u64,
    splats: Option<crate::splats::Rasterizer>,
    splat_cache: Option<(SceneRequest, Vec<crate::splats::Layer>)>,
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
    fn render_texture(
        &mut self,
        layer: &str,
        request: &SceneRequest,
        tier: Tier,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<wgpu::Texture, String> {
        let service = self.service.as_ref().unwrap();
        let device = service.device().unwrap();
        let queue = service.queue().unwrap();
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
            let color = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Studio splat compositor layer"),
                size: wgpu::Extent3d {
                    width: request.width,
                    height: request.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.splats
                .get_or_insert_with(|| crate::splats::Rasterizer::new(device))
                .record(device, queue, encoder, splat, &color)?;
            return Ok(color);
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
            if cfg!(debug_assertions) && std::env::var_os("STUDIO_TIMINGS").is_some() {
                let service = self.service.as_ref().unwrap();
                eprintln!(
                    "STUDIO_FRAME_TIMING ticks={} size={}x{} tier={tier:?} hardware_rt={} prepare_ms={:.3} total_ms={:.3} geometry_uploads={} scene_updates={} blas_builds={} pipeline={:?}",
                    request.ticks, request.width, request.height, rendered.hardware_rt,
                    rendered.timings.prepare_ms, rendered.timings.total_ms,
                    service.geometry_uploads(), service.scene_updates(), service.blas_builds(),
                    rendered.timings.pipeline,
                );
            }
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
        Ok(texture.clone())
    }
}
impl Backend for Pass {
    fn update_source(&mut self, source: &dyn std::any::Any) -> bool {
        let Some(source) = source.downcast_ref::<Adapter>() else {
            return false;
        };
        self.replace_source(source.0.clone());
        true
    }
    fn begin_frame(&mut self) {
        Pass::begin_frame(self);
    }
    fn counters(&self) -> Counters {
        Pass::counters(self)
    }
    fn render(
        &mut self,
        layer: &str,
        request: &SceneRequest,
        tier: Tier,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<wgpu::Texture, String> {
        self.render_texture(layer, request, tier, encoder)
    }
}
/// Freeze a document source onto the generic compositor seam.
pub struct Adapter(pub Arc<dyn Source>);
impl RenderSource for Adapter {
    fn final_samples(&self, request: &SceneRequest) -> Result<u32, String> {
        self.0.final_samples(request)
    }
    fn create(
        &self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Box<dyn Backend>, String> {
        let mut pass = Pass::new(self.0.clone());
        pass.service = Some(StudioService::with_device(device, queue, width, height)?);
        Ok(Box::new(pass))
    }
}
