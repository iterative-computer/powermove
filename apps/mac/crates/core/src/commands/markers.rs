//! Composition markers (`markers: [{ id, t, name }]`, sorted by time).
//!
//! Ports `addMarker` and `removeMarker` of `legacy/core/editing.ts` and the
//! ruler marker menu (`extensions/timeline/markers.ts`).

use pm_model::{Map, Value};

use super::{Cx, values};
use crate::error::CommandError;

/// Marker commands. `comp: None` is the open composition.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MarkerCommand {
    /// `add_marker` (`addMarker`): at `time` clamped to the composition,
    /// named `name` or `M<n>`, kept sorted by time. Label "Add Marker".
    Add {
        comp: Option<String>,
        id: String,
        time: f64,
        name: Option<String>,
    },
    /// `remove_marker` by index in time order (`Marker not found`). Label
    /// "Remove Marker".
    Remove { comp: Option<String>, index: usize },
    /// Remove a marker by id (the ruler menu). Label "Remove Marker".
    RemoveById { comp: Option<String>, id: String },
    /// Drag a marker (clamped, re-sorted). Label "Move Marker".
    Move {
        comp: Option<String>,
        id: String,
        time: f64,
    },
    /// Rename a marker. Label "Rename Marker".
    Rename {
        comp: Option<String>,
        id: String,
        name: String,
    },
}

impl MarkerCommand {
    pub fn label(&self) -> String {
        match self {
            MarkerCommand::Add { .. } => "Add Marker",
            MarkerCommand::Remove { .. } | MarkerCommand::RemoveById { .. } => "Remove Marker",
            MarkerCommand::Move { .. } => "Move Marker",
            MarkerCommand::Rename { .. } => "Rename Marker",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let comp = match self {
            MarkerCommand::Add { comp, .. }
            | MarkerCommand::Remove { comp, .. }
            | MarkerCommand::RemoveById { comp, .. }
            | MarkerCommand::Move { comp, .. }
            | MarkerCommand::Rename { comp, .. } => comp.clone(),
        };
        let mut path = cx.comp_path(comp.as_deref());
        path.push("markers".into());
        let mut markers: Vec<Value> = cx
            .value_at(&path)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let dur = cx.duration(comp.as_deref());
        let not_found = || CommandError::Invalid("Marker not found".into());
        let by_id = |markers: &[Value], id: &str| {
            markers
                .iter()
                .position(|m| values::text(m, "id") == Some(id))
        };
        match self {
            MarkerCommand::Add { id, time, name, .. } => {
                let t = values::clamp(values::finite(*time, "marker time")?, 0.0, dur);
                let mut m = Map::new();
                m.insert("id", values::s(id));
                m.insert("t", Value::Number(t));
                let label = match name.as_deref().filter(|n| !n.is_empty()) {
                    Some(n) => n.to_owned(),
                    None => format!("M{}", markers.len() + 1),
                };
                m.insert("name", values::s(&label));
                markers.push(Value::Object(m));
            }
            MarkerCommand::Remove { index, .. } => {
                if *index >= markers.len() {
                    return Err(not_found());
                }
                markers.remove(*index);
            }
            MarkerCommand::RemoveById { id, .. } => {
                let i = by_id(&markers, id).ok_or_else(not_found)?;
                markers.remove(i);
            }
            MarkerCommand::Move { id, time, .. } => {
                let i = by_id(&markers, id).ok_or_else(not_found)?;
                let t = values::clamp(values::finite(*time, "marker time")?, 0.0, dur);
                values::obj(&mut markers[i]).insert("t", Value::Number(t));
            }
            MarkerCommand::Rename { id, name, .. } => {
                let i = by_id(&markers, id).ok_or_else(not_found)?;
                values::obj(&mut markers[i]).insert("name", values::s(name));
            }
        }
        // `markers.sort((a, b) => a.t - b.t)` (stable).
        markers.sort_by(|a, b| {
            let (x, y) = (
                values::num(a, "t").unwrap_or(0.0),
                values::num(b, "t").unwrap_or(0.0),
            );
            x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
        });
        cx.replace(path, Value::Array(markers))
    }
}
