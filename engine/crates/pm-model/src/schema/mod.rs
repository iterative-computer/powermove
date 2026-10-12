//! Typed views of the stored project document.
//!
//! The types mirror the renderer's TypeScript (`core/types/project.ts`,
//! `core/types/captions.ts`, `core/scene3d/schema.ts`, `shared/blender.ts`) and
//! add the section 6 additive fields. They are a lossless lens over
//! [`Value`], never a second source of truth:
//!
//! * **Unknown members are kept.** Every record has an `extra` map holding the
//!   members it does not model, verbatim.
//! * **Shape mismatches are kept.** A modelled member whose value has another
//!   shape (a string where a number belongs, a channel where a constant
//!   belongs and the type does not allow one) stays in `extra` under its own
//!   key and is reported by [`Codec::mismatches`]. Nothing is coerced,
//!   defaulted, or dropped.
//! * **Numbers stay `f64`** exactly as `JSON.parse` produced them, including
//!   `-0`; strings stay [`JsString`], so lone surrogates survive.
//! * **Absent and `null` differ.** Every modelled member is an `Option`
//!   (absent is `None`); members the TypeScript declares nullable use
//!   [`Nullable`].
//! * **Key order is irrelevant**, as everywhere in pm-model.
//!
//! So `T::decode(v)?.encode()` is identical to `v` (numbers compared by bit
//! pattern) for every document, which the corpus tests check.
//!
//! When a typed member and an `extra` entry share a key (possible only after a
//! caller sets a member that decoded as a mismatch), [`Codec::encode`] writes
//! the typed member.
//!
//! Records accept every member on every value of a union and leave the
//! discriminant to the caller: a layer's [`LayerContent`] models the fields of
//! every layer type, a 3D [`Source`] every source kind. This keeps decoding
//! independent of sibling members and makes unknown layer types lossless.

mod blender;
mod captions;
mod layer;
mod project;
mod scene3d;

pub use blender::*;
pub use captions::*;
pub use layer::*;
pub use project::*;
pub use scene3d::*;

use std::collections::BTreeMap;

use crate::json::{JsString, Map, Value};

/// An object whose keys are data (ids, channel names), each value of type `T`.
pub type JsMap<T> = BTreeMap<JsString, T>;

/// Conversion between a typed view and its JSON value.
pub trait Codec: Sized {
    /// The typed view of `value`, or `value` back, untouched, when its shape
    /// does not fit this type.
    fn decode(value: Value) -> Result<Self, Value>;

    /// The JSON value. `decode(v)?.encode()` is identical to `v`.
    fn encode(&self) -> Value;

    /// Appends the path (`layers[3].d.color`) of every modelled member below
    /// this value that was kept raw because its shape did not fit, and of every
    /// enumerated string outside its known values.
    fn mismatches(&self, path: &mut String, out: &mut Vec<String>) {
        let _ = (path, out);
    }
}

/// Every mismatch below `value`, as paths from the root.
pub fn mismatches<T: Codec>(value: &T) -> Vec<String> {
    let mut out = Vec::new();
    value.mismatches(&mut String::new(), &mut out);
    out
}

/// `null` or a value.
#[derive(Clone, Debug, PartialEq)]
pub enum Nullable<T> {
    Null,
    Value(T),
}

impl<T> Nullable<T> {
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Nullable::Null => None,
            Nullable::Value(v) => Some(v),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Nullable::Null)
    }
}

impl<T: Codec> Codec for Nullable<T> {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::Null => Ok(Nullable::Null),
            other => T::decode(other).map(Nullable::Value),
        }
    }

    fn encode(&self) -> Value {
        match self {
            Nullable::Null => Value::Null,
            Nullable::Value(v) => v.encode(),
        }
    }

    fn mismatches(&self, path: &mut String, out: &mut Vec<String>) {
        if let Nullable::Value(v) = self {
            v.mismatches(path, out);
        }
    }
}

impl Codec for f64 {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::Number(n) => Ok(n),
            other => Err(other),
        }
    }

    fn encode(&self) -> Value {
        Value::Number(*self)
    }
}

