//! Channel values and writes with JavaScript's semantics.
//!
//! The legacy edits mutate live channel objects (`{ v, kf, expr }`) with
//! plain JavaScript operators. These helpers read the scratch document's
//! nodes as the values those operators see and build the values they write.

use pm_eval::js_sem::{self, JsResult, JsValue};
use pm_eval::node::{self, Got};
use pm_model::{JsString, Map, Node, NodeKind, PathPart, Value};

use crate::work::IdSource;

/// A value as the document stores it: `None` is `undefined` (an object
/// member written with `undefined` disappears from the JSON). Non-finite
/// numbers stay numbers here; at the patch boundary they become `null`, as
/// `JSON.stringify` (and every save) makes of them.
pub fn to_value(v: &JsValue) -> Option<Value> {
    match v {
        JsValue::Undefined => None,
        JsValue::Null => Some(Value::Null),
        JsValue::Bool(b) => Some(Value::Bool(*b)),
        JsValue::Number(n) => Some(Value::Number(*n)),
        JsValue::String(s) => Some(Value::String(s.clone())),
        JsValue::Object(v) => Some((**v).clone()),
    }
}

/// A number to write.
pub fn num(n: f64) -> Option<Value> {
    Some(Value::Number(n))
}

/// The JavaScript value of an optional node.
pub fn js(node: Option<&Node>) -> JsValue {
    node::js_value_opt(node)
}

/// `PM.P(v)`: `{ v, kf: [], expr: null }`.
pub fn property(v: Option<Value>) -> Value {
    let mut map = Map::new();
    if let Some(v) = v {
        map.insert("v", v);
    }
    map.insert("kf", Value::Array(Vec::new()));
    map.insert("expr", Value::Null);
    Value::Object(map)
}

/// `PM.KF(t, v, ease)`: `{ t, v, eo, ei, hold: false, i: PM.uid('k') }`.
pub fn keyframe(t: f64, v: Option<Value>, ease: &str, ids: &mut IdSource) -> Value {
    let (eo, ei) = pm_eval::easing::handles(ease);
    let pair = |p: [f64; 2]| Value::Array(vec![Value::Number(p[0]), Value::Number(p[1])]);
    let mut map = Map::new();
    map.insert("t", Value::Number(t));
    if let Some(v) = v {
        map.insert("v", v);
    }
    map.insert("eo", pair(eo));
    map.insert("ei", pair(ei));
    map.insert("hold", Value::Bool(false));
    map.insert("i", Value::String(ids.next("k")));
    Value::Object(map)
}

/// A template literal: the parts' `ToString`, concatenated.
pub fn template(parts: &[JsValue]) -> JsValue {
    let mut out = JsValue::String(JsString::new());
    for part in parts {
        out = js_sem::add(&out, &JsValue::String(part.to_js_string()));
    }
    out
}

/// A string literal part of a template.
pub fn lit(s: &str) -> JsValue {
    JsValue::from(s)
}

/// `base.key` for a literal key, throwing as JavaScript does on `undefined`
/// and `null`.
pub fn read<'a>(base: Option<&'a Node>, key: &str) -> JsResult<Got<'a>> {
    node::get_checked(base, key.as_bytes())
}

/// The elements of `for (const k of prop.kf)`: an array's elements; a string
/// iterates its characters (the writes then throw); anything else throws.
pub fn keys_of(prop: Option<&Node>) -> JsResult<&[Node]> {
    let kf = read(prop, "kf")?;
    match kf.node().map(Node::kind) {
        Some(NodeKind::Array(items)) => Ok(items),
        _ => Err(pm_eval::JsThrow::type_error("prop.kf is not iterable")),
    }
}

/// Throws as a member assignment on a primitive or nullish key does in
/// strict-mode code (`k.v = …` where `k` is not an object).
pub fn assignable(target: &Node, key: &str) -> JsResult<()> {
    match target.kind() {
        NodeKind::Object(_) | NodeKind::Array(_) => Ok(()),
        NodeKind::Null => Err(pm_eval::JsThrow::type_error(format!(
            "Cannot set properties of null (setting '{key}')"
        ))),
        _ => Err(pm_eval::JsThrow::type_error(format!(
            "Cannot create property '{key}' on {} '{}'",
            node::js_value(target).type_of(),
            node::js_value(target).to_js_string().to_string_lossy()
        ))),
    }
}

/// `path` extended with more parts.
pub fn join(path: &[PathPart], more: &[PathPart]) -> Vec<PathPart> {
    let mut out = path.to_vec();
    out.extend_from_slice(more);
    out
}

/// A path part for an object member.
pub fn key(k: &str) -> PathPart {
    PathPart::from(k)
}

/// A path part for an array element.
pub fn at(i: usize) -> PathPart {
    PathPart::from(i)
}
