//! Worker-side adapter for the built-in v4 viewer. No surface/platform dependency.
//! The application routes project channel `viewer.chrome` and command
//! `viewer.chrome.drive` here. These routes still need application registration.
use crate::*;
use pm_model::{Map, Value};

pub const CHANNEL: &str = "viewer.chrome";
pub const COMMAND: &str = "viewer.chrome.drive";

/// One instance per panel/realm generation, retired with that generation.
/// `m.viewer_3d_collapse`: below this stage width the 3D group becomes one
/// pull-down (3d-studio.md §2). A test keeps it equal to the token.
pub const VIEWER_3D_COLLAPSE: f64 = 560.;
/// An opaque ticket keeps full-width revisions and selection guards in Rust;
/// it avoids shipping 60k selected IDs to JS for every chrome publication.
#[derive(Default)]
pub struct Session {
    ticket: u64,
    stamp: Option<Stamp>,
    viewport: Option<Viewport>,
}
impl Session {
    /// Native Controls/BindingSink entry for the two UI-kit pop-ups. The binding
    /// name is the control ID; menu action sentinels aren't numeric zoom values.
    /// The app chooses this OR worker change delivery, never both for one input.
    pub fn popup(
        &self,
        model: &ViewModel,
        request: &Value,
        viewport: Viewport,
        caller: Caller,
    ) -> Result<Plan, Error> {
        let mut normalized = request.clone();
        match request.get("element").and_then(Value::as_str) {
            Some("viewer.quality") | Some("viewer.studio.menu") | Some("viewer.studio.views") => {}
            Some("viewer.zoom") => {
                let (element, argument) = match request.get("argument") {
                    Some(Value::String(s)) if s == "fit" => ("viewer.fit", Value::Null),
                    Some(Value::String(s)) if s == "actual" => ("viewer.zoom", Value::Number(1.0)),
                    Some(Value::String(s)) if s == "viewer.zoom.in" => {
                        ("viewer.zoom.in", Value::Null)
                    }
                    Some(Value::String(s)) if s == "viewer.zoom.out" => {
                        ("viewer.zoom.out", Value::Null)
                    }
                    Some(Value::String(s)) if s == "viewer.fit" => ("viewer.fit", Value::Null),
                    Some(Value::Number(n)) if ZOOM_PRESETS.contains(n) => {
                        ("viewer.zoom", Value::Number(*n))
                    }
                    _ => return Err(Error::Invalid("zoom menu choice")),
                };
                let map = normalized
                    .as_object_mut()
                    .ok_or(Error::Invalid("viewer request"))?;
                map.insert("element", Value::from(element));
                map.insert("argument", argument);
            }
            _ => return Err(Error::Invalid("viewer pop-up")),
        }
        self.drive(model, &normalized, viewport, caller)
    }

