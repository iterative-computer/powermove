//! Transactions (plan v6 §3.2.1): the S5 lease stub's eight scenarios
//! ported to the real document, plus history nesting, external entries,
//! gestures and txids.

use pm_kernel::commands::layers::{AddLayer, LayerCommand};
use pm_kernel::tx::{
    Ack, Call, Caller, Direction, EditCall, EditResult, GestureCall, HistoryCall, KernelError,
    Notice, Project, RealmMessage, Reply, Request, TxId, WorkspaceCall, Writer,
};
use pm_kernel::{Command, DocumentStore, Origin, WriteBody};
use pm_model::Op;
use pm_model::json::{Value, parse};
use pm_model::patch::{Patch, PathPart};

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

fn project() -> Project {
    Project::new(DocumentStore::new_untitled(parse(PROJECT).unwrap()))
}

const PAGE: u32 = 1;
const MCP: u32 = 2;
const AGENT: u32 = 3;

fn native() -> Caller {
    Caller::native()
}
fn page() -> Caller {
    Caller::realm(PAGE, 1, Origin::Extension)
}
fn mcp() -> Caller {
    Caller::realm(MCP, 1, Origin::Mcp)
}
fn agent() -> Caller {
    Caller::realm(AGENT, 1, Origin::Agent)
}

fn call(p: &mut Project, caller: Caller, call: impl Into<Call>) -> Ack {
    p.handle(Request::new(caller, call))
        .reply
        .expect("immediate reply")
}

fn call_tx(p: &mut Project, caller: Caller, txid: u64, call: impl Into<Call>) -> Ack {
    p.handle(Request::new(caller, call).with_txid(txid))
        .reply
        .expect("immediate reply")
}

fn opacity(layer: &str, value: f64) -> WriteBody {
    WriteBody::Commands(vec![Command::SetStaticOpacity {
        layer_id: layer.into(),
        value,
    }])
}

fn rename(layer: &str, name: &str) -> WriteBody {
    WriteBody::Commands(vec![Command::SetLayerName {
        layer_id: layer.into(),
        name: name.into(),
    }])
}

fn transact(body: WriteBody) -> Call {
    Call::Transact {
        label: None,
        body,
        expected_revision: None,
    }
}

fn add_layer(id: &str) -> WriteBody {
    WriteBody::Commands(vec![
        LayerCommand::Add(Box::new(AddLayer {
            id: id.into(),
            layer_type: "shape".into(),
            comp: None,
            name: Some("New".into()),
            from: Some(0.0),
            duration: Some(1.0),
            content: Default::default(),
            properties: Default::default(),
            color: None,
            index: None,
            parent: None,
            blend: None,
            motion_blur: None,
            visible: None,
            shy: None,
            collapsed: None,
            time: 0.0,
        }))
        .into(),
    ])
}

fn delete_layer(id: &str) -> WriteBody {
    WriteBody::Commands(vec![
        LayerCommand::Delete {
            layer_ids: vec![id.into()],
        }
        .into(),
    ])
}

fn layer(p: &Project, id: &str) -> Option<Value> {
    p.store()
        .snapshot()
        .root()
        .map(|r| r.to_value())
        .and_then(|root| {
            root.get("comps")?
                .get("C1")?
                .get("layers")?
                .as_array()?
                .iter()
                .find(|l| l.get("id").and_then(Value::as_str) == Some(id))
                .cloned()
        })
}

fn opacity_of(p: &Project, id: &str) -> f64 {
    layer(p, id)
        .and_then(|l| l.get("p")?.get("opacity")?.get("v")?.as_f64())
        .expect("opacity")
}

fn name_of(p: &Project, id: &str) -> String {
    layer(p, id)
        .and_then(|l| l.get("name")?.as_str().map(str::to_owned))
        .expect("name")
}

fn entries(p: &Project) -> usize {
    p.store().history().len()
}

fn applied(a: &Ack) -> bool {
    matches!(a.reply, Reply::Applied { .. })
}

fn edit_result(a: &Ack) -> EditResult {
    match &a.reply {
        Reply::Edit(r) => r.clone(),
        other => panic!("not an EditResult: {other:?}"),
    }
}

fn notice(a: &Ack) -> Option<Notice> {
    a.notice.clone()
}

fn is_commit_with_notice(a: &Ack) -> bool {
    matches!(a.notice, Some(Notice::CommitWithNotice { .. }))
}

// ── the S5 scenarios ────────────────────────────────────────────────────────

