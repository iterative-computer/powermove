//! Project-owned metadata is durable independently of composition Undo/revision.
use pm_model::{Document, JsString, Map, Node, NodeKind, Op, Value, patch::Patch};

pub const ALL_FIELDS: &[&str] = &[
    "memory",
    "memoryEnabled",
    "nativeMemoryArchive",
    "nativeMediaVersions",
];
pub const FIELDS: &[&str] = &["memory", "memoryEnabled", "nativeMemoryArchive"];
#[derive(Clone, Debug)]
pub enum MemoryCall {
    Mutate {
        /// Exact observed active notes, distinguishing missing from null.
        expected: Option<Value>,
        operation: pm_model::memory::Operation,
        now: f64,
        id: JsString,
    },
    SetEnabled {
        expected: Option<Value>,
        enabled: bool,
    },
}
impl From<MemoryCall> for crate::Call {
    fn from(value: MemoryCall) -> Self {
        Self::Memory(value)
    }
}
pub(crate) fn fields(root: &Node) -> Value {
    let mut fields = Map::new();
    for key in FIELDS {
        if let Some(value) = root.get(key) {
            fields.insert(*key, value.to_value());
        }
    }
    Value::Object(fields)
}
pub(crate) fn patches_for(root: &Node, next: &Value, keys: &[&str]) -> Vec<Op> {
    keys.iter()
        .filter_map(|key| {
            let before = root.get(key).map(Node::to_value);
            let after = next.get(key);
            if match (before.as_ref(), after) {
                (Some(a), Some(b)) => a.identical(b),
                (None, None) => true,
                _ => false,
            } {
                None
            } else {
                Some(Op::Leaf(match after {
                    Some(value) => Patch::set(vec![(*key).into()], value.clone()),
                    None => Patch::delete(vec![(*key).into()]),
                }))
            }
        })
        .collect()
}
pub(crate) fn apply(document: &Document, ops: &[Op]) -> Result<Document, String> {
    let revision = document.revision();
    let mut scratch = document.clone();
    let staging_revision = revision
        .checked_add(1)
        .ok_or("Metadata revision overflow")?;
    scratch
        .apply_ops_at(ops, staging_revision)
        .map_err(|e| e.to_string())?;
    Ok(Document::reconciled_node(
        document,
        scratch.root().cloned(),
        revision,
    ))
}
/// History/root restores keep the latest notes, including unknown archive data.
pub(crate) fn preserve(document: &Document, current: &Node) -> Result<Option<Document>, String> {
    let Some(root) = document.root() else {
        return Ok(None);
    };
    let mut values = Map::new();
    for key in ALL_FIELDS {
        if let Some(value) = current.get(key) {
            values.insert(*key, value.to_value());
        }
    }
    let ops = patches_for(root, &Value::Object(values), ALL_FIELDS);
    if ops.is_empty() {
        Ok(None)
    } else {
        apply(document, &ops).map(Some)
    }
}
pub(crate) fn preserve_value(value: &mut Value, current: &Node) {
    if let Some(value) = value.as_object_mut() {
        for key in ALL_FIELDS {
            if let Some(node) = current.get(key) {
                value.insert(*key, node.to_value());
            } else {
                value.remove(key);
            }
        }
    }
}
/// A metadata change cannot make a composition-wide cancellation probe stale.
pub(crate) fn same_composition(
    a: Option<&Node>,
    b: Option<&Node>,
    equal: impl Fn(&Node, &Node) -> bool,
) -> bool {
    match (a.map(Node::kind), b.map(Node::kind)) {
        (Some(NodeKind::Object(a)), Some(NodeKind::Object(b))) => {
            let owned = |key: &JsString| key.as_str().is_some_and(|key| ALL_FIELDS.contains(&key));
            a.iter()
                .filter(|(key, _)| !owned(key))
                .all(|(key, a)| b.get(key).is_some_and(|b| equal(a, b)))
                && b.keys()
                    .filter(|key| !owned(key))
                    .all(|key| a.contains_key(key))
        }
        (None, None) => true,
        _ => match (a, b) {
            (Some(a), Some(b)) => equal(a, b),
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_preserves_zero_composition_revision_and_shared_values() {
        let document = Document::new(
            Some(pm_model::json::parse(r#"{"id":"P","layers":[],"future":"\ud800"}"#).unwrap()),
            0,
        );
        let op = Op::Leaf(Patch::set(vec!["memoryEnabled".into()], Value::Bool(false)));
        let next = apply(&document, &[op]).unwrap();
        assert_eq!(next.revision(), 0);
        assert_eq!(
            next.root().unwrap().get("future"),
            document.root().unwrap().get("future")
        );
        assert!(document.root().unwrap().get("memoryEnabled").is_none());
    }
}
