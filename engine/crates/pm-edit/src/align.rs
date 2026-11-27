//! Align and distribute (`extensions/inspector/align.ts` and the alignment
//! strip's `align`).
//!
//! The selection's world-space boxes (2D layers only) decide one world
//! translation per layer; each becomes a delta in the layer's parent space
//! and a new `position.x` / `position.y` value at the time, rounded to three
//! decimals. The kernel returns those values; the renderer applies them as
//! `set_property` commands through the edit pipeline (so keyframed channels
//! get keys and Undo names the alignment).

use pm_eval::bounds::LayerHost;
use pm_eval::js_sem::{JsResult, JsValue, math_round};
use pm_eval::node;
use pm_eval::scope::Scope;
use pm_eval::space3d::Rect;
use pm_eval::text_geom::TextMeasure;
use pm_eval::transform::Mat2d;
use pm_model::Node;

use crate::work::Edit;

/// Which edge or center lines up (`AlignMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "left" => Mode::Left,
            "hcenter" => Mode::HCenter,
            "right" => Mode::Right,
            "top" => Mode::Top,
            "vcenter" => Mode::VCenter,
            "bottom" => Mode::Bottom,
            _ => return None,
        })
    }

    fn horizontal(self) -> bool {
        matches!(self, Mode::Left | Mode::HCenter | Mode::Right)
    }

    /// The box coordinate this mode lines up.
    fn of(self, b: &Rect) -> f64 {
        match self {
            Mode::Left => b.x0,
            Mode::Right => b.x1,
            Mode::HCenter => (b.x0 + b.x1) / 2.0,
            Mode::Top => b.y0,
            Mode::Bottom => b.y1,
            Mode::VCenter => (b.y0 + b.y1) / 2.0,
        }
    }
}

/// How `distribute` spaces the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    /// Edges or centers at equal steps between the two outermost layers.
    Position(Mode),
    /// Equal gaps between neighbouring boxes, horizontally.
    HorizontalSpace,
    /// Equal gaps between neighbouring boxes, vertically.
    VerticalSpace,
}

impl Spread {
    pub fn parse(s: &str) -> Option<Spread> {
        match s {
            "hspace" => Some(Spread::HorizontalSpace),
            "vspace" => Some(Spread::VerticalSpace),
            other => Mode::parse(other).map(Spread::Position),
        }
    }
}

/// One `set_property` the renderer applies.
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    /// The layer's index in the edited composition.
    pub layer: usize,
    /// `position.x` or `position.y`.
    pub channel: &'static str,
    /// The value at the time, rounded to three decimals.
    pub value: f64,
}

/// `alignDelta(box, frame, mode)`.
pub fn align_delta(b: &Rect, frame: &Rect, mode: Mode) -> (f64, f64) {
    match mode {
        Mode::Left => (frame.x0 - b.x0, 0.0),
        Mode::Right => (frame.x1 - b.x1, 0.0),
        Mode::HCenter => ((frame.x0 + frame.x1) / 2.0 - (b.x0 + b.x1) / 2.0, 0.0),
        Mode::Top => (0.0, frame.y0 - b.y0),
        Mode::Bottom => (0.0, frame.y1 - b.y1),
        Mode::VCenter => (0.0, (frame.y0 + frame.y1) / 2.0 - (b.y0 + b.y1) / 2.0),
    }
}

/// `deltaInParent(parent, dx, dy)`: a world delta in a parent's axes.
pub fn delta_in_parent(parent: Option<&Mat2d>, dx: f64, dy: f64) -> (f64, f64) {
    let Some([a, b, c, d, ..]) = parent.copied() else {
        return (dx, dy);
    };
    let det = a * d - b * c;
    // `Math.abs(det) < 1e-9` is false for NaN: the delta becomes NaN and
    // moves nothing.
    if det.abs() < 1e-9 {
        return (dx, dy);
    }
    ((d * dx - c * dy) / det, (a * dy - b * dx) / det)
}

/// `Math.min(...xs)` / `Math.max(...xs)` (NaN wins).
fn fold(xs: impl Iterator<Item = f64>, min: bool) -> f64 {
    let mut out = if min {
        f64::INFINITY
    } else {
        f64::NEG_INFINITY
    };
    for x in xs {
        if x.is_nan() {
            return f64::NAN;
        }
        if (min && x < out)
            || (!min && x > out)
            || (x == out && x == 0.0 && (min == x.is_sign_negative()))
        {
            out = x;
        }
    }
    out
}

