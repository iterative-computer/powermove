//! Prepare the selected view's video decoders when document/media answers
//! arrive. This belongs to open/import, before the first redraw is requested.

use super::*;

/// Structural/source edits can reveal a video whose asset was already
/// resolved. Transform drags keep their existing decoder and avoid another
/// plan build for each pointer update.
pub(super) fn changes_video_sources(update: &DocumentUpdate) -> bool {
    use pm_model::ops::Op;
    fn changed(path: &[pm_model::patch::PathPart]) -> bool {
        let key = |i: usize| path.get(i).map(|p| p.property_key().to_string_lossy());
        if path.is_empty() || matches!(key(0).as_deref(), Some("assets" | "color")) {
            return true;
        }
        // The edit log and the project's own metadata, which every edit
        // writes beside its change, name no media.
        if matches!(
            key(0).as_deref(),
            Some("revision" | "edits" | "name" | "created" | "notes" | "exportDefaults")
        ) {
            return false;
        }
        if path.len() == 1 || (key(0).as_deref() == Some("comps") && path.len() <= 2) {
            return true;
        }
        let Some(layer) =
            (0..path.len()).rfind(|&i| matches!(key(i).as_deref(), Some("layers" | "children")))
        else {
            return false;
        };
        if path.len() <= layer + 2 {
            return true;
        }
        match key(layer + 2).as_deref() {
            Some("id" | "type" | "compat" | "on" | "from" | "dur") => true,
            Some("d") => {
                path.len() == layer + 3
                    || matches!(
                        key(layer + 3).as_deref(),
                        Some("asset" | "comp" | "trim" | "speed" | "timeRemap" | "sourceTime")
                    )
            }
            _ => false,
        }
    }
    match update {
        DocumentUpdate::Replace { .. } => true,
        DocumentUpdate::Ops { ops, .. } => ops
            .iter()
            .any(|op| !matches!(op, Op::Order { .. }) && changed(op.path())),
        DocumentUpdate::Patches { patches, .. } => patches.iter().any(|patch| changed(&patch.path)),
    }
}

impl Thread {
    pub(super) fn prepare_video_media(&mut self, key: Key) {
        let (Some(doc), Some(view)) = (self.docs.get(&key), self.views.get(&key)) else {
            return;
        };
        if !doc.has_document() || !doc.media_ready() {
            return;
        }
        let has_video = pm_engine::document::asset_records(doc.document().root())
            .iter()
            .any(|(_, record)| {
                matches!(record.get("kind").map(|n| n.kind()),
                    Some(pm_model::NodeKind::String(s)) if s.to_string_lossy() == "video")
            });
        if !has_video {
            return;
        }
        let composition = view.state.composition.clone();
        let time = Ticks::new(view.clock.media_at(now())).unwrap_or(Ticks::ZERO);
        let mut request = pm_render::RenderRequest::sized(
            view.state.size,
            pm_protocol::color::Precision::Float16,
        );
        request.sampling = pm_render::sampling::Sampling::Playback;
        request.view = view.state.view.clone();
        if self.gpu().is_none() {
            return;
        }
        let doc = self.docs.get(&key).expect("checked above");
        let composer = self.gpu.as_mut().expect("created above").composer_with(
            &self.config,
            self.frame_cache,
            &mut self.prepared,
        );
        let _ = composer.use_definitions(key, doc.definitions(), &[]);
        match composer.renderer().prepare_media(
            doc.document(),
            composition.as_str(),
            time,
            &request,
            doc.media(),
        ) {
            Ok(prepared) => {
                self.media_unprepared.remove(&key);
                for failure in prepared.failures {
                    self.log(LogLevel::Info, format!("video preparation: {failure}"));
                }
            }
            // The view state names a composition the document does not hold
            // yet (it arrives in a later update): prepare on that update.
            Err(pm_render::RenderError::Composition(_)) => {
                self.media_unprepared.insert(key);
            }
            Err(error) => {
                self.media_unprepared.remove(&key);
                self.log(LogLevel::Info, format!("video preparation: {error}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::ops::Op;
    use pm_model::patch::{Patch, PathPart};

    #[test]
    fn resolved_asset_layer_insert_prepares_but_transform_drags_do_not() {
        let layers: Vec<PathPart> = vec!["comps".into(), "c".into(), "layers".into()];
        let insert = DocumentUpdate::Ops {
            ops: vec![Op::Insert {
                path: layers.clone(),
                index: 0,
                value: pm_model::json::parse(r#"{"id":"v","type":"video","d":{"asset":"clip"}}"#)
                    .unwrap(),
            }]
            .into(),
            revision: Revision::ZERO,
        };
        assert!(changes_video_sources(&insert));
        let patch = |tail: &[&str]| {
            let mut path = layers.clone();
            path.push(0usize.into());
            path.extend(tail.iter().map(|part| PathPart::from(*part)));
            DocumentUpdate::Patches {
                patches: vec![Patch::set(path, pm_model::json::Value::Number(1.0))].into(),
                revision: Revision::ZERO,
            }
        };
        assert!(changes_video_sources(&patch(&["d", "asset"])));
        assert!(changes_video_sources(&patch(&["on"])));
        for field in ["trim", "speed", "timeRemap", "sourceTime"] {
            assert!(changes_video_sources(&patch(&["d", field])));
        }
        assert!(changes_video_sources(&DocumentUpdate::Patches {
            patches: vec![Patch::set(
                vec!["color".into(), "working".into()],
                pm_model::json::Value::String("linearLight".into())
            )]
            .into(),
            revision: Revision::ZERO,
        }));
        assert!(!changes_video_sources(&patch(&["p", "position.x", "v"])));
        assert!(!changes_video_sources(&patch(&["p", "rotation", "v"])));
        // Every edit also writes the project's revision and edit log: a
        // keyframe change with them still keeps the decoders.
        let root = |key: &str| Patch::set(vec![key.into()], pm_model::json::Value::Number(1.0));
        let mut keyframe = layers.clone();
        keyframe.extend([0usize.into(), "p".into(), "rotation".into(), "kf".into(), 3usize.into(), "v".into()]);
        assert!(!changes_video_sources(&DocumentUpdate::Patches {
            patches: vec![root("revision"), root("edits"), Patch::set(keyframe, pm_model::json::Value::Number(0.5))].into(),
            revision: Revision::ZERO,
        }));
        for key in ["compId", "comps", "params"] {
            assert!(changes_video_sources(&DocumentUpdate::Patches {
                patches: vec![root("revision"), root(key)].into(),
                revision: Revision::ZERO,
            }));
        }
        assert!(!changes_video_sources(&DocumentUpdate::Ops {
            ops: vec![Op::Order {
                path: layers,
                ids: vec!["v".into()]
            }]
            .into(),
            revision: Revision::ZERO,
        }));
    }
}