impl Codec for bool {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::Bool(b) => Ok(b),
            other => Err(other),
        }
    }

    fn encode(&self) -> Value {
        Value::Bool(*self)
    }
}

impl Codec for JsString {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::String(s) => Ok(s),
            other => Err(other),
        }
    }

    fn encode(&self) -> Value {
        Value::String(self.clone())
    }
}

/// Free-form JSON: always decodes.
impl Codec for Value {
    fn decode(value: Value) -> Result<Self, Value> {
        Ok(value)
    }

    fn encode(&self) -> Value {
        self.clone()
    }
}

/// A free-form object.
impl Codec for Map {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::Object(map) => Ok(map),
            other => Err(other),
        }
    }

    fn encode(&self) -> Value {
        Value::Object(self.clone())
    }
}

/// An array whose every element decodes as `T`. One element of another shape
/// keeps the whole array raw (and reported), so element indices never shift.
impl<T: Codec> Codec for Vec<T> {
    fn decode(value: Value) -> Result<Self, Value> {
        let Value::Array(items) = value else {
            return Err(value);
        };
        let mut out: Vec<T> = Vec::with_capacity(items.len());
        let mut rest = items.into_iter();
        while let Some(item) = rest.next() {
            match T::decode(item) {
                Ok(decoded) => out.push(decoded),
                Err(raw) => {
                    let mut restored: Vec<Value> = out.iter().map(Codec::encode).collect();
                    restored.push(raw);
                    restored.extend(rest);
                    return Err(Value::Array(restored));
                }
            }
        }
        Ok(out)
    }

    fn encode(&self) -> Value {
        Value::Array(self.iter().map(Codec::encode).collect())
    }

    fn mismatches(&self, path: &mut String, out: &mut Vec<String>) {
        for (index, item) in self.iter().enumerate() {
            let len = path.len();
            path.push_str(&format!("[{index}]"));
            item.mismatches(path, out);
            path.truncate(len);
        }
    }
}

/// An object whose every value decodes as `T` (same all-or-nothing rule as
/// arrays).
impl<T: Codec> Codec for JsMap<T> {
    fn decode(value: Value) -> Result<Self, Value> {
        let Value::Object(map) = value else {
            return Err(value);
        };
        let mut out: JsMap<T> = BTreeMap::new();
        let mut rest = map.into_iter();
        while let Some((key, item)) = rest.next() {
            match T::decode(item) {
                Ok(decoded) => {
                    out.insert(key, decoded);
                }
                Err(raw) => {
                    let mut restored: Map =
                        out.iter().map(|(k, v)| (k.clone(), v.encode())).collect();
                    restored.insert(key, raw);
                    for (k, v) in rest {
                        restored.insert(k, v);
                    }
                    return Err(Value::Object(restored));
                }
            }
        }
        Ok(out)
    }

    fn encode(&self) -> Value {
        Value::Object(self.iter().map(|(k, v)| (k.clone(), v.encode())).collect())
    }

    fn mismatches(&self, path: &mut String, out: &mut Vec<String>) {
        for (key, item) in self {
            let len = path.len();
            push_key(path, key);
            item.mismatches(path, out);
            path.truncate(len);
        }
    }
}

/// A fixed-length tuple such as `work: [start, end]` or a 2D point.
impl<T: Codec, const N: usize> Codec for [T; N] {
    fn decode(value: Value) -> Result<Self, Value> {
        let Value::Array(items) = value else {
            return Err(value);
        };
        if items.len() != N {
            return Err(Value::Array(items));
        }
        let decoded: Vec<T> = Vec::<T>::decode(Value::Array(items))?;
        match decoded.try_into() {
            Ok(array) => Ok(array),
            Err(_) => unreachable!("length checked above"),
        }
    }

    fn encode(&self) -> Value {
        Value::Array(self.iter().map(Codec::encode).collect())
    }

    fn mismatches(&self, path: &mut String, out: &mut Vec<String>) {
        for (index, item) in self.iter().enumerate() {
            let len = path.len();
            path.push_str(&format!("[{index}]"));
            item.mismatches(path, out);
            path.truncate(len);
        }
    }
}

