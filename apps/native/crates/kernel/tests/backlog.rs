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
    let dir = common::TempDir::new("selection-clean");
    common::save_as(
        k.project_mut("P").unwrap().store_mut(),
        &dir.path("clean.pmv"),
    )
    .unwrap();
    assert!(!k.project("P").unwrap().store().is_dirty());
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

use pm_kernel::extensions::ExtensionCall;
use pm_kernel::{Origin, Writer};
use pm_model::patch::{Patch, PathPart};
use pm_model::{Op, Value};
fn ext(k: &mut Kernel, generation: u32, c: ExtensionCall) -> Reply {
    k.handle(
        "P",
        Request::new(Caller::realm(7, generation, Origin::Extension), c),
    )
    .unwrap()
    .reply
    .unwrap()
    .reply
}
fn begin(k: &mut Kernel, generation: u32) -> u64 {
    let Reply::Activation(ticket) = ext(
        k,
        generation,
        ExtensionCall::BeginActivation {
            extension: "test".into(),
            definitions: Value::from(format!("shader-{generation}")),
        },
    ) else {
        panic!("begin failed")
    };
    ticket
}
fn name_op(name: &str) -> Op {
    Op::Leaf(Patch::set(vec![PathPart::from("name")], Value::from(name)))
}
fn pinned(k: &mut Kernel) -> pm_kernel::extensions::ExportJob {
    let Reply::Export(job) = ext(k, 99, ExtensionCall::PinExport) else {
        panic!()
    };
    job
}

#[test]
fn activation_stages_without_a_lease_and_export_jobs_pin_one_generation() {
    let mut k = Kernel::new(Default::default());
    open(&mut k, "P");
    let t = begin(&mut k, 1);
    ext(
        &mut k,
        1,
        ExtensionCall::Stage {
            ticket: t,
            ops: vec![name_op("first")],
        },
    );
    assert!(k.project("P").unwrap().open_entry().is_none());
    assert_eq!(
        k.project("P")
            .unwrap()
            .store()
            .snapshot()
            .root()
            .unwrap()
            .get("name")
            .unwrap()
            .to_value(),
        Value::from("Fixture")
    );
    assert!(matches!(
        ext(
            &mut k,
            1,
            ExtensionCall::Finish {
                ticket: t,
                success: true
            }
        ),
        Reply::Bool(true)
    ));
    let job = pinned(&mut k);
    let t = begin(&mut k, 2);
    ext(
        &mut k,
        2,
        ExtensionCall::Stage {
            ticket: t,
            ops: vec![name_op("second")],
        },
    );
    assert!(matches!(
        ext(
            &mut k,
            2,
            ExtensionCall::Finish {
                ticket: t,
                success: true
            }
        ),
        Reply::Bool(true)
    ));
    let next = pinned(&mut k);
    assert_eq!(
        job.generations["test"].writer,
        Writer {
            realm: 7,
            generation: 1
        }
    );
    assert_eq!(job.generations["test"].definitions, Value::from("shader-1"));
    assert_eq!(
        job.snapshot.root().unwrap().get("name").unwrap().to_value(),
        Value::from("first")
    );
    assert_eq!(next.generations["test"].writer.generation, 2);
    assert_eq!(
        next.snapshot
            .root()
            .unwrap()
            .get("name")
            .unwrap()
            .to_value(),
        Value::from("second")
    );
    call(&mut k, "P", HistoryCall::Undo);
    assert_eq!(
        k.project("P")
            .unwrap()
            .store()
            .snapshot()
            .root()
            .unwrap()
            .get("name")
            .unwrap()
            .to_value(),
        Value::from("first")
    );
}

