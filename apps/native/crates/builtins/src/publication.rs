//! Decide whether a shipped panel needs a V8 publication from its native inputs.
use crate::data::Backend;
use pm_kernel::Published;
use serde_json::Value;
use std::sync::Arc;

pub(crate) struct Inputs {
    published: Arc<Published>,
    state_revision: u64,
    media_revision: u64,
    facts: Vec<Value>,
    time: f64,
}

impl Inputs {
    pub(crate) fn read(backend: &Backend) -> Option<Self> {
        let facts = match backend.panel_id.as_str() {
            "inspector" => &["playing", "layerFailures", "focusPart"][..],
            "viewer" => &[
                "stage",
                "effectiveQuality",
                "studioDenoiser",
                "studioRefineProgress",
                "timelineShown",
                "playing",
                "looping",
                "channels",
                "frameLoading",
                "frameFailed",
            ],
            "timeline" => &["playing"],
            "layer-effects" | "color" | "assets" | "fxbrowser" | "easing" => &[],
            _ => return None,
        };
        Some(Self {
            published: backend.published()?,
            state_revision: backend.kernel.state().revision,
            media_revision: if backend.panel_id == "assets" {
                backend.native.media_sequence
            } else {
                0
            },
            facts: facts
                .iter()
                .map(|key| backend.native.facts[*key].clone())
                .collect(),
            time: backend.time(),
        })
    }

    pub(crate) fn unchanged(&self, current: &Self, panel: &str, reads_time: bool) -> bool {
        let old = &self.published;
        let new = &current.published;
        if old.snapshot.doc_id != new.snapshot.doc_id
            || old.revision() != new.revision()
            || (panel != "assets" && self.state_revision != current.state_revision)
            || self.media_revision != current.media_revision
            || self.facts != current.facts
            || (reads_time && self.time != current.time)
        {
            return false;
        }
        match panel {
            "assets" => true,
            "timeline" => old.selection == new.selection && old.workspace == new.workspace,
            "fxbrowser" | "layer-effects" | "color" => {
                old.selection.layers() == new.selection.layers()
            }
            "easing" => {
                old.selection.keys() == new.selection.keys()
                    && old.selection.key_layers() == new.selection.key_layers()
                    && (new.selection.keys().is_empty()
                        || old.selection.layers() == new.selection.layers())
            }
            _ => old.selection == new.selection,
        }
    }

    // The actor can publish during a query. Only record the snapshot when
    // all queries had the same inputs available throughout the worker call.
    pub(crate) fn same_publication(&self, current: &Self) -> bool {
        Arc::ptr_eq(&self.published, &current.published)
            && self.state_revision == current.state_revision
    }
}