/// Appends `.key` (or `["key"]` when the key is not a plain identifier).
pub(crate) fn push_key(path: &mut String, key: &JsString) {
    let text = key.to_string_lossy();
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '-');
    if plain {
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(&text);
    } else {
        path.push_str(&format!("[{text:?}]"));
    }
}

/// Removes `key` from `map` and decodes it; a value of another shape goes back
/// into `map`.
#[doc(hidden)]
pub fn take<T: Codec>(map: &mut Map, key: &str) -> Option<T> {
    let value = map.remove(key)?;
    match T::decode(value) {
        Ok(typed) => Some(typed),
        Err(raw) => {
            map.insert(key, raw);
            None
        }
    }
}

/// Reports `key` when it was kept raw in `extra`, else descends into `value`.
#[doc(hidden)]
pub fn check_member<T: Codec>(
    path: &mut String,
    out: &mut Vec<String>,
    key: &str,
    value: Option<&T>,
    extra: &Map,
) {
    let len = path.len();
    push_key(path, &JsString::from(key));
    if extra.contains_key(key) {
        out.push(path.clone());
    } else if let Some(value) = value {
        value.mismatches(path, out);
    }
    path.truncate(len);
}

/// Defines a record: a struct of optional typed members plus `extra`, with its
/// [`Codec`].
macro_rules! record {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( $(#[$fmeta:meta])* $field:ident : $ty:ty = $key:literal ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name {
            $( $(#[$fmeta])* pub $field: Option<$ty>, )*
            /// Members not modelled here, and modelled members whose value had
            /// another shape, verbatim.
            pub extra: $crate::json::Map,
        }

        impl $name {
            /// The JSON member names this record models.
            pub const KEYS: &'static [&'static str] = &[$($key),*];
        }

        impl $crate::schema::Codec for $name {
            fn decode(value: $crate::json::Value) -> Result<Self, $crate::json::Value> {
                let $crate::json::Value::Object(mut extra) = value else {
                    return Err(value);
                };
                $( let $field = $crate::schema::take::<$ty>(&mut extra, $key); )*
                Ok($name { $($field,)* extra })
            }

            fn encode(&self) -> $crate::json::Value {
                #[allow(unused_mut)]
                let mut map = self.extra.clone();
                $(
                    if let Some(value) = &self.$field {
                        map.insert($key, $crate::schema::Codec::encode(value));
                    }
                )*
                $crate::json::Value::Object(map)
            }

            fn mismatches(&self, path: &mut String, out: &mut Vec<String>) {
                let _ = (&path, &out);
                $( $crate::schema::check_member(path, out, $key, self.$field.as_ref(), &self.extra); )*
            }
        }
    };
}
pub(crate) use record;

/// Defines an enumerated string. Values outside the list decode as `Other`
/// (lossless) and are reported by [`Codec::mismatches`].
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $text:literal ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )*
            /// A string outside the known values, verbatim.
            Other($crate::json::JsString),
        }

        impl $name {
            /// The known values, in declaration order.
            pub const VALUES: &'static [&'static str] = &[$($text),*];

            /// The JSON string.
            pub fn as_js_string(&self) -> $crate::json::JsString {
                match self {
                    $( $name::$variant => $crate::json::JsString::from($text), )*
                    $name::Other(s) => s.clone(),
                }
            }
        }

        impl $crate::schema::Codec for $name {
            fn decode(value: $crate::json::Value) -> Result<Self, $crate::json::Value> {
                match value {
                    $crate::json::Value::String(s) => Ok(match s.as_str() {
                        $( Some($text) => $name::$variant, )*
                        _ => $name::Other(s),
                    }),
                    other => Err(other),
                }
            }

            fn encode(&self) -> $crate::json::Value {
                $crate::json::Value::String(self.as_js_string())
            }

            fn mismatches(&self, path: &mut String, out: &mut Vec<String>) {
                if let $name::Other(s) = self {
                    out.push(format!("{path} (unknown value {:?})", s.to_string_lossy()));
                }
            }
        }
    };
}
pub(crate) use string_enum;
