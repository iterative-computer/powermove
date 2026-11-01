//! DocumentStore semantics: atomic transactions, validation, undo/redo at new
//! revisions, gestures, selection by id, events, and the query mirror.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use pm_mac_core::{
    ChangeCause, Command, CommandError, DocumentStore, Origin, StoreEvent, TransactionError,
};
use pm_model::json::{Value, parse};

const PROJECT: &str = r##"{
  "id": "P", "name": "Test", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 2, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "s1", "type": "shape", "name": "Box", "from": 0, "dur": 2, "on": true, "lock": false,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "anim", "type": "shape", "name": "Animated", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [{"t": 0, "v": 0, "i": "k1"}, {"t": 1, "v": 100, "i": "k2"}], "expr": null}}},
    {"id": "expr", "type": "shape", "name": "Expression", "from": 0, "dur": 2,
     "p": {"opacity": {"v": 100, "kf": [], "expr": "time * 10"}}},
    {"id": "g", "type": "group", "name": "Group", "from": 0, "dur": 2, "lock": true, "p": {}},
    {"id": "child", "type": "shape", "name": "Child", "group": "g", "from": 0, "dur": 2,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "locked", "type": "shape", "name": "Locked", "lock": true, "from": 0, "dur": 2,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "marked", "type": "shape", "name": "Marked", "from": 0, "dur": 2,
     "locked_intent": {"opacity": {"by": "human"}},
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "bare", "type": "null", "name": "No props", "from": 0, "dur": 2, "p": {}},
    {"id": "fakegroup", "type": "shape", "name": "Not a group", "lock": true, "from": 0, "dur": 2, "p": {}},
    {"id": "member", "type": "shape", "name": "Member", "group": "fakegroup", "from": 0, "dur": 2,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}}
  ],
  "comps": {"C2": {"id": "C2", "name": "Pre", "w": 10, "h": 10, "fps": 30, "dur": 1,
    "layers": [{"id": "inner", "type": "shape", "name": "Inner", "from": 0, "dur": 1,
                "p": {"opacity": {"v": 100, "kf": [], "expr": null}}}]}}
}"##;

fn store() -> DocumentStore {
    DocumentStore::new_untitled(parse(PROJECT).unwrap())
}

fn opacity(id: &str, value: f64) -> Command {
    Command::SetStaticOpacity {
        layer_id: id.into(),
        value,
    }
}

fn events(store: &mut DocumentStore) -> Arc<Mutex<Vec<StoreEvent>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    store.subscribe(move |e| sink.lock().unwrap().push(e.clone()));
    log
}

fn name(store: &DocumentStore, id: &str) -> String {
    store
        .layers()
        .into_iter()
        .find(|l| l.id == id)
        .unwrap()
        .name
}

#[test]
fn a_command_publishes_one_revision_and_one_undo_step() {
    let mut store = store();
    let log = events(&mut store);
    let before = store.snapshot();
    assert_eq!(before.revision, 1);
    let committed = store
        .execute(opacity("s1", 40.0), Origin::Interface)
        .unwrap()
        .unwrap();
    assert_eq!(committed.revision, 2);
    assert_eq!(store.revision(), 2);
    assert_eq!(opacity_of(&store, "s1"), Some(40.0));
    // Snapshots are immutable: the old one still says 100.
    let root = before.document.root().unwrap();
    let loc = pm_mac_core::model::find_layer(root, "s1").unwrap();
    let old = pm_mac_core::model::layer_at(root, &loc).unwrap();
    assert_eq!(
        old.get("p")
            .unwrap()
            .get("opacity")
            .unwrap()
            .get("v")
            .unwrap()
            .kind(),
        &pm_model::NodeKind::Number(100.0)
    );
    assert_eq!(store.history().labels(), ["Change Opacity"]);
    assert!(store.is_dirty());
    let log = log.lock().unwrap();
    let StoreEvent::Changed(change) = &log[0] else {
        panic!("{log:?}")
    };
    assert_eq!(
        (change.base_revision, change.revision, change.ops.len()),
        (1, 2, 1)
    );
    assert!(matches!(
        change.cause,
        ChangeCause::Edit {
            origin: Origin::Interface,
            ..
        }
    ));
    assert!(
        log.iter()
            .any(|e| matches!(e, StoreEvent::DirtyChanged(true)))
    );
    assert!(log.iter().any(|e| matches!(e, StoreEvent::HistoryChanged)));
}

