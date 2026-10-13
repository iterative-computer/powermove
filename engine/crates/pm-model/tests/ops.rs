//! Id-keyed structural operations (section 26, amendment 3): revisions are
//! preserved, and the generated operations give the same document (and the
//! same revisioned tree) as `diffPatches`' leaf patches.

use pm_model::binary::{decode, encode, is_canonical};
use pm_model::json::{Map, Value, parse};
use pm_model::ops::{Op, apply_ops, diff_ops, is_id_keyed};
use pm_model::patch::{PathPart, apply_patches, diff_patches};
use pm_model::{Document, path};
use proptest::prelude::*;

fn v(text: &str) -> Value {
    parse(text).unwrap()
}

fn layers(ids: &[&str]) -> Value {
    Value::Array(
        ids.iter()
            .map(|id| {
                v(&format!(
                    r#"{{"id":"{id}","name":"{id}","p":{{"x":{{"v":1,"kf":[]}}}}}}"#
                ))
            })
            .collect(),
    )
}

fn project(ids: &[&str]) -> Value {
    let mut comp = Map::new();
    comp.insert("id", Value::String("c1".into()));
    comp.insert("layers", layers(ids));
    let mut comps = Map::new();
    comps.insert("c1", Value::Object(comp));
    let mut root = Map::new();
    root.insert("compId", Value::String("c1".into()));
    root.insert("comps", Value::Object(comps));
    root.insert("assets", Value::Array(vec![]));
    Value::Object(root)
}

fn rev_of(doc: &Document, path: &[PathPart]) -> u64 {
    doc.lookup(path).unwrap().rev()
}

#[test]
fn reorder_keeps_every_layer_node_and_revision() {
    let before = project(&["a", "b", "c", "d"]);
    let after = project(&["d", "a", "c", "b"]);
    let ops = diff_ops(Some(&before), Some(&after));
    assert_eq!(
        ops,
        vec![Op::Order {
            path: path!["comps", "c1", "layers"].to_vec(),
            ids: vec!["d".into(), "a".into(), "c".into(), "b".into()],
        }]
    );
    let mut doc = Document::new(Some(before), 1);
    let outcome = doc.apply_ops_at(&ops, 5).unwrap();
    assert!(outcome.changed);
    assert_eq!(doc.to_value(), Some(after));
    for i in 0..4 {
        assert_eq!(rev_of(&doc, &path!["comps", "c1", "layers", i]), 1);
        assert_eq!(
            rev_of(&doc, &path!["comps", "c1", "layers", i, "p", "x", "v"]),
            1
        );
    }
    assert_eq!(rev_of(&doc, &path!["comps", "c1", "layers"]), 5);
    assert_eq!(rev_of(&doc, &path!["comps", "c1"]), 5);
    assert_eq!(rev_of(&doc, &path!["comps"]), 5);
    assert_eq!(doc.root().unwrap().rev(), 5);
    // Siblings of the changed path keep their revisions.
    assert_eq!(rev_of(&doc, &path!["assets"]), 1);
    assert_eq!(rev_of(&doc, &path!["compId"]), 1);
}

#[test]
fn insert_and_delete_touch_only_their_layers() {
    let before = project(&["a", "b", "c"]);
    let after = project(&["a", "n", "c"]);
    let ops = diff_ops(Some(&before), Some(&after));
    assert_eq!(ops.len(), 2, "{ops:?}");
    assert!(matches!(&ops[0], Op::Delete { id, .. } if id == "b"));
    assert!(matches!(&ops[1], Op::Insert { index: 1, .. }));
    let mut doc = Document::new(Some(before), 1);
    doc.apply_ops_at(&ops, 2).unwrap();
    assert_eq!(doc.to_value(), Some(after));
    assert_eq!(rev_of(&doc, &path!["comps", "c1", "layers", 0]), 1);
    assert_eq!(rev_of(&doc, &path!["comps", "c1", "layers", 1]), 2);
    assert_eq!(
        rev_of(&doc, &path!["comps", "c1", "layers", 1, "p", "x"]),
        2
    );
    assert_eq!(rev_of(&doc, &path!["comps", "c1", "layers", 2]), 1);
    assert_eq!(rev_of(&doc, &path!["comps", "c1", "layers"]), 2);
}

#[test]
fn an_order_that_changes_nothing_stamps_nothing() {
    let before = project(&["a", "b"]);
    let mut doc = Document::new(Some(before.clone()), 1);
    let op = Op::Order {
        path: path!["comps", "c1", "layers"].to_vec(),
        ids: vec!["a".into(), "b".into()],
    };
    let outcome = doc.apply_ops_at(&[op], 2).unwrap();
    assert!(!outcome.changed);
    assert_eq!(doc.root().unwrap().rev(), 1);
    assert_eq!(doc.revision(), 2);
}

