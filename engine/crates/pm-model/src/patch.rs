//! Leaf patches with the exact semantics of `apps/desktop/src/shared/patch.ts`.
//!
//! [`apply_patches`] reproduces `applyPatch` on the JSON-visible state of the
//! document, and [`crate::doc::Document::apply`] does the same on the
//! revisioned tree with id-keyed reconciliation. Both run one implementation,
//! generic over one private `Tree` trait, so the semantics cannot drift apart.
//!
//! What "exact" covers, beyond ordinary sets and deletes:
//!
//! * JavaScript's property-key coercion: `["layers", 0]` and `["layers", "0"]`
//!   address the same element, `-0` is the key `"0"`, and `1.5` is the
//!   non-index key `"1.5"`.
//! * Navigation stops at `null` and `undefined` (`parent?.[key]` followed by
//!   `parent == null`), and walks into strings (`"abc"[1]` is `"b"`) and array
//!   or string `length`.
//! * Arrays: `length` writes (with `ArraySetLength`'s `RangeError`), writes past
//!   the end leave holes (which serialize as `null`), a numeric-key delete is
//!   `splice(key, 1)` with `ToIntegerOrInfinity` and negative indices, and a
//!   string-key delete leaves a hole.
//! * `undefined` values: an object member set to `undefined` is invisible to
//!   JSON (it is removed here), an array element set to `undefined` serializes
//!   as `null`.
//! * `__proto__`: writing it on an object without an own `__proto__` member
//!   calls the prototype setter, which JSON never sees; `JSON.parse` creates own
//!   `__proto__` members, which behave like any other key.
//! * The `TypeError` and `RangeError` that strict-mode assignment and deletion
//!   throw (creating a property on a primitive, deleting a string's index or an
//!   array's `length`, an invalid array length). As in JavaScript, a throwing
//!   patch leaves the earlier patches of the batch applied.
//!
//! Deliberate, documented limits. pm-model models the JSON-visible document,
//! not JavaScript objects with prototypes and non-JSON properties, so these
//! differ from JavaScript. The renderer's patches never take these paths:
//! `diffPatches` only emits own JSON members and array indices.
//!
//! * Navigating through a property that exists only on a prototype (for
//!   example `["name", "toUpperCase", "x"]`, `["layers", "map", "x"]`, or a
//!   non-own `__proto__`) is a no-op here. In JavaScript such writes land on
//!   shared prototype objects or functions, never on the document, but a few
//!   of them throw (writing a function's read-only `name`).
//! * Assigning an object, array, or `null` to a non-own `__proto__` is a
//!   no-op here. In JavaScript it replaces the prototype, after which holes in
//!   an array serialize as the prototype's elements, navigation can reach
//!   inherited members, a `null` prototype turns later `__proto__` writes into
//!   own members, and an array without `Array.prototype` throws on `splice`.
//! * A member set to `undefined` is removed here. JavaScript keeps an own
//!   member holding `undefined`, which JSON never shows; the difference is
//!   observable only for `__proto__`, where a later write then reaches the
//!   own member instead of the prototype setter.
//! * Writing a non-index key of an array (`["layers", "foo"]`) creates a
//!   property JSON never serializes; it is dropped here, so a later patch that
//!   navigates through it is a no-op here where JavaScript might throw.
//! * Growing an array beyond [`MAX_ARRAY_LENGTH`] elements fails with
//!   [`PatchErrorKind::Limit`]; JavaScript would create a sparse array that
//!   serializes to that many `null`s.

use core::fmt;

use crate::json::{self, JsString, Map, Value, number_to_js_string, string_to_number};

/// Largest array a patch may create (16,777,216 elements).
pub const MAX_ARRAY_LENGTH: usize = 1 << 24;

/// Default byte budget of [`is_patch_list`] (`isPatchList`'s `maxBytes`).
pub const DEFAULT_MAX_PATCH_BYTES: usize = 32 * 1024 * 1024;

/// One path component: JavaScript property keys are strings or numbers.
#[derive(Clone, Debug, PartialEq)]
pub enum PathPart {
    Key(JsString),
    Index(f64),
}

impl PathPart {
    /// `ToPropertyKey`: numbers become their `Number::toString` form.
    pub fn property_key(&self) -> JsString {
        match self {
            PathPart::Key(k) => k.clone(),
            PathPart::Index(n) => JsString::from(number_to_js_string(*n)),
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            PathPart::Key(k) => Value::String(k.clone()),
            PathPart::Index(n) => Value::Number(*n),
        }
    }
}

