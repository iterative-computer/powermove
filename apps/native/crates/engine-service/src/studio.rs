//! Native 3D service boundary. Core supplies immutable evaluated scenes through
//! SceneSource; render/export workers own StudioService and share their wgpu
//! device. These APIs never call JS or mutate the kernel's document.
//!
//! `attach` freezes a native kernel scene/resources source onto pm-render's
//! typed Studio hook. Both the viewer worker and windowless export child use it.
//! Unresolved imported/maps/screen resources fail explicitly; see studio.md.
pub use pm_studio3d_engine::OIDN;
pub use pm_studio3d_engine::extract::{
    MeshInstance, SceneInput, SceneRequest, SceneSource, TextureMaps, Transform, View,
};
pub use pm_studio3d_engine::service::{
    Denoise, FastRenderer, FrameTimings, LayerTexture, RenderedScene, StudioService, Tier,
};
pub use pm_studio3d_engine::status::{Failure, LightKind, Progress, RefineProgress, RefineStage};
/// Called on the engine worker; must not perform UI work or wait for the worker.
pub type RefineObserver = std::sync::Arc<dyn Fn(&SceneRequest, RefineProgress) -> bool + Send + Sync>;

mod views;
pub use views::{Axis, Pane, ViewCamera, ViewCommand, Views};

/// A viewer's transient Free View (3d-studio.md §4): where the editor camera
/// stands and looks. It never edits the document; captures and exports use
/// the composition's camera. `zoom` scales the lens as the camera's zoom does.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FreeView {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub zoom: f64,
}
impl FreeView {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .eye
            .iter()
            .chain(self.target.iter())
            .any(|v| !v.is_finite() || v.abs() > 1e6)
            || !self.zoom.is_finite()
            || self.zoom <= 0.
            || self.zoom > 1e6
            || self
                .eye
                .iter()
                .zip(self.target)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                < 1e-12
        {
            return Err(
                "Free View needs finite distinct eye/target points and a positive zoom".into(),
            );
        }
        Ok(())
    }

    /// Exact bits, for cache identities.
    pub fn key(view: Option<Self>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(v) = view {
            for x in v.eye.iter().chain(v.target.iter()).chain([v.zoom].iter()) {
                x.to_bits().hash(&mut h);
            }
            1u8.hash(&mut h);
        }
        h.finish()
    }
}
/// Transient viewer state. Final uses the composition's saved noise/sample
/// policy; selecting a tier never edits the composition or export override.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
fn keep_electron_renderer(renderer: &mut pm_render::Renderer, doc: &pm_model::Document) -> bool {
    // Legacy scenes retain their authored Three.js lighting and ACES tone curve.
    let legacy = pm_eval::node::is_str(pm_eval::node::member(doc.root(), "render3d"), "electron");
    if legacy {
        renderer.set_studio_source(None, Default::default());
    }
    legacy
}

#[cfg(feature = "kernel")]
pub fn attach(
    renderer: &mut pm_render::Renderer,
    doc: std::sync::Arc<pm_model::Document>,
    media: &pm_export::render::MediaPaths,
) {
    if keep_electron_renderer(renderer, &doc) {
        return;
    }
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
    attach_with_camera_fonts(
        renderer,
        doc,
        media,
        request,
        preview,
        None::<FreeView>.into(),
        None,
    )
}
#[cfg(feature = "kernel")]
pub fn attach_with_camera_fonts(
    renderer: &mut pm_render::Renderer,
    doc: std::sync::Arc<pm_model::Document>,
    media: &pm_export::render::MediaPaths,
    request: &SceneRequest,
    preview: Preview,
    camera: ViewCamera,
    fonts: Option<&pm_render::text::host::RenderFonts>,
) -> Result<(), String> {
    attach_with_camera_fonts_progress(renderer,doc,media,request,preview,camera,fonts,None)
}
#[cfg(feature = "kernel")]
pub(crate) fn attach_with_camera_fonts_progress(
    renderer: &mut pm_render::Renderer,
    doc: std::sync::Arc<pm_model::Document>,
    media: &pm_export::render::MediaPaths,
    request: &SceneRequest,
    preview: Preview,
    camera: ViewCamera,
    fonts: Option<&pm_render::text::host::RenderFonts>,
    observer: Option<RefineObserver>,
) -> Result<(), String> {
    if keep_electron_renderer(renderer, &doc) {
        return Ok(());
    }
    camera.validate()?;
    use pm_studio3d_engine::compositor::Source;
    let source = DocumentSource::new(doc, media.clone())
        .with_native_fonts(fonts)
        .with_camera_for(&request.composition, camera)
        .with_refine_observer(observer);
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


/// Native export command hook. Overrides affect this frozen job only.
#[cfg(feature = "kernel")]
pub fn attach_with_quality(
    renderer: &mut pm_render::Renderer,
    doc: std::sync::Arc<pm_model::Document>,
    media: &pm_export::render::MediaPaths,
    composition: &str,
    quality: serde_json::Value,
) -> Result<(), String> {
    attach_with_export_options_fonts(renderer, doc, media, composition, Some(quality), 1., None)
}
/// Frozen export options; neither quality nor scale writes the document.
#[cfg(feature="kernel")]
pub fn attach_with_export_options_fonts(
    renderer:&mut pm_render::Renderer, doc:std::sync::Arc<pm_model::Document>,
    media:&pm_export::render::MediaPaths, composition:&str,
    quality:Option<serde_json::Value>, scale:f32,
    fonts:Option<&pm_render::text::host::RenderFonts>,
)->Result<(),String>{
    if keep_electron_renderer(renderer, &doc) { return Ok(()); }
    let mut source=DocumentSource::new(doc,media.clone()).with_native_fonts(fonts).with_export_render_scale(scale)?;
    if let Some(quality)=quality {source=source.with_quality_override(composition,quality)?;}
    attach_source(renderer,source,Tier::QualityLive);Ok(())
}
#[cfg(feature = "kernel")]
fn attach_source(renderer: &mut pm_render::Renderer, source: DocumentSource, preview: Tier) {
    let source = source.with_capture_definitions(renderer.definitions());
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
