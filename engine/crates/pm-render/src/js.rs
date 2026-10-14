//! JavaScript value helpers with the legacy compositor's exact semantics.

use pm_eval::JsValue;
use pm_eval::node;
use pm_model::{JsString, Node};

/// `PM.clamp(v, a, b)`: `v < a ? a : v > b ? b : v` (NaN passes through).
pub fn clamp(v: f64, a: f64, b: f64) -> f64 {
    if v < a {
        a
    } else if v > b {
        b
    } else {
        v
    }
}

/// `Math.max(a, b)`: NaN wins.
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a >= b {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)`: NaN wins.
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a <= b {
        a
    } else {
        b
    }
}

/// `Math.round`.
pub fn round(x: f64) -> f64 {
    pm_eval::js_sem::math_round(x)
}

/// `value || fallback` read as a number: a truthy value converts with
/// `ToNumber` (as the arithmetic using it does), a falsy one is replaced.
pub fn or_number(value: &JsValue, fallback: f64) -> f64 {
    if value.truthy() {
        value.to_number()
    } else {
        fallback
    }
}

/// A content member (`d.key`), `undefined` when absent.
pub fn get<'a>(content: &'a pm_eval::Content, key: &str) -> &'a JsValue {
    static UNDEFINED: JsValue = JsValue::Undefined;
    content.get(&JsString::from(key)).unwrap_or(&UNDEFINED)
}

/// `String(value)`.
pub fn string(value: &JsValue) -> String {
    value.to_js_string().to_string_lossy()
}

/// A layer's string member (`L.type`, `L.id`), lossy.
pub fn member_string(layer: &Node, key: &str) -> Option<String> {
    node::string(node::member(Some(layer), key)).map(|s| s.to_string_lossy())
}

/// `!!L[key]`.
pub fn flag(layer: &Node, key: &str) -> bool {
    node::truthy(node::member(Some(layer), key))
}

/// The layer's `type`.
pub fn layer_type(layer: &Node) -> String {
    member_string(layer, "type").unwrap_or_default()
}

/// The layer's id as a string (for reports).
pub fn layer_id(layer: &Node) -> String {
    node::js_value_opt(node::member(Some(layer), "id"))
        .to_js_string()
        .to_string_lossy()
}
