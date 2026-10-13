//! Id-keyed structural operations (docs/rust-engine.md, section 26,
//! amendment 3; contract 5, `docs/engine/rfcs/05-patches.md`).
//!
//! `diffPatches` replaces an array whole when its length or its `id` order
//! changes, so reordering, inserting, or deleting one layer of an 884-layer
//! composition sends (and re-serializes) the whole `layers` array: S5 measured
//! `JSON.stringify` of that array alone at 2.1 ms, over the 2 ms structural
//! budget. The operations here send only what moved:
//!
//! | Operation | Effect on the array at `path` |
//! |---|---|
//! | [`Op::Order`] `{ ids }` | Permutes the elements into the order of `ids` |
//! | [`Op::Insert`] `{ index, value }` | `splice(index, 0, value)`; `value` is an object with a new string `id` |
//! | [`Op::Delete`] `{ id }` | Removes the element whose `id` is `id` |
//! | [`Op::Leaf`] | A `shared/patch.ts` leaf patch, with `applyPatch` semantics |
//!
//! An id-keyed operation requires the array at `path` to be **id-keyed**:
//! every element an object with a string `id`, no two alike. `Order` names
//! every element exactly once; `Insert` adds an id the array does not have;
//! `Delete` names an id it has. Anything else (a missing path, a non-array,
//! an element without an id, an unknown or duplicate id, an index past the
//! end) fails with [`PatchErrorKind::Mismatch`]: the mirror has diverged from
//! the renderer and resynchronizes. As with leaf patches, a failing operation
//! leaves the earlier operations of its batch applied.
//!
//! Revisions: on the revisioned [`crate::Document`], `Order` keeps every
//! element's node (and revision) and stamps the array only when the order
//! actually changes; `Insert` creates one node; `Delete` drops one. The array
//! and its ancestors are stamped whenever it changed. The tree that results is
//! identical, revisions included, to the one id-keyed reconciliation builds
//! from the equivalent whole-array leaf patch (tested).
//!
//! # The generator
//!
//! [`diff_ops`] is `diffPatches` with id-keyed arrays: where `diffPatches`
//! would replace an array whole, and both the old and the new array are
//! id-keyed and share at least one id, it emits, in this order:
//!
//! 1. a `Delete` for each old id the new array lacks, in old order;
//! 2. an `Order` of the surviving ids, when their relative order changed;
//! 3. an `Insert` for each new id, in ascending final index;
//! 4. the ops that turn each surviving element into its new value, addressed
//!    by its final index (recursively, so nested id-keyed arrays get the same
//!    treatment).
//!
//! Arrays that are not id-keyed on both sides, or share no id, fall back to
//! the leaf patch `diffPatches` emits. Object members are visited in UTF-16
//! code unit order (the canonical key order) rather than insertion order, so
//! the generated list is canonical: the same edit always produces the same
//! operations. `apps/desktop/src/shared/patch-ids.ts` is the same generator in
//! TypeScript; the cross-language vectors check that both emit identical
//! lists.

use std::collections::HashMap;

use crate::json::{self, JsString, Map, Value};
use crate::patch::{
    self, MAX_ARRAY_LENGTH, NodeMut, Patch, PatchDecodeError, PatchError, PatchErrorKind, PathPart,
    Tree, clone_patch_value, object_is, record_id,
};

/// One operation of a patch stream: a leaf patch or an id-keyed structural
/// operation (see the module documentation).
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A leaf patch, applied with `applyPatch` semantics.
    Leaf(Patch),
    /// Permute the id-keyed array at `path` into the order of `ids`.
    Order {
        path: Vec<PathPart>,
        ids: Vec<JsString>,
    },
    /// Insert `value` (an object with a string `id` the array lacks) at
    /// `index` of the id-keyed array at `path`.
    Insert {
        path: Vec<PathPart>,
        index: u32,
        value: Value,
    },
    /// Remove the element with `id` from the id-keyed array at `path`.
    Delete { path: Vec<PathPart>, id: JsString },
}

