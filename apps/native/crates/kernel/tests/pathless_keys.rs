//! PM-462: v1.2.0 stores a shape path's vertex keys without ids. They are
//! addressed by a derived id (layer, channel, place in the track) that is
//! never stored: an edit names them, edits, and stores them without ids
//! again, so v1.2.0 reads the file as it wrote it.

mod common;

use common::*;
use pm_kernel::model::{derived_key_exists, derived_key_id};
use pm_kernel::project_io::{SaveOptions, SaveTarget};
use pm_kernel::{Command, DocumentStore, KeyframeCommand, Origin};
use pm_model::canonical::project_of;
use pm_model::container::{ReadOptions, read_container};
use pm_model::json::{Value, parse, to_canonical_string};

fn key(t: f64, v: f64) -> String {
    format!(
        r#"{{"t":{t},"v":{v},"inInterp":"linear","outInterp":"linear","inEase":{{"speed":0,"influence":16.666666666666664}},"outEase":{{"speed":0,"influence":16.666666666666664}},"autoBezier":false,"continuous":false}}"#
    )
}

fn project() -> Value {
    let xs = [key(0.0, 0.0), key(1.0, 10.0), key(2.0, 20.0)].join(",");
    let ys = [key(0.0, 5.0), key(2.0, 25.0)].join(",");
    parse(&format!(
        r##"{{"id":"P","name":"Paths","compId":"C1","compName":"Main","w":200,"h":100,"fps":30,"dur":4,"bg":"#000000","assets":{{}},
        "layers":[{{"id":"L","type":"shape","name":"Outline","from":0,"dur":4,"p":{{}},
          "d":{{"shape":"path","color":"#ffffff","paths":[{{"id":"outline","p":{{"closed":{{"v":true,"kf":[],"expr":null}}}},
            "vertices":[{{"id":"v0","p":{{"x":{{"v":0,"kf":[{xs}],"expr":null}},"y":{{"v":5,"kf":[{ys}],"expr":null}}}}}}]}}]}}}}]}}"##
    ))
    .unwrap()
}

const X: &str = "g.outline.v.v0.x";

fn keys(store: &DocumentStore, axis: &str) -> Vec<Value> {
    let root = store.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "L").unwrap();
    let layer = pm_kernel::model::layer_at(root, &loc).unwrap().to_value();
    layer
        .get("d")
        .unwrap()
        .get("paths")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .get("vertices")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .get("p")
        .unwrap()
        .get(axis)
        .unwrap()
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn times(keys: &[Value]) -> Vec<f64> {
    keys.iter()
        .map(|k| k.get("t").unwrap().as_f64().unwrap())
        .collect()
}

fn run(store: &mut DocumentStore, command: KeyframeCommand) {
    store
        .execute(Command::Keyframe(command), Origin::Interface)
        .unwrap()
        .expect("changed");
}

