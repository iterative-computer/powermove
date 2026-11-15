use pm_kernel::{
    Call, Caller, Command, HistoryCall, Kernel, OpenSource, Origin, Reply, Request, Selection,
    SelectionCall, WriteBody,
};
use pm_model::json::parse;

fn kernel() -> Kernel {
    let mut k = Kernel::new(Default::default());
    k.open("P", OpenSource::Untitled(parse(r#"{"id":"P","compId":"C","w":200,"h":100,"fps":30,"dur":2,"assets":{},"comps":{},"layers":[{"id":"a","type":"shape","p":{}},{"id":"b","type":"shape","p":{}}]}"#).unwrap())).unwrap();
    k
}
fn send(k: &mut Kernel, call: impl Into<Call>) -> Reply {
    k.handle("P", Request::new(Caller::native(), call))
        .unwrap()
        .reply
        .unwrap()
        .reply
}
fn layers(ids: &[&str]) -> SelectionCall {
    SelectionCall::Layers {
        ids: ids.iter().map(|id| (*id).into()).collect(),
        add: false,
    }
}
fn selection(k: &Kernel) -> Selection {
    k.project("P").unwrap().store().selection().clone()
}
fn count(k: &Kernel) -> usize {
    k.project("P").unwrap().store().history().len()
}
fn edit(k: &mut Kernel, name: &str) {
    send(
        k,
        Call::Transact {
            label: Some(name.into()),
            body: WriteBody::Commands(vec![Command::SetLayerName {
                layer_id: "a".into(),
                name: name.into(),
            }]),
            expected_revision: None,
        },
    );
}
#[test]
fn interface_selection_records_and_restores_without_a_document_edit() {
    let mut k = kernel();
    let before = selection(&k);
    let revision = k.project("P").unwrap().store().revision();
    send(&mut k, layers(&["a"]));
    let after = selection(&k);
    assert_eq!(
        k.project("P").unwrap().store().history().labels(),
        ["Selection"]
    );
    send(&mut k, HistoryCall::Undo);
    assert_eq!(selection(&k), before);
    send(&mut k, HistoryCall::Redo);
    assert_eq!(selection(&k), after);
    assert_eq!(k.project("P").unwrap().store().revision(), revision);
}
#[test]
fn unchanged_selection_records_nothing() {
    let mut k = kernel();
    send(&mut k, layers(&["a"]));
    send(&mut k, layers(&["a"]));
    assert_eq!(count(&k), 1);
}
#[test]
fn selection_mid_redo_chain_keeps_every_redo_step() {
    let mut k = kernel();
    edit(&mut k, "First");
    edit(&mut k, "Second");
    send(&mut k, HistoryCall::Undo);
    send(&mut k, HistoryCall::Undo);
    send(&mut k, layers(&["b"]));
    assert_eq!(count(&k), 2);
    send(&mut k, HistoryCall::Redo);
    send(&mut k, HistoryCall::Redo);
    assert!(!k.project("P").unwrap().store().history().can_redo());
    send(&mut k, HistoryCall::Undo);
    edit(&mut k, "Replacement");
    assert!(!k.project("P").unwrap().store().history().can_redo());
}
#[test]
fn restore_filters_missing_layers_but_restores_keys_and_channel() {
    let mut k = kernel();
    let before = Selection::from_parts(
        vec!["a".into(), "deleted".into()],
        vec!["key-a".into()],
        Some("opacity".into()),
    );
    let after = Selection::from_parts(
        vec!["b".into()],
        vec!["key-b".into()],
        Some("position.x".into()),
    );
    send(
        &mut k,
        HistoryCall::Selection {
            before,
            after: after.clone(),
        },
    );
    send(&mut k, HistoryCall::Undo);
    assert_eq!(selection(&k).layers(), ["a"]);
    assert_eq!(selection(&k).keys(), ["key-a"]);
    assert_eq!(selection(&k).channel(), Some("opacity"));
    send(&mut k, HistoryCall::Redo);
    assert_eq!(selection(&k), after);
}
#[test]
fn agent_and_mcp_selection_never_record_history() {
    for origin in [Origin::Agent, Origin::Mcp] {
        let mut k = kernel();
        let caller = Caller::realm(88, 1, origin);
        for call in [
            Call::from(layers(&["a"])),
            HistoryCall::Selection {
                before: Selection::default(),
                after: Selection::from_parts(vec!["a".into()], vec![], None),
            }
            .into(),
        ] {
            k.handle("P", Request::new(caller, call)).unwrap();
        }
        assert_eq!(count(&k), 0);
    }
}
#[test]
fn marquee_samples_record_one_completed_entry() {
    let mut k = kernel();
    let before = selection(&k);
    let document = k.project("P").unwrap().store().snapshot().doc_id;
    for ids in [vec!["a"], vec!["a", "b"], vec!["b"]] {
        send(&mut k, SelectionCall::Preview(vec![layers(&ids)]));
    }
    assert_eq!(count(&k), 0);
    send(
        &mut k,
        SelectionCall::FinishGesture {
            document,
            before: before.clone(),
        },
    );
    assert_eq!(count(&k), 1);
    send(&mut k, HistoryCall::Undo);
    assert_eq!(selection(&k), before);
    send(&mut k, HistoryCall::Redo);
    assert_eq!(selection(&k).layers(), ["b"]);
}
#[test]
fn stale_project_gesture_does_not_record() {
    let mut k = kernel();
    send(
        &mut k,
        SelectionCall::FinishGesture {
            document: u64::MAX,
            before: Selection::from_parts(vec!["a".into()], vec![], None),
        },
    );
    assert_eq!(count(&k), 0);
}
#[test]
fn selection_inside_document_edit_is_suppressed() {
    let mut k = kernel();
    send(
        &mut k,
        HistoryCall::Begin {
            label: "Edit".into(),
        },
    );
    send(&mut k, layers(&["a"]));
    assert_eq!(count(&k), 0);
}