#[test]
fn a_command_that_changes_nothing_publishes_nothing() {
    let mut store = store();
    assert_eq!(
        store
            .execute(opacity("s1", 100.0), Origin::Interface)
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .execute(
                Command::SetLayerName {
                    layer_id: "s1".into(),
                    name: "  \u{feff} ".into()
                },
                Origin::Interface
            )
            .unwrap(),
        None,
        "a blank name keeps the old one"
    );
    assert_eq!(store.revision(), 1);
    assert!(store.history().is_empty());
    assert!(!store.is_dirty());
}

type Check = fn(&CommandError) -> bool;

#[test]
fn invalid_commands_change_nothing() {
    let cases: Vec<(Command, Origin, Check)> = vec![
        (opacity("nope", 1.0), Origin::Interface, |e| {
            matches!(e, CommandError::LayerNotFound(_))
        }),
        (opacity("locked", 1.0), Origin::Interface, |e| {
            matches!(e, CommandError::Locked { .. })
        }),
        (opacity("child", 1.0), Origin::Interface, |e| {
            matches!(e, CommandError::Locked { .. })
        }),
        (opacity("anim", 1.0), Origin::Interface, |e| {
            matches!(e, CommandError::Animated { .. })
        }),
        (opacity("expr", 1.0), Origin::Interface, |e| {
            matches!(e, CommandError::Animated { .. })
        }),
        (opacity("s1", 100.5), Origin::Interface, |e| {
            matches!(e, CommandError::InvalidValue { .. })
        }),
        (opacity("s1", -1.0), Origin::Interface, |e| {
            matches!(e, CommandError::InvalidValue { .. })
        }),
        (opacity("s1", f64::NAN), Origin::Interface, |e| {
            matches!(e, CommandError::InvalidValue { .. })
        }),
        (opacity("s1", f64::INFINITY), Origin::Interface, |e| {
            matches!(e, CommandError::InvalidValue { .. })
        }),
        (opacity("bare", 1.0), Origin::Interface, |e| {
            matches!(e, CommandError::PropertyNotFound { .. })
        }),
        (opacity("marked", 1.0), Origin::Agent, |e| {
            matches!(e, CommandError::PreservedHandEdit { .. })
        }),
        (opacity("marked", 1.0), Origin::Mcp, |e| {
            matches!(e, CommandError::PreservedHandEdit { .. })
        }),
        (
            Command::SetLayerName {
                layer_id: "locked".into(),
                name: "x".into(),
            },
            Origin::Interface,
            |e| matches!(e, CommandError::Locked { .. }),
        ),
        (
            Command::SetLayerLocked {
                layer_id: "locked".into(),
                locked: false,
            },
            Origin::Agent,
            |e| matches!(e, CommandError::Locked { .. }),
        ),
    ];
    for (command, origin, check) in cases {
        let mut store = store();
        let log = events(&mut store);
        let err = store.execute(command.clone(), origin).unwrap_err();
        let TransactionError::Command(inner) = &err else {
            panic!("{command:?}: {err:?}")
        };
        assert!(check(inner), "{command:?}: {err:?}");
        assert_eq!(store.revision(), 1, "{command:?}");
        assert!(store.history().is_empty());
        assert!(!store.is_dirty());
        assert!(log.lock().unwrap().is_empty(), "{command:?}");
    }
    let mut s = store();
    let err = s
        .execute(opacity("locked", 1.0), Origin::Interface)
        .unwrap_err();
    assert_eq!(err.to_string(), "Layer “Locked” is locked");
    let err = s
        .execute(opacity("bare", 1.0), Origin::Interface)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Property “opacity” was not found on “No props”"
    );
}

