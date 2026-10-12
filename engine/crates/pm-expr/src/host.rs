//! Access to raw project values (`layer`, `comp`, `value`, and everything
//! reachable from them).

use crate::string::JsStr;
use crate::value::{Object, Value};

/// What a document handle points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Object,
    Array { len: usize },
}

/// Read access to the document as JavaScript sees it.
///
/// Expressions read raw project values the way `expression.ts` reads the live
/// project objects: own properties only, array elements by index, and object
/// identity for `===` and `==`. `pm-eval` implements this over its document
/// view; [`JsonHost`] implements it over a parsed JSON tree.
pub trait Host {
    /// A handle to a document object or array. Equality must be object
    /// identity: two handles are equal exactly when JavaScript's `===` would
    /// hold for the objects they stand for (for example `comp.layers[i]` and
    /// `layer` when they are the same layer).
    type Obj: Clone + PartialEq;

    fn shape(&self, obj: &Self::Obj) -> Shape;

    /// An element of an array, `index < len`.
    fn element(&self, array: &Self::Obj, index: usize) -> Value<Self::Obj>;

    /// An own property of a (non-array) object.
    fn property(&self, object: &Self::Obj, key: &JsStr) -> Option<Value<Self::Obj>>;
}

/// A [`Host`] over [`pm_model::Value`] trees, with identity by address.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonHost<'a>(std::marker::PhantomData<&'a pm_model::Value>);

/// A reference into a [`pm_model::Value`] tree; equal when it is the same node.
#[derive(Clone, Copy, Debug)]
pub struct JsonRef<'a>(pub &'a pm_model::Value);

impl PartialEq for JsonRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl<'a> JsonHost<'a> {
    pub const fn new() -> JsonHost<'a> {
        JsonHost(std::marker::PhantomData)
    }

    /// The expression value of a JSON node: primitives by value, objects and
    /// arrays by reference.
    pub fn value(node: &'a pm_model::Value) -> Value<JsonRef<'a>> {
        match node {
            pm_model::Value::Null => Value::Null,
            pm_model::Value::Bool(b) => Value::Bool(*b),
            pm_model::Value::Number(n) => Value::Number(*n),
            pm_model::Value::String(s) => Value::String(JsStr::from_units(s.utf16_units().collect::<Vec<u16>>())),
            pm_model::Value::Array(_) | pm_model::Value::Object(_) => Value::Object(Object::Doc(JsonRef(node))),
        }
    }
}

impl<'a> Host for JsonHost<'a> {
    type Obj = JsonRef<'a>;

    fn shape(&self, obj: &JsonRef<'a>) -> Shape {
        match obj.0 {
            pm_model::Value::Array(items) => Shape::Array { len: items.len() },
            _ => Shape::Object,
        }
    }

    fn element(&self, array: &JsonRef<'a>, index: usize) -> Value<JsonRef<'a>> {
        match array.0 {
            pm_model::Value::Array(items) => items.get(index).map_or(Value::Undefined, JsonHost::value),
            _ => Value::Undefined,
        }
    }

    fn property(&self, object: &JsonRef<'a>, key: &JsStr) -> Option<Value<JsonRef<'a>>> {
        match object.0 {
            pm_model::Value::Object(map) => {
                let key = pm_model::JsString::from_utf16_units(key.units());
                map.get_key(&key).map(JsonHost::value)
            }
            _ => None,
        }
    }
}
