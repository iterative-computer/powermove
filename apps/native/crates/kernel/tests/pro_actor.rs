mod common;
mod pro_fixture;
use pm_kernel::pro_editing::{ProAction, ProReply, ProRequest};
use pm_kernel::{
    Call, Caller, GestureCall, HistoryCall, KernelActor, KernelHandle, OpenSource, Reply, Request,
};
use pm_model::Value;
use pro_fixture::*;
use serde_json::{Value as Json, json};
fn value(v: Json) -> Value {
    pm_model::json::parse(&v.to_string()).unwrap()
}
fn actor(layers: Vec<Json>) -> KernelActor {
    let a = KernelActor::spawn(Default::default());
    a.handle()
        .open("P", OpenSource::Untitled(root(&store(layers))))
        .wait()
        .unwrap()
        .unwrap();
    a
}
fn request(h: &KernelHandle, v: Json) -> ProRequest {
    let p = h.published("P").unwrap();
    ProRequest {
        caller: Caller::native(),
        document: p.snapshot.doc_id,
        revision: p.revision(),
        state_revision: Some(h.state().revision),
        txid: None,
        action: ProAction::decode(&value(v)).unwrap(),
    }
}
fn send(h: &KernelHandle, v: Json) -> ProReply {
    h.pro_editing("P", request(h, v)).wait().unwrap().unwrap()
}
fn good(h: &KernelHandle, v: Json) {
    if let ProReply::Document(a) | ProReply::SourceEdit { ack: a, .. } = send(h, v) {
        assert!(!matches!(a.reply, Reply::Throw(_)), "{:?}", a.reply);
    } else {
        panic!("not a document reply")
    }
}
fn tree(h: &KernelHandle) -> Value {
    h.published("P")
        .unwrap()
        .snapshot
        .document
        .root()
        .unwrap()
        .to_value()
}
fn get(h: &KernelHandle, id: &str) -> Value {
    let p = h.published("P").unwrap();
    let root = p.snapshot.document.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, id).unwrap();
    pm_kernel::model::layer_at(root, &loc).unwrap().to_value()
}
fn undo(h: &KernelHandle) {
    assert!(matches!(
        h.call_blocking("P", Request::new(Caller::native(), HistoryCall::Undo))
            .unwrap()
            .reply,
        Reply::Bool(true)
    ));
}
fn redo(h: &KernelHandle) {
    assert!(matches!(
        h.call_blocking("P", Request::new(Caller::native(), HistoryCall::Redo))
            .unwrap()
            .reply,
        Reply::Bool(true)
    ));
}
#[test]
fn ripple_roll_slide_are_published_one_step_and_roundtrip_exactly() {
    for command in [
        json!({"op":"rippleDelete","layerIds":["b"],"tracks":[{"track":0}],"linked":true}),
        json!({"op":"rippleTrim","layerId":"a","edge":"out","delta":0.375,"tracks":[{"track":0}],"linked":true}),
        json!({"op":"roll","leftId":"a","rightId":"b","delta":0.375,"linked":true}),
        json!({"op":"slide","previousId":"a","layerId":"b","nextId":"c","delta":0.375,"linked":true}),
    ] {
        let a = actor(vec![
            clip("a", 0., 4., 0),
            clip("b", 4., 4., 0),
            clip("c", 8., 4., 0),
        ]);
        let h = a.handle();
        let before = tree(&h);
        good(&h, command);
        let after = tree(&h);
        assert_ne!(before, after);
        assert_eq!(h.published("P").unwrap().history.entries, 1);
        undo(&h);
        assert_eq!(tree(&h), before);
        redo(&h);
        assert_eq!(tree(&h), after);
    }
}
#[test]
fn actor_link_select_bypass_sync_split_and_delete_use_stable_ids() {
    let a = actor(vec![clip("v", 0., 4., 0), audio("a", 0., 4., 1)]);
    let h = a.handle();
    let initial = tree(&h);
    good(
        &h,
        json!({"op":"link","videoId":"v","audioId":"a","linkId":"pair"}),
    );
    send(
        &h,
        json!({"op":"select","layerIds":["v"],"bypass":false,"add":false}),
    );
    assert_eq!(
        h.published("P").unwrap().selection.layers(),
        ids(&["v", "a"])
    );
    good(
        &h,
        json!({"op":"move","layerIds":["v"],"delta":1.,"linked":false}),
    );
    match send(&h, json!({"op":"inspect","layerIds":["v","a"]})) {
        ProReply::Inspection(v) => {
            let b = v.get("syncBadges").unwrap().as_array().unwrap();
            assert_eq!(b.len(), 2);
            assert_eq!(n(&b[0], &["offsetSeconds"]), 1.);
        }
        _ => panic!(),
    }
    undo(&h);
    good(
        &h,
        json!({"op":"split","layerId":"v","time":2.,"tails":[{"layerId":"v","tailId":"vt"},{"layerId":"a","tailId":"at"}],"linked":true,"tailLinkId":"tails"}),
    );
    assert_eq!(
        get(&h, "vt")
            .get("editLink")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str(),
        Some("tails")
    );
    good(&h, json!({"op":"delete","layerIds":["vt"],"linked":true}));
    undo(&h);
    undo(&h);
    undo(&h);
    assert_eq!(tree(&h), initial);
}
#[test]
fn stale_document_revision_locks_and_lost_ack_are_preserved() {
    let mut locked = clip("c", 8., 4., 0);
    locked["lock"] = json!(true);
    let a = actor(vec![clip("a", 0., 4., 0), clip("b", 4., 4., 0), locked]);
    let h = a.handle();
    let before = tree(&h);
    let reply = send(
        &h,
        json!({"op":"rippleDelete","layerIds":["b"],"tracks":[{"track":0}],"linked":true}),
    );
    assert!(matches!(
        reply,
        ProReply::Document(pm_kernel::Ack {
            reply: Reply::Throw(_),
            ..
        })
    ));
    assert_eq!(tree(&h), before);
    let mut r = request(
        &h,
        json!({"op":"move","layerIds":["b"],"delta":1.,"linked":true}),
    );
    r.txid = Some(40);
    let replay = r.clone();
    let stale = request(
        &h,
        json!({"op":"move","layerIds":["a"],"delta":1.,"linked":true}),
    );
    h.pro_editing("P", r).wait().unwrap().unwrap();
    let after = tree(&h);
    h.pro_editing("P", replay).wait().unwrap().unwrap();
    assert_eq!(tree(&h), after);
    assert!(matches!(
        h.pro_editing("P", stale.clone()).wait().unwrap().unwrap(),
        ProReply::Document(pm_kernel::Ack {
            reply: Reply::Throw(_),
            ..
        })
    ));
    assert_eq!(tree(&h), after);
    h.close("P").wait().unwrap();
    h.open("P", OpenSource::Untitled(before))
        .wait()
        .unwrap()
        .unwrap();
    assert!(h.pro_editing("P", stale).wait().unwrap().is_err());
}
fn marks(h: &KernelHandle) {
    send(
        h,
        json!({"op":"sourceMarks","assetId":"src","sourceIn":2.,"sourceOut":4.,"timelineIn":4.}),
    );
}
#[test]
fn monitor_marks_insert_and_overwrite_av_with_one_document_undo() {
    for mode in ["insert", "overwrite"] {
        let a = actor(vec![clip("a", 0., 4., 0), clip("b", 4., 4., 0)]);
        let h = a.handle();
        let before = tree(&h);
        marks(&h);
        assert_eq!(h.published("P").unwrap().history.entries, 0);
        good(
            &h,
            json!({"op":"sourceEdit","mode":mode,"targets":[{"media":"video","track":{"track":0},"layerId":"new-v"},{"media":"audio","track":{"track":1},"layerId":"new-a"}],"linkId":"source-pair","splits":[]}),
        );
        let after = tree(&h);
        assert_eq!(
            h.published("P").unwrap().history.undo_label.as_deref(),
            Some(if mode == "insert" {
                "Insert"
            } else {
                "Overwrite"
            })
        );
        assert_eq!(
            n(&h.published("P").unwrap().ui_state.transport, &["time"]),
            6.
        );
        assert_eq!(n(&get(&h, "new-v"), &["from"]), 4.);
        assert_eq!(n(&get(&h, "new-v"), &["dur"]), 2.);
        assert_eq!(n(&get(&h, "b"), &["from"]), 6.);
        undo(&h);
        assert_eq!(tree(&h), before);
        redo(&h);
        assert_eq!(tree(&h), after);
    }
}
#[test]
fn marks_are_partial_state_fenced_and_do_not_lose_foreign_session_fields() {
    use pm_kernel::state::{StateOp, StateTransaction};
    let a = actor(vec![]);
    let h = a.handle();
    let p = h.published("P").unwrap();
    let mut ui = p.ui_state.clone();
    ui.session = value(json!({"foreign":{"keep":true}}));
    h.state_transaction(StateTransaction {
        ops: vec![StateOp::Ui {
            project: "P".into(),
            next: ui,
        }],
        ..Default::default()
    })
    .wait()
    .unwrap()
    .unwrap();
    let revision = h.published("P").unwrap().revision();
    let stale = request(
        &h,
        json!({"op":"sourceMarks","assetId":"src","sourceIn":8.}),
    );
    marks(&h);
    assert!(h.pro_editing("P", stale).wait().unwrap().is_err());
    let p = h.published("P").unwrap();
    assert_eq!(
        p.ui_state.session.get("foreign"),
        value(json!({"foreign":{"keep":true}})).get("foreign")
    );
    assert_eq!(p.history.entries, 0);
    assert_eq!(p.revision(), revision);
    assert!(
        h.pro_editing(
            "P",
            request(
                &h,
                json!({"op":"sourceMarks","assetId":"src","sourceIn":1000.})
            )
        )
        .wait()
        .unwrap()
        .is_err()
    );
}
#[test]
fn clock_binding_shuttle_and_observation_are_actor_owned_and_ephemeral() {
    let a = actor(vec![]);
    let h = a.handle();
    let before = tree(&h);
    assert!(matches!(
        send(&h, json!({"op":"shuttle","key":"reverse"})),
        ProReply::Declined
    ));
    let doc = h.published("P").unwrap().snapshot.doc_id;
    let binding = h
        .bind_pro_clock("P", doc, Some("C1".into()), true)
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(
        send(&h, json!({"op":"shuttle","key":"j"})),
        ProReply::Declined
    ));
    send(&h, json!({"op":"shuttle","key":"reverse"}));
    send(&h, json!({"op":"shuttle","key":"j"}));
    assert_eq!(
        n(&h.published("P").unwrap().ui_state.transport, &["rate"]),
        -2.
    );
    let mut r = request(&h, json!({"op":"inspect","layerIds":[]}));
    r.action = ProAction::ObserveClock {
        binding,
        time: -0.1,
    };
    h.pro_editing("P", r).wait().unwrap().unwrap();
    assert_eq!(
        n(&h.published("P").unwrap().ui_state.transport, &["rate"]),
        0.
    );
    send(
        &h,
        json!({"op":"shuttle","key":"around","before":2.,"after":2.}),
    );
    let mut r = request(&h, json!({"op":"inspect","layerIds":[]}));
    r.action = ProAction::ObserveClock { binding, time: 2.1 };
    h.pro_editing("P", r).wait().unwrap().unwrap();
    assert_eq!(
        n(&h.published("P").unwrap().ui_state.transport, &["time"]),
        0.
    );
    assert_eq!(tree(&h), before);
    assert_eq!(h.published("P").unwrap().history.entries, 0);
    h.close("P").wait().unwrap();
    h.open("P", OpenSource::Untitled(before))
        .wait()
        .unwrap()
        .unwrap();
    assert!(
        h.bind_pro_clock("P", doc, Some("C1".into()), true)
            .wait()
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        send(&h, json!({"op":"shuttle","key":"reverse"})),
        ProReply::Declined
    ));
}
#[test]
fn structured_native_arguments_reject_ambiguous_or_untrusted_inputs() {
    for v in [
        json!({"op":"slide","previousId":"a","layerId":"b","nextId":"c","delta":1.}),
        json!({"op":"move","layerIds":["a"],"delta":1.,"linked":true,"caller":"admin"}),
        json!({"op":"shuttle","key":"reverse","available":true}),
        json!({"op":"shuttle","key":"l","before":1.}),
        json!({"op":"sourceMarks","assetId":"src","sourceIn":"2"}),
        json!({"op":"rippleDelete","layerIds":["a"],"tracks":[{"track":0.5}],"linked":true}),
        json!({"op":"observeClock","time":0.}),
    ] {
        assert!(ProAction::decode(&value(v.clone())).is_err(), "{v}");
    }
}
#[test]
fn actor_gesture_batches_still_cancel_or_commit_as_one_undo_step() {
    use pm_kernel::commands::pro_edit::ProEditCommand;
    let a = actor(vec![
        clip("a", 0., 4., 0),
        clip("b", 4., 4., 0),
        clip("c", 8., 4., 0),
    ]);
    let h = a.handle();
    let before = tree(&h);
    for cancel in [true, false] {
        let reply = h
            .call_blocking(
                "P",
                Request::new(
                    Caller::native(),
                    GestureCall::Begin {
                        label: "Slide Edit".into(),
                    },
                ),
            )
            .unwrap()
            .reply;
        let Reply::Gesture(id) = reply else {
            panic!("{reply:?}")
        };
        for delta in [0.25, 0.5] {
            let body = pm_kernel::Command::from(ProEditCommand::Slide {
                previous_id: "a".into(),
                layer_id: "b".into(),
                next_id: "c".into(),
                delta,
                linked: true,
            })
            .into();
            let r = h
                .call_blocking(
                    "P",
                    Request::new(
                        Caller::native(),
                        GestureCall::Step {
                            gesture: id,
                            body,
                            replace: true,
                        },
                    ),
                )
                .unwrap();
            assert!(!matches!(r.reply, Reply::Throw(_)), "{:?}", r.reply);
        }
        h.call_blocking(
            "P",
            Request::new(
                Caller::native(),
                if cancel {
                    Call::Gesture(GestureCall::Cancel { gesture: id })
                } else {
                    Call::Gesture(GestureCall::Commit { gesture: id })
                },
            ),
        )
        .unwrap();
        if cancel {
            assert_eq!(tree(&h), before);
        } else {
            assert_eq!(n(&get(&h, "b"), &["from"]), 4.5);
            assert_eq!(h.published("P").unwrap().history.entries, 1);
            undo(&h);
            assert_eq!(tree(&h), before);
        }
    }
}

