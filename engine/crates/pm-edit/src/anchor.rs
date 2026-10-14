//! Anchors that follow typography and size (`font-anchor.ts`,
//! `size-anchor.ts`, and the anchor bookkeeping of `editing.ts`'s `runOne`).
//!
//! A text layer's pivot stays at the same relative point of its type when
//! typography changes the bounds: the first typography edit records the
//! bounds as `d.fontAnchorBounds`, and rendering shifts the text by
//! `fontAnchorOffset` from then on. Sized layers do the same with
//! `d.sizeAnchorBounds` for `w` and `h`. Moving the anchor itself (Pan
//! Behind) must not move the artwork, so after an anchor edit the recorded
//! bounds are rescaled around the new anchor ([`anchor_moved`]).

use pm_eval::anchor::{BoundsValues, font_anchor_offset};
use pm_eval::js_sem::{self, JsResult, JsThrow, JsValue};
use pm_eval::node;
use pm_eval::space3d::Rect;
use pm_eval::text_geom::TextMeasure;
use pm_model::{JsString, Map, Node, Value};

use crate::channel::{join, key};
use crate::work::Edit;

/// The typography fields whose change records the font anchor.
pub const TYPOGRAPHY: [&str; 8] = [
    "font", "weight", "size", "tracking", "leading", "italic", "boxWidth", "boxHeight",
];

/// Which recorded bounds an anchor edit maintains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `d.fontAnchorBounds` (text layers).
    Font,
    /// `d.sizeAnchorBounds` (sized layers).
    Size,
}

impl Kind {
    fn member(self) -> &'static str {
        match self {
            Kind::Font => "fontAnchorBounds",
            Kind::Size => "sizeAnchorBounds",
        }
    }
}

fn bounds_value(r: &Rect) -> Value {
    let mut map = Map::new();
    map.insert("x0", Value::Number(r.x0));
    map.insert("y0", Value::Number(r.y0));
    map.insert("x1", Value::Number(r.x1));
    map.insert("y1", Value::Number(r.y1));
    Value::Object(map)
}

fn layer_node<'e>(edit: &'e Edit<'_>, layer: usize) -> JsResult<&'e Node> {
    edit.layer(layer)
        .ok_or_else(|| JsThrow::type_error("Layer not found"))
}

fn d_member<'a>(layer: &'a Node, name: &str) -> Option<&'a Node> {
    node::member(node::member(Some(layer), "d"), name)
}

/// The detached copy `PM.raster` measures: `{ ...layer, d: { ...layer.d, ...patch } }`.
fn detached(layer: &Node, patch: &[(&str, Value)]) -> Node {
    let mut value = layer.to_value();
    if let Some(d) = value.as_object_mut().and_then(|m| m.get_mut("d")).and_then(Value::as_object_mut) {
        for (k, v) in patch {
            d.insert(*k, v.clone());
        }
    }
    Node::from_value(value, 0)
}

/// `captureSizeAnchor(PM, layer, command)` for a command writing `fields`:
/// the first `w` or `h` edit records the native size bounds.
pub fn capture_size_anchor(
    edit: &mut Edit<'_>,
    layer: usize,
    fields: &[JsString],
    time: f64,
) -> JsResult<()> {
    let l = layer_node(edit, layer)?;
    if node::truthy(d_member(l, "sizeAnchorBounds")) {
        return Ok(());
    }
    if !fields.iter().any(|f| f.as_bytes() == b"w" || f.as_bytes() == b"h") {
        return Ok(());
    }
    let bounds = edit.read(|s| s.size_bounds(&s.layers()[layer], time))?;
    if let Some(b) = bounds {
        let path = join(&edit.layer_path(layer), &[key("d"), key("sizeAnchorBounds")]);
        edit.set(path, Some(bounds_value(&b)))?;
    }
    Ok(())
}

/// `captureFontAnchor(PM, layer, command)` for a command writing `fields`:
/// an alignment change drops the recorded bounds (the pivot follows the
/// text's origin); the first typography change records the type's
/// selection bounds without animators.
pub fn capture_font_anchor(
    edit: &mut Edit<'_>,
    layer: usize,
    fields: &[JsString],
    time: f64,
    measure: &mut dyn TextMeasure,
) -> JsResult<()> {
    let l = layer_node(edit, layer)?;
    if !node::is_str(node::member(Some(l), "type"), "text") {
        return Ok(());
    }
    let d_path = join(&edit.layer_path(layer), &[key("d")]);
    if fields.iter().any(|f| f.as_bytes() == b"align") {
        if node::member(node::member(Some(l), "d"), "fontAnchorBounds").is_some() {
            edit.delete(join(&d_path, &[key("fontAnchorBounds")]))?;
        }
        return Ok(());
    }
    if node::truthy(d_member(l, "fontAnchorBounds")) {
        return Ok(());
    }
    let typographic = fields.iter().any(|f| {
        TYPOGRAPHY.iter().any(|t| f.as_bytes() == t.as_bytes()) || f.as_bytes().starts_with(b"fontAxis.")
    });
    if !typographic {
        return Ok(());
    }
    let animated = d_member(l, "animators")
        .map(|a| node::get(a, b"length").truthy())
        .unwrap_or(false);
    let measured = if animated {
        detached(l, &[("animators", Value::Array(Vec::new()))])
    } else {
        l.clone()
    };
    let selection = edit.read_with(&measured, |s, target| Ok(s.text_raster(target, time, measure)?.selection))?;
    edit.set(
        join(&d_path, &[key("fontAnchorBounds")]),
        Some(bounds_value(&selection.rect())),
    )
}

