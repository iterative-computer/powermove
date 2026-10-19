//! Render errors and per-layer failures.

use crate::capability::Unsupported;

/// Why a frame could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The composition uses something this renderer cannot draw yet. The
    /// tracer and export lanes check [`crate::unsupported`] first; this is
    /// the same answer found while rendering (for example a media file
    /// whose format only shows when it is opened).
    #[error("unsupported: {}", .0.iter().map(|u| u.to_string()).collect::<Vec<_>>().join("; "))]
    Unsupported(Vec<Unsupported>),
    #[error("composition {0:?} not found")]
    Composition(String),
    #[error("invalid request: {0}")]
    Request(String),
    #[error("{0}")]
    Device(#[from] pm_gpu::DeviceError),
    #[error("readback: {0}")]
    Readback(#[from] pm_gpu::readback::ReadbackError),
    #[error("GPU: {0}")]
    Gpu(String),
    #[error("media: {0}")]
    Media(String),
    /// Layers drawn by the decision 19 fallback runner
    /// (`crate::shader_layers::fallback`) whose pixels have not arrived:
    /// the caller asks the runner for them and renders the frame again.
    /// The targets are left untouched.
    #[error("waiting for the shader fallback runner ({} layer draws)", .0.len())]
    ShaderFallbackPending(Vec<crate::shader_layers::FallbackRequest>),
    /// The hang guard holds a program this frame draws: a frame that drew
    /// it was seen running past the hang budget and has not ended
    /// (`effects::watchdog`, interactive policy). Nothing was presented; the
    /// caller renders again once `Renderer::watchdog_holding` is false.
    #[error("waiting for the GPU watchdog to judge a frame that has not finished")]
    WatchdogPending,
    /// A program only the WebGL fallback runner draws could not be drawn
    /// by a runner the renderer waits on (a job: no window to run it, its
    /// context was lost); the message names it.
    #[error("{0}")]
    Fallback(String),
    /// A job renderer's watchdog stopped after a runaway program
    /// (`effects::watchdog::Policy::Job`): the message names it.
    #[error("{0}")]
    Watchdog(String),
    /// Level 1 SVG rasters (decision S1) a frame that may not substitute
    /// them (an export, a snapshot: `Renderer::set_svg_wait`) did not
    /// have: the caller gets them from the document's renderer
    /// (`Renderer::put_svg_raster`) and renders the frame again. The
    /// targets are left untouched.
    #[error("waiting for {} SVG raster(s) from the document's renderer", .0.len())]
    SvgRastersPending(Vec<crate::vector::svg_rasters::SvgRasterNeed>),
    /// A level 1 SVG raster the renderer's source could not get (no window
    /// has the document open, the window could not draw it): the frame is
    /// not drawn.
    #[error("{0}")]
    SvgRaster(String),
}

/// A layer that failed while the rest of the frame rendered (section 8,
/// "Isolation": a failing node marks its layer failed and outputs
/// transparency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerFailure {
    /// The composition holding the layer.
    pub composition: String,
    pub layer: String,
    pub message: String,
}