#[test]
fn lock_policy_follows_the_old_app() {
    let mut store = store();
    // A layer in a group made of a non-group layer is not inside a group.
    store
        .execute(opacity("member", 10.0), Origin::Interface)
        .unwrap()
        .unwrap();
    // A human may unlock; an agent may lock but not unlock.
    store
        .execute(
            Command::SetLayerLocked {
                layer_id: "locked".into(),
                locked: false,
            },
            Origin::Interface,
        )
        .unwrap()
        .unwrap();
    store
        .execute(opacity("locked", 10.0), Origin::Agent)
        .unwrap()
        .unwrap();
    store
        .execute(
            Command::SetLayerLocked {
                layer_id: "s1".into(),
                locked: true,
            },
            Origin::Agent,
        )
        .unwrap()
        .unwrap();
    assert!(
        store
            .execute(opacity("s1", 5.0), Origin::Interface)
            .is_err()
    );
    // A human edit may overwrite a hand-edit marker.
    store
        .execute(opacity("marked", 10.0), Origin::Interface)
        .unwrap()
        .unwrap();
    // Layers of another composition are found by id.
    store
        .execute(
            Command::SetLayerName {
                layer_id: "inner".into(),
                name: " Inside ".into(),
            },
            Origin::Interface,
        )
        .unwrap()
        .unwrap();
    let snapshot = store.snapshot();
    let inner = snapshot.layers(Some("C2"));
    assert_eq!(inner[0].name, "Inside");
    store
        .execute(
            Command::SetLayerVisible {
                layer_id: "s1".into(),
                visible: false,
            },
            Origin::Interface,
        )
        .unwrap_err(); // locked now
    store
        .execute(
            Command::SetLayerVisible {
                layer_id: "member".into(),
                visible: false,
            },
            Origin::Interface,
        )
        .unwrap()
        .unwrap();
    assert!(
        !store
            .layers()
            .iter()
            .find(|l| l.id == "member")
            .unwrap()
            .visible
    );
}

#[test]
fn transactions_are_all_or_nothing() {
    let mut store = store();
    let rename = Command::SetLayerName {
        layer_id: "s1".into(),
        name: "Renamed".into(),
    };
    let err = store
        .transact(
            "Two Things",
            Origin::Interface,
            &[rename.clone(), opacity("s1", 500.0)],
        )
        .unwrap_err();
    assert!(matches!(
        err,
        TransactionError::Command(CommandError::InvalidValue { .. })
    ));
    assert_eq!(name(&store, "s1"), "Box");
    assert_eq!(store.revision(), 1);
    // A later command sees an earlier one's effect: lock, then edit fails.
    let lock = Command::SetLayerLocked {
        layer_id: "s1".into(),
        locked: true,
    };
    assert!(
        store
            .transact("x", Origin::Interface, &[lock, opacity("s1", 1.0)])
            .is_err()
    );
    assert_eq!(store.revision(), 1);

    let done = store
        .transact(
            "Two Things",
            Origin::Interface,
            &[rename, opacity("s1", 30.0)],
        )
        .unwrap()
        .unwrap();
    assert_eq!(done.revision, 2);
    assert_eq!(store.history().labels(), ["Two Things"]);
    store.undo().unwrap();
    assert_eq!(name(&store, "s1"), "Box");
    assert_eq!(opacity_of(&store, "s1"), Some(100.0));
    // Optimistic concurrency.
    let err = store
        .transact_with(
            &[opacity("s1", 1.0)],
            pm_mac_core::TransactOptions {
                expected_revision: Some(1),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(
        err,
        TransactionError::Stale {
            expected: 1,
            current: 3
        }
    ));
}

#[test]
fn undo_and_redo_publish_new_revisions() {
    let mut store = store();
    let log = events(&mut store);
    store
        .execute(opacity("s1", 40.0), Origin::Interface)
        .unwrap();
    assert_eq!(store.undo().unwrap(), 3);
    assert_eq!(opacity_of(&store, "s1"), Some(100.0));
    assert!(!store.is_dirty());
    assert!(store.can_redo());
    assert_eq!(store.redo().unwrap(), 4);
    assert_eq!(opacity_of(&store, "s1"), Some(40.0));
    assert!(store.is_dirty());
    assert!(matches!(store.redo(), Err(TransactionError::NothingToRedo)));
    store.undo().unwrap();
    // A new edit discards redo.
    store
        .execute(
            Command::SetLayerName {
                layer_id: "s1".into(),
                name: "New".into(),
            },
            Origin::Interface,
        )
        .unwrap();
    assert!(!store.can_redo());
    assert_eq!(store.history().labels(), ["Rename Layer"]);
    let revisions: Vec<u64> = log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StoreEvent::Changed(c) => Some(c.revision),
            _ => None,
        })
        .collect();
    assert_eq!(revisions, [2, 3, 4, 5, 6], "revisions only increase");
    // Readers that lag catch up from the change log.
    let since = store.changes_since(3).unwrap();
    assert_eq!(
        since.iter().map(|c| c.revision).collect::<Vec<_>>(),
        [4, 5, 6]
    );
    assert!(matches!(since[0].cause, ChangeCause::Redo { .. }));
    assert!(store.changes_since(0).is_none());
    assert!(store.changes_since(6).unwrap().is_empty());
}