#[test]
fn clock_tokens_and_composition_binding_refuse_stale_observations() {
    let a = actor(vec![]);
    let h = a.handle();
    let doc = h.published("P").unwrap().snapshot.doc_id;
    assert!(
        h.bind_pro_clock("P", doc, Some("missing".into()), true)
            .wait()
            .unwrap()
            .is_none()
    );
    let first = h
        .bind_pro_clock("P", doc, Some("C1".into()), true)
        .wait()
        .unwrap()
        .unwrap();
    let second = h
        .bind_pro_clock("P", doc, Some("C1".into()), true)
        .wait()
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    let mut r = request(&h, json!({"op":"inspect","layerIds":[]}));
    r.action = ProAction::ObserveClock {
        binding: first,
        time: 1.,
    };
    assert!(h.pro_editing("P", r).wait().unwrap().is_err());
    let mut r = request(&h, json!({"op":"inspect","layerIds":[]}));
    r.action = ProAction::ObserveClock {
        binding: second,
        time: 1.,
    };
    h.pro_editing("P", r).wait().unwrap().unwrap();
    h.bind_pro_clock("P", doc, Some("C1".into()), false)
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(
        send(&h, json!({"op":"shuttle","key":"reverse"})),
        ProReply::Declined
    ));
    marks(&h);
    // Changing the active composition invalidates a former viewer binding.
    let created = h
        .call_blocking(
            "P",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: None,
                    body: pm_kernel::Command::from(pm_kernel::CompCommand::Create {
                        comp_id: "other".into(),
                        settings: Default::default(),
                    })
                    .into(),
                    expected_revision: None,
                },
            ),
        )
        .unwrap();
    assert!(
        !matches!(created.reply, Reply::Throw(_)),
        "{:?}",
        created.reply
    );
    h.bind_pro_clock("P", doc, Some("C1".into()), true)
        .wait()
        .unwrap()
        .unwrap();
    h.call_blocking(
        "P",
        Request::new(
            Caller::native(),
            Call::DirectWrite {
                ops: vec![
                    pm_model::patch::Patch::set(vec!["compId".into()], Value::from("other")).into(),
                ],
            },
        ),
    )
    .unwrap();
    let source = request(
        &h,
        json!({"op":"sourceEdit","mode":"insert","targets":[{"media":"video","track":{"track":0},"layerId":"wrong-comp"}],"splits":[]}),
    );
    assert!(h.pro_editing("P", source).wait().unwrap().is_err());
    match send(&h, json!({"op":"inspect","layerIds":[]})) {
        ProReply::Inspection(v) => assert_eq!(
            v.get("clockAvailable").and_then(Value::as_bool),
            Some(false)
        ),
        _ => panic!(),
    }
}
#[test]
fn decoded_trim_slip_and_unlink_are_undoable() {
    let a = actor(vec![clip("v", 0., 4., 0), audio("a", 0., 4., 1)]);
    let h = a.handle();
    good(
        &h,
        json!({"op":"link","videoId":"v","audioId":"a","linkId":"pair"}),
    );
    for op in [
        json!({"op":"trim","layerIds":["v"],"edge":"out","delta":0.5,"linked":true}),
        json!({"op":"slip","layerIds":["v"],"delta":0.5,"linked":true}),
        json!({"op":"unlink","layerIds":["v"]}),
    ] {
        let before = tree(&h);
        good(&h, op);
        assert_ne!(tree(&h), before);
        undo(&h);
        assert_eq!(tree(&h), before);
    }
}
#[test]
fn actor_source_edit_history_survives_pmv_save_and_reopen() {
    use pm_kernel::{
        KernelConfig,
        persistence::DiskFormat,
        session::{SessionSource, ShellCall, ShellReply},
    };
    let temp = common::TempDir::new("pro-actor-save");
    let root = temp.0.canonicalize().unwrap();
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(root.join("journals")),
        ..Default::default()
    });
    let h = actor.handle();
    h.shell(ShellCall::AttachLibrary {
        profile: root.clone(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .wait()
    .unwrap()
    .unwrap();
    let source = pm_model::canonical::realize_project(pro_fixture::root(&store(vec![clip(
        "tail", 4., 4., 0,
    )])));
    let reply = h
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::New(source),
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    let ShellReply::Opened { session, .. } = reply else {
        panic!("{reply:?}")
    };
    let before = tree(&h);
    marks(&h);
    good(
        &h,
        json!({"op":"sourceEdit","mode":"insert","targets":[{"media":"video","track":{"track":0},"layerId":"new-v"},{"media":"audio","track":{"track":1},"layerId":"new-a"}],"linkId":"pair","splits":[]}),
    );
    let after = tree(&h);
    let path = root.join("source-edit.pmv");
    let save = h
        .prepare_save(session.clone(), Some(path.clone()))
        .wait()
        .unwrap()
        .unwrap();
    let completion = std::thread::spawn(move || save.run()).join().unwrap();
    h.finish_save(completion).wait().unwrap().unwrap();
    h.shell(ShellCall::Close(session)).wait().unwrap().unwrap();
    let opened = pm_kernel::open_project(&path, Default::default()).unwrap();
    h.open("P", OpenSource::File(opened))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(tree(&h), after);
    undo(&h);
    assert_eq!(tree(&h), before);
    redo(&h);
    assert_eq!(tree(&h), after);
}

#[test]
fn malformed_link_remains_inspectable_but_linked_edit_still_refuses() {
    let mut orphan = clip("orphan", 0., 4., 0);
    orphan["editLink"] = json!({"id":"missing-peer","phase":0.});
    let a = actor(vec![orphan]);
    let h = a.handle();
    let before = tree(&h);
    match send(&h, json!({"op":"inspect","layerIds":[]})) {
        ProReply::Inspection(v) => {
            assert_eq!(v.get("layers").unwrap().as_array().unwrap().len(), 1);
            assert_eq!(
                v.get("syncDiagnostics").unwrap().as_array().unwrap().len(),
                1
            );
            assert!(v.get("syncBadges").unwrap().as_array().unwrap().is_empty());
        }
        _ => panic!(),
    }
    assert!(matches!(
        send(
            &h,
            json!({"op":"move","layerIds":["orphan"],"delta":1.,"linked":true})
        ),
        ProReply::Document(pm_kernel::Ack {
            reply: Reply::Throw(_),
            ..
        })
    ));
    assert_eq!(tree(&h), before);
}

#[test]
fn source_mark_updates_preserve_unknown_fields_and_refuse_malformed_existing_payloads() {
    use pm_kernel::state::{StateOp, StateTransaction};
    let a = actor(vec![]);
    let h = a.handle();
    let mut ui = h.published("P").unwrap().ui_state.clone();
    ui.session = value(json!({"proSource":"retain-malformed-payload"}));
    h.state_transaction(StateTransaction {
        ops: vec![StateOp::Ui {
            project: "P".into(),
            next: ui,
        }],
        ..Default::default()
    })
    .wait()
    .unwrap()
    .unwrap();
    let r = request(
        &h,
        json!({"op":"sourceMarks","assetId":"src","sourceIn":1.}),
    );
    assert!(h.pro_editing("P", r).wait().unwrap().is_err());
    assert_eq!(
        h.published("P")
            .unwrap()
            .ui_state
            .session
            .get("proSource")
            .and_then(Value::as_str),
        Some("retain-malformed-payload")
    );
    let mut ui = h.published("P").unwrap().ui_state.clone();
    ui.session = value(json!({"proSource":{"opaque":{"retain":true}}}));
    h.state_transaction(StateTransaction {
        ops: vec![StateOp::Ui {
            project: "P".into(),
            next: ui,
        }],
        ..Default::default()
    })
    .wait()
    .unwrap()
    .unwrap();
    marks(&h);
    assert_eq!(
        h.published("P")
            .unwrap()
            .ui_state
            .session
            .get("proSource")
            .unwrap()
            .get("opaque"),
        value(json!({"opaque":{"retain":true}})).get("opaque")
    );
}

#[test]
fn source_refusal_retains_marks_playhead_and_document_while_success_publishes_both_facts() {
    use pm_kernel::KernelEvent;
    let a = actor(vec![]);
    let h = a.handle();
    marks(&h);
    let before = tree(&h);
    let ui = h.published("P").unwrap().ui_state.clone();
    let refused = send(
        &h,
        json!({"op":"sourceEdit","mode":"insert","targets":[],"splits":[]}),
    );
    assert!(matches!(
        refused,
        ProReply::SourceEdit {
            ack: pm_kernel::Ack {
                reply: Reply::Throw(_),
                ..
            },
            ..
        }
    ));
    assert_eq!(tree(&h), before);
    assert_eq!(h.published("P").unwrap().ui_state, ui);
    h.sync().wait().unwrap();
    let events = h.subscribe();
    good(
        &h,
        json!({"op":"sourceEdit","mode":"insert","targets":[{"media":"video","track":{"track":0},"layerId":"new"}],"splits":[]}),
    );
    let p = h.published("P").unwrap();
    assert_eq!(p.history.undo_label.as_deref(), Some("Insert"));
    assert_eq!(n(&p.ui_state.transport, &["time"]), 6.);
    let monitor = p.ui_state.session.get("proSource").unwrap();
    assert_eq!(monitor.get("sourceIn"), Some(&Value::Null));
    assert_eq!(monitor.get("sourceOut"), Some(&Value::Null));
    assert_eq!(monitor.get("timelineIn").and_then(Value::as_f64), Some(4.));
    assert!(
        matches!(events.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),KernelEvent::Published{revision,..}if revision==p.revision())
    );
    assert!(
        matches!(events.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),KernelEvent::StateChanged{change:pm_kernel::state::StateChange::Ui{project},..}if project=="P")
    );
}
