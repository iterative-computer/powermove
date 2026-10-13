//! The font registry export (plan section 12): before Phase 2, Electron main
//! hosts the native registry (`pm_text::registry::CoreTextRegistry`) and
//! serves `pm-wasm` from it.
//!
//! The registry boundary crosses as JSON strings in `pm-text`'s serde shapes
//! (`Query`, `Answers`, `FallbackManifest`, `FaceSource`), so main forwards
//! them to the renderer and into `pm-wasm` without reinterpreting numbers.
//! Enumeration for the UI crosses as plain objects.
//!
//! Every call runs on main's JavaScript thread, which is the process's main
//! thread: CoreText matching goes through AppKit's shared font manager.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use pm_text::answers::Query;
use pm_text::registry::{CoreTextRegistry, ProjectDescriptors};

fn err(e: impl std::fmt::Display) -> Error {
    Error::from_reason(e.to_string())
}

fn json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(err)
}

/// `new FontFace(family, data, descriptors)`'s descriptors, as CSS strings.
#[napi(object)]
pub struct FontFaceDescriptors {
    pub weight: Option<String>,
    pub style: Option<String>,
    pub stretch: Option<String>,
}

#[napi(object)]
pub struct FontStyleInfo {
    pub name: String,
    pub weight: f64,
    pub italic: bool,
}

#[napi(object)]
pub struct FontFamilyInfo {
    pub name: String,
    pub styles: Vec<FontStyleInfo>,
    pub variable: bool,
    /// A project family (it shadows an installed family of the same name).
    pub project: bool,
}

/// The native font registry: installed fonts through CoreText, project
/// fonts, the fallback manifest, registry answers, and subsets.
#[napi]
pub struct FontRegistry {
    inner: CoreTextRegistry,
}

#[napi]
impl FontRegistry {
    /// Fails off the main thread.
    #[napi(constructor)]
    pub fn new() -> Result<Self> {
        let inner = CoreTextRegistry::try_new().ok_or_else(|| err("FontRegistry must be created on the main thread"))?;
        Ok(FontRegistry { inner })
    }

    /// Increments whenever project fonts or sources change; answers given
    /// under an older generation may be stale and are dropped by the replay.
    #[napi(getter)]
    pub fn generation(&self) -> f64 {
        self.inner.generation() as f64
    }

    /// Installed families followed by project families, in order.
    #[napi]
    pub fn family_names(&self) -> Vec<String> {
        self.inner.family_names()
    }

    #[napi]
    pub fn family_info(&mut self, family: String) -> Option<FontFamilyInfo> {
        self.inner.family_info(&family).map(|i| FontFamilyInfo {
            name: i.name,
            styles: i.styles.into_iter().map(|s| FontStyleInfo { name: s.name, weight: s.weight as f64, italic: s.italic }).collect(),
            variable: i.variable,
            project: i.project,
        })
    }

    /// `FaceSource[]` as JSON: the family's faces as subset sources, for
    /// level 2 layout.
    #[napi]
    pub fn family_faces(&mut self, family: String) -> Result<String> {
        json(&self.inner.family_faces(&family))
    }

    /// `FallbackManifest` as JSON, for level 2 layout's fallback.
    #[napi]
    pub fn fallback_manifest(&mut self, base_family: String) -> Result<String> {
        json(&self.inner.fallback_manifest(&base_family))
    }

    /// Registers a project font face. Throws for data that is not a font
    /// (after the web font sanitizer) and for invalid descriptors.
    #[napi]
    pub fn add_project_font(&mut self, family: String, data: Uint8Array, descriptors: Option<FontFaceDescriptors>, index: Option<u32>) -> Result<()> {
        let d = descriptors.unwrap_or(FontFaceDescriptors { weight: None, style: None, stretch: None });
        let descriptors = ProjectDescriptors::from_css(d.weight.as_deref().unwrap_or(""), d.style.as_deref().unwrap_or(""), d.stretch.as_deref().unwrap_or("")).map_err(err)?;
        self.inner.add_project_font(&family, &data, index.unwrap_or(0), descriptors).map_err(err)
    }

    /// Sets `family`'s variable-font source (`PM.fontSource`) without
    /// registering a family.
    #[napi]
    pub fn set_font_source(&mut self, family: String, data: Uint8Array) -> Result<()> {
        self.inner.set_font_source(&family, &data).map_err(err)
    }

    /// Removes every project font and source (the project closed).
    #[napi]
    pub fn clear_project_fonts(&mut self) {
        self.inner.clear_project_fonts();
    }

    /// Answers a JSON `Query[]` (a replay's misses, or the families a
    /// document names on open) with `{ generation, faces, answers }`.
    /// Throws for malformed queries and for faces this registry never
    /// handed out.
    #[napi]
    pub fn answer(&mut self, queries: String) -> Result<String> {
        let queries: Vec<Query> = serde_json::from_str(&queries).map_err(err)?;
        let answers = self.inner.answer(&queries).map_err(err)?;
        let mut v = serde_json::to_value(&answers).map_err(err)?;
        v["generation"] = self.inner.generation().into();
        json(&v)
    }

    /// The metrics-and-shaping subset of face `index` of blob `blob` (a
    /// single-face font, read at index 0). Cached per blob and face.
    #[napi]
    pub fn subset(&mut self, blob: String, index: u32) -> Result<Buffer> {
        let bytes = self.inner.subset(&blob, index).map_err(err)?;
        Ok(Buffer::from(bytes.as_ref().clone()))
    }
}
