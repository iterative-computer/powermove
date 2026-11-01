//! Layer groups.
//!
//! Ports `PM.groupLayers`, `PM.ungroupLayers`, `PM.moveToGroup` and
//! `PM.normalizeGroupStack` of `legacy/core/layer-groups.ts`, with the
//! geometry from the engine's `pm-edit` kernels (`groups::group_placement`,
//! `groups::move_to_group`, `groups::ungroup`), and the group lock policy of
//! `policyCommand`.

use super::Cx;
use crate::error::CommandError;

/// Group commands. A locked target refuses the whole command.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GroupCommand {
    /// `group_layers` (Cmd+G): a new group layer `group_id` named `name`
    /// (default "Group") above the topmost selected layer, its pivot at the
    /// members' bounds center; the layers keep their world poses. Layers in
    /// different groups refuse (`Select layers in the same group`). Label
    /// "Group Layers".
    Group {
        layer_ids: Vec<String>,
        group_id: String,
        name: Option<String>,
        time: f64,
    },
    /// `ungroup_layers` (Cmd+Shift+G): dissolves the groups, moving members
    /// to the group's own parent group with world pose, aim, opacity and
    /// visibility preserved (animated groups are baked to keys). Label
    /// "Ungroup".
    Ungroup { group_ids: Vec<String>, time: f64 },
    /// `move_to_group`: moves layers into `group` (or out of every group
    /// with `None`) keeping their world poses. Label "Move to Group".
    MoveToGroup {
        layer_ids: Vec<String>,
        group: Option<String>,
        time: f64,
    },
}

impl GroupCommand {
    pub fn label(&self) -> String {
        match self {
            GroupCommand::Group { .. } => "Group Layers",
            GroupCommand::Ungroup { .. } => "Ungroup",
            GroupCommand::MoveToGroup { .. } => "Move to Group",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            GroupCommand::Group { .. } => "group_layers",
            GroupCommand::Ungroup { .. } => "ungroup_layers",
            GroupCommand::MoveToGroup { .. } => "move_to_group",
        }))
    }
}