#[test]
fn a_scrub_gesture_is_one_undo_step() {
    let mut store = store();
    let g = store
        .begin_gesture("Scrub Opacity", Origin::Interface)
        .unwrap();
    for v in [90.0, 70.0, 50.0, 30.0, 10.0] {
        store
            .update_gesture(g, &[opacity("s1", v)])
            .unwrap()
            .unwrap();
        assert_eq!(
            store
                .queries()
                .evaluate(None, "s1", "opacity", 0.0)
                .unwrap(),
            Value::Number(v)
        );
    }
    assert_eq!(store.revision(), 6);
    assert!(store.history().is_empty());
    assert!(matches!(
        store.execute(opacity("s1", 1.0), Origin::Interface),
        Err(TransactionError::GestureInProgress)
    ));
    assert!(matches!(
        store.undo(),
        Err(TransactionError::GestureInProgress)
    ));
    assert!(store.begin_gesture("again", Origin::Interface).is_err());
    // A failing step changes nothing and the gesture continues.
    assert!(store.update_gesture(g, &[opacity("s1", 300.0)]).is_err());
    assert_eq!(store.revision(), 6);
    assert!(store.commit_gesture(g).unwrap().is_some());
    assert_eq!(store.history().labels(), ["Scrub Opacity"]);
    let record = &store.history().records()[0];
    assert_eq!(record.forward.len(), 1, "one patch, not one per step");
    store.undo().unwrap();
    assert_eq!(opacity_of(&store, "s1"), Some(100.0));
    assert!(!store.is_dirty());
    store.redo().unwrap();
    assert_eq!(opacity_of(&store, "s1"), Some(10.0));
    assert!(matches!(
        store.commit_gesture(g),
        Err(TransactionError::NoSuchGesture)
    ));
}

#[test]
fn a_cancelled_or_net_zero_gesture_leaves_no_step() {
    let mut store = store();
    let g = store.begin_gesture("Drag", Origin::Interface).unwrap();
    store.update_gesture(g, &[opacity("s1", 20.0)]).unwrap();
    store
        .update_gesture(
            g,
            &[Command::SetLayerName {
                layer_id: "s1".into(),
                name: "During".into(),
            }],
        )
        .unwrap();
    assert!(store.is_dirty());
    store.cancel_gesture(g).unwrap();
    assert_eq!(opacity_of(&store, "s1"), Some(100.0));
    assert_eq!(name(&store, "s1"), "Box");
    assert!(store.history().is_empty());
    assert!(!store.is_dirty());

    let g = store.begin_gesture("Drag", Origin::Interface).unwrap();
    store.update_gesture(g, &[opacity("s1", 60.0)]).unwrap();
    store.update_gesture(g, &[opacity("s1", 100.0)]).unwrap();
    assert_eq!(store.commit_gesture(g).unwrap(), None);
    assert!(store.history().is_empty());
    assert!(!store.is_dirty());
}

