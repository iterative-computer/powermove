//! The revisioned document: a JSON tree whose nodes remember the revision at
//! which their content last changed, kept current by leaf patches and id-keyed
//! reconciliation (section 6, "The patch stream").
//!
//! # Revisions
//!
//! A [`Document`] has a revision. Every node carries [`Node::rev`], the
//! document revision at which the node's subtree last changed (or the node was
//! created). Applying a batch at revision `r` stamps `r` on every node whose
//! JSON-visible content changed and on all of its ancestors; untouched subtrees
//! keep their old revisions. Evaluation caches key on node revisions: a layer
//! whose node revision did not move has not changed.
//!
//! # Id-keyed reconciliation
//!
//! Whenever a patch replaces a subtree (a whole `layers` array after a reorder,
//! insert, or delete; a whole layer; the whole document on resync), the new
//! value is reconciled against the old nodes instead of rebuilt:
//!
//! * objects: members with the same key are reconciled recursively; missing
//!   members are removed and new members created;
//! * arrays: an element that is an object with a string `id` is matched to the
//!   first unmatched old element with the same `id`, wherever it was; an
//!   element without an `id` is matched to the old element at the same index
//!   when that one has no `id` either; anything unmatched is created;
//! * scalars: equal values (numbers compared by bit pattern, so `-0` differs
//!   from `0`) keep their node and revision.
//!
//! An array's own revision moves when its length, its order (a matched element
//! came from a different index), or any element changed. Reordering layers
//! therefore stamps the `layers` array and its ancestors but none of the
//! layers.
//!
//! The JSON-visible result is exactly what `applyPatch` produces; only the
//! revisions depend on reconciliation.

use std::collections::{BTreeMap, VecDeque};

use crate::hash::{canonical_digest, canonical_hash};
use crate::json::write::{JsonTree, View};
use crate::json::{JsString, Map, Value};
use crate::patch::{self, NodeMut, Patch, PatchError, PathPart, Tree};

/// One node of the revisioned tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    rev: u64,
    kind: NodeKind,
}

/// The JSON content of a [`Node`].
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Null,
    Bool(bool),
    Number(f64),
    String(JsString),
    Array(Vec<Node>),
    Object(BTreeMap<JsString, Node>),
}

impl Node {
    /// A tree holding `value`, every node at revision `rev`.
    pub fn from_value(value: Value, rev: u64) -> Node {
        let kind = match value {
            Value::Null => NodeKind::Null,
            Value::Bool(b) => NodeKind::Bool(b),
            Value::Number(n) => NodeKind::Number(n),
            Value::String(s) => NodeKind::String(s),
            Value::Array(items) => NodeKind::Array(
                items
                    .into_iter()
                    .map(|v| Node::from_value(v, rev))
                    .collect(),
            ),
            Value::Object(map) => NodeKind::Object(
                map.into_iter()
                    .map(|(k, v)| (k, Node::from_value(v, rev)))
                    .collect(),
            ),
        };
        Node { rev, kind }
    }

    /// The revision at which this subtree last changed.
    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Whether this subtree changed after revision `rev`.
    pub fn changed_since(&self, rev: u64) -> bool {
        self.rev > rev
    }