impl From<Patch> for Op {
    fn from(patch: Patch) -> Op {
        Op::Leaf(patch)
    }
}

impl Op {
    /// The path the operation addresses.
    pub fn path(&self) -> &[PathPart] {
        match self {
            Op::Leaf(patch) => &patch.path,
            Op::Order { path, .. } | Op::Insert { path, .. } | Op::Delete { path, .. } => path,
        }
    }

    /// Decodes the JSON form: a `shared/patch.ts` patch object, or an object
    /// with `op` (`"order"`, `"insert"`, `"delete"`) and `path`, plus `ids`,
    /// `index` and `value`, or `id`.
    pub fn from_value(value: &Value) -> Result<Op, PatchDecodeError> {
        let map = value
            .as_object()
            .ok_or(PatchDecodeError("an operation must be an object"))?;
        let Some(kind) = map.get("op") else {
            return Patch::from_value(value).map(Op::Leaf);
        };
        let path = map
            .get("path")
            .and_then(Value::as_array)
            .ok_or(PatchDecodeError("an operation needs a path array"))?
            .iter()
            .map(|part| match part {
                Value::String(s) => Ok(PathPart::Key(s.clone())),
                Value::Number(n) => Ok(PathPart::Index(*n)),
                _ => Err(PatchDecodeError("path parts must be strings or numbers")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let string = |v: Option<&Value>, what: &'static str| {
            v.and_then(Value::as_js_string)
                .cloned()
                .ok_or(PatchDecodeError(what))
        };
        match kind.as_str() {
            Some("order") => {
                let ids = map
                    .get("ids")
                    .and_then(Value::as_array)
                    .ok_or(PatchDecodeError("order needs an ids array"))?
                    .iter()
                    .map(|id| string(Some(id), "order ids must be strings"))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Op::Order { path, ids })
            }
            Some("insert") => {
                let index = map
                    .get("index")
                    .and_then(Value::as_f64)
                    .filter(|n| *n >= 0.0 && n.fract() == 0.0 && *n <= u32::MAX as f64)
                    .ok_or(PatchDecodeError("insert needs a 32-bit index"))?;
                let value = map
                    .get("value")
                    .cloned()
                    .ok_or(PatchDecodeError("insert needs a value"))?;
                Ok(Op::Insert {
                    path,
                    index: index as u32,
                    value,
                })
            }
            Some("delete") => Ok(Op::Delete {
                path,
                id: string(map.get("id"), "delete needs a string id")?,
            }),
            _ => Err(PatchDecodeError("unknown operation")),
        }
    }

    /// The JSON form (see [`Op::from_value`]).
    pub fn to_value(&self) -> Value {
        let (kind, path) = match self {
            Op::Leaf(patch) => return patch.to_value(),
            Op::Order { path, .. } => ("order", path),
            Op::Insert { path, .. } => ("insert", path),
            Op::Delete { path, .. } => ("delete", path),
        };
        let mut map = Map::new();
        map.insert("op", Value::String(kind.into()));
        map.insert(
            "path",
            Value::Array(path.iter().map(PathPart::to_value).collect()),
        );
        match self {
            Op::Order { ids, .. } => {
                map.insert(
                    "ids",
                    Value::Array(ids.iter().cloned().map(Value::String).collect()),
                );
            }
            Op::Insert { index, value, .. } => {
                map.insert("index", Value::Number(*index as f64));
                map.insert("value", value.clone());
            }
            Op::Delete { id, .. } => {
                map.insert("id", Value::String(id.clone()));
            }
            Op::Leaf(_) => unreachable!(),
        }
        Value::Object(map)
    }
}

/// Decodes a JSON list of operations.
pub fn ops_from_value(value: &Value) -> Result<Vec<Op>, PatchDecodeError> {
    value
        .as_array()
        .ok_or(PatchDecodeError("operations must be an array"))?
        .iter()
        .map(Op::from_value)
        .collect()
}

/// Encodes operations in their JSON form.
pub fn ops_to_value(ops: &[Op]) -> Value {
    Value::Array(ops.iter().map(Op::to_value).collect())
}

// ── Applying ────────────────────────────────────────────────────────────────

/// Applies operations in place to a plain document (`None` is `undefined`).
/// Returns whether the JSON-visible document changed.
pub fn apply_ops(root: &mut Option<Value>, ops: &[Op]) -> Result<bool, PatchError> {
    let mut changed = false;
    for (index, op) in ops.iter().enumerate() {
        changed |= apply_op(root, op, index, 0)?;
    }
    Ok(changed)
}

fn mismatch(index: usize, message: impl Into<String>) -> PatchError {
    PatchError {
        index,
        kind: PatchErrorKind::Mismatch,
        message: message.into(),
    }
}

/// One operation on any tree. Nodes that change are stamped with `rev`.
pub(crate) fn apply_op<T: Tree>(
    root: &mut Option<T>,
    op: &Op,
    index: usize,
    rev: u64,
) -> Result<bool, PatchError> {
    if let Op::Leaf(patch) = op {
        return patch::apply_one(root, patch, index, rev);
    }
    let Some(node) = root.as_mut() else {
        return Err(mismatch(index, "the document is undefined"));
    };
    structural(node, op.path(), op, index, rev)
}

fn structural<T: Tree>(
    node: &mut T,
    path: &[PathPart],
    op: &Op,
    index: usize,
    rev: u64,
) -> Result<bool, PatchError> {
    let changed = match path.split_first() {
        Some((first, rest)) => {
            let key = first.property_key();
            let child = match node.node_mut() {
                NodeMut::Object(map) => map.get_mut(&key),
                NodeMut::Array(items) => key
                    .as_str()
                    .and_then(json::array_index)
                    .and_then(|i| items.get_mut(i as usize)),
                _ => None,
            };
            let Some(child) = child else {
                return Err(mismatch(
                    index,
                    format!("no member '{}' on the path", key.to_string_lossy()),
                ));
            };
            structural(child, rest, op, index, rev)?
        }
        None => match node.node_mut() {
            NodeMut::Array(items) => apply_to_array(items, op, index, rev)?,
            _ => return Err(mismatch(index, "the path does not address an array")),
        },
    };
    if changed {
        node.touch(rev);
    }
    Ok(changed)
}

/// The ids of an id-keyed array, or a mismatch.
fn element_id<T: Tree>(item: &T, index: usize) -> Result<&JsString, PatchError> {
    item.record_id()
        .ok_or_else(|| mismatch(index, "an element of the array has no string id"))
}

fn apply_to_array<T: Tree>(
    items: &mut Vec<T>,
    op: &Op,
    index: usize,
    rev: u64,
) -> Result<bool, PatchError> {
    match op {
        Op::Order { ids, .. } => {
            if ids.len() != items.len() {
                return Err(mismatch(
                    index,
                    format!("order names {} ids for {} elements", ids.len(), items.len()),
                ));
            }
            let mut position: HashMap<&JsString, usize> = HashMap::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                if position.insert(element_id(item, index)?, i).is_some() {
                    return Err(mismatch(index, "the array has duplicate ids"));
                }
            }
            let mut source = Vec::with_capacity(ids.len());
            for id in ids {
                // Removing claims the id, so a repeated id is unknown the second time.
                match position.remove(id) {
                    Some(i) => source.push(i),
                    None => {
                        return Err(mismatch(
                            index,
                            format!(
                                "order names unknown or repeated id '{}'",
                                id.to_string_lossy()
                            ),
                        ));
                    }
                }
            }
            if source.iter().enumerate().all(|(to, from)| to == *from) {
                return Ok(false);
            }
            let mut pool: Vec<Option<T>> = std::mem::take(items).into_iter().map(Some).collect();
            items.extend(
                source
                    .into_iter()
                    .map(|from| pool[from].take().expect("each element moves once")),
            );
            Ok(true)
        }
        Op::Insert {
            index: at, value, ..
        } => {
            let at = *at as usize;
            if at > items.len() {
                return Err(mismatch(
                    index,
                    format!("insert at {at} past the end ({})", items.len()),
                ));
            }
            if items.len() >= MAX_ARRAY_LENGTH {
                return Err(patch::limit(index));
            }
            let value = clone_patch_value(Some(value)).unwrap_or(Value::Null);
            let Some(id) = record_id(&value) else {
                return Err(mismatch(index, "an inserted element needs a string id"));
            };
            for item in items.iter() {
                if element_id(item, index)? == id {
                    return Err(mismatch(
                        index,
                        format!("insert of id '{}' already present", id.to_string_lossy()),
                    ));
                }
            }
            items.insert(at, T::create(value, rev));
            Ok(true)
        }
        Op::Delete { id, .. } => {
            let mut found = None;
            for (i, item) in items.iter().enumerate() {
                if element_id(item, index)? == id {
                    if found.is_some() {
                        return Err(mismatch(index, "the array has duplicate ids"));
                    }
                    found = Some(i);
                }
            }
            match found {
                Some(i) => {
                    items.remove(i);
                    Ok(true)
                }
                None => Err(mismatch(
                    index,
                    format!("delete of unknown id '{}'", id.to_string_lossy()),
                )),
            }
        }
        Op::Leaf(_) => unreachable!("leaf patches never reach an array operation"),
    }
}

