//! A viewer's document on the presenter's render thread (plan section 6,
//! "The patch stream"; section 21, "Capability routing").
//!
//! The engine forwards every change of the session's canonical document
//! ([`DocumentUpdate`]) and every media answer from the host; this keeps
//! the presenter's own mirror, the asset → file map, and the routing
//! decision for the composition the viewer shows:
//!
//! * [`Route::Engine`]: the renderer draws everything in the composition
//!   and the media of every asset it uses is answered;
//! * [`Route::Legacy`]: something the renderer cannot draw yet
//!   (`pm_render::unsupported_with_media`): the viewer keeps the legacy
//!   engine's frames. Offline media instead draws a named layer failure;
//! * [`Route::Pending`]: not decided yet (no document, the composition is
//!   not in it, media answers outstanding, a batch that did not apply).
//!
//! The decision is static per (revision, composition, media answers), as
//! `pm_render::capability` promises, so it is cached on that key.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use pm_engine::document::{DocumentUpdate, asset_records};
use pm_model::{Document, Node};

/// Where a composition's frames come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Engine,
    /// The legacy engine draws it; one sentence per reason.
    Legacy(Vec<String>),
    /// Not decided yet; why.
    Pending(&'static str),
}

/// The asset → file answers of one view, shared with the renderer and the
/// audio thread (a snapshot; answers that come later make a new one).
#[derive(Debug, Clone, Default)]
pub struct MediaMap(pub Arc<BTreeMap<String, Option<PathBuf>>>);

impl MediaMap {
    /// The file an asset resolved to, when it did.
    pub fn path(&self, asset: &str) -> Option<PathBuf> {
        self.0.get(asset).cloned().flatten()
    }
}

impl pm_render::AssetResolver for MediaMap {
    fn path(&self, asset: &str, _record: Option<&Node>) -> Option<PathBuf> {
        self.0.get(asset).cloned().flatten()
    }
}

impl pm_audio::AssetResolver for MediaMap {
    fn resolve(&self, asset: &str) -> Option<PathBuf> {
        self.0.get(asset).cloned().flatten()
    }
}

#[derive(Debug, Clone)]
struct Cached {
    revision: u64,
    composition: String,
    media_version: u64,
    route: Route,
}

/// A view's document, media, and routing.
#[derive(Debug, Default)]
pub struct ViewDocument {
    doc: Document,
    has_document: bool,
    /// A batch did not apply; frames wait for the renderer's resync.
    broken: Option<String>,
    media: MediaMap,
    media_version: u64,
    /// The session's definitions (contract 10) and their version.
    definitions: Option<(Arc<pm_protocol::definitions::DefinitionSetSnapshot>, u64)>,
    /// Definitions registered again since the renderer last loaded the set.
    registered: Vec<(
        pm_protocol::definitions::DefinitionKind,
        pm_protocol::common::DefinitionId,
    )>,
    cached: Option<Cached>,
    /// The renderer lays text out over the engine's fonts: text and
    /// captions layers are drawn.
    fonts: bool,
}

impl ViewDocument {
    /// Whether the renderer has fonts (`pm_render::unsupported_with_fonts`).
    pub fn set_fonts(&mut self, fonts: bool) {
        if self.fonts != fonts {
            self.fonts = fonts;
            self.cached = None;
        }
    }

    /// The session's definitions: effects and transitions are drawn with
    /// them (`pm_render::effects`), routed without them.
    /// `registered`: the definitions the update registered again (a delta's
    /// upserts).
    pub fn set_definitions(
        &mut self,
        set: Arc<pm_protocol::definitions::DefinitionSetSnapshot>,
        registered: Vec<(
            pm_protocol::definitions::DefinitionKind,
            pm_protocol::common::DefinitionId,
        )>,
    ) {
        let version = self.definitions.as_ref().map_or(1, |(_, v)| v + 1);
        self.definitions = Some((set, version));
        self.keep_registered(registered);
        self.cached = None;
    }