#[test]
fn s5_lost_ack_dedup_query() {
    let mut p = project();
    let rev = p.store().revision();
    let ack = call_tx(&mut p, page(), 1, transact(opacity("layer0", 30.0)));
    let retry = call_tx(&mut p, page(), 1, transact(opacity("layer0", 30.0)));
    let queried = call(
        &mut p,
        native(),
        Call::QueryTx {
            txid: TxId {
                realm: PAGE,
                generation: 1,
                txid: 1,
            },
        },
    );
    assert_eq!(ack.revision, rev + 1);
    assert_eq!(
        retry.revision, ack.revision,
        "the retry is the stored answer"
    );
    assert_eq!(retry.txid, ack.txid);
    let Reply::Ack(Some(stored)) = queried.reply else {
        panic!("query answers the stored ack")
    };
    assert_eq!(stored.revision, ack.revision);
    assert_eq!(p.store().revision(), rev + 1, "applied once");
    assert_eq!(entries(&p), 1);
    // The key includes the generation: generation 2's txid 1 is new work.
    let fresh = call_tx(
        &mut p,
        Caller::realm(PAGE, 2, Origin::Extension),
        1,
        transact(opacity("layer0", 40.0)),
    );
    assert_eq!(fresh.revision, rev + 2);
}

#[test]
fn s5_rejection_after_optimistic_mutation_and_deleted_path_resync() {
    let mut p = project();
    let rev = p.store().revision();
    // An invalid write never reaches the authority.
    let bad = call(
        &mut p,
        page(),
        Call::DirectWrite {
            ops: vec![Op::Leaf(Patch::set(
                vec!["comps".into(), "C1".into(), "layers".into()],
                Value::from("not a list"),
            ))],
        },
    );
    assert!(matches!(
        bad.reply,
        Reply::Throw(KernelError::DirectWrite { .. })
    ));
    assert_eq!(p.store().revision(), rev);
    // The native UI deletes the layer; the page's stale flush is refused
    // and resynced with "no longer exists".
    assert!(applied(&call(
        &mut p,
        native(),
        transact(delete_layer("layer0"))
    )));
    assert!(applied(&call(
        &mut p,
        native(),
        transact(delete_layer("layer1"))
    )));
    let path: Vec<PathPart> = vec![
        "comps".into(),
        "C1".into(),
        "layers".into(),
        PathPart::Index(0.0),
        "name".into(),
    ];
    let flush = call(
        &mut p,
        page(),
        Call::DirectWrite {
            ops: vec![Op::Leaf(Patch::set(path.clone(), Value::from("late")))],
        },
    );
    let Reply::Throw(KernelError::DirectWrite { resync, .. }) = flush.reply else {
        panic!("refused: {flush:?}")
    };
    assert_eq!(resync, vec![(path, None)]);
}

#[test]
fn s5_native_drag_with_folded_page_write_cancels_selectively() {
    let mut p = project();
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "Drag".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    call(
        &mut p,
        native(),
        GestureCall::Step {
            gesture: g,
            body: opacity("layer0", 10.0),
            replace: false,
        },
    );
    let fold = call(&mut p, page(), transact(rename("layer0", "y")));
    assert!(fold.folded.is_some(), "the page write folds immediately");
    assert_eq!(name_of(&p, "layer0"), "y", "and is visible at once");
    let cancel = call(&mut p, native(), GestureCall::Cancel { gesture: g });
    assert!(matches!(
        notice(&cancel),
        Some(Notice::ForeignSplit { writes: 1, .. })
    ));
    assert_eq!(opacity_of(&p, "layer0"), 100.0, "the owner's drag reverted");
    assert_eq!(
        name_of(&p, "layer0"),
        "y",
        "the acknowledged page write stays"
    );
    assert_eq!(entries(&p), 1, "in its own undoable entry");
    assert!(matches!(
        call(&mut p, native(), HistoryCall::Undo).reply,
        Reply::Bool(true)
    ));
    assert_eq!(name_of(&p, "layer0"), "Layer");
}

#[test]
fn s5_overlapping_properties_commit_with_notice() {
    let mut p = project();
    call(
        &mut p,
        page(),
        EditCall::Begin {
            label: "Edit".into(),
        },
    );
    call(
        &mut p,
        page(),
        EditCall::Dispatch {
            body: opacity("layer0", 10.0),
        },
    );
    call(&mut p, mcp(), transact(opacity("layer0", 20.0)));
    let cancel = call(&mut p, page(), EditCall::Cancel);
    assert!(matches!(cancel.reply, Reply::Bool(true)));
    assert!(is_commit_with_notice(&cancel));
    assert_eq!(
        opacity_of(&p, "layer0"),
        20.0,
        "the acknowledged MCP value is kept"
    );
    assert_eq!(entries(&p), 1, "the whole entry is kept and undoable");
    call(&mut p, native(), HistoryCall::Undo);
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
}

#[test]
fn s5_owner_create_then_foreign_edit_keeps_the_layer() {
    let mut p = project();
    call(
        &mut p,
        page(),
        EditCall::Begin {
            label: "Add".into(),
        },
    );
    call(
        &mut p,
        page(),
        EditCall::Dispatch {
            body: add_layer("new"),
        },
    );
    call(&mut p, agent(), transact(opacity("new", 7.0)));
    let cancel = call(&mut p, page(), EditCall::Cancel);
    assert!(is_commit_with_notice(&cancel));
    assert_eq!(opacity_of(&p, "new"), 7.0);
}