impl From<&str> for PathPart {
    fn from(s: &str) -> PathPart {
        PathPart::Key(JsString::from(s))
    }
}

impl From<String> for PathPart {
    fn from(s: String) -> PathPart {
        PathPart::Key(JsString::from(s))
    }
}

impl From<JsString> for PathPart {
    fn from(s: JsString) -> PathPart {
        PathPart::Key(s)
    }
}

impl From<usize> for PathPart {
    fn from(i: usize) -> PathPart {
        PathPart::Index(i as f64)
    }
}

impl From<i32> for PathPart {
    fn from(i: i32) -> PathPart {
        PathPart::Index(i as f64)
    }
}

impl From<f64> for PathPart {
    fn from(n: f64) -> PathPart {
        PathPart::Index(n)
    }
}

/// Builds a path (an array of [`PathPart`]s) from mixed string and number
/// parts.
///
/// ```
/// use pm_model::{path, patch::PathPart};
/// assert_eq!(path!["layers", 0, "name"][1], PathPart::Index(0.0));
/// ```
#[macro_export]
macro_rules! path {
    ($($part:expr),* $(,)?) => { [$($crate::patch::PathPart::from($part)),*] };
}

/// `{ path, exists, value? }`. `value: None` is JavaScript `undefined`.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    pub path: Vec<PathPart>,
    pub exists: bool,
    pub value: Option<Value>,
}

impl Patch {
    pub fn set(path: impl Into<Vec<PathPart>>, value: Value) -> Patch {
        Patch {
            path: path.into(),
            exists: true,
            value: Some(value),
        }
    }

    pub fn delete(path: impl Into<Vec<PathPart>>) -> Patch {
        Patch {
            path: path.into(),
            exists: false,
            value: None,
        }
    }

    /// Decodes one patch object. Path parts must be strings or numbers and
    /// `exists` a boolean (what `isPatchList` admits at the boundary).
    pub fn from_value(value: &Value) -> Result<Patch, PatchDecodeError> {
        let map = value
            .as_object()
            .ok_or(PatchDecodeError("a patch must be an object"))?;
        let path = map
            .get("path")
            .and_then(Value::as_array)
            .ok_or(PatchDecodeError("a patch needs a path array"))?
            .iter()
            .map(|part| match part {
                Value::String(s) => Ok(PathPart::Key(s.clone())),
                Value::Number(n) => Ok(PathPart::Index(*n)),
                _ => Err(PatchDecodeError("path parts must be strings or numbers")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let exists = map
            .get("exists")
            .and_then(Value::as_bool)
            .ok_or(PatchDecodeError("a patch needs a boolean exists"))?;
        Ok(Patch {
            path,
            exists,
            value: map.get("value").cloned(),
        })
    }

    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "path",
            Value::Array(self.path.iter().map(PathPart::to_value).collect()),
        );
        map.insert("exists", Value::Bool(self.exists));
        if let Some(v) = &self.value {
            map.insert("value", v.clone());
        }
        Value::Object(map)
    }
}

/// Decodes a JSON patch list.
pub fn patches_from_value(value: &Value) -> Result<Vec<Patch>, PatchDecodeError> {
    value
        .as_array()
        .ok_or(PatchDecodeError("patches must be an array"))?
        .iter()
        .map(Patch::from_value)
        .collect()
}

/// Encodes a patch list as JSON.
pub fn patches_to_value(patches: &[Patch]) -> Value {
    Value::Array(patches.iter().map(Patch::to_value).collect())
}

/// A malformed patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchDecodeError(pub &'static str);

impl fmt::Display for PatchDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for PatchDecodeError {}

/// What JavaScript would throw, and at which patch of the batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchError {
    pub index: usize,
    pub kind: PatchErrorKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchErrorKind {
    /// Strict-mode assignment or deletion that JavaScript rejects.
    TypeError,
    /// An invalid array length.
    RangeError,
    /// Beyond [`MAX_ARRAY_LENGTH`].
    Limit,
    /// An id-keyed operation ([`crate::ops`]) whose target does not match the
    /// document: the path is not an id-keyed array, or an id is unknown,
    /// duplicated, or already present. The mirror has diverged from the
    /// renderer and must resynchronize.
    Mismatch,
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "patch {}: {:?}: {}", self.index, self.kind, self.message)
    }
}

