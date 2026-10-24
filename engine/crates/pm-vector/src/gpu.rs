//! Vello's GPU compute coverage backend (feature `gpu`; plan section 12,
//! amendment 10).
//!
//! Vello 0.11 renders into `Rgba8Unorm` storage textures only, so it renders
//! coverage (white × area coverage, `AaConfig::Area`) into atlas pages laid
//! out by [`AtlasLayout`]; the engine's float paint pass multiplies each
//! cell by its item's paint at working precision. The same atlas is what
//! [`crate::render::cpu::render_coverage`] produces on the CPU fallback.

use kurbo::{Affine, Rect};
use peniko::Fill;
use peniko::color::AlphaColor;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

use crate::render::scene::{AtlasLayout, CoverageScene, FillRule};
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
    /// falls back to `vello_cpu`) when the device lacks what Vello needs.
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

    /// Renders page `page` of `layout` into `target` (a texture made with
    /// [`Self::page_descriptor`] at the page's size). Vello submits its own
    /// command buffer on `queue`; work submitted after this call sees the
    /// coverage.
    pub fn render_page(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &CoverageScene,
        layout: &AtlasLayout,
        page: usize,
        target: &wgpu::TextureView,
    ) -> Result<(), GpuUnavailable> {
        let (width, height) = layout.pages[page];
        self.scene.reset();
        let white = AlphaColor::from_rgba8(255, 255, 255, 255);
        for cell in layout.cells.iter().filter(|c| c.page == page) {
            let item = &scene.items[cell.item];
            let to_cell = AtlasLayout::cell_transform(cell);
            let cell_rect = Rect::new(
                f64::from(cell.x),
                f64::from(cell.y),
                f64::from(cell.x + cell.device.width),
                f64::from(cell.y + cell.device.height),
            );
            self.scene
                .push_clip_layer(Fill::NonZero, Affine::IDENTITY, &cell_rect);
            let (outline, rule) = item.device_outline();
            let fill = match rule {
                FillRule::NonZero => Fill::NonZero,
                FillRule::EvenOdd => Fill::EvenOdd,
            };
            self.scene.fill(fill, to_cell, white, None, &outline);
            self.scene.pop_layer();
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