#[test]
fn a_key_stored_without_an_id_is_edited_by_its_derived_id_and_stored_without_one() {
    let mut store = DocumentStore::new_untitled(project());
    let before = to_canonical_string(&store.root().unwrap().to_value());
    let middle = derived_key_id("L", X, 1);
    assert!(derived_key_exists(store.root().unwrap(), &middle));
    assert!(!derived_key_exists(
        store.root().unwrap(),
        &derived_key_id("L", X, 3)
    ));

    // Hold: the key changes, no key gains an id.
    run(
        &mut store,
        KeyframeCommand::SetHold {
            key_ids: vec![middle.clone()],
            hold: true,
        },
    );
    let x = keys(&store, "x");
    assert_eq!(x[1].get("outInterp").and_then(Value::as_str), Some("hold"));
    assert!(
        x.iter()
            .chain(keys(&store, "y").iter())
            .all(|k| k.get("i").is_none())
    );
    store.undo().unwrap();
    assert_eq!(
        to_canonical_string(&store.root().unwrap().to_value()),
        before
    );

    // A move past its neighbour: the key's place changes, and so does its
    // derived id; the selection follows it.
    store.select_keys([middle.clone()]);
    run(
        &mut store,
        KeyframeCommand::Move {
            key_ids: vec![middle.clone()],
            delta: 1.5,
        },
    );
    let x = keys(&store, "x");
    assert_eq!(times(&x), [0.0, 2.0, 2.5]);
    assert_eq!(x[2].get("v").and_then(Value::as_f64), Some(10.0));
    assert!(x.iter().all(|k| k.get("i").is_none()));
    assert_eq!(store.selection().keys(), [derived_key_id("L", X, 2)]);
    // Undo and Redo keep the same key selected.
    store.undo().unwrap();
    assert_eq!(store.selection().keys(), [middle.clone()]);
    store.redo().unwrap();
    assert_eq!(store.selection().keys(), [derived_key_id("L", X, 2)]);
    store.undo().unwrap();
    assert_eq!(store.selection().keys(), [middle.clone()]);
    assert_eq!(
        to_canonical_string(&store.root().unwrap().to_value()),
        before
    );

    // The value dialog and Delete address it the same way.
    run(
        &mut store,
        KeyframeCommand::Edit {
            key_id: middle.clone(),
            time: None,
            value: Some(Value::from(12.0)),
        },
    );
    assert_eq!(
        keys(&store, "x")[1].get("v").and_then(Value::as_f64),
        Some(12.0)
    );
    run(
        &mut store,
        KeyframeCommand::Delete {
            key_ids: vec![middle.clone()],
        },
    );
    assert_eq!(times(&keys(&store, "x")), [0.0, 2.0]);
    assert!(keys(&store, "x").iter().all(|k| k.get("i").is_none()));
}

#[test]
fn a_drag_of_a_key_stored_without_an_id_keeps_it_selected_through_every_step() {
    let mut store = DocumentStore::new_untitled(project());
    let first = derived_key_id("L", X, 0);
    store.select_keys([first.clone()]);
    let gesture = store
        .begin_gesture("Move Keyframes", Origin::Interface)
        .unwrap();
    for (delta, place) in [(1.5, 1), (0.5, 0), (2.5, 2)] {
        store
            .update_gesture_from_start(
                gesture,
                &[Command::Keyframe(KeyframeCommand::Move {
                    key_ids: vec![first.clone()],
                    delta,
                })],
            )
            .unwrap();
        assert_eq!(
            store.selection().keys(),
            [derived_key_id("L", X, place)],
            "delta {delta}"
        );
    }
    store.cancel_gesture(gesture).unwrap();
    assert_eq!(store.selection().keys(), [first.clone()]);
    // Committed, then undone.
    let gesture = store
        .begin_gesture("Move Keyframes", Origin::Interface)
        .unwrap();
    store
        .update_gesture_from_start(
            gesture,
            &[Command::Keyframe(KeyframeCommand::Move {
                key_ids: vec![first.clone()],
                delta: 2.5,
            })],
        )
        .unwrap();
    store.commit_gesture(gesture).unwrap();
    assert_eq!(store.selection().keys(), [derived_key_id("L", X, 2)]);
    store.undo().unwrap();
    assert_eq!(store.selection().keys(), [first]);
}

#[test]
fn a_saved_edit_keeps_every_other_byte_and_keys_without_ids() {
    let dir = TempDir::new("pathless-keys");
    let path = dir.path("paths.pmv");
    save_as(&mut DocumentStore::new_untitled(project()), &path).unwrap();
    edit_one_and_compare(&path, "L", X, 1);
}

