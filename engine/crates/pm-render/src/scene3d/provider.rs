//! Worker-owned GPU backend seam. Native document evaluation and Studio
//! integration live outside the renderer; the compositor owns ordinary blending.
use pm_gpu::TargetPool;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneRequest {
    pub composition: String,
    pub revision: u64,
    pub ticks: i64,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denoise {
    None,
    Guided,
    Oidn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Fast,
    QualityLive,
    QualityRefine {
        samples: u32,
    },
    Final {
        samples: u32,
        salt: u32,
        denoise: Denoise,
    },
}
/// A frozen document/resources source. Creation and rendering run on a worker.
pub trait Source: Send + Sync + std::any::Any {
    fn final_samples(&self, request: &SceneRequest) -> Result<u32, String>;
    fn create(
        &self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Box<dyn Backend>, String>;
}
pub trait Backend: Send {
    /// Retain GPU resources when a new immutable document source replaces one.
    fn update_source(&mut self, _source: &dyn std::any::Any) -> bool {
        false
    }
    fn begin_frame(&mut self);
    fn counters(&self) -> Counters;
    /// Premultiplied display-sRGB pixels in compositor (bottom-up) row order.
    fn render(
        &mut self,
        layer: &str,
        request: &SceneRequest,
        tier: Tier,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<wgpu::Texture, String>;
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
    backend: Option<Box<dyn Backend>>,
}
impl Pass {
    pub fn new(source: Arc<dyn Source>) -> Self {
        Self {
            source,
            backend: None,
        }
    }
    pub fn replace_source(&mut self, source: Arc<dyn Source>) {
        if self
            .backend
            .as_mut()
            .is_some_and(|b| !b.update_source(source.as_ref()))
        {
            self.backend = None;
        }
        self.source = source;
    }
    pub fn begin_frame(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.begin_frame();
        }
    }
    pub fn counters(&self) -> Counters {
        self.backend
            .as_ref()
            .map_or_else(Counters::default, |b| b.counters())
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
        if self.backend.is_none() {
            self.backend = Some(self.source.create(
                device.clone(),
                queue.clone(),
                request.width,
                request.height,
            )?);
        }
        let texture = self
            .backend
            .as_mut()
            .unwrap()
            .render(layer, request, tier, encoder)?;
        let color = pool.take(
            device,
            texture.width(),
            texture.height(),
            texture.format(),
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