#[test]
fn s5_history_cancel_keeps_edits_and_drops_the_entry() {
    let mut p = project();
    call(
        &mut p,
        page(),
        HistoryCall::Begin {
            label: "keep".into(),
        },
    );
    call(
        &mut p,
        page(),
        Call::DirectWrite {
            ops: rename_ops("kept"),
        },
    );
    let cancel = call(&mut p, page(), HistoryCall::Cancel);
    assert!(matches!(cancel.reply, Reply::Unit));
    assert_eq!(name_of(&p, "layer0"), "kept");
    assert_eq!(entries(&p), 0);
    assert!(p.open_entry().is_none());
}

#[test]
fn s5_open_begin_never_blocks_live_mutate_refuses_owner_failure_seals() {
    let mut p = project();
    let rev = p.store().revision();
    call(
        &mut p,
        page(),
        EditCall::Begin {
            label: "Live".into(),
        },
    );
    let refused = call(
        &mut p,
        mcp(),
        EditCall::Mutate {
            label: "structure".into(),
            ops: vec![],
        },
    );
    assert_eq!(
        edit_result(&refused),
        EditResult {
            ok: false,
            message: Some("Finish the active source edit before changing structure".into()),
            revision: rev,
        }
    );
    call(
        &mut p,
        page(),
        EditCall::Dispatch {
            body: opacity("layer0", 1.0),
        },
    );
    let folded = call(&mut p, mcp(), transact(rename("layer0", "mcp")));
    call(&mut p, native(), transact(opacity("layer0", 3.0)));
    call(&mut p, agent(), transact(rename("layer1", "agent")));
    assert!(folded.folded.is_some());
    assert_eq!(p.store().revision(), rev + 4, "nobody waited");
    let failed = call(&mut p, page(), Call::RealmFailed);
    assert!(matches!(
        notice(&failed),
        Some(Notice::Sealed { writes: 4, .. })
    ));
    assert_eq!(entries(&p), 1);
    assert!(p.open_entry().is_none());
    assert_eq!(opacity_of(&p, "layer0"), 3.0);
    assert_eq!(name_of(&p, "layer0"), "mcp");
    assert_eq!(name_of(&p, "layer1"), "agent");
    // Never-acknowledged work from the dead generation is refused.
    let late = call(&mut p, page(), transact(opacity("layer0", 50.0)));
    assert!(matches!(
        late.reply,
        Reply::Throw(KernelError::StaleGeneration)
    ));
    assert!(matches!(
        call(&mut p, native(), HistoryCall::Undo).reply,
        Reply::Bool(true)
    ));
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
}

#[test]
fn s5_owner_delete_then_foreign_recreate_keeps_it() {
    let mut p = project();
    call(
        &mut p,
        page(),
        EditCall::Begin {
            label: "Delete".into(),
        },
    );
    call(
        &mut p,
        page(),
        EditCall::Dispatch {
            body: delete_layer("layer0"),
        },
    );
    call(&mut p, agent(), transact(add_layer("layer0")));
    call(&mut p, agent(), transact(opacity("layer0", 9.0)));
    let cancel = call(&mut p, page(), EditCall::Cancel);
    assert!(is_commit_with_notice(&cancel));
    assert_eq!(opacity_of(&p, "layer0"), 9.0);
}

fn rename_ops(name: &str) -> Vec<Op> {
    vec![Op::Leaf(Patch::set(
        vec![
            "comps".into(),
            "C1".into(),
            "layers".into(),
            PathPart::Index(0.0),
            "name".into(),
        ],
        Value::from(name),
    ))]
}

// ── history semantics ───────────────────────────────────────────────────────

