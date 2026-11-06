//! All profiles/projects in this suite are disposable synthetic fixtures.
mod common;
use pm_kernel::{persistence::DiskFormat, profile::*, session::*, *};
use pm_model::Value;
use std::{fs, path::Path};
fn value() -> Value {
    pm_model::json::parse(r##"{"id":"p","name":"Synthetic","compId":"c","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","layers":[{"id":"l","type":"shape","name":"Layer","from":0,"dur":2,"p":{"opacity":{"v":100,"kf":[]}}}],"assets":{},"comps":{}}"##).unwrap()
}
fn attach(k: &mut Kernel, root: &Path) {
    k.shell(ShellCall::AttachLibrary {
        profile: root.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
}
fn kernel(root: &Path) -> Kernel {
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(root.join("journals")),
        ..Default::default()
    });
    attach(&mut k, root);
    k
}
fn opened(reply: ShellReply) -> SessionHandle {
    match reply {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}
fn new(k: &mut Kernel) -> SessionHandle {
    opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::New(value()),
            electron_profile: None,
        })
        .unwrap(),
    )
}
fn caller(id: u32) -> Caller {
    Caller::realm(id, 1, Origin::Mcp)
}
fn call(k: &mut Kernel, actor: Caller, call: impl Into<Call>) -> Ack {
    k.handle("p", Request::new(actor, call))
        .unwrap()
        .reply
        .unwrap()
}
fn edit(k: &mut Kernel, actor: Caller, opacity: f64) {
    let ack = call(
        k,
        actor,
        Call::Transact {
            label: Some("Opacity".into()),
            body: vec![Command::SetStaticOpacity {
                layer_id: "l".into(),
                value: opacity,
            }]
            .into(),
            expected_revision: None,
        },
    );
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
}
fn mark(k: &mut Kernel, actor: Caller) -> ActorHistoryMark {
    match call(k, actor, HistoryCall::Mark).reply {
        Reply::Mark(m) => m,
        _ => panic!(),
    }
}
fn squash(k: &mut Kernel, actor: Caller, mark: ActorHistoryMark) -> Reply {
    let revision = k.project("p").unwrap().store().revision();
    call(
        k,
        actor,
        HistoryCall::Squash {
            mark,
            label: "Run".into(),
            expected_revision: revision,
        },
    )
    .reply
}
fn crash(root: &Path) {
    let mut k = kernel(root);
    new(&mut k);
    edit(&mut k, caller(5), 37.);
    k.durability_barrier().unwrap();
}
fn prepare(k: &mut Kernel, root: &Path) -> String {
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "p".into(),
            source: SessionSource::Library {
                profile: root.into(),
                project: "p".into(),
            },
            electron_profile: None,
        })
        .unwrap();
    assert!(k.project("p").is_none());
    match reply {
        ShellReply::RecoveryRequired { ticket, .. } => ticket,
        other => panic!("{other:?}"),
    }
}
#[test]
fn selection_is_atomic_ephemeral_deduplicated_and_published_before_event() {
    let root = common::TempDir::new("selection-actor");
    let actor = KernelActor::spawn(Default::default());
    let k = actor.handle();
    // Open replies before emitting Opened. Subscribe first and consume that
    // exact setup event, so it cannot race into the selection assertions.
    let events = k.subscribe();
    k.open("p", OpenSource::Untitled(value()))
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(events.recv().unwrap(), KernelEvent::Opened { project } if project == "p"));
    let actor = k.for_actor(caller(8));
    actor
        .call(
            "p",
            SelectionCall::Layers {
                ids: vec!["l".into()],
                add: false,
            },
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(k.published("p").unwrap().selection.primary(), Some("l"));
    let p = k.published("p").unwrap();
    assert_eq!(p.history.entries, 0);
    assert_eq!(p.revision(), 1);
    let e = events.recv().unwrap();
    assert!(matches!(e, KernelEvent::Published { .. }));
    assert!(matches!(
        events.recv().unwrap(),
        KernelEvent::SelectionChanged { .. }
    ));
    let ack = actor
        .call(
            "p",
            SelectionCall::Layers {
                ids: vec!["missing".into()],
                add: false,
            },
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    assert_eq!(k.published("p").unwrap().selection.primary(), Some("l"));
    let request = Request::new(caller(8), SelectionCall::ToggleLayer("l".into())).with_txid(12);
    k.call_blocking("p", request.clone()).unwrap();
    k.call_blocking("p", request).unwrap();
    assert!(k.published("p").unwrap().selection.is_empty());
    drop(root);
}
#[test]
fn history_marks_squash_same_actor_and_recover_grouped_undo() {
    let root = common::TempDir::new("squash");
    let mut k = kernel(&root.0);
    new(&mut k);
    let m = mark(&mut k, caller(5));
    edit(&mut k, caller(5), 60.);
    edit(&mut k, caller(5), 30.);
    assert!(matches!(
        squash(&mut k, caller(5), m),
        Reply::Entry(Some(_))
    ));
    k.durability_barrier().unwrap();
    let Recovery::Recovered(r) = k.recover("p", k.base_hash("p"), None) else {
        panic!()
    };
    assert_eq!(r.history.len(), 1);
    let mut recovered = Kernel::new(Default::default());
    recovered
        .open(
            "p",
            OpenSource::Recovered {
                recovered: r,
                supersedes: vec![],
            },
        )
        .unwrap();
    call(&mut recovered, caller(5), HistoryCall::Undo);
    assert_eq!(
        common::opacity_of(recovered.project("p").unwrap().store(), "l"),
        Some(100.)
    );
}
#[test]
fn squash_guards_foreign_actor_open_entry_and_document_lifetime() {
    let root = common::TempDir::new("squash-guards");
    let mut k = kernel(&root.0);
    let s = new(&mut k);
    let m = mark(&mut k, caller(5));
    edit(&mut k, caller(5), 60.);
    edit(&mut k, caller(6), 30.);
    assert!(matches!(
        squash(&mut k, caller(5), m.clone()),
        Reply::Entry(None)
    ));
    let own = mark(&mut k, caller(5));
    edit(&mut k, caller(5), 10.);
    call(
        &mut k,
        caller(6),
        HistoryCall::Begin {
            label: "Foreign".into(),
        },
    );
    assert!(matches!(squash(&mut k, caller(5), own), Reply::Entry(None)));
    call(&mut k, caller(6), HistoryCall::Commit { label: None });
    k.shell(ShellCall::Close(s)).unwrap();
    opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::Library {
                profile: root.0.clone(),
                project: "p".into(),
            },
            electron_profile: None,
        })
        .unwrap(),
    );
    edit(&mut k, caller(5), 12.);
    assert!(matches!(squash(&mut k, caller(5), m), Reply::Entry(None)));
}
#[test]
fn restore_guards_revision_identity_shape_and_open_entry_and_journals_undo() {
    let root = common::TempDir::new("restore");
    let mut k = kernel(&root.0);
    new(&mut k);
    let doc = k.project("p").unwrap().store().doc_id();
    edit(&mut k, caller(5), 33.);
    let rev = k.project("p").unwrap().store().revision();
    for (expected_revision, expected_document, project) in [
        (rev - 1, doc, value()),
        (rev, doc + 1, value()),
        (rev, doc, Value::Null),
    ] {
        assert!(matches!(
            call(
                &mut k,
                caller(5),
                Call::Restore {
                    project,
                    label: "Restore".into(),
                    expected_revision,
                    expected_document
                }
            )
            .reply,
            Reply::Throw(_)
        ));
    }
    call(
        &mut k,
        caller(6),
        HistoryCall::Begin {
            label: "Foreign".into(),
        },
    );
    assert!(matches!(
        call(
            &mut k,
            caller(5),
            Call::Restore {
                project: value(),
                label: "Restore".into(),
                expected_revision: rev,
                expected_document: doc
            }
        )
        .reply,
        Reply::Throw(_)
    ));
    call(&mut k, caller(6), HistoryCall::Commit { label: None });
    assert!(matches!(
        call(
            &mut k,
            caller(5),
            Call::Restore {
                project: value(),
                label: "Restore".into(),
                expected_revision: rev,
                expected_document: doc
            }
        )
        .reply,
        Reply::Applied { changed: true, .. }
    ));
    k.durability_barrier().unwrap();
    let Recovery::Recovered(r) = k.recover("p", k.base_hash("p"), None) else {
        panic!()
    };
    assert_eq!(r.history.len(), 2);
    assert_eq!(
        common::opacity_of(k.project("p").unwrap().store(), "l"),
        Some(100.)
    );
    call(&mut k, caller(5), HistoryCall::Undo);
    assert_eq!(
        common::opacity_of(k.project("p").unwrap().store(), "l"),
        Some(33.)
    );
}
#[test]
fn explicit_recover_adopts_only_after_decision_and_ticket_is_single_use() {
    let root = common::TempDir::new("recover-choice");
    crash(&root.0);
    let mut k = kernel(&root.0);
    let ticket = prepare(&mut k, &root.0);
    opened(
        k.shell(ShellCall::DecideRecovery {
            ticket: ticket.clone(),
            decision: RecoveryDecision::Recover,
        })
        .unwrap(),
    );
    assert_eq!(
        common::opacity_of(k.project("p").unwrap().store(), "l"),
        Some(37.)
    );
    assert!(
        k.shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::Discard
        })
        .is_err()
    );
}
#[test]
fn discard_reopens_base_without_saving_recovered_content_and_archives_journal() {
    let root = common::TempDir::new("discard-choice");
    crash(&root.0);
    let before = fs::read(root.path("store/project.p.json")).unwrap();
    let mut k = kernel(&root.0);
    let ticket = prepare(&mut k, &root.0);
    opened(
        k.shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::Discard,
        })
        .unwrap(),
    );
    assert_eq!(
        common::opacity_of(k.project("p").unwrap().store(), "l"),
        Some(100.)
    );
    assert_eq!(fs::read(root.path("store/project.p.json")).unwrap(), before);
    assert_eq!(
        fs::read_dir(root.path("journals/p/discarded"))
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn open_saved_version_sets_an_unreadable_journal_aside_and_opens_the_saved_project() {
    let root = common::TempDir::new("open-saved");
    crash(&root.0);
    let session = fs::read_dir(root.path("journals/p"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_file(session.join("checkpoint-00000000.json")).unwrap();
    let mut changed = value();
    changed
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Foreign change"));
    fs::write(
        root.path("store/project.p.json"),
        pm_model::json::to_canonical_string(&changed),
    )
    .unwrap();
    let mut k = kernel(&root.0);
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "p".into(),
            source: SessionSource::Library {
                profile: root.0.clone(),
                project: "p".into(),
            },
            electron_profile: None,
        })
        .unwrap();
    let ShellReply::RecoveryRequired {
        ticket,
        recovery: RecoveryResult::Unrecoverable { .. },
    } = reply
    else {
        panic!("{reply:?}");
    };
    opened(
        k.shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::OpenSavedVersion,
        })
        .unwrap(),
    );
    assert_eq!(
        fs::read_dir(root.path("journals/p/discarded"))
            .unwrap()
            .count(),
        1
    );
    assert!(
        fs::read_to_string(root.path("store/project.p.json"))
            .unwrap()
            .contains("Foreign change")
    );
}
#[test]
fn open_saved_version_cannot_walk_past_recoverable_edits() {
    let root = common::TempDir::new("open-saved-refused");
    crash(&root.0);
    let mut k = kernel(&root.0);
    let ticket = prepare(&mut k, &root.0);
    assert!(
        k.shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::OpenSavedVersion,
        })
        .is_err()
    );
    assert!(k.project("p").is_none());
    assert!(!root.path("journals/p/discarded").exists());
}
#[test]
fn recovery_as_copy_preserves_base_and_acknowledged_work() {
    let root = common::TempDir::new("recover-copy");
    crash(&root.0);
    let before = fs::read(root.path("store/project.p.json")).unwrap();
    let mut k = kernel(&root.0);
    let ticket = prepare(&mut k, &root.0);
    opened(
        k.shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::OpenAsCopy,
        })
        .unwrap(),
    );
    assert_eq!(
        common::opacity_of(k.project("p").unwrap().store(), "l"),
        Some(37.)
    );
    let SessionDestination::Library(copy) = k.session_metadata("p").unwrap().destination else {
        panic!()
    };
    assert_ne!(copy, "p");
    assert_eq!(fs::read(root.path("store/project.p.json")).unwrap(), before);
}
#[test]
fn recovery_ticket_refuses_changed_base() {
    let root = common::TempDir::new("recovery-stale");
    crash(&root.0);
    let mut k = kernel(&root.0);
    let ticket = prepare(&mut k, &root.0);
    let mut changed = value();
    changed
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Foreign change"));
    fs::write(
        root.path("store/project.p.json"),
        pm_model::json::to_canonical_string(&changed),
    )
    .unwrap();
    assert!(
        k.shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::Recover
        })
        .is_err()
    );
    assert!(k.project("p").is_none());
}
#[test]
fn save_handoff_allows_foreign_edits_but_refuses_close_quit_and_second_save() {
    let root = common::TempDir::new("save-flight");
    let mut k = kernel(&root.0);
    let s = new(&mut k);
    edit(&mut k, caller(5), 60.);
    let save = k
        .prepare_session_save(s.clone(), Some(root.path("saved.pmv")), false)
        .unwrap();
    assert!(k.shell(ShellCall::Close(s.clone())).is_err());
    assert!(k.shell(ShellCall::PrepareQuit).is_err());
    assert!(k.shell(ShellCall::Save(s.clone())).is_err());
    assert!(!k.close("p"));
    edit(&mut k, caller(6), 22.);
    k.finish_session_save(save.run()).unwrap();
    assert!(k.project("p").unwrap().store().is_dirty());
    assert_eq!(
        common::opacity_of(&common::open_store(&root.path("saved.pmv")), "l"),
        Some(60.)
    );
    k.shell(ShellCall::Close(s)).unwrap();
    assert_eq!(
        common::opacity_of(&common::open_store(&root.path("saved.pmv")), "l"),
        Some(22.)
    );
    assert!(
        pm_kernel::locks::FileLock::acquire(&pm_kernel::locks::project_lock_path(
            &root.path("saved.pmv")
        ))
        .is_ok()
    );
}
#[test]
fn failed_save_completion_clears_flight_and_retains_dirty_journal() {
    let root = common::TempDir::new("save-fail");
    let mut k = kernel(&root.0);
    let s = new(&mut k);
    edit(&mut k, caller(5), 25.);
    fs::write(root.path("exists.pmv"), b"keep").unwrap();
    let save = k
        .prepare_session_save(s.clone(), Some(root.path("exists.pmv")), false)
        .unwrap();
    assert!(k.finish_session_save(save.run()).is_err());
    assert!(k.project("p").unwrap().store().is_dirty());
    assert_eq!(fs::read(root.path("exists.pmv")).unwrap(), b"keep");
    k.shell(ShellCall::Close(s)).unwrap();
}
#[test]
fn defer_close_does_not_save_and_quit_compacts_releases_locks() {
    let root = common::TempDir::new("defer");
    let mut k = kernel(&root.0);
    let s = new(&mut k);
    let before = fs::read(root.path("store/project.p.json")).unwrap();
    edit(&mut k, caller(5), 25.);
    k.shell(ShellCall::DeferClose(s)).unwrap();
    assert_eq!(fs::read(root.path("store/project.p.json")).unwrap(), before);
    k.shell(ShellCall::PrepareQuit).unwrap();
    let p = LibraryStore::open(&root.0).load("p").unwrap().unwrap();
    let mut reopened = Kernel::new(Default::default());
    reopened.open("p", OpenSource::Library(p)).unwrap();
    assert_eq!(
        common::opacity_of(reopened.project("p").unwrap().store(), "l"),
        Some(25.)
    );
}
#[test]
fn local_storage_exact_strings_origin_isolation_delete_and_restart() {
    let root = common::TempDir::new("local-storage");
    fs::write(root.path("local-storage.json"), r#"[{"origin":"a","key":"k","value":"  null 🦀 ","json":null},{"origin":"b","key":"k","value":""}]"#).unwrap();
    let k = kernel(&root.0);
    match k
        .profile(ProfileCall::LocalStorageGet {
            origin: "a".into(),
            key: "k".into(),
        })
        .unwrap()
    {
        ProfileReply::String(v) => assert_eq!(v.as_deref(), Some("  null 🦀 ")),
        _ => panic!(),
    }
    k.profile(ProfileCall::LocalStorageSet {
        origin: "a".into(),
        key: "k".into(),
        value: None,
    })
    .unwrap();
    drop(k);
    let k = kernel(&root.0);
    assert!(matches!(
        k.profile(ProfileCall::LocalStorageGet {
            origin: "a".into(),
            key: "k".into()
        })
        .unwrap(),
        ProfileReply::String(None)
    ));
    match k
        .profile(ProfileCall::LocalStorageGet {
            origin: "b".into(),
            key: "k".into(),
        })
        .unwrap()
    {
        ProfileReply::String(v) => assert_eq!(v.as_deref(), Some("")),
        _ => panic!(),
    }
}
#[test]
fn secret_locators_preserve_versions_refuse_conflicts_and_never_read_keychain() {
    let root = common::TempDir::new("locators");
    let k = kernel(&root.0);
    let locator = SecretLocator {
        id: "env/synthetic".into(),
        version: "v1".into(),
        service: "synthetic service".into(),
        account: "synthetic account".into(),
    };
    k.profile(ProfileCall::RegisterSecret(locator.clone()))
        .unwrap();
    k.profile(ProfileCall::RegisterSecret(locator.clone()))
        .unwrap();
    let mut next = locator.clone();
    next.version = "v2".into();
    k.profile(ProfileCall::RegisterSecret(next)).unwrap();
    let mut conflict = locator;
    conflict.account = "different".into();
    assert!(k.profile(ProfileCall::RegisterSecret(conflict)).is_err());
    drop(k);
    let k = kernel(&root.0);
    match k
        .profile(ProfileCall::SecretLocators {
            id: "env/synthetic".into(),
        })
        .unwrap()
    {
        ProfileReply::Secrets(v) => assert_eq!(v.len(), 2),
        _ => panic!(),
    }
}
#[test]
fn extension_writes_use_overlay_shared_stays_read_only_and_traversal_is_refused() {
    let root = common::TempDir::new("overlay");
    let k = kernel(&root.0);
    fs::create_dir_all(root.path("extensions-shared/ext")).unwrap();
    fs::write(root.path("extensions-shared/ext/main.ts"), b"shared").unwrap();
    let read = || ProfileCall::ReadExtension {
        id: "ext".into(),
        relative: "main.ts".into(),
    };
    assert!(matches!(k.profile(read()).unwrap(), ProfileReply::Bytes(v) if v == b"shared"));
    k.profile(ProfileCall::WriteExtension {
        id: "ext".into(),
        relative: "main.ts".into(),
        bytes: b"beta".to_vec(),
    })
    .unwrap();
    assert!(matches!(k.profile(read()).unwrap(), ProfileReply::Bytes(v) if v == b"beta"));
    assert_eq!(
        fs::read(root.path("extensions-shared/ext/main.ts")).unwrap(),
        b"shared"
    );
    for relative in ["../shared", "/tmp/escape"] {
        assert!(
            k.profile(ProfileCall::WriteExtension {
                id: "ext".into(),
                relative: relative.into(),
                bytes: vec![]
            })
            .is_err()
        );
    }
    #[cfg(unix)]
    {
        fs::create_dir_all(root.path("outside")).unwrap();
        std::os::unix::fs::symlink(
            root.path("outside"),
            root.path("extensions-overlay/ext/link"),
        )
        .unwrap();
        assert!(
            k.profile(ProfileCall::WriteExtension {
                id: "ext".into(),
                relative: "link/escape".into(),
                bytes: vec![]
            })
            .is_err()
        );
        assert!(!root.path("outside/escape").exists());
    }
}
#[test]
fn external_media_read_capability_never_locks_source_and_checks_changed_bytes() {
    let root = common::TempDir::new("external-media");
    let k = kernel(&root.0);
    let source = root.path("external.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &source).unwrap();
    let before = fs::read(&source).unwrap();
    let ProfileReply::Media(media) = k
        .profile(ProfileCall::ExternalMedia {
            path: source.clone(),
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(!pm_kernel::locks::project_lock_path(&source).exists());
    assert!(!media.is_empty());
    let item = media.iter().next().unwrap();
    let mut next = media.clone();
    next.remove(item.id.as_str().unwrap());
    assert_eq!(media.len(), next.len() + 1); // immutable publication keeps its capability
    item.copy_to(&root.path("cache")).unwrap();
    assert_eq!(
        fs::metadata(root.path("cache")).unwrap().len(),
        item.length()
    );
    fs::write(&source, b"changed").unwrap();
    assert!(item.copy_to(&root.path("bad-cache")).is_err());
    assert!(!root.path("bad-cache").exists());
    assert!(!before.is_empty());
}

#[test]
fn actor_save_handoff_publishes_saving_and_finishes_on_same_session() {
    let root = common::TempDir::new("actor-save");
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(root.path("journals")),
        ..Default::default()
    });
    let k = actor.handle();
    k.shell(ShellCall::AttachLibrary {
        profile: root.0.clone(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .wait()
    .unwrap()
    .unwrap();
    let session = opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::New(value()),
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap(),
    );
    let save = k
        .prepare_save(session.clone(), Some(root.path("saved.pmv")))
        .wait()
        .unwrap()
        .unwrap();
    assert!(k.published("p").unwrap().saving);
    assert!(
        k.shell(ShellCall::Close(session.clone()))
            .wait()
            .unwrap()
            .is_err()
    );
    let completion = std::thread::spawn(move || save.run()).join().unwrap();
    k.finish_save(completion).wait().unwrap().unwrap();
    let p = k.published("p").unwrap();
    assert!(!p.saving);
    assert!(!p.dirty);
    assert_eq!(p.session.as_ref().unwrap().handle, session);
    assert!(matches!(
        p.session.as_ref().unwrap().destination,
        SessionDestination::File(_)
    ));
    k.shell(ShellCall::Close(session)).wait().unwrap().unwrap();
    assert!(k.published("p").is_none());
}
#[test]
fn dropped_handoff_requires_explicit_abort_before_close() {
    let root = common::TempDir::new("abort-save");
    let mut k = kernel(&root.0);
    let s = new(&mut k);
    let save = k
        .prepare_session_save(s.clone(), Some(root.path("saved.pmv")), false)
        .unwrap();
    let ticket = save.ticket.clone();
    assert!(
        k.shell(ShellCall::AbortSave {
            session: s.clone(),
            ticket: ticket.clone()
        })
        .is_err()
    );
    drop(save);
    assert!(k.shell(ShellCall::Close(s.clone())).is_err());
    k.shell(ShellCall::AbortSave {
        session: s.clone(),
        ticket,
    })
    .unwrap();
    k.shell(ShellCall::Close(s)).unwrap();
}
#[test]
fn migrated_library_media_association_never_becomes_source_save_destination() {
    let root = common::TempDir::new("migration-media");
    let external = root.path("external.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &external).unwrap();
    let before = fs::read(&external).unwrap();
    let opened_file = open_project(&external, OpenOptions::default()).unwrap();
    {
        let w = pm_kernel::persistence::LibraryWriter::open(&root.0, DiskFormat::V1_2, None)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let meta = serde_json::json!({"embeddedMedia":{"path":external,"savedHash":opened_file.file.version.as_ref().unwrap().hash}});
        w.save_project(
            "copy",
            pm_model::canonical::realize_project(opened_file.document.to_value().unwrap()),
            Value::Null,
            Value::Null,
            pm_model::json::parse(&meta.to_string()).unwrap(),
        )
        .unwrap();
    }
    let mut k = kernel(&root.0);
    let s = opened(
        k.shell(ShellCall::Open {
            id: "copy".into(),
            source: SessionSource::Library {
                profile: root.0.clone(),
                project: "copy".into(),
            },
            electron_profile: None,
        })
        .unwrap(),
    );
    assert!(matches!(
        k.session_metadata("copy").unwrap().destination,
        SessionDestination::Library(_)
    ));
    assert!(!pm_kernel::locks::project_lock_path(&external).exists());
    assert!(!k.project("copy").unwrap().store().media().is_empty());
    k.shell(ShellCall::Close(s)).unwrap();
    assert_eq!(fs::read(&external).unwrap(), before);
    fs::remove_file(external).unwrap();
    opened(
        k.shell(ShellCall::Open {
            id: "copy".into(),
            source: SessionSource::Library {
                profile: root.0.clone(),
                project: "copy".into(),
            },
            electron_profile: None,
        })
        .unwrap(),
    );
    let item = k
        .project("copy")
        .unwrap()
        .store()
        .media()
        .iter()
        .next()
        .unwrap();
    item.copy_to(&root.path("independent-media")).unwrap();
}

#[test]
fn explicit_recover_on_changed_base_opens_copy_instead_of_overwriting_base() {
    let root = common::TempDir::new("changed-recovery-copy");
    crash(&root.0);
    let mut changed = value();
    changed
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Foreign change"));
    fs::write(
        root.path("store/project.p.json"),
        pm_model::json::to_canonical_string(&changed),
    )
    .unwrap();
    let before = fs::read(root.path("store/project.p.json")).unwrap();
    let mut k = kernel(&root.0);
    let ticket = prepare(&mut k, &root.0);
    opened(
        k.shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::Recover,
        })
        .unwrap(),
    );
    assert_eq!(
        common::opacity_of(k.project("p").unwrap().store(), "l"),
        Some(37.)
    );
    assert!(
        matches!(k.session_metadata("p").unwrap().destination, SessionDestination::Library(id) if id != "p")
    );
    assert_eq!(fs::read(root.path("store/project.p.json")).unwrap(), before);
}

#[test]
fn clean_close_reopen_needs_no_recovery_but_history_only_work_still_prompts() {
    let root = common::TempDir::new("clean-reopen");
    let mut k = kernel(&root.0);
    let s = new(&mut k);
    edit(&mut k, caller(5), 50.);
    k.shell(ShellCall::Close(s)).unwrap();
    let s = opened(
        k.shell(ShellCall::PrepareOpen {
            id: "p".into(),
            source: SessionSource::Library {
                profile: root.0.clone(),
                project: "p".into(),
            },
            electron_profile: None,
        })
        .unwrap(),
    );
    // Clearing history changes no document bytes, but is still recoverable work.
    call(&mut k, caller(5), HistoryCall::Clear);
    k.shell(ShellCall::DeferClose(s)).unwrap();
    prepare(&mut k, &root.0);
}