    /// The subtree as a plain JSON value.
    pub fn to_value(&self) -> Value {
        match &self.kind {
            NodeKind::Null => Value::Null,
            NodeKind::Bool(b) => Value::Bool(*b),
            NodeKind::Number(n) => Value::Number(*n),
            NodeKind::String(s) => Value::String(s.clone()),
            NodeKind::Array(items) => Value::Array(items.iter().map(Node::to_value).collect()),
            NodeKind::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), v.to_value()))
                    .collect::<Map>(),
            ),
        }
    }

    /// Object member.
    pub fn get(&self, key: &str) -> Option<&Node> {
        match &self.kind {
            NodeKind::Object(map) => map.get(key.as_bytes()),
            _ => None,
        }
    }

    /// Array element.
    pub fn at(&self, index: usize) -> Option<&Node> {
        match &self.kind {
            NodeKind::Array(items) => items.get(index),
            _ => None,
        }
    }

    /// Follows a path the way patches address nodes: object members by
    /// property key, array elements by canonical index keys.
    pub fn lookup(&self, path: &[PathPart]) -> Option<&Node> {
        let mut node = self;
        for part in path {
            let key = part.property_key();
            node = match &node.kind {
                NodeKind::Object(map) => map.get(&key)?,
                NodeKind::Array(items) => {
                    items.get(crate::json::array_index(key.as_str()?)? as usize)?
                }
                _ => return None,
            };
        }
        Some(node)
    }

    /// The string `id` member of an object node.
    fn record_id(&self) -> Option<&JsString> {
        match &self.kind {
            NodeKind::Object(map) => match map.get("id".as_bytes()).map(|n| &n.kind) {
                Some(NodeKind::String(s)) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    /// Reconciles this node with `value` (see the module documentation).
    /// Returns whether the JSON-visible content changed; if it did, this node
    /// is stamped with `rev`.
    fn reconcile(&mut self, value: Value, rev: u64) -> bool {
        let changed = match (&mut self.kind, value) {
            (NodeKind::Null, Value::Null) => false,
            (NodeKind::Bool(a), Value::Bool(b)) => {
                let changed = *a != b;
                *a = b;
                changed
            }
            (NodeKind::Number(a), Value::Number(b)) => {
                let changed = a.to_bits() != b.to_bits();
                *a = b;
                changed
            }
            (NodeKind::String(a), Value::String(b)) => {
                let changed = *a != b;
                *a = b;
                changed
            }
            (NodeKind::Object(old), Value::Object(new)) => reconcile_object(old, new, rev),
            (NodeKind::Array(old), Value::Array(new)) => reconcile_array(old, new, rev),
            (_, value) => {
                *self = Node::from_value(value, rev);
                return true;
            }
        };
        if changed {
            self.rev = rev;
        }
        changed
    }
}

fn reconcile_object(old: &mut BTreeMap<JsString, Node>, new: Map, rev: u64) -> bool {
    let new = new.into_btree();
    let before = old.len();
    old.retain(|key, _| new.contains_key(key));
    let mut changed = old.len() != before;
    for (key, value) in new {
        match old.get_mut(&key) {
            Some(node) => changed |= node.reconcile(value, rev),
            None => {
                old.insert(key, Node::from_value(value, rev));
                changed = true;
            }
        }
    }
    changed
}

fn value_record_id(value: &Value) -> Option<&JsString> {
    match value.as_object()?.get("id") {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

fn reconcile_array(old: &mut Vec<Node>, new: Vec<Value>, rev: u64) -> bool {
    let mut changed = old.len() != new.len();
    let mut pool: Vec<Option<Node>> = std::mem::take(old).into_iter().map(Some).collect();
    // Old element indices by id, in document order (duplicates queue up).
    let mut by_id: BTreeMap<JsString, VecDeque<usize>> = BTreeMap::new();
    for (index, node) in pool.iter().enumerate() {
        if let Some(id) = node.as_ref().and_then(Node::record_id) {
            by_id.entry(id.clone()).or_default().push_back(index);
        }
    }
    let mut out = Vec::with_capacity(new.len());
    for (position, value) in new.into_iter().enumerate() {
        let source = match value_record_id(&value) {
            Some(id) => by_id.get_mut(id).and_then(VecDeque::pop_front),
            None => pool
                .get(position)
                .and_then(Option::as_ref)
                .filter(|node| node.record_id().is_none())
                .map(|_| position),
        };
        match source.and_then(|index| pool[index].take().map(|node| (index, node))) {
            Some((index, mut node)) => {
                changed |= index != position;
                changed |= node.reconcile(value, rev);
                out.push(node);
            }
            None => {
                out.push(Node::from_value(value, rev));
                changed = true;
            }
        }
    }
    *old = out;
    changed
}

impl Tree for Node {
    fn node_mut(&mut self) -> NodeMut<'_, Node> {
        match &mut self.kind {
            NodeKind::Null => NodeMut::Null,
            NodeKind::Bool(b) => NodeMut::Bool(*b),
            NodeKind::Number(n) => NodeMut::Number(*n),
            NodeKind::String(s) => NodeMut::String(s),
            NodeKind::Array(items) => NodeMut::Array(items),
            NodeKind::Object(map) => NodeMut::Object(map),
        }
    }

    fn create(value: Value, rev: u64) -> Node {
        Node::from_value(value, rev)
    }

    fn assign(&mut self, value: Value, rev: u64) -> bool {
        self.reconcile(value, rev)
    }

    fn touch(&mut self, rev: u64) {
        self.rev = rev;
    }
}

impl JsonTree for Node {
    type Child = Node;
    fn view(&self) -> View<'_, Node> {
        match &self.kind {
            NodeKind::Null => View::Null,
            NodeKind::Bool(b) => View::Bool(*b),
            NodeKind::Number(n) => View::Number(*n),
            NodeKind::String(s) => View::String(s),
            NodeKind::Array(items) => View::Array(items),
            NodeKind::Object(map) => View::Object(map),
        }
    }
}

/// What a batch did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// The document revision after the batch.
    pub revision: u64,
    /// Whether the JSON-visible document changed.
    pub changed: bool,
}

/// The engine's mirror of the document: an optional root (an `undefined`
/// document is `None`) and its revision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    root: Option<Node>,
    revision: u64,
}