#[test]
fn history_begin_nests_and_do_folds() {
    let mut p = project();
    let Reply::Before(Some(outer)) = call(
        &mut p,
        page(),
        HistoryCall::Begin {
            label: "outer".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    let Reply::Before(Some(inner)) = call(
        &mut p,
        page(),
        HistoryCall::Begin {
            label: "inner".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    assert!(
        std::sync::Arc::ptr_eq(&outer.document, &inner.document),
        "nested begin reuses the snapshot"
    );
    let done = call(
        &mut p,
        page(),
        HistoryCall::Do {
            label: "folded".into(),
            body: rename("layer0", "folded"),
        },
    );
    assert!(done.folded.is_some());
    assert!(matches!(
        call(&mut p, page(), HistoryCall::Commit { label: None }).reply,
        Reply::Bool(false)
    ));
    assert!(matches!(
        call(&mut p, page(), HistoryCall::Commit { label: None }).reply,
        Reply::Bool(true)
    ));
    assert_eq!(entries(&p), 1);
    assert!(matches!(
        call(&mut p, page(), HistoryCall::Undo).reply,
        Reply::Bool(true)
    ));
    assert_eq!(name_of(&p, "layer0"), "Layer");
    assert!(matches!(
        call(&mut p, page(), HistoryCall::Redo).reply,
        Reply::Bool(true)
    ));
    assert_eq!(name_of(&p, "layer0"), "folded");
    // A throwing realm callback: Begin, write, Rollback.
    call(
        &mut p,
        page(),
        HistoryCall::Begin {
            label: "throw".into(),
        },
    );
    call(&mut p, page(), transact(rename("layer0", "bad")));
    call(&mut p, page(), HistoryCall::Rollback);
    assert_eq!(name_of(&p, "layer0"), "folded", "throw rolls back");
    assert!(matches!(
        call(&mut p, page(), HistoryCall::Commit { label: None }).reply,
        Reply::Bool(false)
    ));
    assert_eq!(entries(&p), 1);
}

#[test]
fn unchanged_commit_records_nothing() {
    let mut p = project();
    call(
        &mut p,
        page(),
        HistoryCall::Begin {
            label: "noop".into(),
        },
    );
    call(&mut p, page(), transact(opacity("layer0", 5.0)));
    call(&mut p, page(), transact(opacity("layer0", 100.0)));
    assert!(matches!(
        call(&mut p, page(), HistoryCall::Commit { label: None }).reply,
        Reply::Bool(false)
    ));
    assert_eq!(entries(&p), 0);
}

#[test]
fn rollback_by_a_foreign_caller_keeps_the_owner_writes() {
    let mut p = project();
    call(
        &mut p,
        page(),
        HistoryCall::Begin {
            label: "page".into(),
        },
    );
    call(&mut p, page(), transact(opacity("layer0", 10.0)));
    // MCP's own history.do callback throws inside the page's entry.
    call(
        &mut p,
        mcp(),
        HistoryCall::Begin {
            label: "mcp do".into(),
        },
    );
    call(&mut p, mcp(), transact(rename("layer0", "half")));
    let rolled = call(&mut p, mcp(), HistoryCall::Rollback);
    assert!(matches!(notice(&rolled), Some(Notice::ForeignSplit { .. })));
    assert_eq!(
        name_of(&p, "layer0"),
        "Layer",
        "the thrower's write is gone"
    );
    assert_eq!(
        opacity_of(&p, "layer0"),
        10.0,
        "the page's acknowledged write stays"
    );
    assert_eq!(entries(&p), 1);
}

#[test]
fn edit_api_return_forms() {
    let mut p = project();
    let rev = p.store().revision();
    assert!(!edit_result(&call(&mut p, page(), EditCall::Commit { label: None })).ok);
    assert!(matches!(
        call(&mut p, page(), EditCall::Cancel).reply,
        Reply::Bool(false)
    ));
    let empty = call(
        &mut p,
        page(),
        EditCall::Apply {
            body: WriteBody::Commands(vec![]),
            base_revision: None,
            label: None,
        },
    );
    assert_eq!(
        edit_result(&empty).message.as_deref(),
        Some("No source edits supplied")
    );
    let stale = call(
        &mut p,
        page(),
        EditCall::Apply {
            body: opacity("layer0", 1.0),
            base_revision: Some(999),
            label: None,
        },
    );
    assert_eq!(
        edit_result(&stale).message,
        Some(format!(
            "Project changed: expected revision 999, found {rev}"
        ))
    );
    let non_tree = call(
        &mut p,
        page(),
        EditCall::Mutate {
            label: "bad".into(),
            ops: vec![Op::Leaf(Patch::set(vec!["name".into()], Value::from("x")))],
        },
    );
    assert!(!edit_result(&non_tree).ok);
    let noop = call(
        &mut p,
        page(),
        EditCall::Mutate {
            label: "noop".into(),
            ops: vec![],
        },
    );
    assert_eq!(
        edit_result(&noop).message.as_deref(),
        Some("No source changes")
    );
    call(
        &mut p,
        page(),
        EditCall::Begin {
            label: "live".into(),
        },
    );
    assert!(matches!(
        call(
            &mut p,
            page(),
            EditCall::Begin {
                label: "nested".into()
            }
        )
        .reply,
        Reply::Throw(KernelError::SourceEditActive)
    ));
    // history.begin inside a scoped (edit) entry returns null.
    assert!(matches!(
        call(
            &mut p,
            page(),
            HistoryCall::Begin {
                label: "raw".into()
            }
        )
        .reply,
        Reply::Before(None)
    ));
    call(&mut p, page(), HistoryCall::Commit { label: None });
    // Empty cancel keeps the project reference (no revision).
    let before = p.store().snapshot();
    assert!(matches!(
        call(&mut p, page(), EditCall::Cancel).reply,
        Reply::Bool(true)
    ));
    assert!(std::sync::Arc::ptr_eq(
        &before.document,
        &p.store().snapshot().document
    ));
    assert_eq!(p.store().revision(), rev);
    assert_eq!(entries(&p), 0);
}

#[test]
fn edit_cancel_reverts_and_restores_selection() {
    let mut p = project();
    p.store_mut().select_layers(["layer1"], false);
    call(
        &mut p,
        page(),
        EditCall::Begin {
            label: "changed".into(),
        },
    );
    call(
        &mut p,
        page(),
        EditCall::Dispatch {
            body: add_layer("added"),
        },
    );
    p.store_mut().select_layers(["added"], false);
    call(&mut p, page(), EditCall::Cancel);
    assert!(layer(&p, "added").is_none());
    assert_eq!(entries(&p), 0);
    assert_eq!(p.store().selection().layers(), ["layer1"]);
}

fn mutate_begin(p: &mut Project, label: &str) -> u64 {
    let ack = call(
        p,
        page(),
        EditCall::MutateBegin {
            label: label.into(),
        },
    );
    match ack.reply {
        Reply::Mutation(lease) => lease,
        other => panic!("not a lease: {other:?}"),
    }
}

fn mutate_finish(p: &mut Project, lease: u64) -> EditResult {
    edit_result(&call(
        p,
        page(),
        EditCall::MutateFinish {
            lease,
            ops: vec![],
            label: None,
        },
    ))
}

/// A realm's structural edit is a lease: one begun inside another takes
/// back only what was written since it began, and the outer one is still
/// one step.
#[test]
fn a_realms_structural_edit_inside_another_has_its_own_rollback() {
    let mut p = project();
    let outer = mutate_begin(&mut p, "Outer");
    call(&mut p, page(), transact(add_layer("a")));
    let inner = mutate_begin(&mut p, "Inner");
    call(&mut p, page(), transact(add_layer("b")));
    let aborted = call(&mut p, page(), EditCall::MutateAbort { lease: inner });
    assert!(matches!(aborted.reply, Reply::Bool(true)));
    assert!(layer(&p, "b").is_none() && layer(&p, "a").is_some());
    assert!(p.open_entry().is_some(), "the outer edit is still open");
    // The aborted lease is spent; another realm cannot end this one's.
    assert!(!mutate_finish(&mut p, inner).ok);
    let foreign = call(&mut p, mcp(), EditCall::MutateAbort { lease: outer });
    assert!(matches!(foreign.reply, Reply::Bool(false)));
    call(&mut p, page(), transact(add_layer("c")));
    assert!(mutate_finish(&mut p, outer).ok);
    assert!(p.open_entry().is_none());
    assert_eq!(entries(&p), 1, "one step");
    call(&mut p, native(), HistoryCall::Undo);
    assert!(layer(&p, "a").is_none() && layer(&p, "c").is_none());
}

/// An edit never finished or aborted (its realm lost the reply) goes with
/// its writes when the edit around it ends; an edit whose entry was closed
/// under it says so.
#[test]
fn a_structural_edit_left_open_or_closed_under_it_is_never_reported_done() {
    let mut p = project();
    let outer = mutate_begin(&mut p, "Outer");
    call(&mut p, page(), transact(add_layer("a")));
    mutate_begin(&mut p, "Lost");
    call(&mut p, page(), transact(add_layer("b")));
    assert!(mutate_finish(&mut p, outer).ok);
    assert!(layer(&p, "a").is_some() && layer(&p, "b").is_none());
    assert!(p.open_entry().is_none());

    let closed = mutate_begin(&mut p, "Closed");
    call(&mut p, page(), HistoryCall::Rollback);
    let result = mutate_finish(&mut p, closed);
    assert!(!result.ok);
    assert_eq!(
        result.message.as_deref(),
        Some("This edit was closed before it finished; its last changes were not applied")
    );
}

#[test]
fn index_paths_rebase_by_id_and_cancel_preserves_foreign_insert() {
    let mut p = project();
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "Drag".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    call(
        &mut p,
        native(),
        GestureCall::Step {
            gesture: g,
            body: opacity("layer1", 10.0),
            replace: false,
        },
    );
    // The agent inserts a layer above: the drag's index paths now point at
    // a different layer, so a by-index revert would hit the wrong one.
    call(&mut p, agent(), transact(add_layer("top")));
    let cancel = call(&mut p, native(), GestureCall::Cancel { gesture: g });
    assert!(matches!(notice(&cancel), Some(Notice::ForeignSplit { .. })));
    assert_eq!(opacity_of(&p, "layer1"), 100.0);
    assert!(layer(&p, "top").is_some());
    call(&mut p, native(), HistoryCall::Undo);
    assert!(layer(&p, "top").is_none());
    assert_eq!(opacity_of(&p, "layer1"), 100.0);
    call(&mut p, native(), HistoryCall::Redo);
    assert!(layer(&p, "top").is_some());
    assert_eq!(opacity_of(&p, "layer1"), 100.0);
}

// ── gestures ────────────────────────────────────────────────────────────────

#[test]
fn gesture_replace_steps_supersede_and_commit_one_entry() {
    let mut p = project();
    let rev = p.store().revision();
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "Scrub".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    for v in [10.0, 20.0, 30.0] {
        call(
            &mut p,
            native(),
            GestureCall::Step {
                gesture: g,
                body: opacity("layer0", v),
                replace: true,
            },
        );
    }
    assert_eq!(opacity_of(&p, "layer0"), 30.0);
    assert_eq!(p.store().revision(), rev + 3, "one live revision per step");
    assert!(matches!(
        call(&mut p, native(), GestureCall::Commit { gesture: g }).reply,
        Reply::Bool(true)
    ));
    assert_eq!(entries(&p), 1);
    call(&mut p, native(), HistoryCall::Undo);
    assert_eq!(
        opacity_of(&p, "layer0"),
        100.0,
        "one step from start to end"
    );
}

#[test]
fn gesture_replace_never_clobbers_a_foreign_write() {
    let mut p = project();
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "Scrub".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    let step = |p: &mut Project, v: f64| {
        call(
            p,
            native(),
            GestureCall::Step {
                gesture: g,
                body: opacity("layer0", v),
                replace: true,
            },
        )
    };
    step(&mut p, 10.0);
    call(&mut p, mcp(), transact(opacity("layer0", 55.0)));
    step(&mut p, 20.0);
    assert_eq!(opacity_of(&p, "layer0"), 20.0);
    let cancel = call(&mut p, native(), GestureCall::Cancel { gesture: g });
    assert!(
        is_commit_with_notice(&cancel),
        "the MCP value sits between owner steps"
    );
}

#[test]
fn gesture_ids_are_owned() {
    let mut p = project();
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "Drag".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    let stolen = call(&mut p, page(), GestureCall::Commit { gesture: g });
    assert!(matches!(
        stolen.reply,
        Reply::Throw(KernelError::NoSuchGesture)
    ));
}

// ── external entries ────────────────────────────────────────────────────────

fn external(p: &mut Project, caller: Caller, handle: u64, cleanup: bool) -> u64 {
    match call(
        p,
        caller,
        HistoryCall::External {
            label: "external".into(),
            handle,
            bytes: 0,
            cleanup,
        },
    )
    .reply
    {
        Reply::Entry(Some(id)) => id,
        other => panic!("{other:?}"),
    }
}

#[test]
fn external_entries_push_independently_and_undo_inline() {
    let mut p = project();
    call(
        &mut p,
        page(),
        HistoryCall::Begin {
            label: "pending".into(),
        },
    );
    call(&mut p, page(), transact(rename("layer0", "pending")));
    external(&mut p, page(), 7, true);
    assert!(p.open_entry().is_some(), "independent of the pending entry");
    assert_eq!(p.store().history().records()[0].bytes, 256);
    // The page itself is blocked in history.undo(): run H inline.
    let Reply::Continuation(c) = call(&mut p, page(), HistoryCall::Undo).reply else {
        panic!()
    };
    assert_eq!((c.handle, c.direction), (7, Direction::Undo));
    assert!(!p.store().history().can_undo(), "the cursor moved first");
    let done = call(
        &mut p,
        page(),
        Call::ExternalDone {
            ticket: c.ticket,
            ok: true,
            message: None,
        },
    );
    assert!(matches!(done.reply, Reply::Bool(true)));
    // A throwing closure: the cursor stays moved, the throw propagates.
    let Reply::Continuation(c) = call(&mut p, page(), HistoryCall::Redo).reply else {
        panic!()
    };
    let failed = call(
        &mut p,
        page(),
        Call::ExternalDone {
            ticket: c.ticket,
            ok: false,
            message: Some("synthetic external failure".into()),
        },
    );
    assert!(matches!(
        failed.reply,
        Reply::Throw(KernelError::ExternalFailed(_))
    ));
    assert!(p.store().history().can_undo() && !p.store().history().can_redo());
    // Not exported (closure entries never persisted).
    let saved = p.store().history().snapshot().to_saved();
    assert_eq!(
        saved.get("entries").and_then(Value::as_array).map(Vec::len),
        Some(0)
    );
}

#[test]
fn external_undo_from_another_realm_is_deferred_until_the_realm_runs_it() {
    let mut p = project();
    external(&mut p, page(), 9, false);
    let handled = p.handle(Request::new(native(), HistoryCall::Undo).with_txid(5));
    assert!(handled.reply.is_none());
    let ticket = handled.deferred.unwrap();
    let outbox = p.take_outbox();
    let [
        RealmMessage::RunExternal {
            writer,
            continuation,
        },
    ] = outbox.as_slice()
    else {
        panic!("{outbox:?}")
    };
    assert_eq!(
        *writer,
        Writer {
            realm: PAGE,
            generation: 1
        }
    );
    assert_eq!(continuation.ticket, ticket);
    let done = p.handle(Request::new(
        page(),
        Call::ExternalDone {
            ticket,
            ok: true,
            message: None,
        },
    ));
    assert!(matches!(done.reply.unwrap().reply, Reply::Unit));
    let [(t, ack)] = done.resolved.as_slice() else {
        panic!()
    };
    assert_eq!(*t, ticket);
    assert!(matches!(ack.reply, Reply::Bool(true)));
    assert_eq!(ack.txid.map(|t| t.txid), Some(5));
}

#[test]
fn failed_realm_externals_become_barriers_and_pending_runs_fail() {
    let mut p = project();
    call(&mut p, native(), transact(opacity("layer0", 1.0)));
    external(&mut p, page(), 1, false);
    let handled = p.handle(Request::new(native(), HistoryCall::Undo));
    let ticket = handled.deferred.unwrap();
    let failed = p.handle(Request::new(page(), Call::RealmFailed));
    let [(t, ack)] = failed.resolved.as_slice() else {
        panic!()
    };
    assert_eq!(*t, ticket);
    assert!(matches!(
        ack.reply,
        Reply::Throw(KernelError::ExternalFailed(_))
    ));
    // Redo of the dead realm's entry: a barrier, the cursor stays.
    let redo = call(&mut p, native(), HistoryCall::Redo);
    assert!(matches!(redo.reply, Reply::Bool(false)));
    assert!(matches!(notice(&redo), Some(Notice::Barrier { .. })));
    // Undo of the document entry below still works.
    assert!(matches!(
        call(&mut p, native(), HistoryCall::Undo).reply,
        Reply::Bool(true)
    ));
}

#[test]
fn retired_externals_ask_their_realm_for_cleanup() {
    let mut p = project();
    external(&mut p, page(), 3, true);
    external(&mut p, page(), 4, false);
    call(&mut p, native(), HistoryCall::Clear);
    assert_eq!(
        p.take_outbox(),
        vec![RealmMessage::Cleanup {
            writer: Writer {
                realm: PAGE,
                generation: 1
            },
            handle: 3
        }]
    );
}

#[test]
fn workspace_mutate_returns_or_throws_keeping_partial_writes() {
    let mut p = project();
    let next = parse(r#"{"name":"W"}"#).unwrap();
    let ok = call(
        &mut p,
        page(),
        WorkspaceCall::Mutate {
            next: next.clone(),
            threw: None,
        },
    );
    assert!(matches!(ok.reply, Reply::Workspace(ref v) if *v == next));
    let partial = parse(r#"{"name":"Partial"}"#).unwrap();
    let threw = call(
        &mut p,
        page(),
        WorkspaceCall::Mutate {
            next: partial.clone(),
            threw: Some("synthetic workspace failure".into()),
        },
    );
    assert!(matches!(
        threw.reply,
        Reply::Throw(KernelError::Callback(_))
    ));
    assert_eq!(p.workspace(), &partial);
    assert_eq!(entries(&p), 0, "no document history");
}

#[test]
fn transact_refuses_stale_expected_revision() {
    let mut p = project();
    let stale = call(
        &mut p,
        mcp(),
        Call::Transact {
            label: None,
            body: opacity("layer0", 2.0),
            expected_revision: Some(99),
        },
    );
    assert!(matches!(
        stale.reply,
        Reply::Throw(KernelError::Transaction(
            pm_kernel::TransactionError::Stale { .. }
        ))
    ));
}

#[test]
fn nested_numeric_string_indices_rebase_through_both_reorders() {
    let mut p = project();
    call(&mut p,native(),transact(WriteBody::Ops(vec![Op::Leaf(Patch::set(vec!["future".into()],parse(r#"[{"id":"a","children":[{"id":"x","name":"old"},{"id":"y","name":"keep"}]},{"id":"b","children":[]}]"#).unwrap()))])));
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "nested".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    call(
        &mut p,
        native(),
        GestureCall::Step {
            gesture: g,
            body: WriteBody::Ops(vec![Op::Leaf(Patch::set(
                vec![
                    "future".into(),
                    "0".into(),
                    "children".into(),
                    "0".into(),
                    "name".into(),
                ],
                Value::from("owner"),
            ))]),
            replace: false,
        },
    );
    call(
        &mut p,
        agent(),
        transact(WriteBody::Ops(vec![
            Op::Order {
                path: vec!["future".into()],
                ids: vec!["b".into(), "a".into()],
            },
            Op::Order {
                path: vec!["future".into(), 1.into(), "children".into()],
                ids: vec!["y".into(), "x".into()],
            },
        ])),
    );
    let cancel = call(&mut p, native(), GestureCall::Cancel { gesture: g });
    assert!(matches!(notice(&cancel), Some(Notice::ForeignSplit { .. })));
    let after = p
        .store()
        .snapshot()
        .root()
        .unwrap()
        .get("future")
        .unwrap()
        .to_value();
    assert_eq!(after,parse(r#"[{"id":"b","children":[]},{"id":"a","children":[{"id":"y","name":"keep"},{"id":"x","name":"old"}]}]"#).unwrap());
    call(&mut p, native(), HistoryCall::Undo);
    call(&mut p, native(), HistoryCall::Redo);
    assert_eq!(
        p.store()
            .snapshot()
            .root()
            .unwrap()
            .get("future")
            .unwrap()
            .to_value(),
        after
    );
}

#[test]
fn cancel_restores_deleted_element_next_to_its_id_neighbour() {
    let mut p = project();
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "delete".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    call(
        &mut p,
        native(),
        GestureCall::Step {
            gesture: g,
            body: delete_layer("layer1"),
            replace: false,
        },
    );
    call(&mut p, agent(), transact(add_layer("top")));
    let cancel = call(&mut p, native(), GestureCall::Cancel { gesture: g });
    assert!(matches!(notice(&cancel), Some(Notice::ForeignSplit { .. })));
    let ids: Vec<_> = p.store().layers().into_iter().map(|l| l.id).collect();
    assert_eq!(ids, ["top", "layer0", "layer1"]);
    call(&mut p, native(), HistoryCall::Undo);
    assert_eq!(p.store().layers().len(), 2);
    assert!(layer(&p, "layer1").is_some());
}

#[test]
fn owner_reorder_cancel_rebases_foreign_history_for_undo_and_redo() {
    let mut p = project();
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "order".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    call(
        &mut p,
        native(),
        GestureCall::Step {
            gesture: g,
            body: WriteBody::Ops(vec![Op::Order {
                path: vec!["comps".into(), "C1".into(), "layers".into()],
                ids: vec!["layer1".into(), "layer0".into()],
            }]),
            replace: false,
        },
    );
    call(&mut p, agent(), transact(opacity("layer1", 22.0)));
    let cancel = call(&mut p, native(), GestureCall::Cancel { gesture: g });
    assert!(matches!(notice(&cancel), Some(Notice::ForeignSplit { .. })));
    assert_eq!(p.store().layers()[0].id, "layer0");
    assert_eq!(opacity_of(&p, "layer1"), 22.0);
    call(&mut p, native(), HistoryCall::Undo);
    assert_eq!(opacity_of(&p, "layer1"), 100.0);
    call(&mut p, native(), HistoryCall::Redo);
    assert_eq!(opacity_of(&p, "layer1"), 22.0);
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
}

#[test]
fn changed_unkeyed_array_still_commits_with_notice() {
    let mut p = project();
    call(
        &mut p,
        native(),
        transact(WriteBody::Ops(vec![Op::Leaf(Patch::set(
            vec!["future".into()],
            parse(r#"[{"name":"a"},{"name":"b"}]"#).unwrap(),
        ))])),
    );
    let Reply::Gesture(g) = call(
        &mut p,
        native(),
        GestureCall::Begin {
            label: "unkeyed".into(),
        },
    )
    .reply
    else {
        panic!()
    };
    call(
        &mut p,
        native(),
        GestureCall::Step {
            gesture: g,
            body: WriteBody::Ops(vec![Op::Leaf(Patch::set(
                vec!["future".into(), 0.into(), "name".into()],
                Value::from("owner"),
            ))]),
            replace: false,
        },
    );
    call(
        &mut p,
        agent(),
        transact(WriteBody::Ops(vec![Op::Leaf(Patch::set(
            vec!["future".into()],
            parse(r#"[{"name":"foreign"},{"name":"owner"},{"name":"b"}]"#).unwrap(),
        ))])),
    );
    let cancel = call(&mut p, native(), GestureCall::Cancel { gesture: g });
    assert!(is_commit_with_notice(&cancel));
    assert_eq!(
        p.store()
            .snapshot()
            .root()
            .unwrap()
            .get("future")
            .unwrap()
            .to_value(),
        parse(r#"[{"name":"foreign"},{"name":"owner"},{"name":"b"}]"#).unwrap()
    );
}

#[test]
fn document_guard_fences_selection_and_every_gesture_lifecycle_call() {
    let old_document = project().store().doc_id();
    let mut replacement = project(); // same project/layer IDs, another lifetime
    let document = replacement.store().doc_id();
    let original = replacement.store().snapshot().document.to_value();
    for call in [
        Call::Selection(pm_kernel::SelectionCall::Layers {
            ids: vec!["layer0".into()],
            add: false,
        }),
        Call::Gesture(GestureCall::Begin {
            label: "Stale".into(),
        }),
    ] {
        let ack = replacement
            .handle(Request::new(native(), call).with_document(old_document))
            .reply
            .unwrap();
        assert!(matches!(ack.reply, Reply::Throw(KernelError::Guard(_))));
        assert_eq!(replacement.store().snapshot().document.to_value(), original);
        assert!(replacement.store().selection().layers().is_empty());
    }
    let ack = replacement
        .handle(
            Request::new(
                native(),
                GestureCall::Begin {
                    label: "Current".into(),
                },
            )
            .with_document(document),
        )
        .reply
        .unwrap();
    let Reply::Gesture(gesture) = ack.reply else {
        panic!("gesture must open")
    };
    for call in [
        GestureCall::Step {
            gesture,
            body: opacity("layer0", 25.),
            replace: true,
        },
        GestureCall::Commit { gesture },
        GestureCall::Cancel { gesture },
    ] {
        let ack = replacement
            .handle(Request::new(native(), call).with_document(old_document))
            .reply
            .unwrap();
        assert!(matches!(ack.reply, Reply::Throw(KernelError::Guard(_))));
        assert_eq!(replacement.store().snapshot().document.to_value(), original);
    }
    let ack = replacement
        .handle(
            Request::new(
                native(),
                GestureCall::Step {
                    gesture,
                    body: opacity("layer0", 25.),
                    replace: true,
                },
            )
            .with_document(document),
        )
        .reply
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)));
    let ack = replacement
        .handle(Request::new(native(), GestureCall::Cancel { gesture }).with_document(document))
        .reply
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)));
    assert_eq!(replacement.store().snapshot().document.to_value(), original);
}
