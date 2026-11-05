//! Project notes are durable metadata, independent of composition Undo/revision.
use pm_model::{Document, JsString, Map, Node, NodeKind, Op, Value, patch::Patch};

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
pub(crate) fn patches(root: &Node, next: &Value) -> Vec<Op> {
    FIELDS
        .iter()
        .filter_map(|key| {
            let before = root.get(key).map(Node::to_value);
            let after = next.get(key);
            if before.as_ref() == after {
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
    let mut scratch = Document::from_root(document.root().cloned(), revision.saturating_sub(1));
    scratch
        .apply_ops_at(ops, revision)
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
    let ops = patches(root, &fields(current));
    if ops.is_empty() {
        Ok(None)
    } else {
        apply(document, &ops).map(Some)
    }
}
pub(crate) fn preserve_value(value: &mut Value, current: &Node) {
    if let Some(value) = value.as_object_mut() {
        for key in FIELDS {
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
            let owned = |key: &JsString| key.as_str().is_some_and(|key| FIELDS.contains(&key));
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
