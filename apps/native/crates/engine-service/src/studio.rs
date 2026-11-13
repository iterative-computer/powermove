//! Native 3D service boundary. Core supplies immutable evaluated scenes through
//! SceneSource; render/export workers own StudioService and share their wgpu
//! device. These APIs never call JS or mutate the kernel's document.
//!
//! `attach` freezes a native kernel scene/resources source onto pm-render's
//! typed Studio hook. Both the viewer worker and windowless export child use it.
//! Unresolved imported/maps/screen resources fail explicitly; see studio.md.
pub use pm_studio::extract::{
    MeshInstance, SceneInput, SceneRequest, SceneSource, TextureMaps, Transform, View,
};
pub use pm_studio::service::{
    Denoise, FastRenderer, LayerTexture, RenderedScene, StudioService, Tier,
};
pub use pm_studio::status::{Failure, LightKind, Progress};

#[cfg(feature = "kernel")]
mod native;
#[cfg(feature = "kernel")]
pub use native::DocumentSource;

/// Freeze scene/resources onto the renderer on its worker, for both the viewer
/// and same-executable export child. No document mutation or platform UI work.
#[cfg(feature = "kernel")]
pub fn attach(
    renderer: &mut pm_render::Renderer,
    doc: std::sync::Arc<pm_model::Document>,
    media: &pm_export::render::MediaPaths,
) {
    // Legacy Electron scenes were authored in Three.js' rasterizer with
    // ACES Filmic. Attaching the native path tracer would change lighting,
    // materials and the tone curve (Studio uses PBR Neutral), even in a
    // headless snapshot. Preserve the authored renderer across native saves.
    if pm_eval::node::is_str(pm_eval::node::member(doc.root(), "render3d"), "electron") {
        renderer.set_studio_source(None, Default::default());
        return;
    }
    let revision = doc.revision();
    renderer.set_studio_source(
        Some(std::sync::Arc::new(DocumentSource::new(doc, media.clone()))),
        pm_render::scene3d::provider::Settings {
            revision,
            ..Default::default()
        },
    );
}
