//! The project's saved sections (`library.sections`).
//!
//! Ports `createSection` and `updateSection` of `legacy/core/editing.ts`.

use pm_model::Value;

use super::Cx;
use crate::error::CommandError;

/// Library commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LibraryCommand {
    /// `create_section`: the manifest goes first; at most 24 are kept
    /// (`Section id already exists`). Label "Save Section".
    CreateSection { section: Value },
    /// `update_section`: new layers (required, non-empty), thumbnail and
    /// time; a version is appended (12 kept); tags follow the layer types
    /// (`Section not found`). Label "Update Section".
    UpdateSection {
        section_id: String,
        layers: Vec<Value>,
        thumb: Option<Value>,
        /// Milliseconds since the epoch (`at`).
        at: f64,
        version: Option<Value>,
    },
}

impl LibraryCommand {
    pub fn label(&self) -> String {
        match self {
            LibraryCommand::CreateSection { .. } => "Save Section",
            LibraryCommand::UpdateSection { .. } => "Update Section",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            LibraryCommand::CreateSection { .. } => "create_section",
            LibraryCommand::UpdateSection { .. } => "update_section",
        }))
    }
}
