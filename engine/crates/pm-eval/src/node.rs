//! JavaScript property access over the document tree.
//!
//! The legacy evaluator reads the live `JSON.parse` objects with JavaScript's
//! property semantics. These helpers give the same answers on a
//! [`pm_model::Node`]:
//!
//! * `obj[key]` on an object reads the member; on an array, a canonical index
//!   reads the element and `length` the length; on a string, an index reads a
//!   one-code-unit string and `length` the length in UTF-16 code units; on a
//!   number or boolean every key is `undefined`.
//! * Reading a property of `null` or `undefined` throws a `TypeError`.
//! * `for…in` and `Object.entries` list integer keys in ascending order, then
//!   the other keys.
//!
//! Known limits (documented, not reachable from well-formed projects):
//!
//! * Members inherited from `Object.prototype` (`constructor`, `toString`, ...)
//!   read as `undefined`; JavaScript would find the prototype's functions.
//! * The engine does not keep object insertion order (`pm-model` stores
//!   members sorted), so non-integer keys are listed in code point order where
//!   JavaScript lists them in insertion order. Only the *order* of
//!   [`for_in_keys`] and [`entries`] can differ; lookups never depend on it.

use pm_model::{JsString, Node, NodeKind};

use crate::js_sem::{JsResult, JsThrow, JsValue};

/// The JavaScript value of a node (strings and objects are cloned).
pub fn js_value(node: &Node) -> JsValue {
    match node.kind() {
        NodeKind::Null => JsValue::Null,
        NodeKind::Bool(b) => JsValue::Bool(*b),
        NodeKind::Number(n) => JsValue::Number(*n),
        NodeKind::String(s) => JsValue::String(s.clone()),
        NodeKind::Array(_) | NodeKind::Object(_) => JsValue::Object(Box::new(node.to_value())),
    }
}

/// The JavaScript value of an optional node (`None` is `undefined`).
pub fn js_value_opt(node: Option<&Node>) -> JsValue {
    node.map_or(JsValue::Undefined, js_value)
}

/// `typeof node`.
pub fn type_of(node: Option<&Node>) -> &'static str {
    match node.map(Node::kind) {
        None => "undefined",
        Some(NodeKind::Null | NodeKind::Array(_) | NodeKind::Object(_)) => "object",
        Some(NodeKind::Bool(_)) => "boolean",
        Some(NodeKind::Number(_)) => "number",
        Some(NodeKind::String(_)) => "string",
    }
}

/// `!!node`.
pub fn truthy(node: Option<&Node>) -> bool {
    match node.map(Node::kind) {
        None | Some(NodeKind::Null) => false,
        Some(NodeKind::Bool(b)) => *b,
        Some(NodeKind::Number(n)) => *n != 0.0 && !n.is_nan(),
        Some(NodeKind::String(s)) => !s.as_bytes().is_empty(),
        Some(NodeKind::Array(_) | NodeKind::Object(_)) => true,
    }
}

/// Whether the node is `null` or missing (`== null`).
pub fn is_nullish(node: Option<&Node>) -> bool {
    matches!(node.map(Node::kind), None | Some(NodeKind::Null))
}

/// The number when the node is a number (`typeof x === 'number'`).
pub fn number(node: Option<&Node>) -> Option<f64> {
    match node.map(Node::kind) {
        Some(NodeKind::Number(n)) => Some(*n),
        _ => None,
    }
}

/// The finite number when the node is one (`Number.isFinite(x)`).
pub fn finite_number(node: Option<&Node>) -> Option<f64> {
    number(node).filter(|n| n.is_finite())
}

/// The string when the node is a string.
pub fn string(node: Option<&Node>) -> Option<&JsString> {
    match node.map(Node::kind) {
        Some(NodeKind::String(s)) => Some(s),
        _ => None,
    }
}

/// Whether the node is the string `s` (`x === 'literal'`).
pub fn is_str(node: Option<&Node>, s: &str) -> bool {
    string(node).is_some_and(|v| v.as_bytes() == s.as_bytes())
}

/// `ToNumber(node)`.
pub fn to_number(node: Option<&Node>) -> f64 {
    match node.map(Node::kind) {
        None => f64::NAN,
        Some(NodeKind::Number(n)) => *n,
        Some(_) => js_value_opt(node).to_number(),
    }
}

/// An object member (`undefined` for non-objects). Use for literal keys that
/// are not array indices and not `length`.
pub fn member<'a>(node: Option<&'a Node>, key: &str) -> Option<&'a Node> {
    match node?.kind() {
        NodeKind::Object(map) => map.get(key.as_bytes()),
        _ => None,
    }
}

/// `node?.[key]` for a literal key: [`member`], spelled for optional chains.
pub fn opt<'a>(node: Option<&'a Node>, key: &str) -> Option<&'a Node> {
    member(node, key)
}

