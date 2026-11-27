//! Authored media order. Trees keep object members sorted, but `assets` is a
//! list to users: the Project panel shows `Object.values(assets)`, so the order
//! must survive edits, Undo, recovery and files read by the shipped app.
//!
//! `nativeAssetOrder` maps an asset id to a sequence number. It lives in the
//! document so history and the journal carry it, one leaf per asset. A list
//! already in key order has no member. Files never store it: the writer emits
//! `assets` in this order and the reader rebuilds it from the text.
use pm_model::json::member_order;
use pm_model::ops::Op;
use pm_model::patch::{Patch, PathPart};
use pm_model::{JsString, Map, Node, NodeKind, Value};
use std::collections::{BTreeMap, HashSet};

pub const FIELD: &str = "nativeAssetOrder";

/// The stored project's member from the keys a file listed, if they need one.
pub(crate) fn adopt(project: &mut Value, keys: Option<Vec<JsString>>) {
    let Some(map) = project.as_object_mut() else {
        return;
    };
    map.remove(FIELD);
    let Some(keys) = keys else { return };
    if keys.is_sorted() {
        return;
    }
    let mut order = Map::new();
    for (index, key) in keys.into_iter().enumerate() {
        order.insert(key, Value::from(index as f64));
    }
    map.insert(FIELD, Value::Object(order));
}

/// [`adopt`] for an envelope or bare project parsed from `text`.
pub(crate) fn adopt_text(document: &mut Value, text: &str) {
    let bytes = text.as_bytes();
    let keys = member_order(bytes, &["proj", "assets"]);
    match document.as_object_mut().and_then(|d| d.get_mut("proj")) {
        Some(project) if project.is_truthy() => adopt(project, keys),
        _ => adopt(document, member_order(bytes, &["assets"])),
    }
}

fn numbered<'a>(
    order: impl Iterator<Item = (&'a JsString, Option<f64>)>,
) -> BTreeMap<&'a JsString, f64> {
    order.filter_map(|(id, n)| Some((id, n?))).collect()
}

fn sequence(root: &Node) -> BTreeMap<&JsString, f64> {
    match root.get(FIELD).map(Node::kind) {
        Some(NodeKind::Object(map)) => numbered(map.iter().map(|(id, n)| {
            (
                id,
                match n.kind() {
                    NodeKind::Number(n) => Some(*n),
                    _ => None,
                },
            )
        })),
        _ => BTreeMap::new(),
    }
}

/// Unnumbered ids first in key order, then numbered ids by number.
pub fn arrange(keys: Vec<JsString>, sequence: &dyn Fn(&JsString) -> Option<f64>) -> Vec<JsString> {
    let mut keys: Vec<_> = keys.into_iter().map(|id| (sequence(&id), id)).collect();
    keys.sort_by(|(a, x), (b, y)| {
        a.is_some()
            .cmp(&b.is_some())
            .then_with(|| a.unwrap_or(0.).total_cmp(&b.unwrap_or(0.)))
            .then_with(|| x.cmp(y))
    });
    keys.into_iter().map(|(_, id)| id).collect()
}

/// `Object.keys(assets)` of a document tree.
pub fn node_keys(root: &Node) -> Vec<JsString> {
    let Some(NodeKind::Object(assets)) = root.get("assets").map(Node::kind) else {
        return Vec::new();
    };
    let order = sequence(root);
    arrange(assets.keys().cloned().collect(), &|id| {
        order.get(id).copied()
    })
}

/// `Object.keys(assets)` of a project value.
pub fn keys(project: &Value) -> Vec<String> {
    let Some(assets) = project.get("assets").and_then(Value::as_object) else {
        return Vec::new();
    };
    let order = project.get(FIELD).and_then(Value::as_object);
    arrange(assets.keys().cloned().collect(), &|id| {
        order?.get_key(id)?.as_f64()
    })
    .into_iter()
    .map(|id| id.to_string())
    .collect()
}

/// Numbers `ids` after everything in `project`, in this order, for a batch
/// written as one whole project.
pub fn append(project: &mut Value, ids: &[String]) {
    let top = project
        .get(FIELD)
        .and_then(Value::as_object)
        .map_or(-1., |order| {
            order.values().filter_map(Value::as_f64).fold(-1., f64::max)
        });
    let Some(root) = project.as_object_mut() else {
        return;
    };
    if root.get(FIELD).and_then(Value::as_object).is_none() {
        root.insert(FIELD, Value::Object(Map::new()));
    }
    let order = root.get_mut(FIELD).and_then(Value::as_object_mut).unwrap();
    for (offset, id) in ids.iter().enumerate() {
        order.insert(id.as_str(), Value::from(top + 1. + offset as f64));
    }
}

fn member() -> Vec<PathPart> {
    vec![PathPart::Key(FIELD.into())]
}
fn field(id: &JsString) -> Vec<PathPart> {
    vec![PathPart::Key(FIELD.into()), PathPart::Key(id.clone())]
}

