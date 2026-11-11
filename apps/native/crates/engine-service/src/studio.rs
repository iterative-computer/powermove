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
    Denoise, FastRenderer, FrameTimings, LayerTexture, RenderedScene, StudioService, Tier,
};
pub use pm_studio3d_engine::status::{Failure, LightKind, Progress};

/// Transient viewer state. Final uses the composition's saved noise/sample
/// policy; selecting a tier never edits the composition or export override.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preview {
    #[default]
    Live,
    Refine,
    Final,
}
impl Preview {
    /// Development-only worker hook until the designer binds the command.
    pub fn debug_default() -> Result<Self, String> {
        #[cfg(debug_assertions)]
        if let Ok(value) = std::env::var("PM_NATIVE_STUDIO_TIER") {
            return Self::parse(&value);
        }
        Ok(Self::Live)
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "live" => Ok(Self::Live),
            "refine" => Ok(Self::Refine),
            "final" => Ok(Self::Final),
            _ => Err("Studio tier must be live, refine or final".into()),
        }
    }
}

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
    attach_source(
        renderer,
        DocumentSource::new(doc, media.clone()),
        Tier::QualityLive,
    );
}

#[cfg(feature = "kernel")]
pub fn attach_with_preview(
    renderer: &mut pm_render::Renderer,
    doc: std::sync::Arc<pm_model::Document>,
    media: &pm_export::render::MediaPaths,
    request: &SceneRequest,
    preview: Preview,
) -> Result<(), String> {
    attach_with_preview_fonts(renderer,doc,media,request,preview,None)
}
#[cfg(feature="kernel")]
pub fn attach_with_preview_fonts(renderer:&mut pm_render::Renderer,doc:std::sync::Arc<pm_model::Document>,media:&pm_export::render::MediaPaths,request:&SceneRequest,preview:Preview,fonts:Option<&pm_render::text::host::RenderFonts>)->Result<(),String>{
    use pm_studio3d_engine::compositor::Source;
    let source = DocumentSource::new(doc, media.clone()).with_native_fonts(fonts);
    let tier = match preview {
        Preview::Live => Tier::QualityLive,
        Preview::Refine => Tier::QualityRefine { samples: 32 },
        Preview::Final => Tier::Final {
            samples: source.final_samples(request)?,
            salt: 1,
            denoise: Denoise::Oidn,
        },
    };
    attach_source(renderer, source, tier);
    Ok(())
}

#[cfg(feature="kernel")]
pub fn attach_with_fonts(renderer:&mut pm_render::Renderer,doc:std::sync::Arc<pm_model::Document>,media:&pm_export::render::MediaPaths,fonts:&pm_render::text::host::RenderFonts){
    attach_source(renderer,DocumentSource::new(doc,media.clone()).with_native_fonts(Some(fonts)),Tier::QualityLive);
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
    attach_with_quality_fonts(renderer,doc,media,composition,quality,None)
}
#[cfg(feature="kernel")]
pub fn attach_with_quality_fonts(renderer:&mut pm_render::Renderer,doc:std::sync::Arc<pm_model::Document>,media:&pm_export::render::MediaPaths,composition:&str,quality:serde_json::Value,fonts:Option<&pm_render::text::host::RenderFonts>)->Result<(),String>{
    let source = DocumentSource::new(doc, media.clone()).with_native_fonts(fonts).with_quality_override(composition, quality)?;
    attach_source(renderer, source, Tier::QualityLive);
    Ok(())
}
#[cfg(feature = "kernel")]
fn attach_source(renderer: &mut pm_render::Renderer, source: DocumentSource, preview: Tier) {
    let revision = source.revision();
    renderer.set_studio_source(
        Some(std::sync::Arc::new(
            pm_studio3d_engine::compositor::Adapter(std::sync::Arc::new(source)),
        )),
        pm_render::scene3d::provider::Settings {
            revision,
            preview,
            denoise: Denoise::Oidn,
            ..Default::default()
        },
    );
}