impl std::error::Error for PatchError {}

fn type_error(index: usize, message: String) -> PatchError {
    PatchError {
        index,
        kind: PatchErrorKind::TypeError,
        message,
    }
}

pub(crate) fn limit(index: usize) -> PatchError {
    PatchError {
        index,
        kind: PatchErrorKind::Limit,
        message: format!("arrays are limited to {MAX_ARRAY_LENGTH} elements"),
    }
}

/// `clonePatchValue`: `JSON.parse(JSON.stringify(value))`.
pub fn clone_patch_value(value: Option<&Value>) -> Option<Value> {
    value.map(Value::json_normalized)
}

// ── The tree the semantics run on ───────────────────────────────────────────

/// One node of a patchable tree, as navigation sees it.
pub(crate) enum NodeMut<'a, T> {
    Null,
    Bool(bool),
    Number(f64),
    String(&'a JsString),
    Array(&'a mut Vec<T>),
    Object(&'a mut crate::json::ObjectMap<T>),
}

/// A JSON-shaped tree that patches can be applied to: plain [`Value`]s, and the
/// revisioned [`crate::doc::Node`] tree.
pub(crate) trait Tree: Sized {
    fn node_mut(&mut self) -> NodeMut<'_, Self>;
    /// A new node holding `value`, created at revision `rev`.
    fn create(value: Value, rev: u64) -> Self;
    /// Replaces this node's content with `value` (reconciling where the tree
    /// supports it). Returns whether the JSON-visible content changed.
    fn assign(&mut self, value: Value, rev: u64) -> bool;
    /// Records that something below this node changed at `rev`.
    fn touch(&mut self, rev: u64);
    /// The string `id` member when this node is an object that has one.
    fn record_id(&self) -> Option<&JsString>;
}

impl Tree for Value {
    fn node_mut(&mut self) -> NodeMut<'_, Value> {
        match self {
            Value::Null => NodeMut::Null,
            Value::Bool(b) => NodeMut::Bool(*b),
            Value::Number(n) => NodeMut::Number(*n),
            Value::String(s) => NodeMut::String(s),
            Value::Array(items) => NodeMut::Array(items),
            Value::Object(map) => NodeMut::Object(map.as_members_mut()),
        }
    }

    fn create(value: Value, _rev: u64) -> Value {
        value
    }

    fn assign(&mut self, value: Value, _rev: u64) -> bool {
        let changed = !self.identical(&value);
        *self = value;
        changed
    }

    fn touch(&mut self, _rev: u64) {}

    fn record_id(&self) -> Option<&JsString> {
        record_id(self)
    }
}

/// A primitive reached during navigation (a string's code unit, an array's or
/// string's `length`, or a stored scalar).
#[derive(Clone, Debug)]
enum Primitive {
    String(JsString),
    Number(f64),
    Bool(bool),
}

impl Primitive {
    fn describe(&self) -> String {
        match self {
            Primitive::String(s) => format!("string '{}'", s.to_string_lossy()),
            Primitive::Number(n) => format!("number '{}'", number_to_js_string(*n)),
            Primitive::Bool(b) => format!("boolean '{b}'"),
        }
    }

    /// `primitive[key]` when it yields another primitive JavaScript can index.
    fn child(&self, key: &JsString) -> Option<Primitive> {
        match self {
            Primitive::String(s) => {
                let k = key.as_str()?;
                if k == "length" {
                    return Some(Primitive::Number(s.utf16_len() as f64));
                }
                let index = json::array_index(k)? as usize;
                s.unit_at(index).map(Primitive::String)
            }
            // Numbers and booleans have only inherited members.
            _ => None,
        }
    }

    /// The final step of a patch whose parent is a primitive. Never changes
    /// the document; may throw.
    fn finish(&self, index: usize, key: &JsString, exists: bool) -> Result<bool, PatchError> {
        if exists {
            // Strict mode cannot create or assign properties on primitives,
            // except that `__proto__` reaches Object.prototype's setter, which
            // ignores primitive receivers.
            if key == "__proto__" {
                return Ok(false);
            }
            return Err(type_error(
                index,
                format!(
                    "Cannot create property '{}' on {}",
                    key.to_string_lossy(),
                    self.describe()
                ),
            ));
        }
        // delete on a primitive deletes from a temporary wrapper; only a
        // string's own non-configurable index and length properties throw.
        if let Primitive::String(s) = self
            && let Some(k) = key.as_str()
        {
            let own =
                k == "length" || json::array_index(k).is_some_and(|i| (i as usize) < s.utf16_len());
            if own {
                return Err(type_error(
                    index,
                    format!("Cannot delete property '{k}' of [object String]"),
                ));
            }
        }
        Ok(false)
    }
}

/// `ArraySetLength`'s check: `ToUint32(v)` must equal `ToNumber(v)`.
fn array_length_from(value: Option<&Value>, index: usize) -> Result<usize, PatchError> {
    let number = to_number(value);
    let valid =
        number.is_finite() && number >= 0.0 && number.fract() == 0.0 && number <= u32::MAX as f64;
    if !valid {
        return Err(PatchError {
            index,
            kind: PatchErrorKind::RangeError,
            message: "Invalid array length".into(),
        });
    }
    let length = number as u64 as usize;
    if length > MAX_ARRAY_LENGTH {
        return Err(limit(index));
    }
    Ok(length)
}

/// `ToNumber` for JSON values (`undefined` is `None`).
fn to_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => *b as u8 as f64,
        Some(Value::Number(n)) => *n,
        Some(Value::String(s)) => string_to_number(&s.to_string_lossy()),
        Some(v @ Value::Array(_)) => string_to_number(&to_js_string(v)),
        Some(Value::Object(_)) => f64::NAN, // "[object Object]"
    }
}

