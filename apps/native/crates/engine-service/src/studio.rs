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
    #[serde(default)]
    pub ortho: bool,
    #[serde(default)]
    pub roll: f64,
}
impl FreeView {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .eye
            .iter()
            .chain(self.target.iter())
            .any(|v| !v.is_finite() || v.abs() > 1e6)
            || !self.roll.is_finite()
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
            for x in v
                .eye
                .iter()
                .chain(v.target.iter())
                .chain([v.zoom, v.roll, if v.ortho { 1. } else { 0. }].iter())
            {
                x.to_bits().hash(&mut h);
            }
            1u8.hash(&mut h);
        }
        h.finish()
    }
}
/// The editor's Studio camera, converted to the flat-layer projector's lens.
pub(crate) fn plane_camera(
    view: &ViewCamera,
    doc: &pm_model::Document,
    composition: &str,
    ticks: i64,
) -> Option<pm_eval::space3d::Camera3d> {
    use pm_eval::space3d::{Camera3d, default_camera_25d, uses_default_25d_camera};
    if *view == ViewCamera::Active {
        return None;
    }
    let comp = pm_eval::Comp::resolve(doc, composition).ok()?;
    let mut ev = pm_eval::Evaluator::new()
        .with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = pm_eval::Scope::open(&mut ev, &comp);
    let time = comp.seconds_at(ticks);
    if let ViewCamera::Layer { id } = view {
        let layer = scope.layers().iter().find(|layer| {
            pm_eval::parenting::Role3d::of(layer) == Some(pm_eval::parenting::Role3d::Camera)
                && matches!(layer.get("id").map(pm_model::Node::kind),
                    Some(pm_model::NodeKind::String(value)) if value.as_str() == Some(id.as_str()))
        })?;
        return scope.camera_3d(layer, time).ok();
    }
    let pose = view.pose()?;
    let (width, height) = scope.comp_size();
    let fov = scope.active_camera_3d(time).ok().flatten().map_or_else(
        || {
            if uses_default_25d_camera(scope.layers()) {
                default_camera_25d(width, height).fov
            } else {
                45.
            }
        },
        |camera| camera.fov,
    );
    let distance = (0..3)
        .map(|i| (pose.eye[i] - pose.target[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    let ortho_height = match view {
        ViewCamera::Orthographic { span, .. } => *span,
        _ => 2. * distance * (fov.to_radians() / 2.).tan(),
    };
    Some(Camera3d {
        position: pose.eye,
        target: pose.target,
        roll: pose.roll,
        fov,
        zoom: pose.zoom,
        ortho: pose.ortho || matches!(view, ViewCamera::Orthographic { .. }),
        ortho_height,
    })
}

/// Transient viewer state. Final uses the composition's saved noise/sample
/// policy; selecting a tier never edits the composition or export override.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preview {
    Fast,
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
        // The viewer previews with the Fast raster until someone picks Full
        // or Quality; exports never read this (always Full).
        Ok(Self::Fast)
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "fast" => Ok(Self::Fast),
            "live" => Ok(Self::Live),
            "refine" => Ok(Self::Refine),
            "final" => Ok(Self::Final),
            _ => Err("Studio tier must be fast, live, refine or final".into()),
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
pub use native::{DocumentSource, StudioResourceCache};

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
    attach_with_camera_fonts_progress(
        renderer, doc, media, request, preview, camera, fonts, None, None,
    )
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
    resource_cache: Option<StudioResourceCache>,
) -> Result<(), String> {
    if keep_electron_renderer(renderer, &doc) {
        return Ok(());
    }
    camera.validate()?;
    let mut source = DocumentSource::new(doc, media.clone())
        .with_native_fonts(fonts)
        .with_camera_for(&request.composition, camera)
        .with_refine_observer(observer);
    if let Some(cache) = resource_cache {
        source = source.with_resource_cache(cache);
    }
    let tier = preview_tier(&source, request, preview)?;
    let source = std::sync::Arc::new(source.with_capture_definitions(renderer.definitions()));
    source.spawn_prewarm(&request.composition);
    attach_arc_source(renderer, source, tier);
    Ok(())
}

#[cfg(feature = "kernel")]
fn preview_tier(
    source: &DocumentSource,
    request: &SceneRequest,
    preview: Preview,
) -> Result<Tier, String> {
    use pm_studio3d_engine::compositor::Source;
    Ok(match preview {
        Preview::Fast => Tier::Fast,
        Preview::Live => Tier::QualityLive,
        Preview::Refine => Tier::QualityRefine { samples: 32 },
        Preview::Final => Tier::Final {
            samples: source.final_samples(request)?,
            salt: 1,
            denoise: Denoise::Oidn,
        },
    })
}

/// Viewer pose can change while the document, its resolved scene, and
/// resource and material caches stay valid. Selecting a different authored
/// camera still creates a new source because it changes scene resolution.
#[cfg(feature = "kernel")]
pub(crate) struct ViewerSource {
    document: std::sync::RwLock<Option<DocumentSource>>,
    revision: u64,
    tier: Tier,
}

#[cfg(feature = "kernel")]
impl ViewerSource {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        renderer: &pm_render::Renderer,
        doc: std::sync::Arc<pm_model::Document>,
        media: &pm_export::render::MediaPaths,
        request: &SceneRequest,
        preview: Preview,
        camera: ViewCamera,
        fonts: Option<&pm_render::text::host::RenderFonts>,
        observer: Option<RefineObserver>,
        cache: StudioResourceCache,
        prewarm: bool,
    ) -> Result<Option<std::sync::Arc<Self>>, String> {
        if pm_eval::node::is_str(pm_eval::node::member(doc.root(), "render3d"), "electron") {
            return Ok(None);
        }
        camera.validate()?;
        let document = DocumentSource::new(doc.clone(), media.clone())
            .with_native_fonts(fonts)
            .with_camera_for(&request.composition, camera)
            .with_refine_observer(observer)
            .with_resource_cache(cache.clone())
            .with_capture_definitions(renderer.definitions());
        let tier = preview_tier(&document, request, preview)?;
        // The background loader fills the same shared resource cache.
        if prewarm {
            std::sync::Arc::new(DocumentSource::new(doc, media.clone()).with_resource_cache(cache))
                .spawn_prewarm(&request.composition);
        }
        Ok(Some(std::sync::Arc::new(Self {
            revision: document.revision(),
            document: std::sync::RwLock::new(Some(document)),
            tier,
        })))
    }

    pub(crate) fn set_camera(&self, comp: &str, camera: ViewCamera) -> Result<(), String> {
        camera.validate()?;
        let mut document = self
            .document
            .write()
            .map_err(|_| "Studio source lock poisoned")?;
        *document = document
            .take()
            .map(|source| source.with_camera_for(comp, camera));
        Ok(())
    }

    pub(crate) fn attach(self: &std::sync::Arc<Self>, renderer: &mut pm_render::Renderer) {
        attach_provider(renderer, self.clone(), self.revision, self.tier);
    }

    fn read<T>(&self, f: impl FnOnce(&DocumentSource) -> Result<T, String>) -> Result<T, String> {
        let document = self
            .document
            .read()
            .map_err(|_| "Studio source lock poisoned")?;
        f(document
            .as_ref()
            .expect("source is restored under the write lock"))
    }
}

#[cfg(feature = "kernel")]
impl SceneSource for ViewerSource {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        self.read(|s| s.scene(request))
    }

    fn refine_progress(&self, request: &SceneRequest, progress: RefineProgress) -> bool {
        self.read(|s| Ok(s.refine_progress(request, progress)))
            .unwrap_or(false)
    }

    fn layer_failures(
        &self,
        request: &SceneRequest,
    ) -> Result<Vec<pm_render::LayerFailure>, String> {
        self.read(|s| s.layer_failures(request))
    }

    fn auxiliary_layers(
        &self,
        request: &SceneRequest,
    ) -> Result<Vec<pm_studio3d_engine::extract::AuxiliaryLayer>, String> {
        self.read(|s| s.auxiliary_layers(request))
    }

    fn final_quality(
        &self,
        request: &SceneRequest,
    ) -> Result<Option<pm_studio3d_engine::extract::FinalQuality>, String> {
        self.read(|s| s.final_quality(request))
    }
}

