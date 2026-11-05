//! Native 3D service boundary. Core supplies immutable evaluated scenes through
//! SceneSource; render/export workers own StudioService and share their wgpu
//! device. These APIs never call JS or mutate the kernel's document.
//!
//! This facade does not by itself install a scene source in pm-render's private
//! 3D pass. The compositor needs its typed hook before the application can
//! consume this provider; see docs/native/studio.md.
pub use pm_studio::extract::{
    MeshInstance, SceneInput, SceneRequest, SceneSource, TextureMaps, Transform, View,
};
pub use pm_studio::service::{
    Denoise, FastRenderer, LayerTexture, RenderedScene, StudioService, Tier,
};
pub use pm_studio::status::{Failure, LightKind, Progress};