/// What an anchor edit starts from (before the command runs): for a text
/// layer with recorded font bounds, the raster selection with the font
/// anchor applied; for a sized layer with recorded size bounds, the native
/// size bounds shifted by the size anchor offset. `None` when the layer
/// records nothing.
pub fn anchor_reference(
    edit: &mut Edit<'_>,
    layer: usize,
    kind: Kind,
    time: f64,
    measure: &mut dyn TextMeasure,
) -> JsResult<Option<Rect>> {
    let l = layer_node(edit, layer)?;
    if !node::truthy(d_member(l, kind.member())) {
        return Ok(None);
    }
    match kind {
        Kind::Font => {
            if !node::is_str(node::member(Some(l), "type"), "text") {
                return Ok(None);
            }
            let raster = edit.read(|s| s.text_raster(&s.layers()[layer], time, measure))?;
            Ok(Some(raster.selection.rect()))
        }
        Kind::Size => edit.read(|s| {
            let l = &s.layers()[layer];
            let Some(native) = s.size_bounds(l, time)? else {
                return Ok(None);
            };
            let off = s.size_anchor_offset(l, time)?;
            Ok(Some(Rect {
                x0: native.x0 + off.x,
                x1: native.x1 + off.x,
                y0: native.y0 + off.y,
                y1: native.y1 + off.y,
            }))
        }),
    }
}

/// After an anchor edit: rescale the recorded bounds around the new anchor
/// so the artwork stays where `bounds` (from [`anchor_reference`]) had it.
pub fn anchor_moved(
    edit: &mut Edit<'_>,
    layer: usize,
    kind: Kind,
    time: f64,
    bounds: &Rect,
) -> JsResult<()> {
    let l = layer_node(edit, layer)?;
    let reference = d_member(l, kind.member());
    let Some(reference) = reference.filter(|r| node::truthy(Some(r))) else {
        return Ok(());
    };
    let stored: Vec<JsValue> = ["x0", "y0", "x1", "y1"]
        .iter()
        .map(|k| node::get(reference, k.as_bytes()).to_js())
        .collect();
    let read = |k: &str| match k {
        "x0" => stored[0].clone(),
        "y0" => stored[1].clone(),
        "x1" => stored[2].clone(),
        _ => stored[3].clone(),
    };
    let mut values: Vec<(&str, JsValue)> = Vec::new();
    for (axis, start, end, lo, hi) in [("x", "x0", "x1", bounds.x0, bounds.x1), ("y", "y0", "y1", bounds.y0, bounds.y1)] {
        let span = hi - lo;
        let reference_span = js_sem::sub(&read(end), &read(start));
        // `if (span <= 0) continue` (NaN goes on).
        if span <= 0.0 {
            continue;
        }
        let anchor_key = format!("anchor.{axis}");
        let anchor = edit.read(|s| s.channel(&s.layers()[layer], &anchor_key, time))?;
        let a = anchor.to_number();
        let s = a - (a - lo) / span * reference_span;
        values.push((start, JsValue::Number(s)));
        values.push((end, JsValue::Number(s + reference_span)));
    }
    let base = join(&edit.layer_path(layer), &[key("d"), key(kind.member())]);
    let patches = values
        .into_iter()
        .map(|(k, v)| pm_model::Patch {
            path: join(&base, &[key(k)]),
            exists: true,
            value: crate::channel::to_value(&v),
        })
        .collect();
    edit.write(patches)
}

/// `PM.raster(source, 1, time)?.fontOffset || { x: 0, y: 0 }` for a text
/// layer: the shift its recorded font bounds put on the type.
pub fn font_offset(
    edit: &mut Edit<'_>,
    layer: usize,
    time: f64,
    measure: &mut dyn TextMeasure,
) -> JsResult<(f64, f64)> {
    let l = layer_node(edit, layer)?;
    let Some(reference) = BoundsValues::of(d_member(l, "fontAnchorBounds")) else {
        return Ok((0.0, 0.0));
    };
    // Character animator movement must not be canceled by typography
    // anchoring: the type bounds are measured without animators.
    let animated = d_member(l, "animators")
        .map(|a| node::get(a, b"length").truthy())
        .unwrap_or(false);
    let mut patch = vec![("fontAnchorBounds", Value::Null)];
    if animated {
        patch.push(("animators", Value::Array(Vec::new())));
    }
    let plain = detached(l, &patch);
    edit.read_with(&plain, |s, plain| {
        let l = &s.layers()[layer];
        let type_bounds = s.text_raster(plain, time, measure)?.selection;
        let ax = s.channel(l, "anchor.x", time)?;
        let ay = s.channel(l, "anchor.y", time)?;
        let o = font_anchor_offset(
            Some(&reference),
            Some(&BoundsValues::from(type_bounds.rect())),
            &ax,
            &ay,
        );
        Ok((o.x, o.y))
    })
}
