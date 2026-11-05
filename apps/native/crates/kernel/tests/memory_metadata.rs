mod common;
use common::TempDir;
use pm_kernel::{
    Ack, Call, Caller, Command, GestureCall, HistoryCall, Kernel, KernelConfig, MemoryCall,
    OpenSource, Origin, Reply, Request, WriteBody,
};
use pm_model::{Op, Value, json::parse, memory::Operation, patch::Patch};

const SOURCE: &str = r#"{"id":"P","name":"Synthetic","w":320,"h":180,"fps":30,"dur":2,"layers":[{"id":"L","type":"shape","name":"Layer","from":0,"dur":2,"p":{"opacity":{"v":100,"kf":[]}}}],"comps":{},"assets":{}}"#;
struct Fixture {
    kernel: Kernel,
    _dir: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new("memory-metadata");
        let mut kernel = Kernel::new(KernelConfig {
            journal_root: Some(dir.path("journals")),
            ..Default::default()
        });
        kernel
            .open("P", OpenSource::Untitled(parse(SOURCE).unwrap()))
            .unwrap();
        Self { kernel, _dir: dir }
    }
    fn doc(&self) -> u64 {
        self.kernel.project("P").unwrap().store().doc_id()
    }
    fn revision(&self) -> u64 {
        self.kernel.project("P").unwrap().store().revision()
    }
    fn value(&self) -> Value {
        self.kernel
            .project("P")
            .unwrap()
            .store()
            .snapshot()
            .document
            .to_value()
            .unwrap()
    }
    fn call(&mut self, call: impl Into<Call>) -> Ack {
        let document = self.doc();
        self.kernel
            .handle(
                "P",
                Request::new(Caller::native(), call).with_document(document),
            )
            .unwrap()
            .reply
            .unwrap()
    }
    fn memory_request(&self, operation: Operation, id: &str) -> Request {
        let expected = self.value().get("memory").cloned();
        Request::new(
            Caller::realm(7, 1, Origin::Mcp),
            MemoryCall::Mutate {
                expected,
                operation,
                now: 1000.,
                id: id.into(),
            },
        )
        .with_document(self.doc())
    }
    fn note(&mut self, id: &str) -> Ack {
        let request = self.memory_request(
            Operation::Add {
                kind: "decision".into(),
                text: format!("Synthetic {id}").as_str().into(),
                why: None,
                resolved: false,
            },
            id,
        );
        let ack = self.kernel.handle("P", request).unwrap().reply.unwrap();
        assert!(matches!(ack.reply, Reply::Memory { .. }), "{ack:?}");
        assert!(ack.notice.is_none(), "{ack:?}");
        ack
    }
    fn rename(name: &str) -> WriteBody {
        Command::SetLayerName {
            layer_id: "L".into(),
            name: name.into(),
        }
        .into()
    }
    fn name(&self) -> String {
        self.kernel.project("P").unwrap().store().layers()[0]
            .name
            .clone()
    }
}
#[test]
fn notes_during_live_edit_survive_cancel_undo_redo_and_composition_restore() {
    let mut f = Fixture::new();
    let initial = f.value();
    let Reply::Gesture(gesture) = f
        .call(GestureCall::Begin {
            label: "Drag".into(),
        })
        .reply
    else {
        panic!()
    };
    f.call(GestureCall::Step {
        gesture,
        body: Fixture::rename("Preview"),
        replace: true,
    });
    let revision = f.revision();
    f.note("first");
    assert_eq!(f.revision(), revision);
    assert_eq!(f.kernel.project("P").unwrap().store().history().len(), 0);
    f.call(GestureCall::Cancel { gesture });
    assert_eq!(f.name(), "Layer");
    let notes = f.value().get("memory").cloned();
    assert_eq!(notes.as_ref().unwrap().as_array().unwrap().len(), 1);
    assert!(!f.kernel.project("P").unwrap().store().is_dirty());
    assert!(
        f.kernel
            .project("P")
            .unwrap()
            .store()
            .has_pending_metadata()
    );
    f.call(Call::Transact {
        label: None,
        body: Fixture::rename("Committed"),
        expected_revision: None,
    });
    f.note("second");
    let notes = f.value().get("memory").cloned();
    f.call(HistoryCall::Undo);
    assert_eq!(f.name(), "Layer");
    assert_eq!(f.value().get("memory"), notes.as_ref());
    assert!(!f.kernel.project("P").unwrap().store().is_dirty());
    assert!(
        f.kernel
            .project("P")
            .unwrap()
            .store()
            .has_pending_metadata()
    );
    f.call(HistoryCall::Redo);
    assert_eq!(f.name(), "Committed");
    assert_eq!(f.value().get("memory"), notes.as_ref());
    f.call(Call::Restore {
        project: initial,
        label: "Restore".into(),
        expected_revision: f.revision(),
        expected_document: f.doc(),
    });
    assert_eq!(f.name(), "Layer");
    assert_eq!(f.value().get("memory"), notes.as_ref());
}
#[test]
fn whole_root_owner_rollback_and_legacy_history_patches_preserve_latest_notes() {
    let mut f = Fixture::new();
    let before = f.value();
    f.call(HistoryCall::Begin {
        label: "Root edit".into(),
    });
    let mut root = before.clone();
    root.as_object_mut()
        .unwrap()
        .insert("name", Value::from("Changed root"));
    f.call(Call::DirectWrite {
        ops: vec![Op::Leaf(Patch::set(vec![], root))],
    });
    f.note("kept");
    let memory = f.value().get("memory").cloned();
    let ack = f.call(HistoryCall::Rollback);
    assert!(ack.notice.is_none(), "{ack:?}");
    assert_eq!(f.value().get("name"), before.get("name"));
    assert_eq!(f.value().get("memory"), memory.as_ref());
    let mut root = f.value();
    root.as_object_mut()
        .unwrap()
        .insert("name", Value::from("Undo root"));
    f.call(Call::Transact {
        label: Some("Root".into()),
        body: WriteBody::Ops(vec![Op::Leaf(Patch::set(vec![], root))]),
        expected_revision: None,
    });
    f.note("later");
    let memory = f.value().get("memory").cloned();
    f.call(HistoryCall::Undo);
    assert_eq!(f.value().get("name"), before.get("name"));
    assert_eq!(f.value().get("memory"), memory.as_ref());
    f.call(HistoryCall::Redo);
    assert_eq!(
        f.value().get("name").and_then(Value::as_str),
        Some("Undo root")
    );
    assert_eq!(f.value().get("memory"), memory.as_ref());
}
#[test]
fn stale_notes_document_reuse_and_agent_access_changes_are_refused() {
    let mut f = Fixture::new();
    let stale = f.memory_request(
        Operation::Remove {
            id: "missing".into(),
        },
        "unused",
    );
    f.note("first");
    let before = f.value();
    let revision = f.revision();
    let ack = f.kernel.handle("P", stale.clone()).unwrap().reply.unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    let ack = f
        .kernel
        .handle(
            "P",
            Request::new(
                Caller::realm(7, 1, Origin::Mcp),
                MemoryCall::SetEnabled {
                    expected: None,
                    enabled: true,
                },
            )
            .with_document(f.doc()),
        )
        .unwrap()
        .reply
        .unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    assert_eq!(f.value(), before);
    assert_eq!(f.revision(), revision);
    f.call(MemoryCall::SetEnabled {
        expected: None,
        enabled: false,
    });
    let disabled = f.value();
    let request = f.memory_request(Operation::Remove { id: "first".into() }, "unused");
    assert!(matches!(
        f.kernel.handle("P", request).unwrap().reply.unwrap().reply,
        Reply::Throw(_)
    ));
    assert_eq!(f.value(), disabled);
    assert!(f.kernel.close("P"));
    f.kernel
        .open("P", OpenSource::Untitled(parse(SOURCE).unwrap()))
        .unwrap();
    assert!(matches!(
        f.kernel.handle("P", stale).unwrap().reply.unwrap().reply,
        Reply::Throw(_)
    ));
    assert!(f.value().get("memory").is_none());
}
#[test]
fn memory_requires_durable_admission_and_does_not_make_inflight_file_saves_clean() {
    let mut f = Fixture::new();
    f.note("first");
    let revision = f.revision();
    let path = f._dir.path("saved.pmv");
    let plan = f
        .kernel
        .project_mut("P")
        .unwrap()
        .store_mut()
        .prepare_save(
            pm_kernel::SaveTarget::SaveAs {
                path: path.clone(),
                replace: false,
            },
            Default::default(),
        )
        .unwrap();
    f.note("second");
    let receipt = plan.run().unwrap();
    let store = f.kernel.project_mut("P").unwrap().store_mut();
    store.finish_save(receipt).unwrap();
    assert!(!store.is_dirty());
    assert!(store.has_pending_metadata());
    assert_eq!(store.revision(), revision);
    let saved = pm_kernel::open_project(&path, Default::default()).unwrap();
    assert_eq!(
        saved
            .document
            .root()
            .unwrap()
            .get("memory")
            .unwrap()
            .to_value()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut volatile = Kernel::new(Default::default());
    volatile
        .open("P", OpenSource::Untitled(parse(SOURCE).unwrap()))
        .unwrap();
    let doc = volatile.project("P").unwrap().store().doc_id();
    let request = Request::new(
        Caller::native(),
        MemoryCall::SetEnabled {
            expected: None,
            enabled: true,
        },
    )
    .with_document(doc);
    assert!(volatile.handle("P", request).is_err());
    assert!(
        volatile
            .project("P")
            .unwrap()
            .store()
            .root()
            .unwrap()
            .get("memoryEnabled")
            .is_none()
    );
}
#[test]
fn journal_recovery_keeps_same_revision_notes_archive_and_complete_history() {
    let mut f = Fixture::new();
    let base = pm_kernel::base_hash_of(&parse(SOURCE).unwrap());
    f.call(Call::Transact {
        label: None,
        body: Fixture::rename("Committed"),
        expected_revision: None,
    });
    f.note("first");
    let request = f.memory_request(Operation::Remove { id: "first".into() }, "unused");
    assert!(matches!(
        f.kernel.handle("P", request).unwrap().reply.unwrap().reply,
        Reply::Memory { .. }
    ));
    f.note("retained");
    f.call(HistoryCall::Undo);
    let document = f.value();
    let revision = f.revision();
    let history = f
        .kernel
        .project("P")
        .unwrap()
        .store()
        .history()
        .snapshot()
        .to_saved();
    f.kernel.durability_barrier().unwrap();
    let journals = f._dir.path("journals/P");
    let recovered = match pm_kernel::journal::recover(&journals, Some(&base), None) {
        pm_kernel::Recovery::Recovered(r) => r,
        other => panic!("{other:?}"),
    };
    assert_eq!(recovered.revision, revision);
    assert_eq!(recovered.document.to_value(), Some(document));
    assert_eq!(recovered.history.snapshot().to_saved(), history);
    assert!(
        recovered
            .document
            .root()
            .unwrap()
            .get("nativeMemoryArchive")
            .is_some()
    );
}

#[test]
fn replacement_root_gesture_steps_cannot_restore_old_notes() {
    let mut f = Fixture::new();
    let mut from_start = f.value();
    let Reply::Gesture(gesture) = f
        .call(GestureCall::Begin {
            label: "Root drag".into(),
        })
        .reply
    else {
        panic!()
    };
    from_start
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("First preview"));
    f.call(GestureCall::Step {
        gesture,
        replace: true,
        body: WriteBody::Ops(vec![Op::Leaf(Patch::set(vec![], from_start.clone()))]),
    });
    f.note("kept");
    let memory = f.value().get("memory").cloned();
    from_start
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Second preview"));
    let ack = f.call(GestureCall::Step {
        gesture,
        replace: true,
        body: WriteBody::Ops(vec![Op::Leaf(Patch::set(vec![], from_start))]),
    });
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{ack:?}");
    assert_eq!(f.value().get("memory"), memory.as_ref());
    f.call(GestureCall::Commit { gesture });
    f.call(HistoryCall::Undo);
    assert_eq!(f.value().get("memory"), memory.as_ref());
    f.kernel.durability_barrier().unwrap();
    let recovered = pm_kernel::journal::recover(
        &f._dir.path("journals/P"),
        Some(&pm_kernel::base_hash_of(&parse(SOURCE).unwrap())),
        None,
    );
    let pm_kernel::Recovery::Recovered(recovered) = recovered else {
        panic!()
    };
    assert_eq!(recovered.document.to_value(), Some(f.value()));
}

