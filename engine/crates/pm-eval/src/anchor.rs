//! Size and font anchoring (`legacy/core/size-anchor.ts`,
//! `legacy/core/font-anchor.ts`).
//!
//! When a layer's size or typography changes its content bounds, the anchor
//! keeps the same relative point of the bounds: the layer stores the bounds
//! captured before the first such edit (`d.sizeAnchorBounds`,
//! `d.fontAnchorBounds`), and rendering shifts the content by
//! [`font_anchor_offset`] from those to the current bounds.

use pm_model::Node;

use crate::js_sem::{JsResult, JsValue};
use crate::node;
use crate::scope::{Scope, has_paths, is_type};
use crate::space3d::Rect;

/// An offset in layer pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Offset {
    pub x: f64,
    pub y: f64,
}

/// A bounds object as stored or computed: the members `fontAnchorOffset`
/// reads, as JavaScript values (`Number.isFinite` does not coerce).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoundsValues {
    pub x0: JsValue,
    pub y0: JsValue,
    pub x1: JsValue,
    pub y1: JsValue,
}

impl BoundsValues {
    /// Reads `x0`, `y0`, `x1`, `y1` of a stored object (`None` when the value
    /// is falsy, as `!reference` tests).
    pub fn of(node: Option<&Node>) -> Option<BoundsValues> {
        if !node::truthy(node) {
            return None;
        }
        let read = |k: &str| match node.map(|n| node::get(n, k.as_bytes())) {
            Some(g) => g.to_js(),
            None => JsValue::Undefined,
        };
        Some(BoundsValues {
            x0: read("x0"),
            y0: read("y0"),
            x1: read("x1"),
            y1: read("y1"),
        })
    }
}

impl From<Rect> for BoundsValues {
    fn from(r: Rect) -> Self {
        BoundsValues {
            x0: JsValue::Number(r.x0),
            y0: JsValue::Number(r.y0),
            x1: JsValue::Number(r.x1),
            y1: JsValue::Number(r.y1),
        }
    }
}

fn finite(v: &JsValue) -> Option<f64> {
    match v {
        JsValue::Number(n) if n.is_finite() => Some(*n),
        _ => None,
    }
}

/// `fontAnchorOffset(reference, bounds, ax, ay)`: the shift that keeps the
/// anchor at the same relative point when `reference` bounds become `bounds`.
pub fn font_anchor_offset(
    reference: Option<&BoundsValues>,
    bounds: Option<&BoundsValues>,
    ax: &JsValue,
    ay: &JsValue,
) -> Offset {
    let (Some(r), Some(b)) = (reference, bounds) else {
        return Offset::default();
    };
    let offset = |start: &JsValue,
                  end: &JsValue,
                  next_start: &JsValue,
                  next_end: &JsValue,
                  anchor: &JsValue| {
        let (Some(start), Some(end), Some(next_start), Some(next_end), Some(anchor)) = (
            finite(start),
            finite(end),
            finite(next_start),
            finite(next_end),
            finite(anchor),
        ) else {
            return 0.0;
        };
        if end <= start {
            return 0.0;
        }
        anchor - (next_start + (anchor - start) / (end - start) * (next_end - next_start))
    };
    Offset {
        x: offset(&r.x0, &r.x1, &b.x0, &b.x1, ax),
        y: offset(&r.y0, &r.y1, &b.y0, &b.y1, ay),
    }
}

/// The layer types whose content has a native size (`sizedTypes`).
const SIZED_TYPES: [&str; 7] = [
    "shape",
    "solid",
    "image",
    "video",
    "shader",
    "extension",
    "precomp",
];

impl<'d> Scope<'_, 'd> {
    /// `sizeBounds(PM, layer, time)`: native content bounds without stroke or
    /// raster padding, for sized layer types (not shapes with paths).
    /// Centered for shapes, images, and videos; from the origin otherwise.
    pub fn size_bounds(&mut self, layer: &'d Node, time: f64) -> JsResult<Option<Rect>> {
        let ty = node::member(Some(layer), "type");
        if !SIZED_TYPES.iter().any(|t| node::is_str(ty, t)) {
            return Ok(None);
        }
        if is_type(layer, "shape") && has_paths(layer)? {
            return Ok(None);
        }
        let d = self.ev.resolve_content(self.comp, layer, time)?;
        let (pw, ph) = (
            node::js_value_opt(self.project.member("w")),
            node::js_value_opt(self.project.member("h")),
        );
        let pick = |key: &str, fallback: &JsValue| {
            let v = d
                .get(&pm_model::JsString::from(key))
                .cloned()
                .unwrap_or_default();
            if v.truthy() { v } else { fallback.clone() }
        };
        let w = pick("w", &pw).to_number();
        let h = pick("h", &ph).to_number();
        if !(w.is_finite() && w > 0.0 && h.is_finite() && h > 0.0) {
            return Ok(None);
        }
        let centered = ["shape", "image", "video"]
            .iter()
            .any(|t| node::is_str(ty, t));
        Ok(Some(if centered {
            Rect {
                x0: -w / 2.0,
                y0: -h / 2.0,
                x1: w / 2.0,
                y1: h / 2.0,
            }
        } else {
            Rect {
                x0: 0.0,
                y0: 0.0,
                x1: w,
                y1: h,
            }
        }))
    }

    /// `sizeAnchorOffset(PM, layer, time)`.
    pub fn size_anchor_offset(&mut self, layer: &'d Node, time: f64) -> JsResult<Offset> {
        let d = node::member(Some(layer), "d");
        let Some(reference) = BoundsValues::of(node::member(d, "sizeAnchorBounds")) else {
            return Ok(Offset::default());
        };
        let bounds = self.size_bounds(layer, time)?.map(BoundsValues::from);
        let ax = self.channel(layer, "anchor.x", time)?;
        let ay = self.channel(layer, "anchor.y", time)?;
        Ok(font_anchor_offset(
            Some(&reference),
            bounds.as_ref(),
            &ax,
            &ay,
        ))
    }
}