    pub fn publish(&mut self, model: &ViewModel, viewport: Viewport) -> Result<Value, Error> {
        if viewport.composition != model.size {
            return Err(Error::Invalid("viewport composition"));
        }
        let shown = model.chrome.navigation.shown(viewport)?;
        let lost = model.chrome.navigation.out_of_view(viewport, false)?;
        let root = model.snapshot.root().ok_or(Error::Closed)?;
        let comp = pm_kernel::model::comp_node(root, model.composition.as_deref())
            .ok_or(Error::Invalid("composition"))?;
        let title = match comp.get("name").map(Node::kind) {
            Some(NodeKind::String(name)) => name.clone(),
            None => pm_model::JsString::from(""),
            _ => return Err(Error::Invalid("composition name")),
        };
        let empty = match comp.get("layers").map(Node::kind) {
            Some(NodeKind::Array(layers)) => layers.is_empty(),
            _ => return Err(Error::Invalid("layers")),
        };
        // A composition with Studio 3D layers shows the Fast/Quality preview
        // control (3d-studio.md §2); without them the bar is unchanged.
        let studio = match comp.get("layers").map(Node::kind) {
            Some(NodeKind::Array(layers)) => layers.iter().any(|l| {
                matches!(
                    l.get("d").and_then(|d| d.get("definition")).map(Node::kind),
                    Some(NodeKind::String(s)) if s.to_string_lossy().starts_with("powermove.3d.")
                )
            }),
            _ => false,
        };
        let ticket = self
            .ticket
            .checked_add(1)
            .ok_or(Error::Invalid("ticket exhausted"))?;
        let mut value = Map::new();
        value.insert("ticket", Value::from(ticket.to_string()));
        value.insert("project", Value::from(model.stamp.project.as_str()));
        value.insert(
            "composition",
            model
                .composition
                .as_deref()
                .map(Value::from)
                .unwrap_or(Value::Null),
        );
        value.insert("name", Value::String(title));
        value.insert("fit", Value::Bool(model.chrome.navigation.fit));
        value.insert("shownZoom", Value::Number(shown));
        value.insert("quality", Value::from(model.chrome.quality.id()));
        value.insert(
            "effectiveQuality",
            Value::Number(model.chrome.effective_quality),
        );
        value.insert("canvasLost", Value::Bool(lost));
        value.insert(
            "studio",
            if studio {
                let tier = match model.ui.viewer.get("studioTier").and_then(Value::as_str) {
                    Some("refine") => "refine",
                    _ => "live",
                };
                let read = |k: &str, d: &'static str| {
                    model
                        .ui
                        .viewer
                        .get(k)
                        .and_then(Value::as_str)
                        .map(|s| Value::from(s))
                        .unwrap_or_else(|| Value::from(d))
                };
                let mut m = Map::new();
                m.insert("tier", Value::from(tier));
                m.insert("tool", read("studioTool", "move"));
                m.insert("look", read("studioLook", "camera"));
                m.insert("axes", read("studioAxes", "scene"));
                m.insert("views", read("studioViews", "1"));
                // Below the collapse width the group becomes one pull-down.
                m.insert(
                    "compact",
                    // The bar spans every pane: 2 and 4 views are two columns.
                    Value::Bool({
                        let columns = match model.ui.viewer.get("studioViews").and_then(Value::as_str) {
                            Some("2" | "4") => 2.,
                            _ => 1.,
                        };
                        viewport.stage.width * columns < VIEWER_3D_COLLAPSE
                    }),
                );
                Value::Object(m)
            } else {
                Value::Null
            },
        );
        value.insert("showControls", Value::Bool(model.chrome.show_controls));
        value.insert("empty", Value::Bool(empty));
        value.insert(
            "selectionCount",
            Value::Number(model.selection.layers().len() as f64),
        );
        self.ticket = ticket;
        self.stamp = Some(model.stamp.clone());
        self.viewport = Some(viewport);
        Ok(Value::Object(value))
    }

    /// Plan only; Controller::submit preserves the real pending actor ack.
    /// The native binding adapter uses this same entry for pop-up writes;
    /// change notifications must not submit a second copy of a native write.
    pub fn drive(
        &self,
        model: &ViewModel,
        request: &Value,
        viewport: Viewport,
        caller: Caller,
    ) -> Result<Plan, Error> {
        let map = request
            .as_object()
            .ok_or(Error::Invalid("viewer request"))?;
        if map.len() != 3
            || request.get("ticket").and_then(Value::as_str)
                != Some(self.ticket.to_string().as_str())
            || self.stamp.as_ref() != Some(&model.stamp)
            || self.viewport != Some(viewport)
        {
            return Err(Error::Stale);
        }
        let id = request
            .get("element")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid("viewer element"))?;
        let argument = request
            .get("argument")
            .ok_or(Error::Invalid("viewer argument"))?
            .clone();
        model.drive(id, argument, Some(viewport), caller)
    }
}

#[cfg(test)]
mod collapse_tests {
    #[test]
    fn the_collapse_width_is_the_token() {
        let t = pm_design::Tokens::new(Default::default(), Default::default());
        assert_eq!(super::VIEWER_3D_COLLAPSE, t.metrics.viewer_3d_collapse as f64);
    }
}