#[test]
fn ordinary_root_writes_preserve_notes_and_leaf_writes_require_metadata_admission() {
    let mut f = Fixture::new();
    let mut stale = f.value();
    f.note("kept");
    let notes = f.value().get("memory").cloned();
    stale
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Fresh composition edit"));
    let ack = f.call(Call::Transact {
        label: None,
        expected_revision: None,
        body: WriteBody::Ops(vec![Op::Leaf(Patch::set(vec![], stale))]),
    });
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{ack:?}");
    assert_eq!(f.value().get("memory"), notes.as_ref());
    let before = f.value();
    let revision = f.revision();
    for key in ["memory", "memoryEnabled", "nativeMemoryArchive"] {
        let ack = f.call(Call::Transact {
            label: None,
            expected_revision: None,
            body: WriteBody::Ops(vec![Op::Leaf(Patch::set(vec![key.into()], Value::Null))]),
        });
        assert!(matches!(ack.reply, Reply::Throw(_)), "{ack:?}");
        assert_eq!(f.value(), before);
        assert_eq!(f.revision(), revision);
    }
    f.call(HistoryCall::Undo);
    assert_eq!(f.value().get("memory"), notes.as_ref());
}

#[test]
fn actor_metadata_publications_refresh_query_mirrors_without_composition_edits() {
    let dir = TempDir::new("metadata-query-mirror");
    let actor = pm_kernel::KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.path("journals")),
        ..Default::default()
    });
    let handle = actor.handle();
    handle
        .open("P", OpenSource::Untitled(parse(SOURCE).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let before = handle.published("P").unwrap();
    let mut mirror = pm_kernel::QueryMirror::new(&before);
    let ack = handle
        .call(
            "P",
            Request::new(
                Caller::native(),
                MemoryCall::Mutate {
                    expected: None,
                    operation: Operation::Add {
                        kind: "decision".into(),
                        text: "Synthetic intent".into(),
                        why: None,
                        resolved: false,
                    },
                    now: 1000.,
                    id: "note".into(),
                },
            )
            .with_document(before.snapshot.doc_id),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(ack.reply, Reply::Memory { .. }), "{ack:?}");
    let after = handle.published("P").unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.dirty, before.dirty);
    assert!(after.metadata_revision > before.metadata_revision);
    mirror.sync(&after);
    assert!(
        mirror
            .queries()
            .document()
            .root()
            .unwrap()
            .get("memory")
            .is_some()
    );
    assert!(before.snapshot.root().unwrap().get("memory").is_none());
    actor.shutdown();
}