/// Opens `path`, edits key `index` of `channel` on `layer` (stored without
/// an id) through its derived id, saves in place, reopens, and checks the
/// stored project is the same byte for byte except that key's value.
fn edit_one_and_compare(path: &std::path::Path, layer: &str, channel: &str, index: usize) {
    let stored = |path: &std::path::Path| {
        let file = std::fs::File::open(path).unwrap();
        project_of(
            &read_container(&file, ReadOptions::default())
                .unwrap()
                .document,
        )
        .clone()
    };
    let before = stored(path);
    let mut store = open_store(path);
    let id = derived_key_id(layer, channel, index);
    assert!(derived_key_exists(store.root().unwrap(), &id), "{id}");
    let edited = 123.25;
    run(
        &mut store,
        KeyframeCommand::Edit {
            key_id: id.clone(),
            time: None,
            value: Some(Value::from(edited)),
        },
    );
    common::save(&mut store, SaveTarget::InPlace, SaveOptions::default()).unwrap();
    drop(store);
    let mut after = stored(path);
    let reopened = open_store(path);
    assert!(
        derived_key_exists(reopened.root().unwrap(), &id),
        "reopened: {id}"
    );
    // The edited key, then everything else.
    let key = vertex_key(&mut after, layer, channel, index);
    assert_eq!(key.get("v").and_then(Value::as_f64), Some(edited));
    assert!(key.get("i").is_none(), "stored without an id");
    let old = vertex_key(&mut before.clone(), layer, channel, index)
        .get("v")
        .cloned()
        .unwrap();
    key.as_object_mut().unwrap().insert("v", old);
    // A save already gives a v1 video asset V2's reading of it
    // (`videoColor: "electron"`, see `project_io.rs`); v1.2.0 ignores the
    // field. Nothing else may differ.
    let mut differ = Vec::new();
    differences(&before, &after, &mut String::new(), &mut differ);
    differ.retain(|d| !(d.starts_with(".assets.") && d.ends_with(".videoColor added")));
    assert!(
        differ.is_empty(),
        "changed beyond the edited key: {differ:?}"
    );
}

/// The paths where `a` and `b` differ (no values: a real project's data
/// stays out of the output), at most 40.
fn differences(a: &Value, b: &Value, at: &mut String, out: &mut Vec<String>) {
    if out.len() >= 40 {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x.iter() {
                let len = at.len();
                at.push('.');
                at.push_str(&k.to_string_lossy());
                match y.get_key(k) {
                    Some(w) => differences(v, w, at, out),
                    None => out.push(format!("{at} removed")),
                }
                at.truncate(len);
            }
            for (k, _) in y.iter() {
                if x.get_key(k).is_none() {
                    out.push(format!("{at}.{} added", k.to_string_lossy()));
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                let len = at.len();
                at.push_str(&format!("[{i}]"));
                differences(v, w, at, out);
                at.truncate(len);
            }
        }
        _ if to_canonical_string(a) != to_canonical_string(b) => out.push(at.clone()),
        _ => {}
    }
}

/// The stored key `index` of a vertex channel `g.<path>.v.<vertex>.<axis>`.
fn vertex_key<'a>(
    project: &'a mut Value,
    layer: &str,
    channel: &str,
    index: usize,
) -> &'a mut Value {
    let rest = channel.strip_prefix("g.").unwrap();
    let (path_id, rest) = rest.split_once(".v.").unwrap();
    let (vertex_id, axis) = rest.split_once('.').unwrap();
    let by_id = |list: &'a mut Value, id: &str| -> &'a mut Value {
        list.as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
            .unwrap()
    };
    let layer = find_layer_mut(project, layer).unwrap();
    let path = by_id(
        layer
            .as_object_mut()
            .unwrap()
            .get_mut("d")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut("paths")
            .unwrap(),
        path_id,
    );
    let vertex = by_id(
        path.as_object_mut().unwrap().get_mut("vertices").unwrap(),
        vertex_id,
    );
    let kf = vertex
        .as_object_mut()
        .unwrap()
        .get_mut("p")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut(axis)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("kf")
        .unwrap();
    &mut kf.as_array_mut().unwrap()[index]
}