#[test]
fn reorder_plus_edit_equals_reconciliation_of_the_whole_array() {
    let before = project(&["a", "b", "c"]);
    let mut after = project(&["c", "b", "a"]);
    // Edit layer b's x in the new order.
    if let Some(Value::Array(items)) = after
        .as_object_mut()
        .unwrap()
        .get_mut("comps")
        .and_then(|c| c.as_object_mut().unwrap().get_mut("c1"))
        .and_then(|c| c.as_object_mut().unwrap().get_mut("layers"))
    {
        items[1] = v(r#"{"id":"b","name":"b","p":{"x":{"v":9,"kf":[]}}}"#);
    }
    let ops = diff_ops(Some(&before), Some(&after));
    let (leaf, _) = diff_patches(Some(&before), Some(&after));
    let mut by_ops = Document::new(Some(before.clone()), 1);
    by_ops.apply_ops_at(&ops, 2).unwrap();
    let mut by_leaf = Document::new(Some(before), 1);
    by_leaf.apply_at(&leaf, 2).unwrap();
    assert_eq!(by_ops, by_leaf, "same tree, same revisions");
    assert_eq!(by_ops.to_value(), Some(after));
    assert_eq!(rev_of(&by_ops, &path!["comps", "c1", "layers", 0]), 1);
    assert_eq!(rev_of(&by_ops, &path!["comps", "c1", "layers", 1]), 2);
    assert_eq!(
        rev_of(&by_ops, &path!["comps", "c1", "layers", 1, "name"]),
        1
    );
}

#[test]
fn a_failing_operation_leaves_earlier_operations_applied() {
    let before = project(&["a", "b"]);
    let mut doc = Document::new(Some(before), 1);
    let ops = [
        Op::Delete {
            path: path!["comps", "c1", "layers"].to_vec(),
            id: "a".into(),
        },
        Op::Delete {
            path: path!["comps", "c1", "layers"].to_vec(),
            id: "a".into(),
        },
    ];
    let err = doc.apply_ops_at(&ops, 2).unwrap_err();
    assert_eq!(err.index, 1);
    assert_eq!(doc.to_value(), Some(project(&["b"])));
    assert_eq!(doc.revision(), 2);
}

// ── Random documents ────────────────────────────────────────────────────────

/// SplitMix64: a small deterministic generator for document shapes.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

const KEYS: [&str; 8] = ["v", "name", "kf", "p", "x", "10", "2", "\u{e000}"];

fn scalar(rng: &mut Rng) -> Value {
    match rng.below(6) {
        0 => Value::Null,
        1 => Value::Bool(rng.chance(50)),
        2 => Value::Number(rng.below(5) as f64),
        3 => Value::Number(-0.0),
        4 => Value::String(["", "a", "b"][rng.below(3)].into()),
        _ => Value::Number(0.5),
    }
}

fn value(rng: &mut Rng, depth: usize) -> Value {
    if depth == 0 || rng.chance(45) {
        return scalar(rng);
    }
    match rng.below(3) {
        0 => Value::Array((0..rng.below(4)).map(|_| value(rng, depth - 1)).collect()),
        1 => id_array(rng, depth - 1, &mut 0),
        _ => {
            let mut map = Map::new();
            for _ in 0..rng.below(4) {
                map.insert(KEYS[rng.below(KEYS.len())], value(rng, depth - 1));
            }
            Value::Object(map)
        }
    }
}

fn item(rng: &mut Rng, depth: usize, id: usize) -> Value {
    let mut map = match value(rng, depth) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert("id", Value::String(format!("i{id}").into()));
    Value::Object(map)
}

fn id_array(rng: &mut Rng, depth: usize, next_id: &mut usize) -> Value {
    let n = rng.below(6);
    Value::Array(
        (0..n)
            .map(|_| {
                *next_id += 1;
                item(rng, depth, *next_id)
            })
            .collect(),
    )
}

/// Edits `value` in place: structural edits to arrays (most often to
/// id-keyed ones), member edits, type changes, and id corruption.
fn mutate(rng: &mut Rng, value: &mut Value, depth: usize, next_id: &mut usize) {
    match value {
        Value::Array(items) => {
            let keyed = is_id_keyed(items);
            for _ in 0..rng.below(3) {
                match rng.below(9) {
                    0 | 1 if !items.is_empty() => {
                        let i = rng.below(items.len());
                        items.remove(i);
                    }
                    2 | 3 => {
                        let at = rng.below(items.len() + 1);
                        *next_id += 1;
                        let new = if keyed || rng.chance(50) {
                            item(rng, depth, 1000 + *next_id)
                        } else {
                            self::value(rng, depth)
                        };
                        items.insert(at, new);
                    }
                    4 | 5 if items.len() > 1 => {
                        let (i, j) = (rng.below(items.len()), rng.below(items.len()));
                        items.swap(i, j);
                    }
                    6 if items.len() > 1 => {
                        let i = rng.below(items.len());
                        let moved = items.remove(i);
                        let j = rng.below(items.len() + 1);
                        items.insert(j, moved);
                    }
                    7 if !items.is_empty() && rng.chance(25) => {
                        // Corrupt an id: drop it or duplicate another.
                        let i = rng.below(items.len());
                        let j = rng.below(items.len());
                        let other = items[j].get("id").cloned();
                        if let Value::Object(map) = &mut items[i] {
                            match other {
                                Some(id) if rng.chance(50) => {
                                    map.insert("id", id);
                                }
                                _ => {
                                    map.remove("id");
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            for item in items.iter_mut() {
                if rng.chance(40) {
                    mutate(rng, item, depth.saturating_sub(1), next_id);
                }
            }
        }
        Value::Object(map) => {
            let keys: Vec<_> = map.keys().cloned().collect();
            for key in keys {
                if key == "id" {
                    continue;
                }
                match rng.below(10) {
                    0 => {
                        map.remove_key(&key);
                    }
                    1 => {
                        map.insert(key, self::value(rng, depth));
                    }
                    _ => {
                        if let Some(child) = map.get_key_mut(&key) {
                            mutate(rng, child, depth.saturating_sub(1), next_id);
                        }
                    }
                }
            }
            if rng.chance(30) {
                map.insert(KEYS[rng.below(KEYS.len())], self::value(rng, depth));
            }
        }
        other => {
            if rng.chance(30) {
                *other = self::value(rng, depth);
            }
        }
    }
}

fn random_pair(seed: u64) -> (Value, Value) {
    let mut rng = Rng(seed);
    let mut next_id = 0;
    let mut doc = Map::new();
    doc.insert("layers", id_array(&mut rng, 3, &mut next_id));
    doc.insert("markers", id_array(&mut rng, 1, &mut next_id));
    doc.insert("misc", value(&mut rng, 3));
    let before = Value::Object(doc);
    let mut after = before.clone();
    mutate(&mut rng, &mut after, 3, &mut next_id);
    (before, after)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4_000, ..ProptestConfig::default() })]

    #[test]
    fn generated_ops_equal_leaf_patches(seed in any::<u64>()) {
        let (before, after) = random_pair(seed);
        let ops = diff_ops(Some(&before), Some(&after));
        let (leaf, backward) = diff_patches(Some(&before), Some(&after));

        // Mirrors hold JSON-normalized documents (patch values pass through
        // clonePatchValue, so a mirror never holds -0), while the generator
        // diffs the renderer's live values.
        let mirror = before.json_normalized();

        // Plain documents: ops, leaf patches, and the target agree.
        let mut by_ops = Some(mirror.clone());
        apply_ops(&mut by_ops, &ops).unwrap();
        let mut by_leaf = Some(mirror.clone());
        apply_patches(&mut by_leaf, &leaf).unwrap();
        prop_assert_eq!(&by_ops, &by_leaf);
        prop_assert_eq!(by_ops.as_ref(), Some(&after));

        // Revisioned documents: the same tree and the same revisions.
        let mut doc_ops = Document::new(Some(mirror.clone()), 1);
        doc_ops.apply_ops_at(&ops, 2).unwrap();
        let mut doc_leaf = Document::new(Some(mirror), 1);
        doc_leaf.apply_at(&leaf, 2).unwrap();
        prop_assert_eq!(&doc_ops, &doc_leaf);

        // Undo by generated ops in the other direction.
        let back = diff_ops(Some(&after), Some(&before));
        doc_ops.apply_ops_at(&back, 3).unwrap();
        doc_leaf.apply_at(&backward, 3).unwrap();
        prop_assert_eq!(&doc_ops, &doc_leaf);
        prop_assert_eq!(doc_ops.to_value(), Some(before));

        // The binary form round-trips and is canonical.
        let bytes = encode(&ops).unwrap();
        prop_assert!(is_canonical(&bytes));
        prop_assert_eq!(decode(&bytes).unwrap(), ops);
    }
}

#[test]
fn random_pairs_exercise_every_operation() {
    let mut counts = [0usize; 5];
    for seed in 0..4_000u64 {
        let (before, after) = random_pair(seed);
        for op in diff_ops(Some(&before), Some(&after)) {
            counts[match op {
                Op::Leaf(ref p) if p.exists => 0,
                Op::Leaf(_) => 1,
                Op::Order { .. } => 2,
                Op::Insert { .. } => 3,
                Op::Delete { .. } => 4,
            }] += 1;
        }
    }
    assert!(counts.iter().all(|&n| n > 200), "{counts:?}");
}