/// `unionBox(boxes)`.
pub fn union_box(boxes: &[Rect]) -> Option<Rect> {
    if boxes.is_empty() {
        return None;
    }
    Some(Rect {
        x0: fold(boxes.iter().map(|b| b.x0), true),
        y0: fold(boxes.iter().map(|b| b.y0), true),
        x1: fold(boxes.iter().map(|b| b.x1), false),
        y1: fold(boxes.iter().map(|b| b.y1), false),
    })
}

/// The strip's `worldBox(layer, time)`: the layer's bounds through its world
/// matrix, or `None` for 3D layers and layers without bounds.
fn world_box<'d>(
    s: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    measure: &mut dyn TextMeasure,
    host: &mut dyn LayerHost,
) -> JsResult<Option<Rect>> {
    if s.is_3d_layer(layer)? {
        return Ok(None);
    }
    let Some(b) = s.bounds(layer, time, measure, host)? else {
        return Ok(None);
    };
    let m = s.world_matrix(layer, time)?;
    let corners = [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)]
        .map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
    Ok(Some(Rect {
        x0: fold(corners.iter().map(|p| p.0), true),
        y0: fold(corners.iter().map(|p| p.1), true),
        x1: fold(corners.iter().map(|p| p.0), false),
        y1: fold(corners.iter().map(|p| p.1), false),
    }))
}

/// The targets the strip moves (editable: neither locked nor inside a locked
/// group, as `isInspectorLayerEditable`; not audio) with their world boxes.
fn boxed(
    edit: &mut Edit<'_>,
    layers: &[usize],
    time: f64,
    measure: &mut dyn TextMeasure,
    host: &mut dyn LayerHost,
) -> JsResult<Vec<(usize, Rect)>> {
    edit.read(|s| {
        let mut out = Vec::new();
        for &i in layers {
            let Some(l) = s.layers().get(i) else { continue };
            if node::truthy(node::member(Some(l), "lock"))
                || s.group_ancestors(l)?
                    .iter()
                    .any(|g| node::truthy(node::member(Some(g), "lock")))
                || node::is_str(node::member(Some(l), "type"), "audio")
            {
                continue;
            }
            if let Some(b) = world_box(s, l, time, measure, host)? {
                out.push((i, b));
            }
        }
        Ok(out)
    })
}

/// `api.util.round(v, 3)`.
fn round3(v: f64) -> f64 {
    math_round(v * 1000.0) / 1000.0
}

/// The moves for world deltas, one per boxed layer (zero deltas skipped).
fn moves(edit: &mut Edit<'_>, deltas: &[(usize, f64, f64)], time: f64) -> JsResult<Vec<Move>> {
    edit.read(|s| {
        let mut out = Vec::new();
        for &(i, dx, dy) in deltas {
            let l = &s.layers()[i];
            let parent = s.parent_by_id(l);
            let pm = match parent {
                Some(p) => Some(s.world_matrix(p, time)?),
                None => None,
            };
            let (lx, ly) = delta_in_parent(pm.as_ref(), dx, dy);
            // `if (local.dx)`: zero and NaN move nothing.
            if lx != 0.0 && !lx.is_nan() {
                let v = JsValue::Number(s.channel(l, "position.x", time)?.to_number());
                out.push(Move {
                    layer: i,
                    channel: "position.x",
                    value: round3(v.to_number() + lx),
                });
            }
            if ly != 0.0 && !ly.is_nan() {
                let v = s.channel(l, "position.y", time)?.to_number();
                out.push(Move {
                    layer: i,
                    channel: "position.y",
                    value: round3(v + ly),
                });
            }
        }
        Ok(out)
    })
}

/// The strip's `align(mode)` for the selected layers (by index): their
/// common box meets the composition frame `(0, 0, w, h)`, every layer moving
/// by the same world delta.
pub fn align(
    edit: &mut Edit<'_>,
    layers: &[usize],
    mode: Mode,
    time: f64,
    measure: &mut dyn TextMeasure,
    host: &mut dyn LayerHost,
) -> JsResult<Vec<Move>> {
    let boxed = boxed(edit, layers, time, measure, host)?;
    let Some(bounds) = union_box(&boxed.iter().map(|(_, b)| *b).collect::<Vec<_>>()) else {
        return Ok(Vec::new());
    };
    let (w, h) = edit.read(|s| Ok(s.project_size()))?;
    let frame = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: w,
        y1: h,
    };
    let (dx, dy) = align_delta(&bounds, &frame, mode);
    // `if (!world.dx && !world.dy) return`.
    let moves_x = dx != 0.0 && !dx.is_nan();
    let moves_y = dy != 0.0 && !dy.is_nan();
    if !moves_x && !moves_y {
        return Ok(Vec::new());
    }
    let deltas: Vec<(usize, f64, f64)> = boxed.iter().map(|(i, _)| (*i, dx, dy)).collect();
    moves(edit, &deltas, time)
}

