//! JSON values with JavaScript semantics.
//!
//! The project document is produced and consumed by JavaScript (`JSON.parse`,
//! `JSON.stringify`), so the engine models JSON the way JavaScript sees it:
//!
//! * every number is an IEEE-754 `f64` (`JSON.parse` semantics: correctly
//!   rounded, overflow to ±Infinity);
//! * strings are sequences of UTF-16 code units, so lone surrogates written as
//!   `\uD800` escapes survive a round trip ([`JsString`] stores them as WTF-8);
//! * object keys are unique (the last duplicate wins, as in `JSON.parse`) and
//!   their order carries no meaning;
//! * serialization ([`write`](mod@write)) reproduces `JSON.stringify` byte for byte for
//!   strings and numbers, with object keys sorted by UTF-16 code units.

mod jsnum;
pub mod object_map;
mod parse;
mod string;
pub mod write;

use std::collections::BTreeMap;

pub(crate) use jsnum::array_index;
pub use jsnum::{js_trim, number_to_js_string, string_to_number};
pub use object_map::ObjectMap;
pub use parse::{
    ParseError, Prebuilt, TreeBuilder, Unit, UnitScanner, parse, parse_bytes, parse_bytes_parallel,
    parse_bytes_parallel_prebuilt, parse_bytes_with, parse_file_text, parse_unit_with, parse_with,
};
pub use string::JsString;
pub use write::{to_canonical_string, write_canonical};

/// A JSON value as JavaScript holds it after `JSON.parse`.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    String(JsString),
    Array(Vec<Value>),
    Object(Map),
}

/// A JSON object: unique keys, no meaningful order.
///
/// Stored sorted by WTF-8 bytes (code point order); [`write`](mod@write) re-sorts by
/// UTF-16 code units where the two orders differ.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Map(ObjectMap<Value>);

impl Map {
    pub fn new() -> Map {
        Map(ObjectMap::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key.as_bytes())
    }

    pub fn get_key(&self, key: &JsString) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.0.get_mut(key.as_bytes())
    }

    pub fn get_key_mut(&mut self, key: &JsString) -> Option<&mut Value> {
        self.0.get_mut(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key.as_bytes())
    }

    pub fn insert(&mut self, key: impl Into<JsString>, value: Value) -> Option<Value> {
        self.0.insert(key.into(), value)
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.0.remove(key.as_bytes())
    }

    pub fn remove_key(&mut self, key: &JsString) -> Option<Value> {
        self.0.remove(key)
    }

    pub fn iter(&self) -> object_map::Iter<'_, Value> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> object_map::IterMut<'_, Value> {
        self.0.iter_mut()
    }

    pub fn keys(&self) -> object_map::Keys<'_, Value> {
        self.0.keys()
    }

    pub fn values(&self) -> object_map::Values<'_, Value> {
        self.0.values()
    }

    /// The members (the object map documents share with their trees).
    pub fn as_members(&self) -> &ObjectMap<Value> {
        &self.0
    }

    pub fn as_members_mut(&mut self) -> &mut ObjectMap<Value> {
        &mut self.0
    }

    pub fn into_members(self) -> ObjectMap<Value> {
        self.0
    }

    /// [`ObjectMap::from_members`].
    pub(crate) fn from_members(members: &mut Vec<(JsString, Value)>, start: usize) -> Map {
        Map(ObjectMap::from_members(members, start))
    }
}

impl From<ObjectMap<Value>> for Map {
    fn from(map: ObjectMap<Value>) -> Map {
        Map(map)
    }
}

impl From<BTreeMap<JsString, Value>> for Map {
    fn from(map: BTreeMap<JsString, Value>) -> Map {
        Map(map.into_iter().collect())
    }
}

impl FromIterator<(JsString, Value)> for Map {
    fn from_iter<I: IntoIterator<Item = (JsString, Value)>>(iter: I) -> Map {
        Map(iter.into_iter().collect())
    }
}