#[test]
fn metadata_storage_failure_remains_visible_on_retry_and_lost_ack_query() {
    let dir = TempDir::new("memory-ack-failure");
    let mut kernel = Kernel::new(KernelConfig {
        journal_root: Some(dir.path("journals")),
        journal: pm_kernel::journal::JournalConfig {
            max_segment_bytes: 1,
            ..Default::default()
        },
    });
    kernel
        .open("P", OpenSource::Untitled(parse(SOURCE).unwrap()))
        .unwrap();
    let journal = kernel.journal_dir("P").unwrap();
    // Block rotation's exclusive new-file creation, after the original segment
    // was opened. This exercises a real storage error without a mock/flag.
    std::fs::write(journal.join("journal-00000001.log"), b"occupied").unwrap();
    let document = kernel.project("P").unwrap().store().doc_id();
    let request = Request::new(
        Caller::realm(9, 1, Origin::Mcp),
        MemoryCall::Mutate {
            expected: None,
            operation: Operation::Add {
                kind: "constraint".into(),
                text: "Retained despite failed barrier".into(),
                why: None,
                resolved: false,
            },
            now: 1000.,
            id: "ack-note".into(),
        },
    )
    .with_document(document)
    .with_txid(41);
    let ack = kernel.handle("P", request.clone()).unwrap().reply.unwrap();
    assert!(matches!(ack.reply, Reply::Memory { .. }), "{ack:?}");
    assert!(
        matches!(ack.notice, Some(pm_kernel::Notice::NotDurable { .. })),
        "{ack:?}"
    );
    let retried = kernel.handle("P", request).unwrap().reply.unwrap();
    assert!(
        matches!(retried.notice, Some(pm_kernel::Notice::NotDurable { .. })),
        "{retried:?}"
    );
    let query = Request::new(
        Caller::native(),
        Call::QueryTx {
            txid: ack.txid.unwrap(),
        },
    )
    .with_document(document);
    let Reply::Ack(Some(retained)) = kernel.handle("P", query).unwrap().reply.unwrap().reply else {
        panic!("missing lost acknowledgement")
    };
    assert!(
        matches!(retained.notice, Some(pm_kernel::Notice::NotDurable { .. })),
        "{retained:?}"
    );
    let notes = kernel
        .project("P")
        .unwrap()
        .store()
        .root()
        .unwrap()
        .get("memory")
        .unwrap();
    assert_eq!(notes.to_value().as_array().unwrap().len(), 1);
}
