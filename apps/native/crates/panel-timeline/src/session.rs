//! Semantic UI session patches preserve opaque legacy/extension fields (N4).
use crate::{Disclosure, GraphDisplay, GraphMode, TimelineMode, TimelineModel};
use pm_kernel::state::UiState;
use pm_model::Value;
impl TimelineModel {
    pub fn restore_ui(&mut self, ui: &UiState) {
        let value = &ui.timeline;
        self.graph_display = if value.get("graph").and_then(Value::as_bool) == Some(true) {
            if value.get("graphCombined").and_then(Value::as_bool) == Some(true) {
                GraphDisplay::Combined
            } else {
                GraphDisplay::Graph
            }
        } else {
            GraphDisplay::DopeSheet
        };
        self.graph_mode = if value.get("graphMode").and_then(Value::as_str) == Some("speed") {
            GraphMode::Speed
        } else {
            GraphMode::Value
        };
        self.key_selection_active = value
            .get("keySelectionActive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.track_snap_disabled = value.get("trackSnap").and_then(Value::as_bool) == Some(false);
        self.hide_shy = value.get("hideShy").and_then(Value::as_bool) == Some(true);
        self.mode = if value.get("mode").and_then(Value::as_str) == Some("tracks") {
            TimelineMode::Tracks
        } else {
            TimelineMode::Layers
        };
        if let Some(layers) = value
            .get("nativeModel")
            .and_then(|v| v.get("disclosure"))
            .and_then(Value::as_object)
        {
            for (id, d) in layers.iter() {
                if let Some(id) = id.as_str() {
                    let paths = d.get("paths").and_then(Value::as_array).map(|p| {
                        p.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    });
                    self.reveal.layers.insert(
                        id.into(),
                        Disclosure {
                            collapsed: d.get("collapsed").and_then(Value::as_bool).unwrap_or(true),
                            group_collapsed: d
                                .get("groupCollapsed")
                                .and_then(Value::as_bool)
                                .unwrap_or(true),
                            paths,
                        },
                    );
                }
            }
        }
        self.rebuild_rows();
    }
    /// Patch only owned semantic members. pps, scroll, extension state, unknown
    /// disclosure fields, transport, viewer and session values remain intact.
    pub fn patch_ui(&self, old: &UiState) -> UiState {
        let mut ui = old.clone();
        let mut timeline = ui.timeline.as_object().cloned().unwrap_or_default();
        timeline.insert(
            "graph",
            Value::Bool(self.graph_display != GraphDisplay::DopeSheet),
        );
        timeline.insert(
            "graphCombined",
            Value::Bool(self.graph_display == GraphDisplay::Combined),
        );
        timeline.insert(
            "graphMode",
            Value::from(if self.graph_mode == GraphMode::Speed {
                "speed"
            } else {
                "value"
            }),
        );
        timeline.insert("keySelectionActive", Value::Bool(self.key_selection_active));
        timeline.insert(
            "mode",
            Value::from(if self.mode == TimelineMode::Tracks {
                "tracks"
            } else {
                "layers"
            }),
        );
        timeline.insert("trackSnap", Value::Bool(!self.track_snap_disabled));
        timeline.insert("hideShy", Value::Bool(self.hide_shy));
        let mut native = timeline
            .get("nativeModel")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut layers = native
            .get("disclosure")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (id, d) in &self.reveal.layers {
            let mut value = layers
                .get(id.as_str())
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            value.insert("collapsed", Value::Bool(d.collapsed));
            value.insert("groupCollapsed", Value::Bool(d.group_collapsed));
            value.insert(
                "paths",
                d.paths
                    .as_ref()
                    .map(|p| Value::Array(p.iter().cloned().map(Value::from).collect()))
                    .unwrap_or(Value::Null),
            );
            layers.insert(id.as_str(), Value::Object(value));
        }
        native.insert("disclosure", Value::Object(layers));
        timeline.insert("nativeModel", Value::Object(native));
        ui.timeline = Value::Object(timeline);
        ui
    }
}
