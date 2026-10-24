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
//!
//! # Shared subtrees
//!
//! Arrays and objects hold their children behind an [`Arc`], so cloning a
//! node or a [`Document`] is cheap and the copies share every subtree until
//! one of them changes it: a patch copies only the containers on its path
//! (copy on write, `Arc::make_mut`). The interactive engine keeps one mirror
//! per session, its presenter another, and every snapshot job a third at its
//! revision; for a 180 MB document (1.2 million keyframes) these used to be
//! full copies of about 1.8 GB each (docs/engine/host-processes.md, "Large
//! documents"). A tree that is never cloned (pm-wasm) never copies.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, OnceLock};

use crate::hash::canonical_digest;
use crate::json::write::{JsonTree, View};
use crate::json::{JsString, Map, ObjectMap, Value};
use crate::ops::{self, Op};
use crate::patch::{self, NodeMut, Patch, PatchError, PathPart, Tree};

/// One node of the revisioned tree.
#[derive(Debug, PartialEq)]
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
    Array(Arc<Vec<Node>>),
    Object(Arc<ObjectMap<Node>>),
}

impl Clone for Node {
    fn clone(&self) -> Self {
        // A cloned input may be consumed after the original tree is left.
        // Retain its complete dependency before its address changes.
        #[cfg(feature = "read-tracking")]
        reads::record(self);
        Self {
            rev: self.rev,
            kind: self.kind.clone(),
        }
    }
}

impl Node {
    /// Apply detached-scope updates with the document's patch semantics.
    pub fn apply_patches(&mut self, patches: &[Patch], rev: u64) -> Result<(), PatchError> {
        let mut root = Some(std::mem::replace(self, Node::from_value(Value::Null, rev)));
        let result = patches
            .iter()
            .enumerate()
            .try_for_each(|(i, p)| patch::apply_one(&mut root, p, i, rev).map(|_| ()));
        *self = root.unwrap_or_else(|| Node::from_value(Value::Null, rev));
        result
    }

    /// A tree holding `value`, every node at revision `rev`.
    pub fn from_value(value: Value, rev: u64) -> Node {
        let kind = match value {
            Value::Null => NodeKind::Null,
            Value::Bool(b) => NodeKind::Bool(b),
            Value::Number(n) => NodeKind::Number(n),
            Value::String(s) => NodeKind::String(s),
            Value::Array(items) => NodeKind::Array(Arc::new(
                items
                    .into_iter()
                    .map(|v| Node::from_value(v, rev))
                    .collect(),
            )),
            Value::Object(map) => NodeKind::Object(Arc::new(
                map.into_iter()
                    .map(|(k, v)| (k, Node::from_value(v, rev)))
                    .collect(),
            )),
        };
        Node { rev, kind }
    }

    /// Parses JSON text straight into a tree at revision `rev`: the same
    /// tree as `Node::from_value(parse_bytes(bytes)?, rev)` without building
    /// the intermediate `Value` (for a large document, its memory again).
    pub fn parse_bytes(bytes: &[u8], rev: u64) -> Result<Node, crate::json::ParseError> {
        crate::json::parse_bytes_with(bytes, &mut NodeBuilder { rev })
    }

    /// The revision at which this subtree last changed.
    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn kind(&self) -> &NodeKind {
        #[cfg(feature = "read-tracking")]
        reads::record(self);
        &self.kind
    }

    /// Whether this subtree changed after revision `rev`.
    pub fn changed_since(&self, rev: u64) -> bool {
        self.rev > rev
    }

