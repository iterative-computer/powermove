//! Studio 3D scene layers (models, lights, cameras, environment).
//!
//! Ports the 3D branches of `setContent`/`setProperty` in
//! `legacy/core/editing.ts` (`parseLayer3DData`, `parseScene`,
//! `validateLayer3DChannel`, `validateSceneLayer` of `core/scene3d/*`) and
//! the MCP `edit_3d` tool. Not ported yet: the Studio lane defines the scene
//! schema in Rust first.

use pm_model::Value;

use super::Cx;
use crate::error::CommandError;

/// 3D commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Scene3dCommand {
    /// Replace a 3D layer's `d.data` (parsed and validated against the
    /// scene schema); generated origins may not change hand-edited 3D
    /// channels. Label "Edit 3D Scene".
    SetData { layer_id: String, data: Value },
    /// The MCP `edit_3d` operation list against a scene layer. Label "Edit
    /// 3D Scene".
    Edit { layer_id: String, ops: Value },
}

impl Scene3dCommand {
    pub fn label(&self) -> String {
        "Edit 3D Scene".to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            Scene3dCommand::SetData { .. } => "set_3d_data",
            Scene3dCommand::Edit { .. } => "edit_3d",
        }))
    }
}