impl IntoIterator for Map {
    type Item = (JsString, Value);
    type IntoIter = std::vec::IntoIter<(JsString, Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Map {
    type Item = (&'a JsString, &'a Value);
    type IntoIter = object_map::Iter<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Value {
    /// `"object"`-style kind name for diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Map> {
        match self {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The string when it is well-formed UTF-16 (no lone surrogates).
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => s.as_str(),
            _ => None,
        }
    }

    pub fn as_js_string(&self) -> Option<&JsString> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// JavaScript truthiness (`if (value)`).
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.as_bytes().is_empty(),
            Value::Array(_) | Value::Object(_) => true,
        }
    }

    /// `String(value)` for JSON values.
    pub fn to_js_string(&self) -> JsString {
        match self {
            Value::Null => JsString::from("null"),
            Value::Bool(b) => JsString::from(if *b { "true" } else { "false" }),
            Value::Number(n) => JsString::from(number_to_js_string(*n)),
            Value::String(s) => s.clone(),
            Value::Array(items) => {
                // Array.prototype.join: null elements are empty.
                let parts: Vec<String> = items
                    .iter()
                    .map(|item| match item {
                        Value::Null => String::new(),
                        other => other.to_js_string().to_string_lossy(),
                    })
                    .collect();
                JsString::from(parts.join(","))
            }
            Value::Object(_) => JsString::from("[object Object]"),
        }
    }

    /// Object member lookup; `None` for non-objects.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object().and_then(|map| map.get(key))
    }

    /// Structural identity: like `==` but numbers compare by bit pattern, so
    /// `-0` differs from `0` and `NaN` equals itself. Used to prove lossless
    /// round trips.
    pub fn identical(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a.to_bits() == b.to_bits(),
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.identical(y))
            }
            (Value::Object(a), Value::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|((ka, va), (kb, vb))| ka == kb && va.identical(vb))
            }
            _ => self == other,
        }
    }

    /// What `JSON.parse(JSON.stringify(value))` yields: `-0` becomes `0` and
    /// non-finite numbers become `null`. This is `clonePatchValue` in
    /// `src/shared/patch.ts`.
    pub fn json_normalized(&self) -> Value {
        match self {
            Value::Number(n) if !n.is_finite() => Value::Null,
            Value::Number(n) if *n == 0.0 => Value::Number(0.0),
            Value::Array(items) => Value::Array(items.iter().map(Value::json_normalized).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), v.json_normalized()))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// Normalizes in place (see [`Value::json_normalized`]).
    pub fn json_normalize(&mut self) {
        match self {
            Value::Number(n) if !n.is_finite() => *self = Value::Null,
            Value::Number(n) if *n == 0.0 => *n = 0.0,
            Value::Array(items) => items.iter_mut().for_each(Value::json_normalize),
            Value::Object(map) => map.iter_mut().for_each(|(_, v)| v.json_normalize()),
            _ => {}
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Value {
        Value::Number(n)
    }
}

macro_rules! number_from {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(n: $t) -> Value {
                Value::Number(n as f64)
            }
        }
    )*};
}
number_from!(i32, i64, u32, u64, usize, f32);

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::String(JsString::from(s))
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::String(JsString::from(s))
    }
}

impl From<JsString> for Value {
    fn from(s: JsString) -> Value {
        Value::String(s)
    }
}

impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Value {
        Value::Array(items)
    }
}

impl From<Map> for Value {
    fn from(map: Map) -> Value {
        Value::Object(map)
    }
}

/// Builds a [`Value`] from JSON-like syntax in tests and examples.
///
/// ```
/// use pm_model::json;
/// let v = json!({"a": [1, true, null, "x"]});
/// assert_eq!(v.get("a").unwrap().as_array().unwrap().len(), 4);
/// ```
#[macro_export]
macro_rules! json {
    (null) => { $crate::json::Value::Null };
    ([ $($item:tt),* $(,)? ]) => { $crate::json::Value::Array(vec![ $( $crate::json!($item) ),* ]) };
    ({ $($key:literal : $value:tt),* $(,)? }) => {{
        #[allow(unused_mut)]
        let mut map = $crate::json::Map::new();
        $( map.insert($key, $crate::json!($value)); )*
        $crate::json::Value::Object(map)
    }};
    ($other:expr) => { $crate::json::Value::from($other) };
}
