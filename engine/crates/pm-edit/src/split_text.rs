//! Split text (`legacy/core/split-text.ts`): the layers a split by words,
//! characters, or lines creates.
//!
//! Each piece of the source's layout at the time becomes a copy of the
//! source with the authored text and typography fixed at the time (the
//! resolved content), the piece's text, left alignment, the text styles that
//! overlap the piece, and the source's animators continuing at the piece's
//! place in the sequence. The copy's anchor moves to the piece's origin (plus
//! the font anchor's offset), so rotation, scale, skew, and parenting are
//! preserved. The renderer inserts the copies before the source, hides the
//! source, and groups them (one Undo step); the kernel returns the copies.

use pm_eval::js_sem::{self, JsResult, JsThrow, JsValue};
use pm_eval::node;
use pm_eval::text_geom::TextMeasure;
use pm_model::{JsString, Map, Value};
use pm_text::compat::{GlyphLayout, Unit};

use crate::anchor::font_offset;
use crate::channel::{self, lit, template};
use crate::work::{Edit, IdSource};

/// How text splits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Words,
    Characters,
    Lines,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "words" => Mode::Words,
            "characters" => Mode::Characters,
            "lines" => Mode::Lines,
            _ => return None,
        })
    }
}

fn units<'a>(layout: &'a GlyphLayout, unit: &str) -> Option<&'a [Unit]> {
    match unit {
        "words" => Some(&layout.words),
        "characters" => Some(&layout.characters),
        "lines" => Some(&layout.lines),
        _ => None,
    }
}

/// One source's split.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The source layer's index.
    pub source: usize,
    /// The new layers, in stack order.
    pub children: Vec<Value>,
}

/// `renew(prop)`: fresh key ids (`(prop?.kf || []).forEach(kf => kf.i = PM.uid('k'))`).
fn renew(prop: Option<&mut Value>, ids: &mut IdSource) -> JsResult<()> {
    let Some(prop) = prop else { return Ok(()) };
    let Some(map) = prop.as_object_mut() else { return Ok(()) };
    let Some(kf) = map.get_mut("kf") else { return Ok(()) };
    match kf {
        Value::Array(keys) => {
            for k in keys {
                match k.as_object_mut() {
                    Some(k) => {
                        k.insert("i", Value::String(ids.next("k")));
                    }
                    None => return Err(JsThrow::type_error("Cannot create property 'i' on a key that is not an object")),
                }
            }
            Ok(())
        }
        Value::Null | Value::Bool(false) => Ok(()),
        Value::Number(n) if *n == 0.0 || n.is_nan() => Ok(()),
        Value::String(s) if s.utf16_len() == 0 => Ok(()),
        _ => Err(JsThrow::type_error("(prop?.kf || []).forEach is not a function")),
    }
}

