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
    /// Leaf digests computed ahead of a first digest, in visiting order,
    /// each with its node's address.
    ahead: std::collections::VecDeque<(usize, String)>,
    pub hashed: usize,
    pub reused: usize,
}

/// A first digest computes its leaves on several threads when there are at
/// least this many.
const PARALLEL_LEAVES: usize = 64;

/// Whether `node`, `depth` members below the root, is digested as canonical
/// JSON rather than from its members' digests.
fn is_leaf(node: &Node, depth: usize) -> bool {
    match node.kind() {
        // An array of many members that are not all records with string ids.
        NodeKind::Array(items) if items.len() > 64 => {
            depth >= 6
                || !items.iter().all(|item| {
                    matches!(item.kind(), NodeKind::Object(map)
                    if matches!(map.get(b"id".as_slice()).map(Node::kind), Some(NodeKind::String(_))))
                })
        }
        NodeKind::Array(_) | NodeKind::Object(_) => depth >= 6,
        _ => true,
    }
}

/// The leaves a digest without cached entries visits, in order.
fn leaves<'a>(node: &'a Node, depth: usize, out: &mut Vec<&'a Node>) {
    if is_leaf(node, depth) {
        out.push(node);
        return;
    }
    match node.kind() {
        NodeKind::Array(items) => items.iter().for_each(|item| leaves(item, depth + 1, out)),
        NodeKind::Object(map) => map
            .iter()
            .for_each(|(_, child)| leaves(child, depth + 1, out)),
        _ => {}
    }
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
        if self.entries.is_empty() && crate::parallel::workers() > 1 {
            let mut found = Vec::new();
            leaves(root, 0, &mut found);
            if found.len() >= PARALLEL_LEAVES {
                self.ahead = crate::parallel::map(found.len(), |i| {
                    (core::ptr::from_ref(found[i]) as usize, leaf(found[i]))
                })
                .into();
            }
        }
        let digest = self.visit(root, &mut Vec::new());
        self.ahead.clear();
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
        let digest = if is_leaf(node, path.len()) {
            match self.ahead.front() {
                Some((address, _)) if *address == core::ptr::from_ref(node) as usize => {
                    self.ahead.pop_front().expect("front").1
                }
                _ => leaf(node),
            }
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
                _ => unreachable!("only containers have member digests"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_digested_ahead_give_the_digest_made_in_turn() {
        let layers: Vec<String> = (0..80)
            .map(|i| {
                let keys: Vec<String> = (0..[2, 70, 300][i % 3]).map(|k| format!(r#"{{"t":{k},"v":[{i},{k}]}}"#)).collect();
                format!(r#"{{"id":"L{i}","p":{{"a":{{"b":{{"c":{{"d":[{}]}}}}}}}},"keys":[{}],"n":{i}}}"#, i % 5, keys.join(","))
            })
            .collect();
        let text = format!(
            r#"{{"name":"p","comps":{{"c":{{"layers":[{}]}}}},"ids":[{}]}}"#,
            layers.join(","),
            (0..100)
                .map(|n| format!(r#"{{"id":"{n}"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        let root = Node::parse_bytes(text.as_bytes(), 1).unwrap();
        let mut found = Vec::new();
        leaves(&root, 0, &mut found);
        assert!(found.len() >= PARALLEL_LEAVES);
        let mut ahead = ConsistencyHash::default();
        let digest = ahead.digest(Some(&root));
        assert!(ahead.ahead.is_empty());
        // A hasher holding (expired) entries visits every leaf in turn.
        let other = Node::parse_bytes(br#"{"other":[{"id":"x"}]}"#, 1).unwrap();
        let mut in_turn = ConsistencyHash::default();
        in_turn.digest(Some(&other));
        drop(other);
        assert!(!in_turn.entries.is_empty());
        assert_eq!(in_turn.digest(Some(&root)), digest);
        assert_eq!((in_turn.hashed, in_turn.reused), (ahead.hashed, 0));
        assert_eq!(in_turn.entries.len(), ahead.entries.len());
    }
}
