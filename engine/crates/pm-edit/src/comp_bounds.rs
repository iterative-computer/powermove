//! Composition bounds for Pre-compose › Leave all attributes
//! (`compositions.ts`, `leaveAttributes`).
//!
//! The new composition is the size of the layer's source (its `GL.bounds`,
//! else its content size, else the open composition's), at least 16 px, and
//! the layer's anchor moves by the source's origin inside its raster box so
//! nothing moves on screen.

use pm_eval::bounds::LayerHost;
use pm_eval::js_sem::{JsResult, JsThrow, JsValue, math_round};
use pm_eval::node;
use pm_eval::text_geom::TextMeasure;
use pm_model::{Patch, Value};

use crate::channel::{at, join, key};
use crate::work::Edit;

/// The new composition's size and the anchor shift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeaveBounds {
    pub w: f64,
    pub h: f64,
    /// `Number(bounds?.ax) || 0`.
    pub ax: f64,
    pub ay: f64,
}

/// `Math.max(16, Math.round(a || b || c))` over JavaScript values.
fn size(values: [JsValue; 3]) -> f64 {
    let picked = values
        .iter()
        .find(|v| v.truthy())
        .unwrap_or(&values[2])
        .to_number();
    let rounded = math_round(picked);
    if rounded.is_nan() { f64::NAN } else { rounded.max(16.0) }
}

/// `Number(x) || 0`.
fn or0(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 { 0.0 } else { x }
}

/// Computes the bounds and shifts the layer's `anchor.x` and `anchor.y`
/// (static value and keys) by the source's origin. The renderer creates the
/// composition and the inner layer, then applies the returned operations.
pub fn leave_attributes(
    edit: &mut Edit<'_>,
    layer: usize,
    time: f64,
    measure: &mut dyn TextMeasure,
    host: &mut dyn LayerHost,
) -> JsResult<LeaveBounds> {
    let (bounds, dw, dh, pw, ph) = edit.read(|s| {
        let l = s
            .layers()
            .get(layer)
            .ok_or_else(|| JsThrow::type_error("Layer not found"))?;
        let b = s.bounds(l, time, measure, host)?;
        let d = node::member(Some(l), "d");
        let (pw, ph) = (
            node::js_value_opt(s.project.member("w")),
            node::js_value_opt(s.project.member("h")),
        );
        Ok((
            b,
            node::js_value_opt(node::member(d, "w")),
            node::js_value_opt(node::member(d, "h")),
            pw,
            ph,
        ))
    })?;
    let bw = bounds.map_or(JsValue::Undefined, |b| JsValue::Number(b.w));
    let bh = bounds.map_or(JsValue::Undefined, |b| JsValue::Number(b.h));
    let out = LeaveBounds {
        w: size([bw, dw, pw]),
        h: size([bh, dh, ph]),
        ax: or0(bounds.map_or(f64::NAN, |b| b.ax)),
        ay: or0(bounds.map_or(f64::NAN, |b| b.ay)),
    };
    for (name, by) in [("anchor.x", out.ax), ("anchor.y", out.ay)] {
        shift(edit, layer, name, by)?;
    }
    Ok(out)
}

/// `shift(prop, by)`: `prop.v = Number(prop.v) + by` and every key's `v`.
fn shift(edit: &mut Edit<'_>, layer: usize, name: &str, by: f64) -> JsResult<()> {
    let base = join(&edit.layer_path(layer), &[key("p"), key(name)]);
    let Some(prop) = edit.node(&base).filter(|p| node::truthy(Some(p))) else {
        return Ok(());
    };
    let mut patches = vec![Patch::set(
        join(&base, &[key("v")]),
        Value::Number(node::get(prop, b"v").to_js().to_number() + by),
    )];
    // `for (const key of prop.kf || [])`.
    for (i, k) in node::array_or_empty(node::member(Some(prop), "kf"))?.iter().enumerate() {
        patches.push(Patch::set(
            join(&base, &[key("kf"), at(i), key("v")]),
            Value::Number(node::get(k, b"v").to_js().to_number() + by),
        ));
    }
    edit.write(patches)
}