/// `ToString` for JSON values, as `Array.prototype.join` uses it.
fn to_js_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(), // join() writes null and undefined as ""
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => number_to_js_string(*n),
        Value::String(s) => s.to_string_lossy(),
        Value::Array(items) => items.iter().map(to_js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// The start index `Array.prototype.splice(key, 1)` removes, if any.
fn splice_start(key: f64, len: usize) -> Option<usize> {
    // ToIntegerOrInfinity
    let relative = if key.is_nan() { 0.0 } else { key.trunc() };
    let len_f = len as f64;
    let start = if relative < 0.0 {
        (len_f + relative).max(0.0)
    } else {
        relative.min(len_f)
    };
    let start = start as usize;
    (start < len).then_some(start)
}

/// Applies patches in place with `applyPatch` semantics. `root: None` is an
/// `undefined` document. Returns whether the JSON-visible document changed.
pub fn apply_patches(root: &mut Option<Value>, patches: &[Patch]) -> Result<bool, PatchError> {
    let mut changed = false;
    for (index, patch) in patches.iter().enumerate() {
        changed |= apply_one(root, patch, index, 0)?;
    }
    Ok(changed)
}

/// One patch on any [`Tree`]. Nodes that change are stamped with `rev`.
pub(crate) fn apply_one<T: Tree>(
    root: &mut Option<T>,
    patch: &Patch,
    index: usize,
    rev: u64,
) -> Result<bool, PatchError> {
    let Some((last, parents)) = patch.path.split_last() else {
        let value = if patch.exists {
            clone_patch_value(patch.value.as_ref())
        } else {
            None
        };
        return Ok(match (root.as_mut(), value) {
            (Some(node), Some(value)) => node.assign(value, rev),
            (None, Some(value)) => {
                *root = Some(T::create(value, rev));
                true
            }
            (node, None) => {
                let had = node.is_some();
                *root = None;
                had
            }
        });
    };
    match root.as_mut() {
        Some(node) => descend(node, parents, last, patch, index, rev),
        None => Ok(false),
    }
}

enum Step<'a, T> {
    Stop,
    Node(&'a mut T),
    Primitive(Primitive),
}

fn descend<T: Tree>(
    node: &mut T,
    parents: &[PathPart],
    last: &PathPart,
    patch: &Patch,
    index: usize,
    rev: u64,
) -> Result<bool, PatchError> {
    let changed = match parents.split_first() {
        None => finish(node, last, patch, index, rev)?,
        Some((first, rest)) => match child_step(node, &first.property_key()) {
            Step::Stop => false,
            Step::Node(child) => descend(child, rest, last, patch, index, rev)?,
            Step::Primitive(mut primitive) => {
                for part in rest {
                    match primitive.child(&part.property_key()) {
                        Some(next) => primitive = next,
                        None => return Ok(false),
                    }
                }
                primitive.finish(index, &last.property_key(), patch.exists)?
            }
        },
    };
    if changed {
        node.touch(rev);
    }
    Ok(changed)
}

/// `node?.[key]` during navigation.
fn child_step<'a, T: Tree>(node: &'a mut T, key: &JsString) -> Step<'a, T> {
    match node.node_mut() {
        NodeMut::Null | NodeMut::Number(_) | NodeMut::Bool(_) => Step::Stop,
        NodeMut::String(s) => match Primitive::String(s.clone()).child(key) {
            Some(p) => Step::Primitive(p),
            None => Step::Stop,
        },
        NodeMut::Object(map) => match map.get_mut(key) {
            Some(child) => scalar_or_node(child),
            None => Step::Stop,
        },
        NodeMut::Array(items) => {
            let Some(k) = key.as_str() else {
                return Step::Stop;
            };
            if k == "length" {
                return Step::Primitive(Primitive::Number(items.len() as f64));
            }
            match json::array_index(k).and_then(|i| items.get_mut(i as usize)) {
                Some(child) => scalar_or_node(child),
                None => Step::Stop,
            }
        }
    }
}

