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
fn every_new_value_and_change_takes_a_new_edition() {
    let mut d = doc(r#"{"comps":{"C":{"layers":[{"id":"a","p":{}}]}}}"#);
    let mut seen = std::collections::BTreeSet::from([d.edition()]);
    let mut fresh = |e: u64| assert!(seen.insert(e), "edition {e} seen before");
    let copy = d.clone();
    fresh(copy.edition());
    assert_eq!(copy, d, "the edition is not part of the value");
    fresh(Document::default().edition());
    d.apply(&[Patch::set(vec![PathPart::Key("x".into())], v("1"))])
        .unwrap();
    fresh(d.edition());
    let revision = d.revision() + 1;
    d.apply_ops_at(&[], revision).unwrap();
    fresh(d.edition());
    let value = d.to_value();
    let revision = d.revision() + 1;
    d.resync(value, revision);
    fresh(d.edition());
    // Reading never changes it.
    let e = d.edition();
    let _ = (d.root(), d.to_value(), d.canonical_hash());
    assert_eq!(d.edition(), e);
}