    /// The subtree as a plain JSON value.
    pub fn to_value(&self) -> Value {
        match self.kind() {
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
        match self.kind() {
            NodeKind::Object(map) => map.get(key.as_bytes()),
            _ => None,
        }
    }

    /// Array element.
    pub fn at(&self, index: usize) -> Option<&Node> {
        match self.kind() {
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
            node = match node.kind() {
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
        match self.kind() {
            NodeKind::Object(map) => match map.get("id".as_bytes()).map(Node::kind) {
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
            // A container another tree shares is copied only when its
            // content differs: reconciling a clone against the same document
            // (a resync, a structural diff) keeps every unchanged subtree
            // shared. A container this tree owns alone is reconciled in place.
            (NodeKind::Object(old), Value::Object(new)) => {
                if Arc::get_mut(old).is_none() && same_object(old, &new) {
                    false
                } else {
                    reconcile_object(Arc::make_mut(old), new, rev)
                }
            }
            (NodeKind::Array(old), Value::Array(new)) => {
                if Arc::get_mut(old).is_none() && same_array(old, &new) {
                    false
                } else {
                    reconcile_array(Arc::make_mut(old), new, rev)
                }
            }
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

/// Whether `node` holds exactly `value` as reconciliation compares it
/// (numbers by bit pattern).
fn same(node: &Node, value: &Value) -> bool {
    match (&node.kind, value) {
        (NodeKind::Null, Value::Null) => true,
        (NodeKind::Bool(a), Value::Bool(b)) => a == b,
        (NodeKind::Number(a), Value::Number(b)) => a.to_bits() == b.to_bits(),
        (NodeKind::String(a), Value::String(b)) => a == b,
        (NodeKind::Object(a), Value::Object(b)) => same_object(a, b),
        (NodeKind::Array(a), Value::Array(b)) => same_array(a, b),
        _ => false,
    }
}

fn same_object(old: &ObjectMap<Node>, new: &Map) -> bool {
    old.len() == new.len()
        && old
            .iter()
            .zip(new.iter())
            .all(|((k, n), (key, v))| k == key && same(n, v))
}

fn same_array(old: &[Node], new: &[Value]) -> bool {
    old.len() == new.len() && old.iter().zip(new).all(|(n, v)| same(n, v))
}

/// Builds [`Node`]s while parsing (`Node::parse_bytes`).
struct NodeBuilder {
    rev: u64,
}

impl crate::json::TreeBuilder for NodeBuilder {
    type Out = Node;
    fn null(&mut self) -> Node {
        Node {
            rev: self.rev,
            kind: NodeKind::Null,
        }
    }
    fn bool(&mut self, b: bool) -> Node {
        Node {
            rev: self.rev,
            kind: NodeKind::Bool(b),
        }
    }
    fn number(&mut self, n: f64) -> Node {
        Node {
            rev: self.rev,
            kind: NodeKind::Number(n),
        }
    }
    fn string(&mut self, s: JsString) -> Node {
        Node {
            rev: self.rev,
            kind: NodeKind::String(s),
        }
    }
    fn array(&mut self, items: Vec<Node>) -> Node {
        Node {
            rev: self.rev,
            kind: NodeKind::Array(Arc::new(items)),
        }
    }
    fn object(&mut self, members: Vec<(JsString, Node)>) -> Node {
        Node {
            rev: self.rev,
            kind: NodeKind::Object(Arc::new(members.into_iter().collect())),
        }
    }
}

impl Node {
    /// [`Node::reconcile`] with a parsed tree (every node at `rev`) in place
    /// of a `Value`: identical results, revisions included.
    fn reconcile_node(&mut self, new: Node, rev: u64) -> bool {
        let changed = match (&mut self.kind, new.kind) {
            (NodeKind::Null, NodeKind::Null) => false,
            (NodeKind::Bool(a), NodeKind::Bool(b)) => {
                let changed = *a != b;
                *a = b;
                changed
            }
            (NodeKind::Number(a), NodeKind::Number(b)) => {
                let changed = a.to_bits() != b.to_bits();
                *a = b;
                changed
            }
            (NodeKind::String(a), NodeKind::String(b)) => {
                let changed = *a != b;
                *a = b;
                changed
            }
            (NodeKind::Object(old), NodeKind::Object(new)) => {
                if Arc::get_mut(old).is_none() && same_nodes_object(old, &new) {
                    false
                } else {
                    let new = Arc::try_unwrap(new).unwrap_or_else(|shared| (*shared).clone());
                    reconcile_object_nodes(Arc::make_mut(old), new, rev)
                }
            }
            (NodeKind::Array(old), NodeKind::Array(new)) => {
                if Arc::get_mut(old).is_none() && same_nodes_array(old, &new) {
                    false
                } else {
                    let new = Arc::try_unwrap(new).unwrap_or_else(|shared| (*shared).clone());
                    reconcile_array_nodes(Arc::make_mut(old), new, rev)
                }
            }
            (_, kind) => {
                *self = Node { rev, kind };
                return true;
            }
        };
        if changed {
            self.rev = rev;
        }
        changed
    }
}

/// Whether two trees hold the same JSON (numbers by bit pattern).
fn same_nodes(a: &Node, b: &Node) -> bool {
    match (&a.kind, &b.kind) {
        (NodeKind::Null, NodeKind::Null) => true,
        (NodeKind::Bool(x), NodeKind::Bool(y)) => x == y,
        (NodeKind::Number(x), NodeKind::Number(y)) => x.to_bits() == y.to_bits(),
        (NodeKind::String(x), NodeKind::String(y)) => x == y,
        (NodeKind::Object(x), NodeKind::Object(y)) => Arc::ptr_eq(x, y) || same_nodes_object(x, y),
        (NodeKind::Array(x), NodeKind::Array(y)) => Arc::ptr_eq(x, y) || same_nodes_array(x, y),
        _ => false,
    }
}

fn same_nodes_object(old: &ObjectMap<Node>, new: &ObjectMap<Node>) -> bool {
    old.len() == new.len()
        && old
            .iter()
            .zip(new.iter())
            .all(|((k, n), (key, v))| k == key && same_nodes(n, v))
}

fn same_nodes_array(old: &[Node], new: &[Node]) -> bool {
    old.len() == new.len() && old.iter().zip(new).all(|(n, v)| same_nodes(n, v))
}

fn reconcile_object_nodes(old: &mut ObjectMap<Node>, new: ObjectMap<Node>, rev: u64) -> bool {
    let before = old.len();
    old.retain(|key, _| new.contains_key(key.as_bytes()));
    let mut changed = old.len() != before;
    for (key, value) in new {
        match old.get_mut(key.as_bytes()) {
            Some(node) => changed |= node.reconcile_node(value, rev),
            None => {
                old.insert(key, value);
                changed = true;
            }
        }
    }
    changed
}

fn reconcile_array_nodes(old: &mut Vec<Node>, new: Vec<Node>, rev: u64) -> bool {
    let mut changed = old.len() != new.len();
    let mut pool: Vec<Option<Node>> = std::mem::take(old).into_iter().map(Some).collect();
    let mut by_id: BTreeMap<JsString, VecDeque<usize>> = BTreeMap::new();
    for (index, node) in pool.iter().enumerate() {
        if let Some(id) = node.as_ref().and_then(Node::record_id) {
            by_id.entry(id.clone()).or_default().push_back(index);
        }
    }
    let mut out = Vec::with_capacity(new.len());
    for (position, value) in new.into_iter().enumerate() {
        let source = match value.record_id() {
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
                changed |= node.reconcile_node(value, rev);
                out.push(node);
            }
            None => {
                out.push(value);
                changed = true;
            }
        }
    }
    *old = out;
    changed
}

fn reconcile_object(old: &mut ObjectMap<Node>, new: Map, rev: u64) -> bool {
    let new = new.into_members();
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
            NodeKind::Array(items) => NodeMut::Array(Arc::make_mut(items)),
            NodeKind::Object(map) => NodeMut::Object(Arc::make_mut(map)),
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

    fn record_id(&self) -> Option<&JsString> {
        Node::record_id(self)
    }
}

impl JsonTree for Node {
    type Child = Node;
    fn view(&self) -> View<'_, Node> {
        match self.kind() {
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
///
/// # Editions
///
/// [`Document::edition`] names one document value in one state: every
/// constructor, clone, and mutation takes a new, process-unique edition. While
/// the edition is the same, no node of the tree was created, dropped, moved,
/// or changed, so a node's address names one node with one content. Caches
/// keyed by node address (a renderer's per-document memo across frames,
/// `pm_eval`) are valid exactly while the edition they were filled at holds.
/// The edition is not part of the document's value (`==` ignores it).
pub struct Document {
    root: Option<Node>,
    revision: u64,
    digest: OnceLock<Option<[u8; 32]>>,
    edition: u64,
}

/// The next process-unique [`Document::edition`].
fn next_edition() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static EDITIONS: AtomicU64 = AtomicU64::new(1);
    EDITIONS.fetch_add(1, Ordering::Relaxed)
}

impl Clone for Document {
    /// A copy is other nodes: it takes its own edition.
    fn clone(&self) -> Document {
        Document {
            root: self.root.clone(),
            revision: self.revision,
            // The same content: its canonical digest holds for the copy.
            digest: self.digest.clone(),
            edition: next_edition(),
        }
    }
}

impl Default for Document {
    fn default() -> Document {
        Document {
            root: None,
            revision: 0,
            digest: OnceLock::new(),
            edition: next_edition(),
        }
    }
}

impl PartialEq for Document {
    fn eq(&self, other: &Document) -> bool {
        self.revision == other.revision && self.root == other.root
    }
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("root", &self.root)
            .field("revision", &self.revision)
            .finish()
    }
}

impl Document {
    /// A document at revision `revision` holding `root`.
    pub fn new(root: Option<Value>, revision: u64) -> Document {
        Document {
            root: root.map(|v| Node::from_value(v, revision)),
            revision,
            digest: OnceLock::new(),
            edition: next_edition(),
        }
    }

    /// A document at revision `revision` holding an existing tree (shared
    /// with wherever it came from until either changes it).
    pub fn from_root(root: Option<Node>, revision: u64) -> Document {
        Document {
            root,
            revision,
            digest: OnceLock::new(),
            edition: next_edition(),
        }
    }

    /// The same checked content at a later receiver revision. Shared nodes
    /// retain their original stamps and the canonical digest remains valid.
    pub fn shared_at_revision(&self, revision: u64) -> Document {
        assert!(revision >= self.revision);
        let mut document = self.clone();
        document.revision = revision;
        document
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// This value's edition (see "Editions" above): unique to this document
    /// value in its current state.
    pub fn edition(&self) -> u64 {
        self.edition
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
        // A failed batch may have applied an earlier operation.
        self.digest.take();
        self.edition = next_edition();
        let mut changed = false;
        for (index, patch) in patches.iter().enumerate() {
            changed |= patch::apply_one(&mut self.root, patch, index, revision)?;
        }
        Ok(ApplyOutcome { revision, changed })
    }

    /// Applies a batch of operations (leaf patches and id-keyed structural
    /// operations, [`crate::ops`]) as the next revision.
    pub fn apply_ops(&mut self, ops: &[Op]) -> Result<ApplyOutcome, PatchError> {
        self.apply_ops_at(ops, self.revision + 1)
    }

    /// Applies a batch of operations as revision `revision`, with the same
    /// error behavior as [`Document::apply_at`].
    ///
    /// # Panics
    ///
    /// Panics if `revision` does not exceed the current revision.
    pub fn apply_ops_at(&mut self, ops: &[Op], revision: u64) -> Result<ApplyOutcome, PatchError> {
        assert!(
            revision > self.revision,
            "revision {revision} must exceed {}",
            self.revision
        );
        self.revision = revision;
        // A failed batch may have applied an earlier operation.
        self.digest.take();
        self.edition = next_edition();
        let mut changed = false;
        for (index, op) in ops.iter().enumerate() {
            changed |= ops::apply_op(&mut self.root, op, index, revision)?;
        }
        Ok(ApplyOutcome { revision, changed })
    }

    /// `root` at `revision`, reconciled against `base` (another mirror of the
    /// same document, at or before `revision`): unchanged subtrees keep their
    /// revisions and stay shared with `base`, so two sessions or two
    /// consecutive resyncs of one large document hold one tree plus what
    /// differs.
    pub fn reconciled(base: &Document, root: Option<Value>, revision: u64) -> Document {
        assert!(
            revision >= base.revision,
            "revision {revision} must not precede {}",
            base.revision
        );
        let mut document = base.clone();
        document.revision = revision;
        document.digest.take();
        document.edition = next_edition();
        match (document.root.as_mut(), root) {
            (Some(node), Some(value)) => {
                node.reconcile(value, revision);
            }
            (_, value) => document.root = value.map(|v| Node::from_value(v, revision)),
        }
        document
    }

    /// [`Document::reconciled`] with a tree parsed at `revision`
    /// ([`Node::parse_bytes`]): identical results, without a `Value`.
    pub fn reconciled_node(base: &Document, root: Option<Node>, revision: u64) -> Document {
        assert!(
            revision >= base.revision,
            "revision {revision} must not precede {}",
            base.revision
        );
        let mut document = base.clone();
        document.revision = revision;
        document.digest.take();
        document.edition = next_edition();
        match (document.root.as_mut(), root) {
            (Some(node), Some(new)) => {
                node.reconcile_node(new, revision);
            }
            (_, new) => document.root = new,
        }
        document
    }

    /// [`Document::resync`] with a tree parsed at `revision`.
    pub fn resync_node(&mut self, root: Option<Node>, revision: u64) -> ApplyOutcome {
        assert!(
            revision > self.revision,
            "revision {revision} must exceed {}",
            self.revision
        );
        self.revision = revision;
        self.digest.take();
        self.edition = next_edition();
        let changed = match (self.root.as_mut(), root) {
            (Some(node), Some(new)) => node.reconcile_node(new, revision),
            (None, Some(new)) => {
                self.root = Some(new);
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

    /// Replaces the whole document (a resync), reconciling against the current
    /// tree so unchanged subtrees keep their revisions.
    pub fn resync(&mut self, root: Option<Value>, revision: u64) -> ApplyOutcome {
        assert!(
            revision > self.revision,
            "revision {revision} must exceed {}",
            self.revision
        );
        self.revision = revision;
        self.digest.take();
        self.edition = next_edition();
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
        self.canonical_digest().as_ref().map(crate::hash::hex)
    }

    /// The canonical digest as bytes.
    pub fn canonical_digest(&self) -> Option<[u8; 32]> {
        *self
            .digest
            .get_or_init(|| self.root.as_ref().map(canonical_digest))
    }
}

/// Per-thread input tracing, enabled only by pm-wasm. Native rendering does
/// not pay for it. Container reads depend on their own shape; child values
/// are separate reads. The caller must reset evaluator memos before tracing.
#[cfg(feature = "read-tracking")]
pub mod reads {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    thread_local! { static READS: RefCell<Option<HashSet<usize>>> = const { RefCell::new(None) }; }
    pub fn begin() {
        READS.with(|r| *r.borrow_mut() = Some(HashSet::new()));
    }
    pub(super) fn record(n: &Node) {
        READS.with(|r| {
            if let Some(set) = r.borrow_mut().as_mut() {
                set.insert(n as *const Node as usize);
            }
        });
    }
    /// Stable node addresses to input paths. Rebuild after a patch; queries
    /// at another time only walk their actual read set, never the whole scope.
    pub struct Index {
        root: usize,
        parents: HashMap<usize, (usize, PathPart)>,
    }
    impl Index {
        pub fn new(root: &Node) -> Self {
            fn walk(node: &Node, parents: &mut HashMap<usize, (usize, PathPart)>) {
                let parent = node as *const Node as usize;
                match &node.kind {
                    NodeKind::Array(a) => {
                        for (i, child) in a.iter().enumerate() {
                            parents.insert(child as *const Node as usize, (parent, i.into()));
                            walk(child, parents);
                        }
                    }
                    NodeKind::Object(o) => {
                        for (k, child) in o.iter() {
                            parents
                                .insert(child as *const Node as usize, (parent, k.clone().into()));
                            walk(child, parents);
                        }
                    }
                    _ => {}
                }
            }
            let mut parents = HashMap::new();
            walk(root, &mut parents);
            Self {
                root: root as *const Node as usize,
                parents,
            }
        }
    }
    /// Stop before resolving paths, so tracing never records its own work.
    pub fn finish(index: Option<&Index>) -> Vec<Vec<PathPart>> {
        let set = READS.with(|r| r.borrow_mut().take()).unwrap_or_default();
        let Some(index) = index else {
            return Vec::new();
        };
        set.into_iter()
            .filter_map(|mut address| {
                let mut path = Vec::new();
                while address != index.root {
                    let (parent, part) = index.parents.get(&address)?;
                    path.push(part.clone());
                    address = *parent;
                }
                path.reverse();
                Some(path)
            })
            .collect()
    }
}
