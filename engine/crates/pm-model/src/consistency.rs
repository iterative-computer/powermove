//! Incremental integrity digest for local mirrors. This is deliberately distinct
//! from the RFC 05 canonical SHA-256 used on the wire. Containers above depth 6
//! hash their keys and child digests; deep values hash canonical JSON. Unchanged
//! shared subtrees reuse their digest without serializing or hashing them.
use crate::{JsString, Node, NodeKind, Value, json::ObjectMap};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

enum Identity {
    Array(Weak<Vec<Node>>),
    Object(Weak<ObjectMap<Node>>),
}
impl Identity {
    fn of(node: &Node) -> Option<Self> {
        match node.kind() {
            NodeKind::Array(items) => Some(Self::Array(Arc::downgrade(items))),
            NodeKind::Object(map) => Some(Self::Object(Arc::downgrade(map))),
            _ => None,
        }
    }
    fn matches(&self, node: &Node) -> bool {
        match (self, node.kind()) {
            (Self::Array(old), NodeKind::Array(now)) => {
                old.ptr_eq(&Arc::downgrade(now)) && old.strong_count() > 0
            }
            (Self::Object(old), NodeKind::Object(now)) => {
                old.ptr_eq(&Arc::downgrade(now)) && old.strong_count() > 0
            }
            _ => false,
        }
    }
}
struct Entry {
    identity: Identity,
    revision: u64,
    digest: String,
}
#[derive(Default)]
pub struct ConsistencyHash {
    entries: HashMap<Vec<JsString>, Entry>,
    pub hashed: usize,
    pub reused: usize,
}
impl ConsistencyHash {
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn digest(&mut self, root: Option<&Node>) -> String {
        self.hashed = 0;
        self.reused = 0;
        let Some(root) = root else {
            self.clear();
            return String::new();
        };
        let digest = self.visit(root, &mut Vec::new());
        // Removed subtrees' weak entries must not accumulate over an editing session.
        self.entries.retain(|_, e| match &e.identity {
            Identity::Array(w) => w.strong_count() > 0,
            Identity::Object(w) => w.strong_count() > 0,
        });
        digest
    }
    fn visit(&mut self, node: &Node, path: &mut Vec<JsString>) -> String {
        if let Some(entry) = self.entries.get(path)
            && entry.revision == node.rev()
            && entry.identity.matches(node)
        {
            self.reused += 1;
            return entry.digest.clone();
        }
        self.hashed += 1;
        let bulk = match node.kind() {
            NodeKind::Array(items) if items.len() > 64 => !items.iter().all(|item| {
                matches!(item.kind(), NodeKind::Object(map)
                    if matches!(map.get(b"id".as_slice()).map(Node::kind), Some(NodeKind::String(_))))
            }),
            _ => false,
        };
        let digest = if path.len() >= 6 || bulk {
            leaf(node)
        } else {
            match node.kind() {
                NodeKind::Array(items) => {
                    let hashes = items
                        .iter()
                        .enumerate()
                        .map(|(i, child)| {
                            path.push(JsString::from(i.to_string()));
                            let hash = self.visit(child, path);
                            path.pop();
                            Value::from(hash)
                        })
                        .collect();
                    crate::hash::canonical_hash(&Value::Array(vec![
                        Value::from("a"),
                        Value::Array(hashes),
                    ]))
                }
                NodeKind::Object(map) => {
                    let hashes = map
                        .iter()
                        .map(|(key, child)| {
                            path.push(key.clone());
                            let hash = self.visit(child, path);
                            path.pop();
                            Value::Array(vec![Value::String(key.clone()), Value::from(hash)])
                        })
                        .collect();
                    crate::hash::canonical_hash(&Value::Array(vec![
                        Value::from("o"),
                        Value::Array(hashes),
                    ]))
                }
                _ => leaf(node),
            }
        };
        // Keep the root and composition containers cacheable even when a
        // customized document fills the leaf-entry budget.
        if self.entries.len() >= 16384
            && path.len() < 4
            && !self.entries.contains_key(path)
            && let Some(victim) = self.entries.keys().max_by_key(|key| key.len()).cloned()
        {
            self.entries.remove(&victim);
        }
        if (self.entries.len() < 16384 || self.entries.contains_key(path))
            && let Some(identity) = Identity::of(node)
        {
            self.entries.insert(
                path.clone(),
                Entry {
                    identity,
                    revision: node.rev(),
                    digest: digest.clone(),
                },
            );
        }
        digest
    }
}
fn leaf(node: &Node) -> String {
    use sha2::{Digest, Sha256};
    struct Sink(Sha256);
    impl crate::json::write::Sink for Sink {
        fn push_str(&mut self, text: &str) {
            self.0.update(text.as_bytes());
        }
    }
    let mut sink = Sink(Sha256::new());
    sink.0.update(b"v");
    crate::json::write::write_canonical(node, &mut sink);
    crate::hash::hex(&sink.0.finalize().into())
}
