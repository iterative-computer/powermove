//! Transform shortcuts that are not a single property write: nudges, anchor
//! centering, align and distribute.
//!
//! Ports the arrow-key nudge (`extensions/keymap-default/keymap.ts`),
//! Center Anchor (Cmd+Option+Home, double-click on the Pan Behind tool) and
//! the alignment strip (`extensions/inspector/align.ts`, the engine's
//! `pm_edit::align` and `pm_edit::anchor` kernels).

use super::properties::{self, SetProperty};
use super::{Cx, values};
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
    /// Arrow keys (`nudgeSelection`): a screen-space move of (dx, dy) (one
    /// unit, ten with Shift) mapped into each layer's parent space, keyed at
    /// `time` when position is animated. Locked and audio layers are
    /// skipped. Label "Nudge".
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
        match self {
            TransformCommand::Nudge {
                layer_ids,
                dx,
                dy,
                time,
            } => nudge(cx, layer_ids, *dx, *dy, *time),
            TransformCommand::CenterAnchor { .. } => {
                Err(CommandError::NotImplemented("center_anchor"))
            }
            TransformCommand::Align { .. } => Err(CommandError::NotImplemented("align")),
            TransformCommand::Distribute { .. } => Err(CommandError::NotImplemented("distribute")),
        }
    }
}

/// `nudgeSelection(PM, dx, dy)`: a world-space nudge mapped into each
/// layer's parent space (`worldNudgeToLocal`), keyed when animated.
fn nudge(
    cx: &mut Cx<'_>,
    layer_ids: &[String],
    dx: f64,
    dy: f64,
    time: f64,
) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    for id in layer_ids {
        let loc = cx.find(id)?;
        if cx.check_lock(&loc, id).is_err() {
            continue;
        }
        let (_, layer) = cx.layer(id)?;
        if values::text(&layer, "type") == Some("audio") {
            continue;
        }
        let index = loc.index;
        let matrix = cx.kernel(loc.comp_id.as_deref(), |edit| {
            edit.read(|s| {
                let l = &s.layers()[index];
                s.transform_parent_matrix(l, time, None)
            })
        })?;
        let det = matrix[0] * matrix[3] - matrix[1] * matrix[2];
        let (lx, ly) = if !det.is_finite() || det.abs() < 1e-9 {
            (0.0, 0.0)
        } else {
            (
                (dx * matrix[3] - dy * matrix[2]) / det,
                (dy * matrix[0] - dx * matrix[1]) / det,
            )
        };
        for (channel, d) in [("position.x", lx), ("position.y", ly)] {
            if d == 0.0 {
                continue;
            }
            let (_, now) = cx.layer(id)?;
            let Some(prop) = now.get("p").and_then(|p| p.get(channel)) else {
                continue;
            };
            let local = super::track::local_time(&now, time);
            let current =
                super::track::js_number(&super::keyframes::value_at(prop, local, channel));
            let mut c = SetProperty::new(id.clone(), channel, current + d, time);
            c.preserve_hand_edits = false;
            properties::set_property(cx, &c)?;
        }
    }
    Ok(())
}