// ── The generator ───────────────────────────────────────────────────────────

/// Operations turning `before` into `after` (see the module documentation).
/// `None` is `undefined`.
pub fn diff_ops(before: Option<&Value>, after: Option<&Value>) -> Vec<Op> {
    let mut out = Vec::new();
    diff_into(before, after, &mut Vec::new(), &mut out);
    out
}

/// Operations turning the array `before` at `path` into `after`: id-keyed
/// operations when both arrays are id-keyed and share an id, otherwise what
/// `diffPatches` emits for the pair.
pub fn diff_array_ops(before: &[Value], after: &[Value], path: &[PathPart]) -> Vec<Op> {
    let mut out = Vec::new();
    let mut path = path.to_vec();
    diff_arrays(before, after, &mut path, &mut out);
    out
}

/// Whether every element is an object with a string `id`, no two alike.
pub fn is_id_keyed(items: &[Value]) -> bool {
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    items
        .iter()
        .all(|item| record_id(item).is_some_and(|id| seen.insert(id)))
}

fn leaf_set(path: &[PathPart], value: Option<&Value>) -> Op {
    Op::Leaf(Patch {
        path: path.to_vec(),
        exists: true,
        value: clone_patch_value(value),
    })
}

fn diff_into(
    before: Option<&Value>,
    after: Option<&Value>,
    path: &mut Vec<PathPart>,
    out: &mut Vec<Op>,
) {
    if object_is(before, after) {
        return;
    }
    match (before, after) {
        (Some(Value::Array(b)), Some(Value::Array(a))) => diff_arrays(b, a, path, out),
        (Some(Value::Object(b)), Some(Value::Object(a))) => {
            let mut keys: Vec<&JsString> = b
                .keys()
                .chain(a.keys().filter(|k| b.get_key(k).is_none()))
                .collect();
            keys.sort_by(|x, y| x.cmp_utf16(y));
            for key in keys {
                let (x, y) = (b.get_key(key), a.get_key(key));
                path.push(PathPart::Key(key.clone()));
                match (x, y) {
                    (Some(_), Some(_)) => diff_into(x, y, path, out),
                    (_, Some(y)) => {
                        out.push(Op::Leaf(Patch::set(path.clone(), y.json_normalized())))
                    }
                    _ => out.push(Op::Leaf(Patch::delete(path.clone()))),
                }
                path.pop();
            }
        }
        _ => out.push(leaf_set(path, after)),
    }
}

