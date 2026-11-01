//! Composition markers (`markers: [{ id, t, name }]`, sorted by time).
//!
//! Ports `addMarker` and `removeMarker` of `legacy/core/editing.ts` and the
//! ruler marker menu (`extensions/timeline/markers.ts`).

use super::Cx;
use crate::error::CommandError;

/// Marker commands. `comp: None` is the open composition.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MarkerCommand {
    /// `add_marker` (`addMarker`): at `time` clamped to the composition,
    /// named `name` or `M<n>`. Label "Add Marker".
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
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            MarkerCommand::Add { .. } => "add_marker",
            MarkerCommand::Remove { .. } => "remove_marker",
            MarkerCommand::RemoveById { .. } => "remove_marker",
            MarkerCommand::Move { .. } => "move_marker",
            MarkerCommand::Rename { .. } => "rename_marker",
        }))
    }
}
