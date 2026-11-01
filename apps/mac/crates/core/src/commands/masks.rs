//! Masks and vector paths.
//!
//! Ports `PM.mkMask` (`legacy/core/model.ts`), the inspector's mask rows,
//! and the pen tool's results (`extensions/viewer/path-editing.ts`): the
//! viewer computes vertices and tangents in layer space; the core writes the
//! resulting path, keyed at the playhead when the path is animated.

use pm_model::Value;

use super::Cx;
use crate::error::CommandError;

/// Mask shapes (`PM.MASK_SHAPES`, plus free paths from the pen).
#[derive(Clone, Debug, PartialEq)]
pub enum MaskShape {
    Rect,
    Ellipse,
    /// A path value `{ closed, vertices: [{ x, y, in, out }] }`.
    Path(Value),
}

/// The editable mask fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaskPatch {
    /// `add`, `subtract`, `intersect`, `difference`.
    pub mode: Option<String>,
    pub on: Option<bool>,
    pub inverted: Option<bool>,
    pub name: Option<String>,
}

/// Where a pen edit lands.
#[derive(Clone, Debug, PartialEq)]
pub enum PathTarget {
    /// A mask's `shape` channel.
    Mask(String),
    /// A shape layer's vector path channel (the structured key, `mp.<id>`
    /// or `g.<group>.path`, as `PM.findProp` names it).
    Vector(String),
}

/// Mask and path commands. Locked layers (or locked groups above) refuse.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MaskCommand {
    /// A new mask `mask_id` (`PM.mkMask`: half the composition's shorter
    /// side, feather 24, mode `add`). Label "Add Mask".
    Add {
        layer_id: String,
        mask_id: String,
        shape: MaskShape,
    },
    /// Remove masks by id. Label "Remove Mask".
    Remove {
        layer_id: String,
        mask_ids: Vec<String>,
    },
    /// Mode, on, inverted, name. Label "Edit Mask".
    Set {
        layer_id: String,
        mask_id: String,
        patch: MaskPatch,
    },
    /// Reorder a mask. Label "Reorder Mask".
    Reorder {
        layer_id: String,
        mask_id: String,
        index: usize,
    },
    /// A pen edit (add/move/delete vertex, tangents, close): the whole new
    /// path, keyed at `time` when the channel is animated. Label "Edit
    /// Path".
    SetPath {
        layer_id: String,
        target: PathTarget,
        path: Value,
        time: f64,
    },
}

impl MaskCommand {
    pub fn label(&self) -> String {
        match self {
            MaskCommand::Add { .. } => "Add Mask",
            MaskCommand::Remove { .. } => "Remove Mask",
            MaskCommand::Set { .. } => "Edit Mask",
            MaskCommand::Reorder { .. } => "Reorder Mask",
            MaskCommand::SetPath { .. } => "Edit Path",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            MaskCommand::Add { .. } => "add_mask",
            MaskCommand::Remove { .. } => "remove_mask",
            MaskCommand::Set { .. } => "set_mask",
            MaskCommand::Reorder { .. } => "reorder_mask",
            MaskCommand::SetPath { .. } => "set_path",
        }))
    }
}