fn scalar_or_node<T: Tree>(child: &mut T) -> Step<'_, T> {
    let primitive = match child.node_mut() {
        NodeMut::Null => return Step::Stop,
        NodeMut::String(s) => Primitive::String(s.clone()),
        NodeMut::Number(n) => Primitive::Number(n),
        NodeMut::Bool(b) => Primitive::Bool(b),
        NodeMut::Array(_) | NodeMut::Object(_) => return Step::Node(child),
    };
    Step::Primitive(primitive)
}

fn finish<T: Tree>(
    parent: &mut T,
    last: &PathPart,
    patch: &Patch,
    index: usize,
    rev: u64,
) -> Result<bool, PatchError> {
    let key = last.property_key();
    match parent.node_mut() {
        NodeMut::Null => Ok(false),
        NodeMut::String(s) => Primitive::String(s.clone()).finish(index, &key, patch.exists),
        NodeMut::Number(n) => Primitive::Number(n).finish(index, &key, patch.exists),
        NodeMut::Bool(b) => Primitive::Bool(b).finish(index, &key, patch.exists),
        NodeMut::Object(map) => {
            if patch.exists {
                if key == "__proto__" && !map.contains_key(&key) {
                    return Ok(false); // the prototype setter: invisible to JSON
                }
                Ok(match clone_patch_value(patch.value.as_ref()) {
                    Some(value) => match map.get_mut(&key) {
                        Some(child) => child.assign(value, rev),
                        None => {
                            map.insert(key, T::create(value, rev));
                            true
                        }
                    },
                    // An own member holding undefined serializes as absent.
                    None => map.remove(&key).is_some(),
                })
            } else {
                Ok(map.remove(&key).is_some())
            }
        }
        NodeMut::Array(items) => {
            let k = key.as_str();
            if patch.exists {
                if let Some(i) = k.and_then(json::array_index) {
                    let i = i as usize;
                    let value = clone_patch_value(patch.value.as_ref()).unwrap_or(Value::Null);
                    if i < items.len() {
                        return Ok(items[i].assign(value, rev));
                    }
                    if i >= MAX_ARRAY_LENGTH {
                        return Err(limit(index));
                    }
                    items.resize_with(i, || T::create(Value::Null, rev));
                    items.push(T::create(value, rev));
                    Ok(true)
                } else if k == Some("length") {
                    let len = array_length_from(patch.value.as_ref(), index)?;
                    let changed = len != items.len();
                    items.resize_with(len, || T::create(Value::Null, rev));
                    Ok(changed)
                } else {
                    // Other keys create non-index properties JSON never writes.
                    Ok(false)
                }
            } else if let PathPart::Index(n) = last {
                Ok(match splice_start(*n, items.len()) {
                    Some(start) => {
                        items.remove(start);
                        true
                    }
                    None => false,
                })
            } else if let Some(i) = k.and_then(json::array_index) {
                Ok(match items.get_mut(i as usize) {
                    Some(slot) => slot.assign(Value::Null, rev), // a hole
                    None => false,
                })
            } else if k == Some("length") {
                Err(type_error(
                    index,
                    "Cannot delete property 'length' of [object Array]".into(),
                ))
            } else {
                Ok(false)
            }
        }
    }
}

// ── diffPatches and isPatchList ─────────────────────────────────────────────