#[cfg(feature = "kernel")]
impl pm_studio3d_engine::compositor::Source for ViewerSource {
    fn final_samples(&self, request: &SceneRequest) -> Result<u32, String> {
        self.read(|s| s.final_samples(request))
    }

    fn splat_layers(
        &self,
        request: &SceneRequest,
    ) -> Result<Vec<pm_studio3d_engine::splats::Layer>, String> {
        self.read(|s| s.splat_layers(request))
    }

    fn needs_materials(&self, request: &SceneRequest) -> Result<bool, String> {
        self.read(|s| s.needs_materials(request))
    }

    fn prewarm_materials(
        &self,
        renderer: &mut pm_render::Renderer,
        request: &SceneRequest,
    ) -> Result<(), String> {
        self.read(|s| s.prewarm_materials(renderer, request))
    }

    fn prepare_materials(
        &self,
        renderer: &mut pm_render::Renderer,
        request: &SceneRequest,
        tier: Tier,
    ) -> Result<bool, String> {
        self.read(|s| s.prepare_materials(renderer, request, tier))
    }
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
    let source = std::sync::Arc::new(source.with_capture_definitions(renderer.definitions()));
    attach_arc_source(renderer, source, preview);
}
#[cfg(feature = "kernel")]
fn attach_arc_source(
    renderer: &mut pm_render::Renderer,
    source: std::sync::Arc<DocumentSource>,
    preview: Tier,
) {
    let revision = source.revision();
    attach_provider(renderer, source, revision, preview);
}

