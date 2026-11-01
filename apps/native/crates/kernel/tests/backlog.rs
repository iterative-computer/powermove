mod common;

use pm_kernel::tx::{Call, Caller, HistoryCall, Reply, Request, SelectionCall};
use pm_kernel::{Command, Kernel, OpenSource, WriteBody};
use pm_model::json::parse;

const PROJECT: &str = r##"{
  "id": "P", "name": "Fixture", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 2, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "layer0", "type": "shape", "name": "Layer", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "layer1", "type": "shape", "name": "Other", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}}
  ],
  "comps": {}
}"##;

fn open(k: &mut Kernel, id: &str) {
    k.open(id, OpenSource::Untitled(parse(PROJECT).unwrap()))
        .unwrap();
}
fn call(k: &mut Kernel, id: &str, c: impl Into<Call>) -> Reply {
    k.handle(id, Request::new(Caller::native(), c))
        .unwrap()
        .reply
        .unwrap()
        .reply
}
fn edit(k: &mut Kernel, id: &str, name: &str) {
    let layer = k.project(id).unwrap().store().layers()[0].id.clone();
    call(
        k,
        id,
        Call::Transact {
            label: Some(name.into()),
            body: WriteBody::Commands(vec![Command::SetLayerName {
                layer_id: layer,
                name: name.into(),
            }]),
            expected_revision: None,
        },
    );
}

#[test]
fn selection_history_is_ephemeral_and_preserves_redo() {
    let mut k = Kernel::new(Default::default());
    open(&mut k, "P");
    let before = k.project("P").unwrap().store().selection().clone();
    let layer = k.project("P").unwrap().store().layers()[0].id.clone();
    call(
        &mut k,
        "P",
        SelectionCall::Layers {
            ids: vec![layer.clone()],
            add: false,
        },
    );
    let after = k.project("P").unwrap().store().selection().clone();
    let revision = k.project("P").unwrap().store().revision();
    let dirty = k.project("P").unwrap().store().is_dirty();
    assert!(matches!(
        call(
            &mut k,
            "P",
            HistoryCall::Selection {
                before: before.clone(),
                after: after.clone()
            }
        ),
        Reply::Entry(Some(_))
    ));
    assert_eq!(
        k.project("P")
            .unwrap()
            .store()
            .history()
            .snapshot()
            .to_saved()
            .get("entries")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
    call(&mut k, "P", HistoryCall::Undo);
    assert_eq!(k.project("P").unwrap().store().selection(), &before);
    assert_eq!(k.project("P").unwrap().store().revision(), revision);
    assert_eq!(k.project("P").unwrap().store().is_dirty(), dirty);
    assert!(matches!(
        call(
            &mut k,
            "P",
            HistoryCall::Selection {
                before: before.clone(),
                after: after.clone()
            }
        ),
        Reply::Entry(None)
    ));
    call(&mut k, "P", HistoryCall::Redo);
    assert_eq!(k.project("P").unwrap().store().selection(), &after);
    call(
        &mut k,
        "P",
        HistoryCall::Begin {
            label: "pending".into(),
        },
    );
    assert!(matches!(
        call(
            &mut k,
            "P",
            HistoryCall::Selection {
                before: after,
                after: before
            }
        ),
        Reply::Entry(None)
    ));
}

#[test]
fn shared_budget_trims_parked_redo_then_undo_before_active_and_keeps_latest() {
    let mut k = Kernel::new(Default::default());
    open(&mut k, "A");
    open(&mut k, "B");
    for name in ["a1", "a2", "a3"] {
        edit(&mut k, "A", name);
    }
    call(&mut k, "A", HistoryCall::Undo);
    for name in ["b1", "b2", "b3"] {
        edit(&mut k, "B", name);
    }
    k.set_history_budget(4, usize::MAX);
    assert_eq!(k.project("A").unwrap().store().history().labels(), ["a2"]);
    assert_eq!(
        k.project("B").unwrap().store().history().labels(),
        ["b1", "b2", "b3"]
    );
    k.set_history_budget(1000, 0);
    for id in ["A", "B"] {
        assert_eq!(k.project(id).unwrap().store().history().len(), 1);
    }
}

