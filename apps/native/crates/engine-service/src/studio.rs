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

/// Worker-only HDR/RGBE/EXR admission probe; preserve the original dimensions.
pub fn probe_environment(path: &std::path::Path) -> Result<[u32; 2], String> {
    pm_studio3d_engine::environment::EnvironmentMap::load_with_dimensions(path)
        .map(|(_, dimensions)| dimensions)
}

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
    attach_source(renderer, DocumentSource::new(doc, media.clone()));
}

/// Native export command hook. Overrides affect this frozen job only.
#[cfg(feature = "kernel")]
pub fn attach_with_quality(
    renderer: &mut pm_render::Renderer,
    doc: std::sync::Arc<pm_model::Document>,
    media: &pm_export::render::MediaPaths,
    composition: &str,
    quality: serde_json::Value,
) -> Result<(), String> {
    let source =
        DocumentSource::new(doc, media.clone()).with_quality_override(composition, quality)?;
    attach_source(renderer, source);
    Ok(())
}
#[cfg(feature = "kernel")]
fn attach_source(renderer: &mut pm_render::Renderer, source: DocumentSource) {
    let revision = source.revision();
    renderer.set_studio_source(
        Some(std::sync::Arc::new(
            pm_studio3d_engine::compositor::Adapter(std::sync::Arc::new(source)),
        )),
        pm_render::scene3d::provider::Settings {
            revision,
            denoise: Denoise::Oidn,
            ..Default::default()
        },
    );
}
