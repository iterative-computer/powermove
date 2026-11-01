//! Transform shortcuts that are not a single property write: nudges, anchor
//! centering, align and distribute.
//!
//! Ports the arrow-key nudge (`extensions/keymap-default/keymap.ts`),
//! Center Anchor (Cmd+Option+Home, double-click on the Pan Behind tool) and
//! the alignment strip (`extensions/inspector/align.ts`, the engine's
//! `pm_edit::align` and `pm_edit::anchor` kernels).

use super::Cx;
use crate::error::CommandError;

/// Alignment edge or center (`pm_edit::align::Mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignMode {
    Left,
    HorizontalCenter,
    Right,
    Top,
    VerticalCenter,
    Bottom,
}

/// What alignment is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignTo {
    Selection,
    Composition,
}

/// Distribution axis (`pm_edit::align::Spread`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    Horizontal,
    Vertical,
}

/// Transform commands. Locked layers refuse.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TransformCommand {
    /// Arrow keys: `position.x += dx`, `position.y += dy` (one unit, ten
    /// with Shift), keyed at `time` when position is animated. Label "Nudge".
    Nudge {
        layer_ids: Vec<String>,
        dx: f64,
        dy: f64,
        time: f64,
    },
    /// Center Anchor Point in layer content without moving the picture.
    /// Not ported (needs bounds: `pm_edit::anchor`).
    CenterAnchor { layer_ids: Vec<String>, time: f64 },
    /// Align. Not ported (needs bounds: `pm_edit::align::align`).
    Align {
        layer_ids: Vec<String>,
        mode: AlignMode,
        to: AlignTo,
        time: f64,
    },
    /// Distribute. Not ported (`pm_edit::align::distribute`).
    Distribute {
        layer_ids: Vec<String>,
        spread: Spread,
        time: f64,
    },
}

impl TransformCommand {
    pub fn label(&self) -> String {
        match self {
            TransformCommand::Nudge { .. } => "Nudge",
            TransformCommand::CenterAnchor { .. } => "Center Anchor Point",
            TransformCommand::Align { .. } => "Align Layers",
            TransformCommand::Distribute { .. } => "Distribute Layers",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            TransformCommand::Nudge { .. } => "nudge",
            TransformCommand::CenterAnchor { .. } => "center_anchor",
            TransformCommand::Align { .. } => "align",
            TransformCommand::Distribute { .. } => "distribute",
        }))
    }
}
