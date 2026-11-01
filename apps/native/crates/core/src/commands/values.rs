//! Small helpers over `pm_model::Value` with the old code's JavaScript
//! semantics (`Math.round`, `PM.snapF`, `isProperty`, `PM.P`).

use pm_model::{JsString, Map, Node, Value};

use crate::error::CommandError;

/// Whether a stored node already equals `value` (`SameValue` for numbers).
pub(crate) fn same(node: &Node, value: &Value) -> bool {
    node.to_value().identical(value)
}

/// `Math.round` (halves toward +∞).
pub(crate) fn js_round(x: f64) -> f64 {
    pm_eval::js_sem::math_round(x)
}

/// `PM.snapF(t, fps)`: the nearest frame start.
pub(crate) fn snap(t: f64, fps: f64) -> f64 {
    js_round(t * fps) / fps
}

/// `PM.clamp`.
pub(crate) fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    x.max(lo).min(hi)
}

/// `finite(value, label)`: a finite number or the old error.
pub(crate) fn finite(value: f64, label: &str) -> Result<f64, CommandError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CommandError::Invalid(format!(
            "{label} must be a finite number"
        )))
    }
}

/// `isProperty(value)`: an object with `v` and a `kf` array.
pub(crate) fn is_property(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|m| m.contains_key("v") && m.get("kf").is_some_and(|k| k.as_array().is_some()))
}

/// `PM.P(v)`: `{ v, kf: [], expr: null }`.
pub(crate) fn property(v: Value) -> Value {
    let mut m = Map::new();
    m.insert("v", v);
    m.insert("kf", Value::Array(Vec::new()));
    m.insert("expr", Value::Null);
    Value::Object(m)
}

/// A string value.
pub(crate) fn s(text: &str) -> Value {
    Value::String(JsString::from(text))
}

/// The members of an object value, turning anything else into `{}` first.
pub(crate) fn obj(value: &mut Value) -> &mut Map {
    if value.as_object().is_none() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("just made an object")
}

/// `value[key]` as a number.
pub(crate) fn num(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

/// `value[key]` as a string.
pub(crate) fn text<'v>(value: &'v Value, key: &str) -> Option<&'v str> {
    value.get(key).and_then(Value::as_str)
}

/// `value[key] === true`.
pub(crate) fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}

/// The channel key of a path (`properties.` and `transform.` prefixes
/// dropped, as `channelPath` does).
pub(crate) fn channel_key(path: &str) -> &str {
    let p = path.strip_prefix("properties.").unwrap_or(path);
    p.strip_prefix("transform.").unwrap_or(p)
}

/// The layer's display name for messages (`layer.name`, else its id).
pub(crate) fn layer_name(layer: &Value, id: &str) -> String {
    layer
        .get("name")
        .and_then(Value::as_js_string)
        .map(JsString::to_string_lossy)
        .unwrap_or_else(|| id.to_owned())
}

/// `String.prototype.trim`.
pub(crate) fn js_trim(s: &str) -> &str {
    pm_model::json::js_trim(s)
}
