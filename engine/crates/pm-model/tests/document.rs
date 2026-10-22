//! Revisions and id-keyed reconciliation in the revisioned document.

use pm_model::json::{Value, parse, to_canonical_string};
use pm_model::patch::{Patch, PathPart, patches_from_value};
use pm_model::{Document, Node, NodeKind, hash, path};

fn doc(text: &str) -> Document {
    Document::new(Some(parse(text).unwrap()), 1)
}

fn v(text: &str) -> Value {
    parse(text).unwrap()
}

fn rev(document: &Document, path: impl AsRef<[PathPart]>) -> u64 {
    let path = path.as_ref();
    document
        .lookup(path)
        .unwrap_or_else(|| panic!("no node at {path:?}"))
        .rev()
}

const PROJECT: &str = r#"{
  "compId": "C1",
  "comps": {"C1": {"id": "C1", "layers": [
    {"id": "a", "name": "A", "p": {"opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "b", "name": "B", "p": {"opacity": {"v": 50, "kf": [], "expr": null}}},
    {"id": "c", "name": "C", "p": {}}
  ]}},
  "assets": {}
}"#;

fn layers() -> Vec<PathPart> {
    path!["comps", "C1", "layers"].to_vec()
}

fn layer_values(document: &Document) -> Vec<Value> {
    document
        .lookup(&layers())
        .unwrap()
        .to_value()
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn reordering_keeps_every_layer_revision() {
    let mut d = doc(PROJECT);
    let mut list = layer_values(&d);
    list.rotate_left(1); // b, c, a
    let outcome = d
        .apply(&[Patch::set(layers(), Value::Array(list))])
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(outcome.revision, 2);
    assert_eq!(rev(&d, layers()), 2);
    assert_eq!(d.root().unwrap().rev(), 2);
    for i in 0..3usize {
        let mut p = layers();
        p.push(i.into());
        assert_eq!(rev(&d, &p), 1, "layer {i} must keep its revision");
    }
    assert_eq!(rev(&d, path!["assets"]), 1);
}

#[test]
fn replacing_the_array_stamps_only_the_changed_layer() {
    let mut d = doc(PROJECT);
    let mut list = layer_values(&d);
    // Change b's opacity, insert d at the front, drop c.
    list[1]
        .as_object_mut()
        .unwrap()
        .get_mut("p")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("opacity")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("v", Value::from(75));
    list.pop();
    list.insert(0, v(r#"{"id":"d","name":"D","p":{}}"#));
    d.apply(&[Patch::set(layers(), Value::Array(list))])
        .unwrap();
    let mut l = layers();
    l.push(0.into());
    assert_eq!(rev(&d, &l), 2, "the new layer is created at revision 2");
    assert_eq!(
        rev(&d, path!["comps", "C1", "layers", 1]),
        1,
        "a moved but did not change"
    );
    assert_eq!(rev(&d, path!["comps", "C1", "layers", 2]), 2, "b changed");
    assert_eq!(rev(&d, path!["comps", "C1", "layers", 2, "name"]), 1);
    assert_eq!(
        rev(&d, path!["comps", "C1", "layers", 2, "p", "opacity", "kf"]),
        1
    );
    assert_eq!(
        rev(&d, path!["comps", "C1", "layers", 2, "p", "opacity", "v"]),
        2
    );
}

#[test]
fn leaf_patches_stamp_the_path_only() {
    let mut d = doc(PROJECT);
    d.apply(&[Patch::set(
        path!["comps", "C1", "layers", 0, "p", "opacity", "v"],
        Value::from(10),
    )])
    .unwrap();
    for (p, expected) in [
        (vec![], 2),
        (path!["comps"].to_vec(), 2),
        (path!["comps", "C1", "layers"].to_vec(), 2),
        (path!["comps", "C1", "layers", 0].to_vec(), 2),
        (
            path!["comps", "C1", "layers", 0, "p", "opacity", "v"].to_vec(),
            2,
        ),
        (path!["comps", "C1", "layers", 0, "name"].to_vec(), 1),
        (path!["comps", "C1", "layers", 1].to_vec(), 1),
        (path!["assets"].to_vec(), 1),
    ] {
        assert_eq!(rev(&d, &p), expected, "{p:?}");
    }
    // Same value again: the revision advances, nothing is stamped.
    let outcome = d
        .apply(&[Patch::set(
            path!["comps", "C1", "layers", 0, "p", "opacity", "v"],
            Value::from(10),
        )])
        .unwrap();
    assert!(!outcome.changed);
    assert_eq!(d.revision(), 3);
    assert_eq!(d.root().unwrap().rev(), 2);
}

#[test]
fn deletes_and_splices_stamp_the_parent() {
    let mut d = doc(PROJECT);
    d.apply(&[Patch::delete(path!["comps", "C1", "layers", 0])])
        .unwrap();
    assert_eq!(rev(&d, layers()), 2);
    assert_eq!(rev(&d, path!["comps", "C1", "layers", 0]), 1); // b, untouched
    let ids: Vec<_> = layer_values(&d)
        .iter()
        .map(|l| l.get("id").unwrap().as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["b", "c"]);
    d.apply(&[Patch::delete(path!["assets", "missing"])])
        .unwrap();
    assert_eq!(
        rev(&d, path!["assets"]),
        1,
        "deleting nothing changes nothing"
    );
}

#[test]
fn elements_without_ids_match_by_position() {
    let mut d = doc(r#"{"kf":[{"t":0,"v":1},{"t":1,"v":2},{"t":2,"v":3}]}"#);
    d.apply(&[Patch::set(
        path!["kf"],
        v(r#"[{"t":0,"v":1},{"t":1,"v":5},{"t":2,"v":3},{"t":3,"v":4}]"#),
    )])
    .unwrap();
    assert_eq!(rev(&d, path!["kf", 0]), 1);
    assert_eq!(rev(&d, path!["kf", 1]), 2);
    assert_eq!(rev(&d, path!["kf", 2]), 1);
    assert_eq!(rev(&d, path!["kf", 3]), 2);
    assert_eq!(rev(&d, path!["kf"]), 2);
}

#[test]
fn id_and_positional_matching_never_mix() {
    let mut d = doc(r#"{"a":[{"id":"x","n":1},{"n":2}]}"#);
    // Position 0 now holds an element without an id: it must not reuse "x".
    d.apply(&[Patch::set(path!["a"], v(r#"[{"n":1},{"id":"x","n":1}]"#))])
        .unwrap();
    assert_eq!(rev(&d, path!["a", 0]), 2);
    assert_eq!(rev(&d, path!["a", 1]), 1, "x moved to index 1 unchanged");
}

#[test]
fn duplicate_ids_match_in_order() {
    let mut d = doc(r#"{"a":[{"id":"x","n":1},{"id":"x","n":2}]}"#);
    d.apply(&[Patch::set(
        path!["a"],
        v(r#"[{"id":"x","n":1},{"id":"x","n":3}]"#),
    )])
    .unwrap();
    assert_eq!(rev(&d, path!["a", 0]), 1);
    assert_eq!(rev(&d, path!["a", 1]), 2);
}

#[test]
fn type_changes_rebuild_the_node() {
    let mut d = doc(r#"{"a":{"b":1},"n":0}"#);
    d.apply(&[
        Patch::set(path!["a"], v("[1,2]")),
        Patch::set(path!["n"], v("-0")),
    ])
    .unwrap();
    assert_eq!(rev(&d, path!["a"]), 2);
    assert_eq!(
        rev(&d, path!["n"]),
        1,
        "patch values are cloned through JSON: -0 is 0"
    );
    d.apply(&[Patch::set(path!["a"], v("[1,2]"))]).unwrap();
    assert_eq!(rev(&d, path!["a"]), 2);
}

#[test]
fn resync_reconciles_against_the_mirror() {
    let mut d = doc(PROJECT);
    let mut value = d.to_value().unwrap();
    let outcome = d.resync(Some(value.clone()), 5);
    assert!(!outcome.changed);
    assert_eq!(d.root().unwrap().rev(), 1);
    value
        .as_object_mut()
        .unwrap()
        .insert("assets", v(r#"{"x":{"id":"x"}}"#));
    let outcome = d.resync(Some(value), 6);
    assert!(outcome.changed);
    assert_eq!(rev(&d, path!["assets"]), 6);
    assert_eq!(rev(&d, path!["comps"]), 1);
    assert!(d.resync(None, 7).changed);
    assert!(d.root().is_none());
}

#[test]
fn errors_leave_earlier_patches_applied() {
    let mut d = doc(PROJECT);
    let err = d
        .apply(&[
            Patch::set(path!["assets", "x"], v("{}")),
            Patch::set(path!["compId", "oops"], v("1")),
            Patch::set(path!["assets", "y"], v("{}")),
        ])
        .unwrap_err();
    assert_eq!(err.index, 1);
    assert!(d.lookup(&path!["assets", "x"]).is_some());
    assert!(d.lookup(&path!["assets", "y"]).is_none());
    assert_eq!(d.revision(), 2);
}

#[test]
fn hash_matches_the_canonical_value() {
    let d = doc(PROJECT);
    let value = d.to_value().unwrap();
    assert_eq!(d.canonical_hash().unwrap(), hash::canonical_hash(&value));
    assert_eq!(
        hash::canonical_hash(&value),
        hash::sha256_hex(to_canonical_string(&value).as_bytes())
    );
    assert_eq!(Document::new(None, 0).canonical_hash(), None);
}

fn check_monotone(node: &Node) {
    let children: Vec<&Node> = match node.kind() {
        NodeKind::Array(items) => items.iter().collect(),
        NodeKind::Object(map) => map.values().collect(),
        _ => vec![],
    };
    for child in children {
        assert!(
            child.rev() <= node.rev(),
            "a child is newer than its parent"
        );
        check_monotone(child);
    }
}

/// Over every generated patch case: a parent's revision is never older than
/// its children's, and revisions only move when content moved.
#[test]
fn revisions_are_monotone_over_the_vectors() {
    let vectors = parse(include_str!("vectors/patch-vectors.json")).unwrap();
    for case in vectors.get("cases").unwrap().as_array().unwrap() {
        let start = match case.get("doc").unwrap() {
            Value::String(text) => Some(parse(&text.to_string_lossy()).unwrap()),
            _ => None,
        };
        let patches = patches_from_value(case.get("patches").unwrap()).unwrap();
        let mut d = Document::new(start.clone(), 0);
        let mut revision = 0;
        for patch in patches {
            let before = d.to_value().map(|v| to_canonical_string(&v));
            revision += 1;
            let Ok(outcome) = d.apply_at(std::slice::from_ref(&patch), revision) else {
                break;
            };
            let after = d.to_value().map(|v| to_canonical_string(&v));
            assert_eq!(outcome.changed, before != after, "{patch:?}");
            if let Some(root) = d.root() {
                check_monotone(root);
                if !outcome.changed {
                    assert!(root.rev() < revision);
                }
            }
        }
    }
}

#[test]
fn clones_share_subtrees_until_one_changes_them() {
    // The interactive engine keeps a mirror per session, its presenter and
    // audio threads one each, and every snapshot job one at its revision:
    // clones must not copy a large document (C1, 1.2 million keyframes).
    let original = doc(PROJECT);
    let mut edited = original.clone();
    let a = path!["comps", "C1", "layers", 0];
    let b = path!["comps", "C1", "layers", 1];
    assert!(std::ptr::eq(
        original.lookup(&a).unwrap(),
        edited.lookup(&a).unwrap()
    ));
    edited
        .apply(&[Patch::set(
            path!["comps", "C1", "layers", 0, "name"],
            v(r#""A2""#),
        )])
        .unwrap();
    // The edit copied the containers on its path only.
    assert!(
        to_canonical_string(&original.lookup(&a).unwrap().to_value()).contains(r#""name":"A""#)
    );
    assert!(to_canonical_string(&edited.lookup(&a).unwrap().to_value()).contains(r#""name":"A2""#));
    assert!(!std::ptr::eq(
        original.lookup(&a).unwrap(),
        edited.lookup(&a).unwrap()
    ));
    let b_original = original.lookup(&b).unwrap();
    let b_edited = edited.lookup(&b).unwrap();
    let NodeKind::Object(left) = b_original.kind() else {
        panic!()
    };
    let NodeKind::Object(right) = b_edited.kind() else {
        panic!()
    };
    assert!(
        std::ptr::eq(&**left, &**right),
        "an untouched layer stays shared"
    );
    assert_eq!(rev(&original, &a), 1);
    assert_eq!(rev(&edited, &a), 2);
    assert_eq!(
        hash::canonical_hash(original.root().unwrap()),
        hash::canonical_hash(&parse(PROJECT).unwrap())
    );
    // A tree from an existing root keeps sharing it.
    let rebuilt = Document::from_root(edited.root().cloned(), 9);
    assert_eq!(rebuilt.revision(), 9);
    assert!(std::ptr::eq(
        match rebuilt.lookup(&b).unwrap().kind() {
            NodeKind::Object(m) => &**m,
            _ => panic!(),
        },
        &**right
    ));
}

#[test]
fn a_resync_reconciled_against_another_mirror_shares_what_did_not_change() {
    // Two sessions (or two resyncs) of one large document hold one tree
    // plus what differs, with unchanged nodes keeping their revisions.
    let base = doc(PROJECT);
    let same = Document::reconciled(&base, Some(v(PROJECT)), 4);
    assert_eq!(same.revision(), 4);
    let shared = |a: &Document, b: &Document, at: &[PathPart]| match (
        a.lookup(at).unwrap().kind(),
        b.lookup(at).unwrap().kind(),
    ) {
        (NodeKind::Object(x), NodeKind::Object(y)) => std::ptr::eq(&**x, &**y),
        (NodeKind::Array(x), NodeKind::Array(y)) => std::ptr::eq(&**x, &**y),
        _ => panic!("not a container"),
    };
    assert!(shared(&base, &same, &[]));
    assert_eq!(rev(&same, layers()), 1);
    let edited = PROJECT.replace(r#""name": "B""#, r#""name": "B2""#);
    let next = Document::reconciled(&same, Some(v(&edited)), 5);
    assert!(shared(&same, &next, &path!["comps", "C1", "layers", 0]));
    assert!(shared(&same, &next, &path!["comps", "C1", "layers", 2]));
    assert!(!shared(&same, &next, &path!["comps", "C1", "layers", 1]));
    assert_eq!(rev(&next, path!["comps", "C1", "layers", 1]), 5);
    assert_eq!(rev(&next, path!["comps", "C1", "layers", 0]), 1);
    assert_eq!(
        hash::canonical_hash(next.root().unwrap()),
        hash::canonical_hash(&v(&edited))
    );
    assert_eq!(
        hash::canonical_hash(same.root().unwrap()),
        hash::canonical_hash(&v(PROJECT))
    );
}

/// Seeded JSON documents shaped like projects: id-keyed arrays (reordered,
/// inserted, removed), nested objects, every scalar kind, `-0`, duplicate
/// keys, and keys whose byte order differs from their UTF-16 order.
fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn random_json(seed: &mut u64, depth: u32) -> String {
    let pick = xorshift(seed) % if depth == 0 { 6 } else { 9 };
    match pick {
        0 => "null".into(),
        1 => {
            if xorshift(seed).is_multiple_of(2) {
                "true".into()
            } else {
                "false".into()
            }
        }
        2 => ["0", "-0", "1.5", "1e21", "-3", "0.1"][(xorshift(seed) % 6) as usize].into(),
        3 => format!("\"s{}\"", xorshift(seed) % 4),
        4 => "\"\\ud800x\"".into(),
        5 => "\"é😀\"".into(),
        6 | 7 => {
            let n = xorshift(seed) % 5;
            let ids = xorshift(seed).is_multiple_of(2);
            let mut items = Vec::new();
            for i in 0..n {
                let shift = xorshift(seed) % 3;
                let inner = random_json(seed, depth - 1);
                items.push(if ids {
                    format!("{{\"id\":\"L{}\",\"v\":{inner}}}", (i + shift) % 4)
                } else {
                    inner
                });
            }
            format!("[{}]", items.join(","))
        }
        _ => {
            let n = xorshift(seed) % 4;
            let mut members = Vec::new();
            for _ in 0..n {
                let key =
                    ["a", "b", "id", "\u{ff61}", "\u{1f600}", "a"][(xorshift(seed) % 6) as usize];
                members.push(format!("\"{key}\":{}", random_json(seed, depth - 1)));
            }
            format!("{{{}}}", members.join(","))
        }
    }
}

#[test]
fn parsing_straight_into_a_tree_and_reconciling_trees_match_the_value_path() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..3000 {
        let a = format!(
            "{{\"layers\":{},\"x\":{}}}",
            random_json(&mut seed, 4),
            random_json(&mut seed, 3)
        );
        let b = format!(
            "{{\"layers\":{},\"x\":{}}}",
            random_json(&mut seed, 4),
            random_json(&mut seed, 3)
        );
        // The same tree, revisions included.
        assert_eq!(
            Node::parse_bytes(a.as_bytes(), 3).unwrap(),
            Node::from_value(v(&a), 3),
            "{a}"
        );
        // Reconciling a parsed tree equals reconciling the parsed value.
        let base = Document::new(Some(v(&a)), 1);
        let mut by_value = base.clone();
        by_value.resync(Some(v(&b)), 2);
        let mut by_node = base.clone();
        by_node.resync_node(Some(Node::parse_bytes(b.as_bytes(), 2).unwrap()), 2);
        assert_eq!(by_value, by_node, "{a} -> {b}");
        let shared_value = Document::reconciled(&base, Some(v(&b)), 2);
        let shared_node =
            Document::reconciled_node(&base, Some(Node::parse_bytes(b.as_bytes(), 2).unwrap()), 2);
        assert_eq!(shared_value, shared_node, "{a} -> {b}");
        assert_eq!(base, Document::new(Some(v(&a)), 1), "the base is untouched");
    }
    assert!(Node::parse_bytes(b"{\"a\":", 1).is_err());
}

#[test]
fn cached_canonical_digest_survives_clones_and_invalidates_on_partial_failure() {
    let mut d = doc(r#"{"a":1,"items":[{"id":"a"},{"id":"b"}]}"#);
    let initial = d.canonical_digest();
    let clone = d.clone();
    assert_eq!(clone.canonical_digest(), initial);
    d.apply(&[Patch::set(path!["a"].to_vec(), Value::from(2))])
        .unwrap();
    assert_ne!(d.canonical_digest(), initial);
    assert_eq!(clone.canonical_digest(), initial);
    // The first patch remains applied when a later patch fails.
    let result = d.apply_ops(&[
        pm_model::Op::Leaf(Patch::set(path!["a"].to_vec(), Value::from(3))),
        pm_model::Op::Order {
            path: path!["items"].to_vec(),
            ids: vec!["missing".into()],
        },
    ]);
    assert!(result.is_err());
    assert_eq!(
        d.canonical_digest(),
        Some(hash::canonical_digest(&d.to_value().unwrap()))
    );
    let mut hasher = pm_model::consistency::ConsistencyHash::default();
    let first = hasher.digest(d.root());
    assert_eq!(first, hasher.digest(d.root()));
    assert_eq!(hasher.hashed, 0);
    d.apply(&[Patch::set(path!["a"].to_vec(), Value::from(4))])
        .unwrap();
    let incremental = hasher.digest(d.root());
    assert_eq!(
        incremental,
        pm_model::consistency::ConsistencyHash::default().digest(d.root())
    );
    assert!(hasher.reused > 0);
}
