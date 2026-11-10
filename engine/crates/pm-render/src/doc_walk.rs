//! Whole-document walks that run on every new revision, kept per unchanged
//! subtree. The document is copy on write: an edit copies only the
//! containers on its path, so a container that is the same allocation as at
//! the last walk holds the same subtree. A drag step on a project with dense
//! keyframes re-walks a few dozen containers instead of the whole tree.

use std::collections::HashMap;
use std::sync::Arc;

use pm_model::{Node, NodeKind};

/// Containers this shallow are remembered (a layer's properties sit at about
/// depth 6); so are large arrays deeper down (keyframe lists).
const DEPTH: usize = 7;
const LARGE: usize = 32;

/// What a walk found under each remembered container. Each entry keeps a
/// clone of its container, so the allocation is never freed and reused for
/// different content while it is remembered.
pub(crate) struct SubtreeMemo<T> {
    seen: HashMap<usize, (Node, Vec<T>)>,
}

impl<T> Default for SubtreeMemo<T> {
    fn default() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }
}

impl<T: Clone> SubtreeMemo<T> {
    /// Everything `visit` finds at `root` and every node under it (in the
    /// walk's order, possibly repeated). `visit` looks at one node only; the
    /// memo descends. Entries for containers not met again are dropped.
    pub fn walk(&mut self, root: &Node, visit: &mut dyn FnMut(&Node, &mut Vec<T>)) -> Vec<T> {
        let mut old = std::mem::take(&mut self.seen);
        let mut out = Vec::new();
        go(root, 0, &mut old, &mut self.seen, visit, &mut out);
        out
    }
}

fn go<T: Clone>(
    node: &Node,
    depth: usize,
    old: &mut HashMap<usize, (Node, Vec<T>)>,
    new: &mut HashMap<usize, (Node, Vec<T>)>,
    visit: &mut dyn FnMut(&Node, &mut Vec<T>),
    out: &mut Vec<T>,
) {
    let (key, len) = match node.kind() {
        NodeKind::Object(map) => (Arc::as_ptr(map) as *const () as usize, map.len()),
        NodeKind::Array(items) => (Arc::as_ptr(items) as *const () as usize, items.len()),
        _ => {
            visit(node, out);
            return;
        }
    };
    let remember = depth < DEPTH || len >= LARGE;
    if remember && let Some((kept, found)) = old.remove(&key) {
        out.extend(found.iter().cloned());
        new.insert(key, (kept, found));
        return;
    }
    let start = out.len();
    visit(node, out);
    match node.kind() {
        NodeKind::Object(map) => {
            for child in map.values() {
                go(child, depth + 1, old, new, visit, out);
            }
        }
        NodeKind::Array(items) => {
            for child in items.iter() {
                go(child, depth + 1, old, new, visit, out);
            }
        }
        _ => {}
    }
    if remember {
        new.insert(key, (node.clone(), out[start..].to_vec()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::Document;
    use pm_model::json::parse;
    use pm_model::patch::{Patch, PathPart};

    fn strings(node: &Node, out: &mut Vec<String>) {
        if let NodeKind::String(s) = node.kind() {
            out.push(s.to_string_lossy());
        }
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    /// A walk of an edited revision finds what a walk from scratch finds,
    /// and re-walks only the containers the edit copied.
    #[test]
    fn an_edited_revision_matches_a_fresh_walk() {
        let value =
            parse(r#"{"a":{"keys":[1,2,3],"name":"x"},"b":{"deep":{"deeper":["y","z"]}},"c":"w"}"#)
                .unwrap();
        let mut doc = Document::new(Some(value), 1);
        let mut memo = SubtreeMemo::default();
        let first = memo.walk(doc.root().unwrap(), &mut strings);
        assert_eq!(sorted(first), ["w", "x", "y", "z"]);
        let path = vec![PathPart::Key("a".into()), PathPart::Key("name".into())];
        doc.apply(&[Patch::set(path, parse(r#""v""#).unwrap())])
            .unwrap();
        let mut visited = 0;
        let edited = memo.walk(doc.root().unwrap(), &mut |n, out| {
            visited += 1;
            strings(n, out)
        });
        let fresh = SubtreeMemo::default().walk(doc.root().unwrap(), &mut strings);
        assert_eq!(sorted(edited), sorted(fresh));
        // root, the copied "a", its new "name" and the string "c" (strings
        // are visited, not remembered); "a.keys" and "b" come from the memo.
        assert_eq!(visited, 4, "an untouched subtree was walked again");
    }
}
