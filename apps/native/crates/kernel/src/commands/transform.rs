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

/// How Distribute spaces the layers (`pm_edit::align::Spread`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    /// Edges or centers at equal steps between the two outermost layers.
    Position(AlignMode),
    /// Equal gaps between neighbouring boxes.
    HorizontalSpace,
    VerticalSpace,
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
    /// Center Anchor Point in layer content without moving the picture
    /// (`centerAnchor`). Locked layers and layers without bounds are
    /// skipped. Label "Center Anchor Point".
    CenterAnchor { layer_ids: Vec<String>, time: f64 },
    /// The alignment strip's Align (`pm_edit::align::align`): the layers'
    /// common box meets the composition frame's edge or center, every layer
    /// moving by the same world delta, written as `set_property` on
    /// position (keyed when animated, overriding hand-edit markers as the strip
    /// does). Text needs the host's fonts. Label "Align Layers".
    Align {
        layer_ids: Vec<String>,
        mode: AlignMode,
        time: f64,
    },
    /// Distribute (`pm_edit::align::distribute`): at least three layers;
    /// the outermost stay. Text needs the host's fonts. Label "Distribute
    /// Layers".
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
            TransformCommand::CenterAnchor { layer_ids, time } => {
                center_anchor(cx, layer_ids, *time)
            }
            TransformCommand::Align {
                layer_ids,
                mode,
                time,
            } => align(cx, layer_ids, Some(kernel_mode(*mode)), None, *time),
            TransformCommand::Distribute {
                layer_ids,
                spread,
                time,
            } => {
                let spread = match spread {
                    Spread::Position(m) => pm_edit::align::Spread::Position(kernel_mode(*m)),
                    Spread::HorizontalSpace => pm_edit::align::Spread::HorizontalSpace,
                    Spread::VerticalSpace => pm_edit::align::Spread::VerticalSpace,
                };
                align(cx, layer_ids, None, Some(spread), *time)
            }
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

fn kernel_mode(m: AlignMode) -> pm_edit::align::Mode {
    use pm_edit::align::Mode;
    match m {
        AlignMode::Left => Mode::Left,
        AlignMode::HorizontalCenter => Mode::HCenter,
        AlignMode::Right => Mode::Right,
        AlignMode::Top => Mode::Top,
        AlignMode::VerticalCenter => Mode::VCenter,
        AlignMode::Bottom => Mode::Bottom,
    }
}

/// `centerAnchor`: the anchor moves to the center of the layer's bounds and
/// the position compensates through the layer matrix, so the picture stays.
fn center_anchor(cx: &mut Cx<'_>, layer_ids: &[String], time: f64) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    for id in layer_ids {
        let loc = cx.find(id)?;
        if cx.check_lock(&loc, id).is_err() {
            continue;
        }
        let is_text = cx
            .layer_node(&loc)?
            .get("type")
            .and_then(crate::model::str_of)
            == Some("text");
        let mut measure = cx.measure(is_text)?;
        let index = loc.index;
        let centered = cx.kernel(loc.comp_id.as_deref(), |edit| {
            edit.read(|s| {
                let l = &s.layers()[index];
                let Some(b) = s.bounds(l, time, &mut *measure, &mut pm_eval::bounds::NoHost)?
                else {
                    return Ok(None);
                };
                let (next_x, next_y) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
                let dx = next_x - s.channel(l, "anchor.x", time)?.to_number();
                let dy = next_y - s.channel(l, "anchor.y", time)?.to_number();
                let m = s.local_matrix(l, time)?;
                let x = s.channel(l, "position.x", time)?.to_number();
                let y = s.channel(l, "position.y", time)?.to_number();
                Ok(Some([
                    next_x,
                    next_y,
                    x + m[0] * dx + m[2] * dy,
                    y + m[1] * dx + m[3] * dy,
                ]))
            })
        })?;
        cx.check_fonts()?;
        let Some(values) = centered else { continue };
        for (channel, value) in ["anchor.x", "anchor.y", "position.x", "position.y"]
            .into_iter()
            .zip(values)
        {
            let mut c = SetProperty::new(id.clone(), channel, value, time);
            c.preserve_hand_edits = false;
            c.mark_intent = true;
            properties::set_property(cx, &c)?;
        }
    }
    Ok(())
}

/// Align or distribute through the engine's kernel, then the moves as
/// property writes.
fn align(
    cx: &mut Cx<'_>,
    layer_ids: &[String],
    mode: Option<pm_edit::align::Mode>,
    spread: Option<pm_edit::align::Spread>,
    time: f64,
) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    let Some(first) = layer_ids.first() else {
        return Ok(());
    };
    let comp = cx.find(first)?.comp_id;
    let mut indexes = Vec::new();
    let mut has_text = false;
    for id in layer_ids {
        let loc = cx.find(id)?;
        cx.check_lock(&loc, id)?;
        if loc.comp_id != comp {
            continue;
        }
        let layer = cx.layer_node(&loc)?;
        has_text |= layer.get("type").and_then(crate::model::str_of) == Some("text");
        indexes.push(loc.index);
    }
    let ids = cx.layer_ids(comp.as_deref());
    let mut measure = cx.measure(has_text)?;
    let moves = cx.kernel(comp.as_deref(), |edit| match (mode, spread) {
        (Some(m), _) => pm_edit::align::align(
            edit,
            &indexes,
            m,
            time,
            &mut *measure,
            &mut pm_eval::bounds::NoHost,
        ),
        (_, Some(s)) => pm_edit::align::distribute(
            edit,
            &indexes,
            s,
            time,
            &mut *measure,
            &mut pm_eval::bounds::NoHost,
        ),
        _ => Ok(Vec::new()),
    })?;
    cx.check_fonts()?;
    for m in moves {
        let Some(id) = ids.get(m.layer) else { continue };
        let mut c = SetProperty::new(id.clone(), m.channel, m.value, time);
        c.preserve_hand_edits = false;
        properties::set_property(cx, &c)?;
    }
    Ok(())
}