/// Distributes the selected layers (at least three 2D layers with bounds):
/// the two outermost stay, and the others move along one axis so the chosen
/// edges or centers fall at equal steps between them, or so the gaps between
/// neighbouring boxes are equal. Layers keep the other axis.
pub fn distribute(
    edit: &mut Edit<'_>,
    layers: &[usize],
    spread: Spread,
    time: f64,
    measure: &mut dyn TextMeasure,
    host: &mut dyn LayerHost,
) -> JsResult<Vec<Move>> {
    let mut boxed = boxed(edit, layers, time, measure, host)?;
    if boxed.len() < 3 {
        return Ok(Vec::new());
    }
    let horizontal = match spread {
        Spread::Position(m) => m.horizontal(),
        Spread::HorizontalSpace => true,
        Spread::VerticalSpace => false,
    };
    // Ordered by the coordinate being spread (selection order breaks ties).
    let reference = |b: &Rect| match spread {
        Spread::Position(m) => m.of(b),
        Spread::HorizontalSpace => b.x0,
        Spread::VerticalSpace => b.y0,
    };
    boxed.sort_by(|a, b| {
        reference(&a.1)
            .partial_cmp(&reference(&b.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let n = boxed.len();
    let mut deltas = Vec::with_capacity(n);
    match spread {
        Spread::Position(_) => {
            let first = reference(&boxed[0].1);
            let last = reference(&boxed[n - 1].1);
            for (k, (i, b)) in boxed.iter().enumerate() {
                let target = first + (last - first) * k as f64 / (n - 1) as f64;
                let d = if k == 0 || k == n - 1 {
                    0.0
                } else {
                    target - reference(b)
                };
                deltas.push(if horizontal {
                    (*i, d, 0.0)
                } else {
                    (*i, 0.0, d)
                });
            }
        }
        Spread::HorizontalSpace | Spread::VerticalSpace => {
            let lo = |b: &Rect| if horizontal { b.x0 } else { b.y0 };
            let hi = |b: &Rect| if horizontal { b.x1 } else { b.y1 };
            let span = hi(&boxed[n - 1].1) - lo(&boxed[0].1);
            let sizes: f64 = boxed.iter().map(|(_, b)| hi(b) - lo(b)).sum();
            let gap = (span - sizes) / (n - 1) as f64;
            let mut at = lo(&boxed[0].1);
            for (k, (i, b)) in boxed.iter().enumerate() {
                let d = if k == 0 || k == n - 1 {
                    0.0
                } else {
                    at - lo(b)
                };
                deltas.push(if horizontal {
                    (*i, d, 0.0)
                } else {
                    (*i, 0.0, d)
                });
                at += hi(b) - lo(b) + gap;
            }
        }
    }
    deltas.retain(|(_, dx, dy)| *dx != 0.0 || *dy != 0.0);
    // Selection order for the commands.
    deltas.sort_by_key(|(i, _, _)| layers.iter().position(|l| l == i));
    moves(edit, &deltas, time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_match_the_strip() {
        let b = Rect {
            x0: 100.0,
            y0: 50.0,
            x1: 300.0,
            y1: 150.0,
        };
        let f = Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 1920.0,
            y1: 1080.0,
        };
        assert_eq!(align_delta(&b, &f, Mode::Left), (-100.0, 0.0));
        assert_eq!(align_delta(&b, &f, Mode::Right), (1620.0, 0.0));
        assert_eq!(align_delta(&b, &f, Mode::HCenter), (760.0, 0.0));
        assert_eq!(align_delta(&b, &f, Mode::Top), (0.0, -50.0));
        assert_eq!(align_delta(&b, &f, Mode::Bottom), (0.0, 930.0));
        assert_eq!(align_delta(&b, &f, Mode::VCenter), (0.0, 440.0));
        assert_eq!(delta_in_parent(None, 10.0, 20.0), (10.0, 20.0));
        assert_eq!(
            delta_in_parent(Some(&[2.0, 0.0, 0.0, 2.0, 500.0, 500.0]), 10.0, 20.0),
            (5.0, 10.0)
        );
        let (x, y) = delta_in_parent(Some(&[0.0, 1.0, -1.0, 0.0, 0.0, 0.0]), 10.0, 0.0);
        assert!((x - 0.0).abs() < 1e-12 && (y + 10.0).abs() < 1e-12);
        assert_eq!(delta_in_parent(Some(&[0.0; 6]), 3.0, 4.0), (3.0, 4.0));
        assert!(union_box(&[]).is_none());
    }
}