/// Every stored layer: the open composition's, then the others'.
fn layers_mut(project: &mut Value) -> Vec<&mut Value> {
    let m = project.as_object_mut().unwrap();
    let mut out = Vec::new();
    let mut comps = None;
    for (k, v) in m.iter_mut() {
        match k.to_string_lossy().as_str() {
            "layers" => out.extend(v.as_array_mut().unwrap().iter_mut()),
            "comps" => comps = Some(v),
            _ => {}
        }
    }
    for (_, comp) in comps
        .and_then(Value::as_object_mut)
        .into_iter()
        .flat_map(|c| c.iter_mut())
    {
        if let Some(layers) = comp.as_object_mut().and_then(|c| c.get_mut("layers")) {
            out.extend(layers.as_array_mut().unwrap().iter_mut());
        }
    }
    out
}

/// The stored layer `id`, in the open composition or another one.
fn find_layer_mut<'a>(project: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    layers_mut(project)
        .into_iter()
        .find(|l| l.get("id").and_then(Value::as_str) == Some(id))
}

/// The keys of the open composition's shape path vertices stored without
/// an id, as `(layer, channel, index)`.
fn keys_without_ids(project: &Value) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();
    let list = |v: Option<&Value>| v.and_then(Value::as_array).cloned().unwrap_or_default();
    for layer in list(project.get("layers")) {
        let layer_id = layer
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        for path in list(layer.get("d").and_then(|d| d.get("paths"))) {
            let path_id = path
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            for vertex in list(path.get("vertices")) {
                let vertex_id = vertex.get("id").and_then(Value::as_str).unwrap_or_default();
                for (axis, prop) in vertex
                    .get("p")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|p| p.iter())
                {
                    for (index, k) in list(prop.get("kf")).iter().enumerate() {
                        if k.get("i").is_none() {
                            let channel =
                                format!("g.{path_id}.v.{vertex_id}.{}", axis.to_string_lossy());
                            out.push((layer_id.clone(), channel, index));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Every key in the open composition's layers, as v1's status line counts
/// them (`kf` entries anywhere under a layer).
fn v1_key_count(project: &Value) -> usize {
    fn walk(v: &Value) -> usize {
        match v {
            Value::Object(m) => m
                .iter()
                .map(|(k, child)| {
                    let own = (k.to_string_lossy() == "kf")
                        .then(|| child.as_array().map_or(0, Vec::len))
                        .unwrap_or(0);
                    own + walk(child)
                })
                .sum(),
            Value::Array(a) => a.iter().map(walk).sum(),
            _ => 0,
        }
    }
    project.get("layers").map_or(0, walk)
}

/// The same on a real project, when `PM_PATHLESS_KEYS_PROJECT` names one
/// (a v1.2.0 file with path vertex keys stored without ids; it is copied,
/// never written): every key v1 counts is one the kernel can address, and an
/// edit of one saves everything else byte for byte.
#[test]
fn a_real_project_with_keys_without_ids() {
    let Some(source) = std::env::var_os("PM_PATHLESS_KEYS_PROJECT") else {
        eprintln!("PM_PATHLESS_KEYS_PROJECT unset: skipped");
        return;
    };
    let dir = TempDir::new("pathless-keys-real");
    let path = dir.path("project.pmv");
    std::fs::copy(&source, &path).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let project = project_of(
        &read_container(&file, ReadOptions::default())
            .unwrap()
            .document,
    )
    .clone();
    let without = keys_without_ids(&project);
    assert!(
        !without.is_empty(),
        "the project has no keys stored without ids"
    );
    let store = open_store(&path);
    let root = store.root().unwrap();
    let addressed = without
        .iter()
        .filter(|(l, c, i)| derived_key_exists(root, &derived_key_id(l, c, *i)))
        .count();
    let count = v1_key_count(&project);
    eprintln!(
        "{count} keys as v1 counts them; {} stored without ids",
        without.len()
    );
    assert_eq!(
        addressed,
        without.len(),
        "every key without an id is addressed"
    );
    drop(store);
    let (layer, channel, index) = without[0].clone();
    edit_one_and_compare(&path, &layer, &channel, index);
}