#[test]
fn selection_is_by_id_and_survives_reorder() {
    let mut store = store();
    let log = events(&mut store);
    store.select_layers(["s1", "nope", "child", "inner"], false);
    assert_eq!(
        store.selection().layers(),
        ["s1", "child"],
        "only open-composition layers"
    );
    assert_eq!(store.selection().primary(), Some("s1"));
    store.select_keys(["k1", "gone"]);
    store.toggle_layer("child");
    store.select_layers(["anim"], true);
    assert_eq!(store.selection().layers(), ["s1", "anim"]);

    // Reverse the layer order (one id-keyed Order operation).
    let mut project = store.snapshot().document.to_value().unwrap();
    let layers = project
        .as_object_mut()
        .unwrap()
        .get_mut("comps")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("C1")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap();
    layers.reverse();
    let reversed = project.clone();
    store
        .restore_project(project, "Reverse", Origin::Interface)
        .unwrap()
        .unwrap();
    let change = store.changes_since(1).unwrap().pop().unwrap();
    assert!(
        change
            .ops
            .iter()
            .all(|op| matches!(op, pm_model::Op::Order { .. })),
        "{:?}",
        change.ops
    );
    assert_eq!(
        store.selection().layers(),
        ["s1", "anim"],
        "ids, not indices"
    );
    assert_eq!(store.selection().keys(), ["k1"], "dead keys pruned");

    // Deleting a selected layer prunes it.
    let mut without = reversed;
    let layers = without
        .as_object_mut()
        .unwrap()
        .get_mut("comps")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("C1")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap();
    layers.retain(|l| l.get("id").and_then(Value::as_str) != Some("anim"));
    store
        .restore_project(without, "Delete", Origin::Interface)
        .unwrap()
        .unwrap();
    assert_eq!(store.selection().layers(), ["s1"]);
    assert!(store.selection().keys().is_empty());
    store.undo().unwrap();
    assert!(store.layers().iter().any(|l| l.id == "anim"));
    assert_eq!(store.selection().layers(), ["s1"], "undo does not reselect");
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, StoreEvent::SelectionChanged))
    );
}

#[test]
fn squash_combines_an_agent_run() {
    let mut store = store();
    store
        .execute(opacity("s1", 90.0), Origin::Interface)
        .unwrap();
    let mark = store.history_mark();
    for v in [80.0, 70.0, 60.0] {
        store.execute(opacity("s1", v), Origin::Agent).unwrap();
    }
    store
        .execute(
            Command::SetLayerName {
                layer_id: "s1".into(),
                name: "Agent".into(),
            },
            Origin::Agent,
        )
        .unwrap();
    assert!(store.squash_since(&mark, "Agent change").is_some());
    assert_eq!(store.history().labels(), ["Change Opacity", "Agent change"]);
    store.undo().unwrap();
    assert_eq!(opacity_of(&store, "s1"), Some(90.0));
    assert_eq!(name(&store, "s1"), "Box");
    store.redo().unwrap();
    assert_eq!(opacity_of(&store, "s1"), Some(60.0));
    assert_eq!(name(&store, "s1"), "Agent");
}

#[test]
fn snapshots_cross_threads() {
    let mut store = store();
    store
        .execute(opacity("s1", 12.0), Origin::Interface)
        .unwrap();
    let snapshot = store.snapshot();
    let ids = std::thread::spawn(move || {
        snapshot
            .layers(None)
            .into_iter()
            .map(|l| l.id)
            .collect::<Vec<_>>()
    })
    .join()
    .unwrap();
    assert_eq!(ids.len(), 10);
}

