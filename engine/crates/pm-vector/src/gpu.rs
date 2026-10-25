//! The native GPU painter for SVG atlas pages.

use kurbo::{Affine, Rect};
use vello::peniko::color::AlphaColor;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

use crate::render::svg::SvgDocument;

/// Why the GPU backend is unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuUnavailable(pub String);

impl core::fmt::Display for GpuUnavailable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Vello GPU coverage unavailable: {}", self.0)
    }
}

impl std::error::Error for GpuUnavailable {}

/// One SVG document in a [`GpuCoverage::render_svgs`] pass.
pub struct SvgCell<'a> {
    pub doc: &'a SvgDocument,
    /// User units to target pixels.
    pub transform: Affine,
    /// The document's clip polygon in target pixels.
    pub clip: [kurbo::Point; 4],
    /// The part of the target that is this document's.
    pub cell: Rect,
}

/// A Vello renderer on the engine's device, area coverage only.
pub struct GpuCoverage {
    renderer: Renderer,
    scene: Scene,
}

impl GpuCoverage {
    /// Compiles Vello's area pipelines on `device`. Fails (and the caller
    /// falls back to resvg) when the device lacks what Vello needs.
    pub fn new(device: &wgpu::Device) -> Result<Self, GpuUnavailable> {
        let options = RendererOptions {
            use_cpu: false,
            antialiasing_support: AaSupport::area_only(),
            num_init_threads: core::num::NonZeroUsize::new(1),
            pipeline_cache: None,
        };
        let renderer = Renderer::new(device, options).map_err(|e| GpuUnavailable(e.to_string()))?;
        Ok(Self {
            renderer,
            scene: Scene::new(),
        })
    }

    /// The texture format and usages a coverage page needs.
    pub fn page_descriptor(width: u32, height: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("pm vector coverage"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        }
    }

    /// Renders SVG documents into `target` (a texture made with
    /// [`Self::page_descriptor`] at `width` × `height`) in one pass: each
    /// under its transform (user units to `target` pixels), clipped to its
    /// polygon and to its cell of the target. Straight-alpha sRGB values,
    /// as Vello stores them. Only for [`SvgDocument::gpu_paintable`]
    /// documents.
    pub fn render_svgs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cells: &[SvgCell<'_>],
        (width, height): (u32, u32),
        target: &wgpu::TextureView,
    ) -> Result<(), GpuUnavailable> {
        self.scene.reset();
        for cell in cells {
            // One clip: the document's polygon within its cell.
            let clip = crate::render::svg_vello::clip_to_rect(&cell.clip, cell.cell);
            if clip.len() >= 3 {
                cell.doc.encode(&mut self.scene, cell.transform, &clip);
            }
        }

        let params = RenderParams {
            base_color: AlphaColor::from_rgba8(0, 0, 0, 0),
            width,
            height,
            antialiasing_method: AaConfig::Area,
        };
        self.renderer
            .render_to_texture(device, queue, &self.scene, target, &params)
            .map_err(|e| GpuUnavailable(e.to_string()))
    }

}