fn diff_arrays(b: &[Value], a: &[Value], path: &mut Vec<PathPart>, out: &mut Vec<Op>) {
    let identity_order_changed = b.len() == a.len()
        && b.iter()
            .zip(a)
            .any(|(x, y)| matches!((record_id(x), record_id(y)), (Some(i), Some(j)) if i != j));
    if b.len() == a.len() && !identity_order_changed {
        for (i, (x, y)) in b.iter().zip(a).enumerate() {
            path.push(PathPart::Index(i as f64));
            diff_into(Some(x), Some(y), path, out);
            path.pop();
        }
        return;
    }
    if !(is_id_keyed(b) && is_id_keyed(a)) {
        out.push(Op::Leaf(Patch {
            path: path.clone(),
            exists: true,
            value: Some(Value::Array(a.to_vec()).json_normalized()),
        }));
        return;
    }
    let old: HashMap<&JsString, usize> = b
        .iter()
        .enumerate()
        .map(|(i, item)| (record_id(item).expect("id-keyed"), i))
        .collect();
    let new: HashMap<&JsString, usize> = a
        .iter()
        .enumerate()
        .map(|(i, item)| (record_id(item).expect("id-keyed"), i))
        .collect();
    if !b
        .iter()
        .any(|item| new.contains_key(record_id(item).expect("id-keyed")))
    {
        out.push(Op::Leaf(Patch {
            path: path.clone(),
            exists: true,
            value: Some(Value::Array(a.to_vec()).json_normalized()),
        }));
        return;
    }
    // 1. Deletes, in old order.
    for item in b {
        let id = record_id(item).expect("id-keyed");
        if !new.contains_key(id) {
            out.push(Op::Delete {
                path: path.clone(),
                id: id.clone(),
            });
        }
    }
    // 2. The survivors' new order, when it changed.
    let survivors_old: Vec<&JsString> = b
        .iter()
        .map(|item| record_id(item).expect("id-keyed"))
        .filter(|id| new.contains_key(id))
        .collect();
    let survivors_new: Vec<&JsString> = a
        .iter()
        .map(|item| record_id(item).expect("id-keyed"))
        .filter(|id| old.contains_key(id))
        .collect();
    if survivors_old != survivors_new {
        out.push(Op::Order {
            path: path.clone(),
            ids: survivors_new.into_iter().cloned().collect(),
        });
    }
    // 3. Inserts, in ascending final index.
    for (i, item) in a.iter().enumerate() {
        if !old.contains_key(record_id(item).expect("id-keyed")) {
            out.push(Op::Insert {
                path: path.clone(),
                index: i as u32,
                value: item.json_normalized(),
            });
        }
    }
    // 4. The survivors' content, at their final indices.
    for (i, item) in a.iter().enumerate() {
        if let Some(&j) = old.get(record_id(item).expect("id-keyed")) {
            path.push(PathPart::Index(i as f64));
            diff_into(Some(&b[j]), Some(item), path, out);
            path.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;
    use crate::path;

    fn v(text: &str) -> Value {
        parse(text).unwrap()
    }

    #[test]
    fn reorder_insert_delete() {
        let before = v(r#"{"layers":[{"id":"a","x":1},{"id":"b"},{"id":"c"}]}"#);
        let after = v(r#"{"layers":[{"id":"c"},{"id":"n","y":2},{"id":"a","x":2}]}"#);
        let ops = diff_ops(Some(&before), Some(&after));
        assert_eq!(
            ops,
            vec![
                Op::Delete {
                    path: path!["layers"].to_vec(),
                    id: "b".into()
                },
                Op::Order {
                    path: path!["layers"].to_vec(),
                    ids: vec!["c".into(), "a".into()]
                },
                Op::Insert {
                    path: path!["layers"].to_vec(),
                    index: 1,
                    value: v(r#"{"id":"n","y":2}"#)
                },
                Op::Leaf(Patch::set(path!["layers", 2, "x"], Value::Number(2.0))),
            ]
        );
        let mut root = Some(before);
        assert!(apply_ops(&mut root, &ops).unwrap());
        assert_eq!(root, Some(after));
    }

    #[test]
    fn falls_back_without_ids() {
        let before =
            v(r#"{"a":[1,2],"b":[{"id":"x"},{"name":"no id"}],"c":[{"id":"x"},{"id":"x"}]}"#);
        let after = v(r#"{"a":[2],"b":[{"name":"no id"}],"c":[{"id":"x"}]}"#);
        let ops = diff_ops(Some(&before), Some(&after));
        assert!(ops.iter().all(|op| matches!(op, Op::Leaf(_))), "{ops:?}");
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn mismatches() {
        let doc = v(r#"{"l":[{"id":"a"},{"id":"b"}],"n":[{"x":1}],"o":{}}"#);
        let cases = [
            Op::Delete {
                path: path!["l"].to_vec(),
                id: "z".into(),
            },
            Op::Delete {
                path: path!["n"].to_vec(),
                id: "a".into(),
            },
            Op::Delete {
                path: path!["o"].to_vec(),
                id: "a".into(),
            },
            Op::Delete {
                path: path!["missing"].to_vec(),
                id: "a".into(),
            },
            Op::Order {
                path: path!["l"].to_vec(),
                ids: vec!["a".into()],
            },
            Op::Order {
                path: path!["l"].to_vec(),
                ids: vec!["a".into(), "a".into()],
            },
            Op::Insert {
                path: path!["l"].to_vec(),
                index: 3,
                value: v(r#"{"id":"c"}"#),
            },
            Op::Insert {
                path: path!["l"].to_vec(),
                index: 0,
                value: v(r#"{"id":"a"}"#),
            },
            Op::Insert {
                path: path!["l"].to_vec(),
                index: 0,
                value: v(r#"{"id":1}"#),
            },
        ];
        for op in cases {
            let mut root = Some(doc.clone());
            let err = apply_ops(&mut root, std::slice::from_ref(&op)).unwrap_err();
            assert_eq!(err.kind, PatchErrorKind::Mismatch, "{op:?}");
            assert_eq!(
                root.as_ref(),
                Some(&doc),
                "{op:?} left the document unchanged"
            );
        }
        let mut undefined = None;
        let op = Op::Delete {
            path: vec![],
            id: "a".into(),
        };
        assert_eq!(
            apply_ops(&mut undefined, &[op]).unwrap_err().kind,
            PatchErrorKind::Mismatch
        );
    }

    #[test]
    fn json_form_round_trips() {
        let ops = vec![
            Op::Leaf(Patch::set(path!["a", 0], Value::Bool(true))),
            Op::Leaf(Patch::delete(path!["b"])),
            Op::Leaf(Patch {
                path: path!["c"].to_vec(),
                exists: true,
                value: None,
            }),
            Op::Order {
                path: path!["l"].to_vec(),
                ids: vec!["x".into(), "y".into()],
            },
            Op::Insert {
                path: path!["l"].to_vec(),
                index: 4,
                value: v(r#"{"id":"q"}"#),
            },
            Op::Delete {
                path: vec![],
                id: "x".into(),
            },
        ];
        let value = ops_to_value(&ops);
        assert_eq!(ops_from_value(&value).unwrap(), ops);
        assert!(Op::from_value(&v(r#"{"op":"move","path":[]}"#)).is_err());
        assert!(Op::from_value(&v(r#"{"op":"insert","path":[],"index":-1,"value":{}}"#)).is_err());
    }
}