/// `diffPatches(before, after)`: leaf patches turning `before` into `after`
/// (`forward`) and back (`backward`). Arrays whose length or object-`id` order
/// changed are replaced whole. Sibling keys come out in code-point order,
/// which may differ from JavaScript's insertion order; sibling patches never
/// overlap, so applying either order gives the same document.
pub fn diff_patches(before: Option<&Value>, after: Option<&Value>) -> (Vec<Patch>, Vec<Patch>) {
    let mut forward = Vec::new();
    let mut backward = Vec::new();
    diff_into(before, after, &mut Vec::new(), &mut forward, &mut backward);
    (forward, backward)
}

/// `Object.is` for JSON values; distinct arrays and objects are never the same.
pub(crate) fn object_is(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => {
            x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan())
        }
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

pub(crate) fn record_id(v: &Value) -> Option<&JsString> {
    match v.as_object()?.get("id") {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

fn diff_into(
    before: Option<&Value>,
    after: Option<&Value>,
    path: &mut Vec<PathPart>,
    forward: &mut Vec<Patch>,
    backward: &mut Vec<Patch>,
) {
    if object_is(before, after) {
        return;
    }
    match (before, after) {
        (Some(Value::Array(b)), Some(Value::Array(a))) => {
            let identity_order_changed = b.len() == a.len()
                && b.iter().zip(a).any(
                    |(x, y)| matches!((record_id(x), record_id(y)), (Some(i), Some(j)) if i != j),
                );
            if b.len() != a.len() || identity_order_changed {
                forward.push(Patch {
                    path: path.clone(),
                    exists: true,
                    value: clone_patch_value(after),
                });
                backward.push(Patch {
                    path: path.clone(),
                    exists: true,
                    value: clone_patch_value(before),
                });
                return;
            }
            for (i, (x, y)) in b.iter().zip(a).enumerate() {
                path.push(PathPart::Index(i as f64));
                diff_into(Some(x), Some(y), path, forward, backward);
                path.pop();
            }
        }
        (Some(Value::Object(b)), Some(Value::Object(a))) => {
            let mut keys: Vec<&JsString> = b
                .keys()
                .chain(a.keys().filter(|k| b.get_key(k).is_none()))
                .collect();
            keys.sort();
            for key in keys {
                let (x, y) = (b.get_key(key), a.get_key(key));
                path.push(PathPart::Key(key.clone()));
                match (x, y) {
                    (Some(x), Some(y)) => diff_into(Some(x), Some(y), path, forward, backward),
                    _ => {
                        forward.push(match y {
                            Some(y) => Patch::set(path.clone(), y.json_normalized()),
                            None => Patch::delete(path.clone()),
                        });
                        backward.push(match x {
                            Some(x) => Patch::set(path.clone(), x.json_normalized()),
                            None => Patch::delete(path.clone()),
                        });
                    }
                }
                path.pop();
            }
        }
        _ => {
            forward.push(Patch {
                path: path.clone(),
                exists: true,
                value: clone_patch_value(after),
            });
            backward.push(Patch {
                path: path.clone(),
                exists: true,
                value: clone_patch_value(before),
            });
        }
    }
}

/// `isPatchList(value, maxBytes)`: a non-empty array of `{path, exists}`
/// objects whose path parts are strings or non-negative integers, with at most
/// `max_bytes` UTF-16 code units of `JSON.stringify(value ?? null)` summed
/// over the patches that set a value.
pub fn is_patch_list(value: &Value, max_bytes: usize) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let mut bytes = 0usize;
    for item in items {
        let Some(map) = item.as_object() else {
            return false;
        };
        let Some(path) = map.get("path").and_then(Value::as_array) else {
            return false;
        };
        let Some(exists) = map.get("exists").and_then(Value::as_bool) else {
            return false;
        };
        // Number.isInteger(part) && part >= 0 (no upper bound; -0 passes).
        let parts_ok = path.iter().all(|part| match part {
            Value::String(_) => true,
            Value::Number(n) => n.is_finite() && n.fract() == 0.0 && *n >= 0.0,
            _ => false,
        });
        if !parts_ok {
            return false;
        }
        if exists {
            // Key order does not change the length, so the canonical text counts.
            let text = map
                .get("value")
                .map_or_else(|| "null".to_owned(), json::to_canonical_string);
            bytes += JsString::from(text).utf16_len();
            if bytes > max_bytes {
                return false;
            }
        }
    }
    true
}
