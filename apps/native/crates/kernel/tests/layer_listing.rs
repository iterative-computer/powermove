//! The layer list (`model::list_layers`) and the names-only walk
//! (`model::layer_names`) against the per-layer queries they stand for.
use pm_kernel::DocumentStore;
use pm_kernel::model::{LayerLoc, is_locked, layer_names, list_layers};
use pm_model::json::parse;

/// Nested groups, a locked outer group, a group cycle, a dangling group id,
/// a duplicated group id (the first group with the id is the one followed)
/// and a locked layer of its own.
const PROJECT: &str = r##"{
  "id": "P", "name": "Locks", "w": 200, "h": 100, "fps": 30, "dur": 10, "bg": "#000000",
  "assets": {}, "comps": {},
  "layers": [
    {"id": "outer", "type": "group", "name": "Outer", "from": 0, "dur": 10, "on": true, "lock": true},
    {"id": "inner", "type": "group", "name": "Inner", "group": "outer", "from": 0, "dur": 10, "on": true},
    {"id": "deep", "type": "solid", "name": "Deep", "group": "inner", "from": 0, "dur": 10, "on": true},
    {"id": "free", "type": "solid", "name": "Free", "from": 0, "dur": 10, "on": true},
    {"id": "own", "type": "solid", "name": "Own lock", "from": 0, "dur": 10, "on": true, "lock": true},
    {"id": "a", "type": "group", "name": "Cycle A", "group": "b", "from": 0, "dur": 10, "on": true},
    {"id": "b", "type": "group", "name": "Cycle B", "group": "a", "from": 0, "dur": 10, "on": true},
    {"id": "in-cycle", "type": "solid", "name": "In cycle", "group": "a", "from": 0, "dur": 10, "on": true},
    {"id": "dangling", "type": "solid", "name": "Dangling", "group": "gone", "from": 0, "dur": 10, "on": true},
    {"id": "twin", "type": "group", "name": "Twin open", "from": 0, "dur": 10, "on": true},
    {"id": "twin", "type": "group", "name": "Twin locked", "from": 0, "dur": 10, "on": true, "lock": true},
    {"id": "in-twin", "type": "solid", "name": "In twin", "group": "twin", "from": 0, "dur": 10, "on": true}
  ]
}"##;

/// Each layer's lock in the list is the one `is_locked` gives for it, and
/// the names-only walk lists the same layers, named the same, in order.
#[test]
fn the_layer_list_locks_and_names_as_the_per_layer_queries() {
    let store = DocumentStore::new_untitled(parse(PROJECT).unwrap());
    let snapshot = store.snapshot();
    let root = snapshot.document.root().unwrap();
    let layers = list_layers(root, None);
    assert_eq!(layers.len(), 12);
    for layer in &layers {
        let loc = LayerLoc {
            comp_id: None,
            index: layer.index,
        };
        assert_eq!(
            layer.effectively_locked,
            is_locked(root, &loc),
            "{}",
            layer.name
        );
    }
    let locked: Vec<&str> = layers
        .iter()
        .filter(|l| l.effectively_locked)
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(locked, ["Outer", "Inner", "Deep", "Own lock", "Twin locked"]);
    let names: Vec<(String, String)> = layers.into_iter().map(|l| (l.id, l.name)).collect();
    assert_eq!(layer_names(root, None), names);
}