/// `node[key]` for a literal key, throwing on `null` and `undefined`.
pub fn member_checked<'a>(node: Option<&'a Node>, key: &str) -> JsResult<Option<&'a Node>> {
    match node.map(Node::kind) {
        None | Some(NodeKind::Null) => Err(JsThrow::type_error(format!(
            "Cannot read properties of {} (reading '{key}')",
            if node.is_none() { "undefined" } else { "null" }
        ))),
        _ => Ok(member(node, key)),
    }
}

/// The result of `base[key]` for an arbitrary key.
#[derive(Clone, Debug, PartialEq)]
pub enum Got<'a> {
    /// A member or element of the document.
    Node(&'a Node),
    /// A computed value (`length`, a character of a string).
    Value(JsValue),
    Undefined,
}

impl<'a> Got<'a> {
    pub fn node(&self) -> Option<&'a Node> {
        match self {
            Got::Node(n) => Some(n),
            _ => None,
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Got::Node(n) => truthy(Some(n)),
            Got::Value(v) => v.truthy(),
            Got::Undefined => false,
        }
    }

    pub fn to_js(&self) -> JsValue {
        match self {
            Got::Node(n) => js_value(n),
            Got::Value(v) => v.clone(),
            Got::Undefined => JsValue::Undefined,
        }
    }

    pub fn is_nullish(&self) -> bool {
        match self {
            Got::Node(n) => is_nullish(Some(n)),
            Got::Value(v) => matches!(v, JsValue::Undefined | JsValue::Null),
            Got::Undefined => true,
        }
    }
}

/// The canonical array index of a key (`"0"`, `"42"`; not `"01"` or `"-1"`).
pub fn array_index(key: &[u8]) -> Option<usize> {
    if key.is_empty() || key.len() > 10 || (key.len() > 1 && key[0] == b'0') {
        return None;
    }
    let mut value: u64 = 0;
    for &c in key {
        if !c.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u64::from(c - b'0');
    }
    (value < u64::from(u32::MAX)).then_some(value as usize)
}

/// `base[key]` for a non-nullish base. A `null` base answers `undefined`
/// here; use [`get_checked`] where JavaScript would throw.
pub fn get<'a>(base: &'a Node, key: &[u8]) -> Got<'a> {
    match base.kind() {
        NodeKind::Object(map) => map.get(key).map_or(Got::Undefined, Got::Node),
        NodeKind::Array(items) => {
            if key == b"length" {
                return Got::Value(JsValue::Number(items.len() as f64));
            }
            array_index(key)
                .and_then(|i| items.get(i))
                .map_or(Got::Undefined, Got::Node)
        }
        NodeKind::String(s) => {
            if key == b"length" {
                return Got::Value(JsValue::Number(s.utf16_len() as f64));
            }
            array_index(key)
                .and_then(|i| s.unit_at(i))
                .map_or(Got::Undefined, |c| Got::Value(JsValue::String(c)))
        }
        _ => Got::Undefined,
    }
}

/// `base[key]`, throwing on a `null` or `undefined` base.
pub fn get_checked<'a>(base: Option<&'a Node>, key: &[u8]) -> JsResult<Got<'a>> {
    match base {
        None => Err(JsThrow::type_error(format!(
            "Cannot read properties of undefined (reading '{}')",
            String::from_utf8_lossy(key)
        ))),
        Some(n) if matches!(n.kind(), NodeKind::Null) => Err(JsThrow::type_error(format!(
            "Cannot read properties of null (reading '{}')",
            String::from_utf8_lossy(key)
        ))),
        Some(n) => Ok(get(n, key)),
    }
}

/// `isProperty(value)` (`legacy/core/content-properties.ts`): an object with
/// an array `kf` and an own `v`.
pub fn is_property(node: Option<&Node>) -> bool {
    match node.map(Node::kind) {
        Some(NodeKind::Object(map)) => {
            map.contains_key("v".as_bytes())
                && matches!(
                    map.get("kf".as_bytes()).map(Node::kind),
                    Some(NodeKind::Array(_))
                )
        }
        _ => false,
    }
}

/// Whether `node.kf` is an array (`Array.isArray(prop.kf)`).
pub fn has_kf_array(node: Option<&Node>) -> bool {
    matches!(member(node, "kf").map(Node::kind), Some(NodeKind::Array(_)))
}

fn object_keys_in_js_order(map: &std::collections::BTreeMap<JsString, Node>) -> Vec<&JsString> {
    let mut integers: Vec<(usize, &JsString)> = Vec::new();
    let mut others: Vec<&JsString> = Vec::new();
    for key in map.keys() {
        match array_index(key.as_bytes()) {
            Some(i) => integers.push((i, key)),
            None => others.push(key),
        }
    }
    integers.sort_by_key(|(i, _)| *i);
    integers.into_iter().map(|(_, k)| k).chain(others).collect()
}

