//! Transform shortcuts that are not a single property write: nudges, anchor
//! centering, align and distribute.
//!
//! Ports the arrow-key nudge (`extensions/keymap-default/keymap.ts`),
//! Center Anchor (Cmd+Option+Home, double-click on the Pan Behind tool) and
//! the alignment strip. Native alignment uses projected anchors; the frozen
//! Electron strip and pm_edit::align retain their original bounds semantics.

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

impl AlignMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Align Left",
            Self::HorizontalCenter => "Align Horizontal Centres",
            Self::Right => "Align Right",
            Self::Top => "Align Top",
            Self::VerticalCenter => "Align Vertical Centres",
            Self::Bottom => "Align Bottom",
        }
    }
}

/// How Distribute spaces the layer anchors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    /// Anchors at equal steps along the mode's axis.
    Position(AlignMode),
    /// Equal spacing between anchors along the selected axis.
    HorizontalSpace,
    VerticalSpace,
}

impl Spread {
    pub fn label(self) -> &'static str {
        match self {
            Self::Position(m) => match m {
                AlignMode::Left | AlignMode::HorizontalCenter | AlignMode::Right => {
                    "Distribute Horizontally"
                }
                _ => "Distribute Vertically",
            },
            Self::HorizontalSpace => "Distribute Horizontally",
            Self::VerticalSpace => "Distribute Vertically",
        }
    }
}

/// Transform commands. Locked layers are skipped.
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
    /// Align projected anchors to the comp for one editable layer, or to the
    /// selected anchor extent for several. Position writes form one undo step.
    Align {
        layer_ids: Vec<String>,
        mode: AlignMode,
        time: f64,
    },
    /// Distribute: at least three editable layers;
    /// the outermost anchors stay.
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
            TransformCommand::Align { mode, .. } => mode.label(),
            TransformCommand::Distribute { spread, .. } => spread.label(),
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

/// Position coordinates mapped to composition pixels, including enclosing
/// groups, parenting and perspective. The layer's own anchor cancels its own
/// transform at this point, so its position is the point to project.
fn position_plane<'d>(
    s: &mut pm_eval::Scope<'_, 'd>,
    layer: &'d pm_model::Node,
    time: f64,
) -> pm_eval::JsResult<pm_eval::space3d::Mat3> {
    if s.is_3d_layer(layer)? {
        s.plane_matrix(layer, time, true)
    } else {
        let m = s.transform_parent_matrix(layer, time, None)?;
        Ok([m[0], m[1], 0.0, m[2], m[3], 0.0, m[4], m[5], 1.0])
    }
}

fn align(
    cx: &mut Cx<'_>,
    layer_ids: &[String],
    mode: Option<pm_edit::align::Mode>,
    spread: Option<pm_edit::align::Spread>,
    time: f64,
) -> Result<(), CommandError> {
    use pm_edit::align::{Mode, Spread};
    use pm_eval::space3d::{inverse_plane, project_point};
    let time = values::finite(time, "time")?;
    let Some(first) = layer_ids.first() else {
        return Ok(());
    };
    let comp = cx.find(first)?.comp_id;
    let mut indexes = Vec::new();
    for id in layer_ids {
        let loc = cx.find(id)?;
        if loc.comp_id == comp && cx.check_lock(&loc, id).is_ok() && !indexes.contains(&loc.index) {
            indexes.push(loc.index);
        }
    }
    let ids = cx.layer_ids(comp.as_deref());
    let (mut points, size) = cx.kernel(comp.as_deref(), |edit| {
        edit.read(|s| {
            let mut points = Vec::new();
            for &i in &indexes {
                let l = &s.layers()[i];
                if pm_eval::node::is_str(pm_eval::node::member(Some(l), "type"), "audio") {
                    continue;
                }
                let m = position_plane(s, l, time)?;
                if inverse_plane(&m).is_none() {
                    continue;
                }
                let x = s.value3d(l, "position.x", time, 0.0)?;
                let y = s.value3d(l, "position.y", time, 0.0)?;
                let (x, y) = project_point(&m, x, y);
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                // Ancestors must be written first. Re-evaluate the child's plane
                // afterwards so selecting both never applies the parent's move twice.
                let mut ancestors = Vec::new();
                let mut pending = s.group_ancestors(l)?;
                pending.extend(s.parent_by_id(l));
                while let Some(p) = pending.pop() {
                    let Some(j) = s.layers().iter().position(|n| std::ptr::eq(n, p)) else {
                        continue;
                    };
                    if ancestors.contains(&j) {
                        continue;
                    }
                    ancestors.push(j);
                    pending.extend(s.group_ancestors(p)?);
                    pending.extend(s.parent_by_id(p));
                }
                points.push((i, x, y, ancestors.len()));
            }
            Ok((points, s.comp_size()))
        })
    })?;
    if points.is_empty() {
        return Ok(());
    }
    let horizontal = match (mode, spread) {
        (Some(m), _) | (_, Some(Spread::Position(m))) => {
            matches!(m, Mode::Left | Mode::HCenter | Mode::Right)
        }
        (_, Some(Spread::HorizontalSpace)) => true,
        _ => false,
    };
    let coordinate = |p: &(usize, f64, f64, usize)| if horizontal { p.1 } else { p.2 };
    let lo = points.iter().map(&coordinate).fold(f64::INFINITY, f64::min);
    let hi = points
        .iter()
        .map(&coordinate)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut targets = Vec::new();
    if let Some(mode) = mode {
        let (lo, hi) = if points.len() == 1 {
            (0.0, if horizontal { size.0 } else { size.1 })
        } else {
            (lo, hi)
        };
        let target = match mode {
            Mode::Left | Mode::Top => lo,
            Mode::Right | Mode::Bottom => hi,
            _ => (lo + hi) / 2.0,
        };
        for &(i, x, y, depth) in &points {
            targets.push((
                i,
                if horizontal { target } else { x },
                if horizontal { y } else { target },
                depth,
            ));
        }
    } else {
        if points.len() < 3 {
            return Ok(());
        }
        points.sort_by(|a, b| coordinate(a).total_cmp(&coordinate(b)));
        for (k, &(i, x, y, depth)) in points.iter().enumerate() {
            let target = lo + (hi - lo) * k as f64 / (points.len() - 1) as f64;
            targets.push((
                i,
                if horizontal { target } else { x },
                if horizontal { y } else { target },
                depth,
            ));
        }
    }
    targets.sort_by_key(|p| p.3);
    for (i, x, y, _) in targets {
        let next = cx.kernel(comp.as_deref(), |edit| {
            edit.read(|s| {
                let l = &s.layers()[i];
                let m = position_plane(s, l, time)?;
                let Some(inv) = inverse_plane(&m) else {
                    return Ok(None);
                };
                let (x, y) = project_point(&inv, x, y);
                let old_x = s.value3d(l, "position.x", time, 0.0)?;
                let old_y = s.value3d(l, "position.y", time, 0.0)?;
                Ok(Some((x, y, old_x, old_y)))
            })
        })?;
        let Some((x, y, old_x, old_y)) = next else {
            continue;
        };
        for (channel, value, old) in [("position.x", x, old_x), ("position.y", y, old_y)] {
            if !value.is_finite() || (value - old).abs() < 1e-9 {
                continue;
            }
            let mut c = SetProperty::new(ids[i].clone(), channel, value, time);
            c.preserve_hand_edits = false;
            properties::set_property(cx, &c)?;
        }
    }
    Ok(())
}