#[test]
fn activation_throw_stale_invalid_and_foreign_ticket_never_publish_partial_edits() {
    let mut k = Kernel::new(Default::default());
    open(&mut k, "P");
    let t = begin(&mut k, 1);
    ext(
        &mut k,
        1,
        ExtensionCall::Stage {
            ticket: t,
            ops: vec![name_op("discard")],
        },
    );
    assert!(matches!(
        ext(
            &mut k,
            2,
            ExtensionCall::Finish {
                ticket: t,
                success: true
            }
        ),
        Reply::Throw(_)
    ));
    assert!(matches!(
        ext(
            &mut k,
            1,
            ExtensionCall::Finish {
                ticket: t,
                success: false
            }
        ),
        Reply::Bool(false)
    ));
    let t = begin(&mut k, 2);
    ext(
        &mut k,
        2,
        ExtensionCall::Stage {
            ticket: t,
            ops: vec![name_op("stale")],
        },
    );
    edit(&mut k, "P", "foreign"); // Proves activate isn't holding the writer lease.
    assert!(matches!(
        ext(
            &mut k,
            2,
            ExtensionCall::Finish {
                ticket: t,
                success: true
            }
        ),
        Reply::Throw(_)
    ));
    let t = begin(&mut k, 3);
    let revision = k.project("P").unwrap().store().revision();
    ext(
        &mut k,
        3,
        ExtensionCall::Stage {
            ticket: t,
            ops: vec![
                name_op("partial"),
                Op::Leaf(Patch::set(
                    vec!["comps".into(), "C1".into(), "layers".into()],
                    Value::Null,
                )),
            ],
        },
    );
    assert!(matches!(
        ext(
            &mut k,
            3,
            ExtensionCall::Finish {
                ticket: t,
                success: true
            }
        ),
        Reply::Throw(_)
    ));
    assert_eq!(k.project("P").unwrap().store().revision(), revision);
    assert_eq!(
        k.project("P")
            .unwrap()
            .store()
            .snapshot()
            .root()
            .unwrap()
            .get("name")
            .unwrap()
            .to_value(),
        Value::from("Fixture")
    );
    assert!(pinned(&mut k).generations.is_empty());
}

#[test]
fn default_shared_count_budget_publishes_parked_histories_and_delivers_cleanup() {
    let actor = pm_kernel::KernelActor::spawn(Default::default());
    let h = actor.handle();
    let caller = Caller::realm(7, 1, Origin::Extension);
    let mailbox = h.register_realm(caller.writer);
    for id in ["A", "B"] {
        h.open(id, OpenSource::Untitled(parse(PROJECT).unwrap()))
            .wait()
            .unwrap()
            .unwrap();
        for handle in 0..600 {
            h.call(
                id,
                Request::new(
                    caller,
                    HistoryCall::External {
                        label: format!("{id}-{handle}"),
                        handle,
                        bytes: 256,
                        cleanup: true,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        }
    }
    assert_eq!(h.published("A").unwrap().history.entries, 400);
    assert_eq!(h.published("B").unwrap().history.entries, 600);
    let messages: Vec<_> = mailbox.try_iter().collect();
    assert_eq!(messages.len(), 200);
    for (index, message) in messages.iter().enumerate() {
        assert_eq!(message.project, "A");
        assert!(
            matches!(message.message,pm_kernel::RealmMessage::Cleanup {handle,..} if handle == index as u64)
        );
    }
}

#[test]
fn generation_only_activation_races_and_dead_realms_are_fenced() {
    let mut k = Kernel::new(Default::default());
    open(&mut k, "P");
    let older = begin(&mut k, 1);
    let newer = begin(&mut k, 2);
    assert!(matches!(
        ext(
            &mut k,
            2,
            ExtensionCall::Finish {
                ticket: newer,
                success: true
            }
        ),
        Reply::Bool(true)
    ));
    assert!(matches!(
        ext(
            &mut k,
            1,
            ExtensionCall::Finish {
                ticket: older,
                success: true
            }
        ),
        Reply::Throw(_)
    ));
    let job = pinned(&mut k);
    k.handle(
        "P",
        Request::new(Caller::realm(7, 2, Origin::Extension), Call::RealmFailed),
    )
    .unwrap();
    assert!(matches!(
        ext(&mut k, 99, ExtensionCall::PinExport),
        Reply::Throw(_)
    ));
    assert_eq!(job.generations["test"].writer.generation, 2); // Never rebound.
    let t = begin(&mut k, 3);
    k.handle(
        "P",
        Request::new(Caller::realm(7, 3, Origin::Extension), Call::RealmFailed),
    )
    .unwrap();
    assert!(matches!(
        ext(
            &mut k,
            3,
            ExtensionCall::Finish {
                ticket: t,
                success: true
            }
        ),
        Reply::Throw(_)
    ));
}