    /// The definitions registered again that the renderer has not seen.
    pub fn take_registered(
        &mut self,
    ) -> Vec<(
        pm_protocol::definitions::DefinitionKind,
        pm_protocol::common::DefinitionId,
    )> {
        std::mem::take(&mut self.registered)
    }

    /// Keeps `registered` for the renderer's next load of the set.
    pub fn keep_registered(
        &mut self,
        registered: Vec<(
            pm_protocol::definitions::DefinitionKind,
            pm_protocol::common::DefinitionId,
        )>,
    ) {
        for r in registered {
            if !self.registered.contains(&r) {
                self.registered.push(r);
            }
        }
    }

    pub fn definitions(
        &self,
    ) -> Option<&(Arc<pm_protocol::definitions::DefinitionSetSnapshot>, u64)> {
        self.definitions.as_ref()
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn has_document(&self) -> bool {
        self.has_document
    }

    pub fn revision(&self) -> u64 {
        self.doc.revision()
    }

    pub fn media(&self) -> &MediaMap {
        &self.media
    }

    /// Applies an update from the engine. A batch that does not apply
    /// leaves the view [`Route::Pending`] until a resync replaces the
    /// mirror; returns the reason.
    pub fn update(&mut self, update: &DocumentUpdate) -> Result<(), String> {
        let replace = matches!(update, DocumentUpdate::Replace { .. });
        if self.broken.is_some() && !replace {
            return Err("waiting for a resync".into());
        }
        match update.apply_to(&mut self.doc) {
            Ok(()) => {
                self.has_document = true;
                self.broken = None;
                Ok(())
            }
            Err(error) => {
                self.broken = Some(error.clone());
                Err(error)
            }
        }
    }

    /// The host's answer for an asset (`None`: offline or gone).
    pub fn set_media(&mut self, asset: &str, path: Option<PathBuf>) {
        let map = Arc::make_mut(&mut self.media.0);
        if map.get(asset) == Some(&path) {
            return;
        }
        map.insert(asset.to_owned(), path);
        self.media_version += 1;
    }

    /// The route for `composition` at the current revision.
    pub fn route(&mut self, composition: &str) -> Route {
        if let Some(c) = &self.cached
            && c.revision == self.doc.revision()
            && c.composition == composition
            && c.media_version == self.media_version
            && self.broken.is_none()
        {
            return c.route.clone();
        }
        let route = self.decide(composition);
        self.cached = Some(Cached {
            revision: self.doc.revision(),
            composition: composition.to_owned(),
            media_version: self.media_version,
            route: route.clone(),
        });
        route
    }

    /// Records a render that met something unsupported only at exec time
    /// (media facts the capability check could not see): the composition
    /// stays with the legacy viewer at this revision.
    pub fn force_legacy(&mut self, composition: &str, reasons: Vec<String>) {
        self.cached = Some(Cached {
            revision: self.doc.revision(),
            composition: composition.to_owned(),
            media_version: self.media_version,
            route: Route::Legacy(reasons),
        });
    }

    fn decide(&self, composition: &str) -> Route {
        if !self.has_document {
            return Route::Pending("the document has not arrived");
        }
        if self.broken.is_some() {
            return Route::Pending("a patch batch did not apply; waiting for a resync");
        }
        let Some(root) = self.doc.root() else {
            return Route::Pending("the document is empty");
        };
        if pm_eval::Comp::resolve(&self.doc, composition).is_err() {
            return Route::Pending("the composition is not in the document yet");
        }
        // Every asset needs the host's answer before the first frame, so a
        // frame never shows media missing that is only on its way.
        let records = asset_records(Some(root));
        if records
            .iter()
            .any(|(id, _)| !self.media.0.contains_key(id.as_str()))
        {
            return Route::Pending("media is being resolved");
        }
        // The host has answered: an offline file is now a named layer
        // failure from the renderer, not a different rendering engine.
        let mut reasons = Vec::new();
        let issues = if self.fonts {
            pm_render::unsupported_with_fonts(&self.doc, composition, Some(&self.media))
        } else {
            pm_render::unsupported_with_media(&self.doc, composition, &self.media)
        };
        for u in issues {
            // With definitions, effects, transitions, adjustment layers
            // with effects, and extension layers are drawn; one the renderer cannot draw (no
            // definition, a program that needs the fallback runner) fails
            // the render as unsupported and routes the composition then
            // (`force_legacy`).
            if self.definitions.is_some()
                && matches!(
                    u.feature,
                    pm_render::Feature::Effects
                        | pm_render::Feature::Transition
                        | pm_render::Feature::Adjustment
                        | pm_render::Feature::ExtensionDefinition(_)
                )
            {
                continue;
            }
            // The viewer never draws final Blender frames: it draws native
            // shading, and Blender surfaces while paused in Material
            // Preview or Rendered (`super::blender`), as today. Its renderer
            // also runs the decision 19 fallback runner: programs only it
            // can draw render through it.
            if u.feature == pm_render::Feature::Blender
                || matches!(u.feature, pm_render::Feature::ShaderFallback(_))
            {
                continue;
            }
            reasons.push(describe(&u));
        }
        if reasons.is_empty() {
            Route::Engine
        } else {
            reasons.sort();
            reasons.dedup();
            Route::Legacy(reasons)
        }
    }
}

/// One routing reason in a sentence fragment (logs and the renderer's
/// diagnostics; never shown as an error).
pub fn describe(u: &pm_render::Unsupported) -> String {
    use pm_render::Feature;
    let what = match &u.feature {
        Feature::LayerType(t) => format!("{t} layers"),
        Feature::ExtensionDefinition(d) => format!("extension layer {d}"),
        Feature::Scene3d(s) => format!("3D: {s}"),
        Feature::Effects => "effects".into(),
        Feature::Masks => "masks".into(),
        Feature::PathMasks => "path masks".into(),
        Feature::TrackMatte => "track mattes".into(),
        Feature::Transition => "transitions".into(),
        Feature::BlendMode(m) => format!("blend mode {m}"),
        Feature::MotionBlur => "motion blur".into(),
        Feature::TwoAndAHalfD => "2.5D layers".into(),
        Feature::Adjustment => "adjustment layers".into(),
        Feature::CompatLevel2 => "compat level 2 content".into(),
        Feature::ProjectColor => "project color settings".into(),
        Feature::RequiresEngine => "native-only features".into(),
        Feature::Blender => "Blender rendering".into(),
        Feature::SvgImage => "SVG images".into(),
        Feature::ImageSequence => "animated images or sequences".into(),
        Feature::Media(m) => format!("media: {m}"),
        Feature::Request(r) => format!("request: {r}"),
        Feature::Definition(d) => format!("no definition for {d}"),
        Feature::ShaderFallback(d) => format!("{d} needs the GLSL fallback runner"),
        Feature::Text(t) => format!("text: {t}"),
    };
    match (&u.composition, &u.layer) {
        (Some(c), Some(l)) => format!("{what} (layer {l} in {c})"),
        (Some(c), None) => format!("{what} (in {c})"),
        _ => what,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::Revision;

    fn replace(json: &str, revision: u64) -> DocumentUpdate {
        DocumentUpdate::Replace {
            root: Some(Arc::new(pm_model::json::parse(json).unwrap())),
            revision: Revision::new(revision).unwrap(),
        }
    }

    fn solid(id: &str) -> String {
        format!(
            r##"{{"id":"{id}","type":"solid","on":true,"from":0,"dur":2,"blend":"normal","fx":[],"masks":[],"p":{{}},"d":{{"color":"#ff0000","w":10,"h":10}}}}"##
        )
    }

    #[test]
    fn routing_follows_features_media_and_composition() {
        let mut view = ViewDocument::default();
        assert_eq!(
            view.route("c"),
            Route::Pending("the document has not arrived")
        );
        let doc = format!(
            r#"{{"id":"P","compId":"c","w":64,"h":36,"fps":30,"dur":2,"layers":[{}],"assets":{{}},"comps":{{}}}}"#,
            solid("a")
        );
        view.update(&replace(&doc, 1)).unwrap();
        assert_eq!(view.route("c"), Route::Engine);
        assert!(matches!(view.route("missing"), Route::Pending(_)));

        // An effect routes the composition to the legacy engine.
        let with_fx = doc.replace(r#""fx":[]"#, r#""fx":[{"id":"e","type":"blur","on":true}]"#);
        view.update(&replace(&with_fx, 2)).unwrap();
        let Route::Legacy(reasons) = view.route("c") else {
            panic!("{:?}", view.route("c"))
        };
        assert!(
            reasons.iter().any(|r| r.starts_with("effects")),
            "{reasons:?}"
        );

        // An image waits for the host, then draws a named offline failure.
        let image = doc
            .replace(
                r#""assets":{}"#,
                r#""assets":{"img":{"id":"img","name":"photo.png","kind":"image","format":"png"}}"#,
            )
            .replace(
                r##""d":{"color":"#ff0000","w":10,"h":10}"##,
                r#""d":{"asset":"img","fit":"contain","w":10,"h":10}"#,
            )
            .replace(r#""type":"solid""#, r#""type":"image""#);
        view.update(&replace(&image, 3)).unwrap();
        assert_eq!(view.route("c"), Route::Pending("media is being resolved"));
        view.set_media("img", None);
        assert_eq!(view.route("c"), Route::Engine);
    }

    #[test]
    fn blender_compositions_are_drawn_by_the_engine_in_the_viewer() {
        let object = r#"{"id":"o","type":"extension","on":true,"from":0,"dur":2,"blend":"normal","fx":[],"masks":[],"threeD":true,"p":{},"d":{"definition":"powermove.3d.object","version":1,"w":64,"h":36,"params":{},"data":{"object":{"source":{"primitive":"box","parameters":{}},"material":{"p":{},"maps":{},"doubleSided":true},"useSourceMaterials":false,"slots":[],"castShadow":true,"receiveShadow":true}}}}"#;
        let doc = |render3d: &str| {
            format!(
                r#"{{"id":"P","compId":"c","w":64,"h":36,"fps":30,"dur":2,{render3d}"layers":[{object}],"assets":{{}},"comps":{{}}}}"#
            )
        };
        let mut native = ViewDocument::default();
        native.update(&replace(&doc(""), 1)).unwrap();
        let mut blender = ViewDocument::default();
        let with = doc(r#""render3d":{"enabled":true,"engine":"eevee","samples":8},"#);
        blender.update(&replace(&with, 1)).unwrap();
        // The static check lists Blender for exports ...
        assert!(
            pm_render::unsupported(blender.document(), "c")
                .iter()
                .any(|u| u.feature == pm_render::Feature::Blender)
        );
        // ... and the viewer routes it exactly as the same scene without.
        assert_eq!(blender.route("c"), native.route("c"));
        if let Route::Legacy(reasons) = blender.route("c") {
            assert!(
                !reasons.iter().any(|r| r.contains("Blender")),
                "{reasons:?}"
            );
        }
    }

    #[test]
    fn a_batch_that_does_not_apply_waits_for_a_resync() {
        let mut view = ViewDocument::default();
        view.update(&replace(r#"{"layers":[]}"#, 1)).unwrap();
        // Removing an id the array does not hold diverges from the mirror.
        let bad = DocumentUpdate::Ops {
            ops: vec![pm_model::Op::Delete {
                path: vec![pm_model::PathPart::Key("layers".into())],
                id: "nope".into(),
            }]
            .into(),
            revision: Revision::new(2).unwrap(),
        };
        assert!(view.update(&bad).is_err());
        assert!(matches!(view.route("c"), Route::Pending(_)));
        // Later batches wait too, until a resync replaces the mirror.
        let later = DocumentUpdate::Ops {
            ops: vec![].into(),
            revision: Revision::new(3).unwrap(),
        };
        assert!(view.update(&later).is_err());
        view.update(&replace(r#"{"layers":[]}"#, 4)).unwrap();
        assert!(!matches!(
            view.route("c"),
            Route::Pending("a patch batch did not apply; waiting for a resync")
        ));
    }
}