#[cfg(feature = "kernel")]
fn attach_provider(
    renderer: &mut pm_render::Renderer,
    source: std::sync::Arc<dyn pm_studio3d_engine::compositor::Source>,
    revision: u64,
    preview: Tier,
) {
    renderer.set_studio_source(
        Some(std::sync::Arc::new(
            pm_studio3d_engine::compositor::Adapter(source),
        )),
        pm_render::scene3d::provider::Settings {
            revision,
            preview,
            denoise: Denoise::Oidn,
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod plane_camera_tests {
    use super::*;

    #[test]
    fn free_view_reprojects_grouped_25d_layers_without_a_camera_layer() {
        let project = r#"{
            "id":"c","w":640,"h":360,"fps":30,"dur":5,
            "layers":[
                {"id":"card","type":"group","threeD":true,"on":true,"from":0,"dur":2,"p":{}},
                {"id":"child","type":"shape","group":"card","on":true,"from":0,"dur":2,
                 "p":{"position.x":{"v":320},"position.y":{"v":180}}},
                {"id":"control","type":"null","threeD":true,"on":true,"from":0,"dur":2,"p":{}},
                {"id":"background","type":"solid","on":true,"from":0,"dur":5,"p":{}}
            ]
        }"#;
        let doc = pm_model::Document::new(Some(pm_model::json::parse(project).unwrap()), 1);
        let default = pm_eval::space3d::default_camera_25d(640., 360.);
        let view = FreeView {
            eye: [1., 0., default.position[2] - 1.],
            target: [1., 0., -1.],
            zoom: 1.,
            ortho: false,
            roll: 0.,
        };
        let camera = plane_camera(&Some(view).into(), &doc, "c", 0).unwrap();
        assert_eq!(camera.position, view.eye);
        assert_eq!(camera.target, view.target);
        assert_eq!(camera.fov, default.fov);

        let comp = pm_eval::Comp::resolve(&doc, "c").unwrap();
        let layers = comp.layers();
        let mut ev = pm_eval::Evaluator::new();
        let mut scope = pm_eval::Scope::open(&mut ev, &comp);
        let shot = scope.plane_matrix(&layers[1], 0., false).unwrap();
        let background = scope.plane_matrix(&layers[3], 0., false).unwrap();
        scope.view_camera = Some(camera);
        assert_ne!(scope.plane_matrix(&layers[1], 0., false).unwrap(), shot);
        assert_eq!(scope.plane_matrix(&layers[3], 0., false).unwrap(), background);
    }
}

#[cfg(test)]
mod preview_tests {
    use super::Preview;
    #[test]
    fn fast_is_a_transient_preview_choice() {
        assert_eq!(Preview::parse("fast").unwrap(),Preview::Fast);
        assert_eq!(Preview::parse("live").unwrap(),Preview::Live);
        assert_eq!(Preview::parse("refine").unwrap(),Preview::Refine);
        assert_eq!(Preview::parse("final").unwrap(),Preview::Final);
    }
}