impl Document {
    /// A document at revision `revision` holding `root`.
    pub fn new(root: Option<Value>, revision: u64) -> Document {
        Document {
            root: root.map(|v| Node::from_value(v, revision)),
            revision,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn root(&self) -> Option<&Node> {
        self.root.as_ref()
    }

    /// The document as plain JSON.
    pub fn to_value(&self) -> Option<Value> {
        self.root.as_ref().map(Node::to_value)
    }

    /// The node at `path`, if any.
    pub fn lookup(&self, path: &[PathPart]) -> Option<&Node> {
        self.root.as_ref()?.lookup(path)
    }

    /// Applies a batch as the next revision.
    pub fn apply(&mut self, patches: &[Patch]) -> Result<ApplyOutcome, PatchError> {
        self.apply_at(patches, self.revision + 1)
    }

    /// Applies a batch as revision `revision` (which must exceed the current
    /// one). As in JavaScript, a throwing patch leaves the earlier patches of
    /// the batch applied; the document then holds revision `revision` and the
    /// caller should resynchronize.
    ///
    /// # Panics
    ///
    /// Panics if `revision` does not exceed the current revision.
    pub fn apply_at(
        &mut self,
        patches: &[Patch],
        revision: u64,
    ) -> Result<ApplyOutcome, PatchError> {
        assert!(
            revision > self.revision,
            "revision {revision} must exceed {}",
            self.revision
        );
        self.revision = revision;
        let mut changed = false;
        for (index, patch) in patches.iter().enumerate() {
            changed |= patch::apply_one(&mut self.root, patch, index, revision)?;
        }
        Ok(ApplyOutcome { revision, changed })
    }

    /// Replaces the whole document (a resync), reconciling against the current
    /// tree so unchanged subtrees keep their revisions.
    pub fn resync(&mut self, root: Option<Value>, revision: u64) -> ApplyOutcome {
        assert!(
            revision > self.revision,
            "revision {revision} must exceed {}",
            self.revision
        );
        self.revision = revision;
        let changed = match (self.root.as_mut(), root) {
            (Some(node), Some(value)) => node.reconcile(value, revision),
            (None, Some(value)) => {
                self.root = Some(Node::from_value(value, revision));
                true
            }
            (node, None) => {
                let had = node.is_some();
                self.root = None;
                had
            }
        };
        ApplyOutcome { revision, changed }
    }

    /// The canonical hash (section 6, item 7). An `undefined` document has no
    /// hash.
    pub fn canonical_hash(&self) -> Option<String> {
        self.root.as_ref().map(canonical_hash)
    }

    /// The canonical digest as bytes.
    pub fn canonical_digest(&self) -> Option<[u8; 32]> {
        self.root.as_ref().map(canonical_digest)
    }
}