/// The query mirror against a real corpus project: values, bounds and pick
/// follow edits and undo at the committed revision.
#[test]
fn queries_follow_the_committed_revision() {
    let text =
        std::fs::read_to_string(engine_fixture("corpus/compositing-blend-modes.json")).unwrap();
    let mut store = DocumentStore::new_untitled(parse(&text).unwrap());
    let q = store.queries();
    assert_eq!(
        q.evaluate(None, "L00a", "opacity", 0.0).unwrap(),
        Value::Number(85.0)
    );
    let bounds = q
        .bounds(None, "L00a", 0.0)
        .unwrap()
        .expect("a shape has bounds");
    assert!(bounds.w > 0.0 && bounds.h > 0.0, "{bounds:?}");
    let [_, _, _, _, cx, cy] = q.world_matrix(None, "L00a", 0.0).unwrap();
    // The top layer is text: without fonts the pick must say so, not "miss".
    match q.pick(None, cx, cy, 0.0, &Default::default()) {
        Err(pm_mac_core::QueryError::NeedsFonts) => {}
        other => assert!(other.is_ok(), "{other:?}"),
    }
    assert!(matches!(
        q.evaluate(None, "missing", "opacity", 0.0),
        Err(pm_mac_core::QueryError::LayerNotFound(_))
    ));
    assert_eq!(q.layers(None).len(), store.layers().len());

    store
        .execute(opacity("L00a", 30.0), Origin::Interface)
        .unwrap();
    assert_eq!(store.queries().revision(), 2);
    assert_eq!(
        store
            .queries()
            .evaluate(None, "L00a", "opacity", 0.0)
            .unwrap(),
        Value::Number(30.0)
    );
    store.undo().unwrap();
    assert_eq!(store.queries().revision(), 3);
    assert_eq!(
        store
            .queries()
            .evaluate(None, "L00a", "opacity", 0.0)
            .unwrap(),
        Value::Number(85.0)
    );
    let world = store.queries().world_opacity(None, "L00a", 0.0).unwrap();
    assert!((0.0..=100.0).contains(&world));
}

#[test]
fn pick_binds_the_hit_to_a_stable_id() {
    let text =
        std::fs::read_to_string(engine_fixture("corpus/compositing-blend-modes.json")).unwrap();
    let mut project = parse(&text).unwrap();
    let layers = project
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap();
    layers.retain(|l| l.get("type").and_then(Value::as_str) == Some("shape"));
    let mut store = DocumentStore::new_untitled(project);
    let q = store.queries();
    let ids: Vec<String> = q.layers(None).into_iter().map(|l| l.id).collect();
    let b = q.bounds(None, &ids[0], 0.0).unwrap().unwrap();
    // Bounds are in the layer's plane; the world matrix maps them to the
    // composition.
    let to_comp = |q: &mut pm_mac_core::QueryContext, id: &str, x: f64, y: f64| {
        let [a, b, c, d, e, f] = q.world_matrix(None, id, 0.0).unwrap();
        (a * x + c * y + e, b * x + d * y + f)
    };
    let (cx, cy) = to_comp(q, &ids[0], (b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
    let hit = q.pick(None, cx, cy, 0.0, &Default::default()).unwrap();
    assert_eq!(hit.revision, 1);
    let id = hit.layer_id.expect("the centre of a visible shape hits");
    let hb = q.bounds(None, &id, 0.0).unwrap().unwrap();
    let (x0, y0) = to_comp(q, &id, hb.x0, hb.y0);
    let (x1, y1) = to_comp(q, &id, hb.x1, hb.y1);
    assert!(
        x0.min(x1) <= cx && cx <= x0.max(x1) && y0.min(y1) <= cy && cy <= y0.max(y1),
        "{hb:?}"
    );
    assert_eq!(
        q.pick(None, -5000.0, -5000.0, 0.0, &Default::default())
            .unwrap()
            .layer_id,
        None
    );
    // Hidden layers are not hit: hide the one that was, pick again.
    store
        .execute(
            Command::SetLayerVisible {
                layer_id: id.clone(),
                visible: false,
            },
            Origin::Interface,
        )
        .unwrap()
        .unwrap();
    let again = store
        .queries()
        .pick(None, cx, cy, 0.0, &Default::default())
        .unwrap();
    assert_eq!(again.revision, 2);
    assert_ne!(again.layer_id.as_deref(), Some(id.as_str()));
}