/// `for (const k in node)` keys for a JSON value (own enumerable keys; a
/// primitive other than a string has none).
pub fn for_in_keys(node: Option<&Node>) -> Vec<JsString> {
    match node.map(Node::kind) {
        Some(NodeKind::Object(map)) => object_keys_in_js_order(map).into_iter().cloned().collect(),
        Some(NodeKind::Array(items)) => (0..items.len())
            .map(|i| JsString::from(i.to_string()))
            .collect(),
        Some(NodeKind::String(s)) => (0..s.utf16_len())
            .map(|i| JsString::from(i.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// `Object.entries(node)` for a non-nullish node (`Object.entries(null)`
/// throws; callers write `x || {}`).
pub fn entries(node: &Node) -> Vec<(JsString, Got<'_>)> {
    match node.kind() {
        NodeKind::Object(map) => object_keys_in_js_order(map)
            .into_iter()
            .map(|k| (k.clone(), Got::Node(&map[k.as_bytes()])))
            .collect(),
        NodeKind::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, n)| (JsString::from(i.to_string()), Got::Node(n)))
            .collect(),
        NodeKind::String(s) => s
            .utf16_units()
            .enumerate()
            .map(|(i, u)| {
                (
                    JsString::from(i.to_string()),
                    Got::Value(JsValue::String(JsString::from_utf16_units(&[u]))),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// One item of `for (const x of node)`.
#[derive(Clone, Debug)]
pub enum Item<'a> {
    Node(&'a Node),
    /// A code point of a string, as a string.
    Char(JsString),
}

impl<'a> Item<'a> {
    pub fn node(&self) -> Option<&'a Node> {
        match self {
            Item::Node(n) => Some(n),
            Item::Char(_) => None,
        }
    }
}

/// `for (const x of (node || []))`: arrays yield elements, strings code
/// points; any other truthy value is not iterable and throws.
pub fn for_of_or_empty(node: Option<&Node>) -> JsResult<Vec<Item<'_>>> {
    if !truthy(node) {
        return Ok(Vec::new());
    }
    match node.map(Node::kind) {
        Some(NodeKind::Array(items)) => Ok(items.iter().map(Item::Node).collect()),
        Some(NodeKind::String(s)) => Ok(s
            .code_points()
            .map(|cp| {
                let units: Vec<u16> = if cp >= 0x1_0000 {
                    let v = cp - 0x1_0000;
                    vec![0xd800 | (v >> 10) as u16, 0xdc00 | (v & 0x3ff) as u16]
                } else {
                    vec![cp as u16]
                };
                Item::Char(JsString::from_utf16_units(&units))
            })
            .collect()),
        _ => Err(JsThrow::type_error("value is not iterable")),
    }
}

/// `(node || []).forEach(...)` / `.find(...)`: arrays only; any other truthy
/// value has no such method and throws.
pub fn array_or_empty(node: Option<&Node>) -> JsResult<&[Node]> {
    if !truthy(node) {
        return Ok(&[]);
    }
    match node.map(Node::kind) {
        Some(NodeKind::Array(items)) => Ok(items),
        _ => Err(JsThrow::type_error("value is not an array")),
    }
}

/// The elements when the node is an array.
pub fn array(node: Option<&Node>) -> Option<&[Node]> {
    match node.map(Node::kind) {
        Some(NodeKind::Array(items)) => Some(items),
        _ => None,
    }
}

/// `String(x)` for a member read (`${x}` in a template literal).
pub fn to_js_string(node: Option<&Node>) -> JsString {
    js_value_opt(node).to_js_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    fn n(text: &str) -> Node {
        Node::from_value(parse(text).unwrap(), 0)
    }

    #[test]
    fn property_reads_follow_javascript() {
        let arr = n("[1,2]");
        assert_eq!(get(&arr, b"length").to_js(), JsValue::Number(2.0));
        assert_eq!(get(&arr, b"1").to_js(), JsValue::Number(2.0));
        assert_eq!(get(&arr, b"01"), Got::Undefined);
        let s = n("\"ab\"");
        assert_eq!(get(&s, b"1").to_js(), JsValue::from("b"));
        assert!(get_checked(Some(&n("null")), b"x").is_err());
        assert!(get_checked(None, b"x").is_err());
        assert_eq!(get(&n("5"), b"v"), Got::Undefined);
    }

    #[test]
    fn integer_keys_come_first() {
        let o = n(r#"{"b":1,"10":2,"a":3,"2":4}"#);
        let keys: Vec<String> = for_in_keys(Some(&o))
            .iter()
            .map(|k| k.to_string_lossy())
            .collect();
        assert_eq!(keys, ["2", "10", "a", "b"]);
    }

    #[test]
    fn is_property_needs_kf_array_and_v() {
        assert!(is_property(Some(&n(r#"{"v":1,"kf":[]}"#))));
        assert!(is_property(Some(&n(r#"{"v":null,"kf":[]}"#))));
        assert!(!is_property(Some(&n(r#"{"kf":[]}"#))));
        assert!(!is_property(Some(&n(r#"{"v":1,"kf":{}}"#))));
    }
}
