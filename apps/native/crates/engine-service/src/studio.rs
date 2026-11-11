//! Native 3D service boundary. Core supplies immutable evaluated scenes through
//! SceneSource; render/export workers own StudioService and share their wgpu
//! device. These APIs never call JS or mutate the kernel's document.
//!
//! `attach` freezes a native kernel scene/resources source onto pm-render's
//! typed Studio hook. Both the viewer worker and windowless export child use it.
//! Unresolved imported/maps/screen resources fail explicitly; see studio.md.
pub use pm_studio3d_engine::extract::{
    MeshInstance, SceneInput, SceneRequest, SceneSource, TextureMaps, Transform, View,
};
pub use pm_studio3d_engine::service::{
    Denoise, FastRenderer, LayerTexture, RenderedScene, StudioService, Tier,
};
pub use pm_studio3d_engine::status::{Failure, LightKind, Progress};

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
    let revision = doc.revision();
    renderer.set_studio_source(
        Some(std::sync::Arc::new(
            pm_studio3d_engine::compositor::Adapter(std::sync::Arc::new(DocumentSource::new(
                doc,
                media.clone(),
            ))),
        )),
        pm_render::scene3d::provider::Settings {
            revision,
            denoise: Denoise::Oidn,
            ..Default::default()
        },
    );
}