/// The leaf edits that keep the order true for a change of `assets`. New ids
/// follow every number in use, in the order the operations touched them.
pub(crate) fn reconcile(old: Option<&Node>, new: Option<&Node>, forward: &[Op]) -> Vec<Op> {
    let (Some(new), old_assets) = (new, old.and_then(|o| o.get("assets"))) else {
        return Vec::new();
    };
    let new_assets = new.get("assets");
    let same = match (old_assets.map(Node::kind), new_assets.map(Node::kind)) {
        (Some(NodeKind::Object(a)), Some(NodeKind::Object(b))) => std::sync::Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    let stored = sequence(new);
    if same
        && old.and_then(|o| o.get(FIELD)).map(Node::to_value) == new.get(FIELD).map(Node::to_value)
    {
        return Vec::new();
    }
    // A whole-project write may omit the member; the numbers still apply.
    let known = match (new.get(FIELD), old) {
        (None, Some(old)) => sequence(old),
        _ => stored.clone(),
    };
    let Some(NodeKind::Object(assets)) = new_assets.map(Node::kind) else {
        return match new.get(FIELD) {
            Some(_) => vec![Op::Leaf(Patch::delete(member()))],
            None => Vec::new(),
        };
    };
    let before: HashSet<&JsString> = match old_assets.map(Node::kind) {
        Some(NodeKind::Object(map)) => map.keys().collect(),
        _ => HashSet::new(),
    };
    let mut wanted: BTreeMap<JsString, f64> = known
        .iter()
        .filter(|(id, _)| assets.contains_key(id.as_bytes()))
        .map(|(id, n)| ((*id).clone(), *n))
        .collect();
    let mut next = wanted.values().fold(-1f64, |top, n| top.max(*n)) + 1.;
    let mut add = |id: &JsString, wanted: &mut BTreeMap<JsString, f64>| {
        if !before.contains(id) && !wanted.contains_key(id) && assets.contains_key(id.as_bytes()) {
            wanted.insert(id.clone(), next);
            next += 1.;
        }
    };
    for op in forward {
        if let [first, PathPart::Key(id), ..] = op.path()
            && first.property_key().as_str() == Some("assets")
        {
            add(id, &mut wanted);
        }
    }
    for id in assets.keys() {
        add(id, &mut wanted);
    }
    // The form a reader rebuilds: nothing for a sorted list, else every id.
    let list = arrange(assets.keys().cloned().collect(), &|id| {
        wanted.get(id).copied()
    });
    if list.is_sorted() {
        wanted.clear();
    } else if wanted.len() != list.len() {
        wanted = list
            .into_iter()
            .enumerate()
            .map(|(index, id)| (id, index as f64))
            .collect();
    }
    if stored.len() == wanted.len()
        && stored
            .iter()
            .all(|(id, n)| wanted.get(*id).is_some_and(|w| w == n))
    {
        return Vec::new();
    }
    let mut ops = Vec::new();
    if new.get(FIELD).is_none() {
        ops.push(Op::Leaf(Patch::set(member(), Value::Object(Map::new()))));
    }
    for id in stored.keys().filter(|id| !wanted.contains_key(**id)) {
        ops.push(Op::Leaf(Patch::delete(field(id))));
    }
    for (id, n) in &wanted {
        if stored.get(id) != Some(n) {
            ops.push(Op::Leaf(Patch::set(field(id), Value::from(*n))));
        }
    }
    if wanted.is_empty() {
        return match new.get(FIELD) {
            Some(_) => vec![Op::Leaf(Patch::delete(member()))],
            None => Vec::new(),
        };
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    fn root(text: &str) -> Node {
        Node::parse_bytes(text.as_bytes(), 1).unwrap()
    }
    #[test]
    fn numbered_ids_follow_unnumbered_ones_and_a_sorted_project_stays_clean() {
        let value =
            parse(r#"{"assets":{"a":1,"b":1,"c":1,"d":1},"nativeAssetOrder":{"a":5,"c":2}}"#)
                .unwrap();
        assert_eq!(keys(&value), ["b", "d", "c", "a"]);
        let mut sorted = parse(r#"{"assets":{"a":1,"b":1}}"#).unwrap();
        adopt(&mut sorted, Some(vec!["a".into(), "b".into()]));
        assert!(sorted.get(FIELD).is_none());
        let mut authored = parse(r#"{"assets":{"a":1,"b":1},"nativeAssetOrder":{"x":1}}"#).unwrap();
        adopt(&mut authored, Some(vec!["b".into(), "a".into()]));
        assert_eq!(keys(&authored), ["b", "a"]);
    }
    #[test]
    fn a_new_id_lands_last_and_a_removed_id_leaves_no_number() {
        let old = root(r#"{"assets":{"m":1,"z":1},"nativeAssetOrder":{"z":0,"m":1}}"#);
        let new = root(r#"{"assets":{"a":1,"m":1},"nativeAssetOrder":{"z":0,"m":1}}"#);
        let ops = reconcile(Some(&old), Some(&new), &[]);
        assert_eq!(ops.len(), 2);
        let mut value = new.to_value();
        for op in &ops {
            let Op::Leaf(patch) = op else { panic!() };
            let id = patch.path[1].property_key();
            let order = value.as_object_mut().unwrap().get_mut(FIELD).unwrap();
            let order = order.as_object_mut().unwrap();
            match &patch.value {
                Some(v) => {
                    order.insert(id, v.clone());
                }
                None => {
                    order.remove_key(&id);
                }
            }
        }
        assert_eq!(keys(&value), ["m", "a"]);
    }
}