/// `Object.values(x || {}).forEach(renew)`.
fn renew_values(x: Option<&mut Value>, ids: &mut IdSource) -> JsResult<()> {
    match x {
        Some(Value::Object(map)) => {
            // `Object.values` lists integer-like keys first; key order only
            // decides which fresh id goes where.
            for (_, v) in map.iter_mut() {
                renew(Some(v), ids)?;
            }
            Ok(())
        }
        Some(Value::Array(items)) => {
            for v in items {
                renew(Some(v), ids)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn member<'a>(v: &'a mut Value, k: &str) -> Option<&'a mut Value> {
    v.as_object_mut().and_then(|m| m.get_mut(k))
}

/// `PM.cloneLayer(L)` for a text layer: a deep copy with a fresh id and fresh
/// key ids on every channel the legacy clone renews (text layers have no 3D
/// scene channels).
fn clone_layer(source: &Value, ids: &mut IdSource) -> JsResult<Value> {
    let mut c = pm_model::patch::clone_patch_value(Some(source)).unwrap_or(Value::Null);
    if let Some(map) = c.as_object_mut() {
        map.insert("id", Value::String(ids.next("L")));
    }
    renew_values(member(&mut c, "p"), ids)?;
    if let Some(Value::Array(fx)) = member(&mut c, "fx") {
        for f in fx {
            renew(member(f, "on"), ids)?;
            renew_values(member(f, "p"), ids)?;
        }
    }
    if let Some(Value::Array(masks)) = member(&mut c, "masks") {
        for m in masks {
            renew_values(member(m, "p"), ids)?;
        }
    }
    renew_values(member(&mut c, "d"), ids)?;
    for k in ["blend", "mblur", "on"] {
        renew(member(&mut c, k), ids)?;
    }
    if let Some(Value::Array(masks)) = member(&mut c, "masks") {
        for m in masks {
            for k in ["shape", "mode", "on"] {
                renew(member(m, k), ids)?;
            }
        }
    }
    if let Some(d) = member(&mut c, "d") {
        renew_values(member(d, "uniforms"), ids)?;
    }
    if let Some(d) = member(&mut c, "d") {
        renew_values(member(d, "params"), ids)?;
    }
    for field in ["transitionIn", "transitionOut"] {
        if let Some(t) = member(&mut c, field) {
            renew_values(member(t, "p"), ids)?;
        }
    }
    Ok(c)
}

fn utf16(s: &str) -> JsString {
    JsString::from(s)
}

/// `PM.textLayout`-based split of the text layers at `sources` (by index),
/// after the renderer's checks (text layers, unlocked, outside locked
/// groups). Sources without pieces get an empty plan.
pub fn split_text(
    edit: &mut Edit<'_>,
    sources: &[usize],
    mode: Mode,
    time: f64,
    measure: &mut dyn TextMeasure,
) -> JsResult<Vec<Plan>> {
    // Every layout first, as the legacy code plans before it mutates.
    let mut layouts = Vec::with_capacity(sources.len());
    for &s in sources {
        let layout = edit.read(|sc| sc.text_layout(&sc.layers()[s], time, &mut *measure))?;
        layouts.push(layout);
    }
    let mut plans = Vec::new();
    for (&s, layout) in sources.iter().zip(&layouts) {
        let pieces = match mode {
            Mode::Words => &layout.words,
            Mode::Characters => &layout.characters,
            Mode::Lines => &layout.lines,
        };
        if pieces.is_empty() {
            plans.push(Plan { source: s, children: Vec::new() });
            continue;
        }
        let content = edit.read(|sc| sc.ev.resolve_content(sc.comp, &sc.layers()[s], time))?;
        let (fx, fy) = font_offset(edit, s, time, measure)?;
        let source = edit.layer(s).ok_or_else(|| JsThrow::type_error("Layer not found"))?;
        let source_value = source.to_value();
        let source_name = node::js_value_opt(node::member(Some(source), "name"));
        // The source's stored animators and their legacy `p.unit` channels.
        let animators: Vec<Value> = match node::member(node::member(Some(source), "d"), "animators") {
            Some(a) if node::truthy(Some(a)) => match node::array(Some(a)) {
                Some(items) => items.iter().map(|n| n.to_value()).collect(),
                None => return Err(JsThrow::type_error("(source.d.animators || []).map is not a function")),
            },
            _ => Vec::new(),
        };
        let mut legacy_units: Vec<JsValue> = Vec::with_capacity(animators.len());
        for (i, _) in animators.iter().enumerate() {
            let unit = edit.read(|sc| {
                let l = &sc.layers()[s];
                let a = &node::array(node::member(node::member(Some(l), "d"), "animators")).expect("listed")[i];
                let p = node::member(Some(a), "p");
                let prop = node::member(p, "unit");
                if !node::truthy(prop) {
                    return Ok(JsValue::Null);
                }
                let id = node::to_js_string(node::member(Some(a), "id"));
                let key = format!("ta.{}.unit", id.to_string_lossy());
                sc.channel_p(l, prop, time, &key)
            })?;
            legacy_units.push(unit);
        }
        let content_text = content
            .get(&JsString::from("text"))
            .cloned()
            .unwrap_or(JsValue::Undefined)
            .to_js_string();
        let mut children = Vec::with_capacity(pieces.len());
        for (index, piece) in pieces.iter().enumerate() {
            let mut child = clone_layer(&source_value, &mut edit.ids)?;
            let piece_text = utf16(&piece.text);
            let name = template(&[
                source_name.clone(),
                lit(" · "),
                JsValue::Number((index + 1) as f64),
                lit(" · "),
                JsValue::String(piece_text.clone()),
            ]);
            let obj = child
                .as_object_mut()
                .ok_or_else(|| JsThrow::type_error("A layer must be an object"))?;
            obj.insert("name", channel::to_value(&name).unwrap_or(Value::Null));
            // `child.d = { ...child.d, ...content, text, align: 'left', paragraph: false, fontAnchorBounds: null }`.
            let mut d = match obj.get("d") {
                Some(Value::Object(m)) => m.clone(),
                _ => Map::new(),
            };
            for (k, v) in &content {
                match channel::to_value(v) {
                    Some(v) => {
                        d.insert(k.clone(), v);
                    }
                    None => {
                        d.remove_key(k);
                    }
                }
            }
            d.insert("text", Value::String(piece_text.clone()));
            d.insert("align", Value::String(utf16("left")));
            d.insert("paragraph", Value::Bool(false));
            d.insert("fontAnchorBounds", Value::Null);
            // A split fixes the authored text and typography at the
            // playhead; transform channels, timing, effects, and stack
            // membership stay editable.
            let source_start = piece.source_start as f64;
            let _ = &content_text;
            let piece_len = piece_text.utf16_len() as f64;
            let styles = match d.get("styles") {
                None | Some(Value::Null) | Some(Value::Bool(false)) => Vec::new(),
                Some(Value::Array(items)) => items.clone(),
                Some(_) => return Err(JsThrow::type_error("(child.d.styles || []).flatMap is not a function")),
            };
            let mut sliced = Vec::new();
            for style in styles {
                let Some(map) = style.as_object() else {
                    return Err(JsThrow::type_error("Cannot read properties of null (reading 'start')"));
                };
                let read = |k: &str| map.get(k).map_or(JsValue::Undefined, JsValue::from);
                let start = js_max(read("start").to_number(), source_start);
                let end = js_min(read("end").to_number(), source_start + piece_len);
                if end > start {
                    let mut next = map.clone();
                    next.insert("id", Value::String(edit.ids.next("ts")));
                    next.insert("start", Value::Number(start - source_start));
                    next.insert("end", Value::Number(end - source_start));
                    sliced.push(Value::Object(next));
                }
            }
            d.insert("styles", Value::Array(sliced));
            // Each piece keeps its place in the source's sequence, so a split
            // word's first character keeps its selector weight and stagger
            // timing instead of restarting at character zero.
            let end = piece.source_start + piece.text.encode_utf16().count();
            let first = layout
                .characters
                .iter()
                .find(|g| g.source_start >= piece.source_start && g.source_start < end);
            let mut next_animators = Vec::with_capacity(animators.len());
            for (a, legacy) in animators.iter().zip(&legacy_units) {
                let Some(amap) = a.as_object() else {
                    return Err(JsThrow::type_error("Cannot read properties of null (reading 'p')"));
                };
                let stored = amap.get("unit").map_or(JsValue::Undefined, JsValue::from);
                let valid = |v: &JsValue| matches!(v, JsValue::String(s) if matches!(s.as_str(), Some("characters" | "words" | "lines")));
                let unit = [stored, legacy.clone()]
                    .into_iter()
                    .find(valid)
                    .map(|v| v.to_js_string().to_string_lossy())
                    .unwrap_or_else(|| "characters".to_owned());
                let local = match unit.as_str() {
                    "lines" => first.and_then(|g| g.line_unit).unwrap_or(piece.line) as f64,
                    "words" => first.and_then(|g| g.word).unwrap_or(0) as f64,
                    _ => first.map_or(0, |g| g.index) as f64,
                };
                let mut clone = pm_model::patch::clone_patch_value(Some(a)).unwrap_or(Value::Null);
                let cmap = clone.as_object_mut().expect("an object clones to an object");
                match cmap.get_mut("p") {
                    Some(Value::Object(p)) => {
                        p.remove("unit");
                    }
                    Some(Value::Array(_)) => {}
                    Some(Value::Null) | None => {
                        return Err(JsThrow::type_error("Cannot convert undefined or null to object"));
                    }
                    Some(_) => {}
                }
                let truthy_or_0 = |k: &str| {
                    let v = amap.get(k).map_or(JsValue::Undefined, JsValue::from);
                    if v.truthy() { v.to_number() } else { 0.0 }
                };
                let total = units(layout, &unit).map_or(0, |u| u.len());
                cmap.insert("id", Value::String(edit.ids.next("ta")));
                cmap.insert("unit", Value::String(utf16(&unit)));
                cmap.insert("unitOffset", Value::Number(truthy_or_0("unitOffset") + local));
                cmap.insert(
                    "unitTotal",
                    Value::Number(js_max(truthy_or_0("unitTotal"), if total > 0 { total as f64 } else { 1.0 })),
                );
                next_animators.push(clone);
            }
            d.insert("animators", Value::Array(next_animators));
            obj.insert("d", Value::Object(d));
            // Moving the local origin preserves rotation, scale, skew, and
            // parenting.
            for (axis, offset) in [("x", piece.x + fx), ("y", piece.y + fy)] {
                let p = obj
                    .get_mut("p")
                    .and_then(Value::as_object_mut)
                    .and_then(|p| p.get_mut(format!("anchor.{axis}").as_str()))
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| JsThrow::type_error("Cannot read properties of undefined (reading 'v')"))?;
                let v = p.get("v").map_or(f64::NAN, js_sem::value_to_number);
                p.insert("v", Value::Number(v - offset));
                if let Some(Value::Array(keys)) = p.get_mut("kf") {
                    for k in keys {
                        if let Some(k) = k.as_object_mut() {
                            let kv = k.get("v").map_or(f64::NAN, js_sem::value_to_number);
                            k.insert("v", Value::Number(kv - offset));
                        }
                    }
                }
                let expr = p.get("expr").map_or(JsValue::Undefined, JsValue::from);
                if expr.truthy() {
                    let text = template(&[lit("("), expr, lit(") - ("), JsValue::Number(offset), lit(")")]);
                    p.insert("expr", channel::to_value(&text).unwrap_or(Value::Null));
                }
            }
            children.push(child);
        }
        plans.push(Plan { source: s, children });
    }
    Ok(plans)
}

/// `Math.max(a, b)`.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { b } else { a }
    } else if a > b {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)`.
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}
