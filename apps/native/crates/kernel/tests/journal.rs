//! Journal, checkpoints and recovery, including the kill test (plan v6 P1
//! gate: "the journal kill test").

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::time::Duration;

use common::TempDir;
use pm_kernel::journal::{self, JournalConfig, Recovery};
use pm_kernel::{
    Call, Caller, Command, HistoryCall, Kernel, KernelActor, KernelConfig, OpenSource, Reply,
    Request, WorkspaceCall, WriteBody, base_hash_of,
};
use pm_model::json::{Value, parse};

const PROJECT: &str = r##"{
  "id": "P", "name": "Fixture", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 2, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "layer0", "type": "shape", "name": "Layer", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}}
  ],
  "comps": {}
}"##;

fn config(root: &Path, checkpoint_revisions: u64) -> KernelConfig {
    KernelConfig {
        backup_root: None,
        journal_root: Some(root.to_path_buf()),
        journal: JournalConfig {
            flush_interval: Duration::from_millis(50),
            checkpoint_revisions,
            max_segment_bytes: 4 * 1024 * 1024,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn rename(name: &str) -> Call {
    Call::Transact {
        label: Some(format!("Rename {name}")),
        body: WriteBody::Commands(vec![Command::SetLayerName {
            layer_id: "layer0".into(),
            name: name.into(),
        }]),
        expected_revision: None,
    }
}

fn handle(k: &mut Kernel, call: impl Into<Call>) -> Reply {
    k.handle("P", Request::new(Caller::native(), call))
        .unwrap()
        .reply
        .unwrap()
        .reply
}

fn layer_name(doc: &pm_model::Document) -> String {
    doc.to_value()
        .and_then(|v| {
            v.get("comps")?.get("C1")?.get("layers")?.as_array()?[0]
                .get("name")?
                .as_str()
                .map(str::to_owned)
        })
        .unwrap()
}

fn recovered(root: &Path, current_base: Option<&str>) -> pm_kernel::Recovered {
    match journal::recover(&root.join("P"), current_base, None) {
        Recovery::Recovered(r) => r,
        other => panic!("not recovered: {other:?}"),
    }
}

#[test]
fn recovery_restores_document_history_cursor_and_workspace() {
    let dir = TempDir::new("journal-roundtrip");
    let mut k = Kernel::new(config(&dir.0, 50));
    let base = parse(PROJECT).unwrap();
    let info = k.open("P", OpenSource::Untitled(base.clone())).unwrap();
    assert_eq!(info.base_hash, base_hash_of(&base));
    for name in ["a", "b", "c"] {
        handle(&mut k, rename(name));
    }
    assert!(matches!(
        handle(&mut k, HistoryCall::Undo),
        Reply::Bool(true)
    ));
    handle(
        &mut k,
        WorkspaceCall::Mutate {
            next: parse(r#"{"name":"W"}"#).unwrap(),
            threw: None,
        },
    );
    let expected_doc = k
        .project("P")
        .unwrap()
        .store()
        .snapshot()
        .document
        .to_value();
    let expected_history = k
        .project("P")
        .unwrap()
        .store()
        .history()
        .snapshot()
        .to_saved();
    k.sync_all();
    drop(k);

    let r = recovered(&dir.0, Some(&info.base_hash));
    assert!(!r.truncated);
    assert_eq!(r.document.to_value(), expected_doc);
    assert_eq!(layer_name(&r.document), "b");
    assert_eq!(r.history.snapshot().to_saved(), expected_history);
    assert_eq!(r.history.labels(), ["Rename a", "Rename b", "Rename c"]);
    assert_eq!(r.history.applied(), 2, "the cursor survives (redo kept)");
    assert_eq!(r.workspace, parse(r#"{"name":"W"}"#).unwrap());

    // Adopting the recovered work: undo and redo still work.
    let session = r.session.clone();
    let mut k = Kernel::new(config(&dir.0, 50));
    k.open(
        "P",
        OpenSource::Recovered {
            recovered: r,
            supersedes: vec![session.clone()],
        },
    )
    .unwrap();
    assert!(
        k.project("P").unwrap().store().is_dirty(),
        "recovered work is unsaved"
    );
    assert!(matches!(
        handle(&mut k, HistoryCall::Redo),
        Reply::Bool(true)
    ));
    assert_eq!(
        layer_name(&k.project("P").unwrap().store().snapshot().document),
        "c"
    );
    handle(&mut k, HistoryCall::Undo);
    handle(&mut k, HistoryCall::Undo);
    assert_eq!(
        layer_name(&k.project("P").unwrap().store().snapshot().document),
        "a"
    );
    k.sync_all();
    assert!(
        !session.exists(),
        "the superseded session goes once the new checkpoint is durable"
    );
}

#[test]
fn checkpoints_rotate_and_prune_and_recovery_matches() {
    let dir = TempDir::new("journal-rotate");
    let mut k = Kernel::new(config(&dir.0, 5));
    k.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .unwrap();
    for i in 0..23 {
        handle(&mut k, rename(&format!("n{i}")));
    }
    k.sync_all();
    let session = journal::sessions(&dir.0.join("P")).pop().unwrap();
    let mut files: Vec<String> = std::fs::read_dir(&session)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["checkpoint-00000004.json", "journal-00000004.log"]);
    let expected = k
        .project("P")
        .unwrap()
        .store()
        .snapshot()
        .document
        .to_value();
    let r = recovered(&dir.0, None);
    assert_eq!(r.document.to_value(), expected);
    assert_eq!(r.history.len(), 23);
    assert_eq!(
        r.replayed,
        3 * 2,
        "3 renames after the last checkpoint (rev + push)"
    );
}

#[test]
fn a_torn_tail_is_ignored() {
    let dir = TempDir::new("journal-torn");
    let mut k = Kernel::new(config(&dir.0, 50));
    k.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .unwrap();
    handle(&mut k, rename("kept"));
    k.sync_all();
    drop(k);
    let session = journal::sessions(&dir.0.join("P")).pop().unwrap();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(session.join("journal-00000000.log"))
        .unwrap();
    f.write_all(b"0123456789abcdef {\"t\":\"rev\",\"b\":2,\"r\":3,\"op")
        .unwrap();
    let r = recovered(&dir.0, None);
    assert!(r.truncated);
    assert_eq!(layer_name(&r.document), "kept");
}

#[test]
fn recovery_refuses_a_different_base_and_offers_a_copy() {
    let dir = TempDir::new("journal-base");
    let mut k = Kernel::new(config(&dir.0, 50));
    k.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .unwrap();
    handle(&mut k, rename("journalled"));
    k.sync_all();
    drop(k);
    // The library slot changed elsewhere since the session began.
    let mut changed = parse(PROJECT).unwrap();
    changed
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Edited elsewhere"));
    match journal::recover(&dir.0.join("P"), Some(&base_hash_of(&changed)), None) {
        Recovery::Copy { recovered, .. } => {
            assert_eq!(layer_name(&recovered.document), "journalled")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn without_a_checkpoint_only_the_true_base_replays() {
    let dir = TempDir::new("journal-nockpt");
    let mut k = Kernel::new(config(&dir.0, 50));
    let base = parse(PROJECT).unwrap();
    k.open("P", OpenSource::Untitled(base.clone())).unwrap();
    handle(&mut k, rename("x"));
    k.sync_all();
    drop(k);
    let session = journal::sessions(&dir.0.join("P")).pop().unwrap();
    std::fs::remove_file(session.join("checkpoint-00000000.json")).unwrap();
    let canonical = pm_kernel::model::to_canonical(base.clone());
    match journal::recover(&dir.0.join("P"), None, Some(&canonical)) {
        Recovery::Recovered(r) => assert_eq!(layer_name(&r.document), "x"),
        other => panic!("{other:?}"),
    }
    let mut wrong = canonical.clone();
    wrong
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("other"));
    assert!(matches!(
        journal::recover(&dir.0.join("P"), None, Some(&wrong)),
        Recovery::Unrecoverable { .. }
    ));
    assert!(session.exists(), "an unrecoverable journal is kept");
}

// ── the kill test ───────────────────────────────────────────────────────────

const CHILD_ENV: &str = "PM_KERNEL_KILL_CHILD";

/// The child: edits as fast as it can, printing each acknowledged revision.
fn child(dir: PathBuf) {
    let actor = KernelActor::spawn(config(&dir, 20));
    let kernel = actor.handle();
    kernel
        .open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let stdout = std::io::stdout();
    for i in 1.. {
        let ack = kernel
            .call_blocking(
                "P",
                Request::new(Caller::native(), rename(&format!("w{i}"))),
            )
            .unwrap();
        assert!(matches!(ack.reply, Reply::Applied { changed: true, .. }));
        let mut out = stdout.lock();
        writeln!(out, "ACK {i} {}", ack.revision).unwrap();
        out.flush().unwrap();
    }
}

#[test]
fn kill_test_sigkill_loses_no_acknowledged_write() {
    if let Ok(dir) = std::env::var(CHILD_ENV) {
        child(PathBuf::from(dir));
        return;
    }
    let dir = TempDir::new("journal-kill");
    let mut proc = Process::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "kill_test_sigkill_loses_no_acknowledged_write",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, &dir.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut last = 0u64;
    let mut lines = BufReader::new(proc.stdout.take().unwrap()).lines();
    while let Some(Ok(line)) = lines.next() {
        if let Some(rest) = line.strip_prefix("ACK ") {
            last = rest.split(' ').next().unwrap().parse().unwrap();
            if last >= 300 {
                break;
            }
        }
    }
    // Let the child keep writing (and checkpointing) at full speed, then
    // SIGKILL it wherever it is.
    std::thread::sleep(Duration::from_millis(7));
    proc.kill().unwrap();
    let status = proc.wait().unwrap();
    assert!(!status.success());
    // Drain whatever acks reached the pipe before the kill.
    for line in lines.map_while(Result::ok) {
        if let Some(rest) = line.strip_prefix("ACK ") {
            last = rest.split(' ').next().unwrap().parse().unwrap();
        }
    }
    assert!(last >= 300, "the child acknowledged {last} writes");

    let r = recovered(&dir.0, None);
    let name = layer_name(&r.document);
    let recovered_n: u64 = name.trim_start_matches('w').parse().unwrap();
    assert!(
        recovered_n >= last,
        "acknowledged w{last} but recovered only {name}"
    );
    assert_eq!(
        r.history.len() as u64,
        recovered_n,
        "every acknowledged step is undoable"
    );
    eprintln!(
        "kill test: {last} acknowledged, {recovered_n} recovered, {} lines replayed, torn tail: {}",
        r.replayed, r.truncated
    );

    // Undo after recovery.
    let mut k = Kernel::new(KernelConfig::default());
    k.open(
        "P",
        OpenSource::Recovered {
            recovered: r,
            supersedes: vec![],
        },
    )
    .unwrap();
    assert!(matches!(
        handle(&mut k, HistoryCall::Undo),
        Reply::Bool(true)
    ));
    assert_eq!(
        layer_name(&k.project("P").unwrap().store().snapshot().document),
        format!("w{}", recovered_n - 1)
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The Quit Now journey: a file save publishes while the session's first
/// checkpoint is stuck behind a slow disk, the person keeps editing, the
/// save's completion waits on that checkpoint, and Quit Now syncs the
/// journals from outside the busy actor and exits. What the disk holds at
/// that moment (copied, as an exit freezes it) reopens with every edit
/// recovered onto the saved file: none is stranded on the replaced base.
#[test]
fn quit_now_after_a_save_during_a_slow_checkpoint_recovers_every_edit() {
    use pm_kernel::session::{
        RecoveryDecision, RecoveryResult, SessionSource, ShellCall, ShellReply,
    };
    use std::sync::{Arc, Condvar, Mutex};
    let dir = TempDir::new("save-point");
    let file = dir.0.join("project.pmv");
    let mut untitled = pm_kernel::DocumentStore::new_untitled(parse(PROJECT).unwrap());
    common::save_as(&mut untitled, &file).unwrap();
    drop(untitled);
    let hold = Arc::new((Mutex::new(true), Condvar::new()));
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.0.join("journals")),
        backup_root: Some(dir.0.join("backups")),
        journal: JournalConfig {
            hold_checkpoints: Some(hold.clone()),
            ..Default::default()
        },
        ..Default::default()
    });
    let k = actor.handle();
    let open = |k: &pm_kernel::KernelHandle, path: &Path| {
        k.shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::File(path.into()),
            electron_profile: None,
        })
    };
    let session = match open(&k, &file).wait().unwrap().unwrap() {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    };
    let edit = |name: &str| {
        k.call("P", Request::new(Caller::native(), rename(name)))
            .wait()
            .unwrap()
            .unwrap();
    };
    edit("before save");
    let plan = k
        .prepare_save(session.clone(), None)
        .wait()
        .unwrap()
        .unwrap();
    edit("while saving");
    let completion = plan.run();
    edit("after publishing");
    // Completing the save waits on the stuck checkpoint: the actor is busy.
    let finishing = k.finish_save(completion);
    k.sync_journals().unwrap();
    let exited = dir.0.join("after-quit-now");
    copy_tree(&dir.0.join("journals"), &exited.join("journals"));
    std::fs::copy(&file, exited.join("project.pmv")).unwrap();
    let checkpoints = std::fs::read_dir(exited.join("journals/P"))
        .unwrap()
        .flat_map(|s| std::fs::read_dir(s.unwrap().path()).unwrap())
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("checkpoint-")
        })
        .count();
    assert_eq!(checkpoints, 0, "the checkpoint never landed");
    *hold.0.lock().unwrap() = false;
    hold.1.notify_all();
    finishing.wait().unwrap().unwrap();
    drop(actor);

    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(exited.join("journals")),
        ..Default::default()
    });
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(exited.join("project.pmv")),
            electron_profile: None,
        })
        .unwrap();
    let ShellReply::RecoveryRequired { ticket, recovery } = reply else {
        panic!("{reply:?}");
    };
    assert!(
        matches!(recovery, RecoveryResult::Recovered { truncated: false }),
        "{recovery:?}"
    );
    let reply = k
        .shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::Recover,
        })
        .unwrap();
    assert!(matches!(reply, ShellReply::Opened { .. }), "{reply:?}");
    let store = k.project("P").unwrap().store();
    assert_eq!(layer_name(&store.snapshot().document), "after publishing");
    let labels = store.history().labels();
    assert_eq!(
        labels,
        [
            "Rename before save",
            "Rename while saving",
            "Rename after publishing"
        ]
    );
}

/// Recovering work starts a session from checkpoint 0 of the recovered
/// state. When Quit Now comes before that checkpoint lands (a large project,
/// a slow disk), the next open continues from the session it superseded,
/// which is kept until then: the recovered edits and the ones after are all
/// there.
#[test]
fn quit_now_before_a_recovered_sessions_first_checkpoint_keeps_all_the_work() {
    use pm_kernel::session::{
        RecoveryDecision, RecoveryResult, SessionSource, ShellCall, ShellReply,
    };
    use std::sync::{Arc, Condvar, Mutex};
    let dir = TempDir::new("continued");
    let file = dir.0.join("project.pmv");
    let mut untitled = pm_kernel::DocumentStore::new_untitled(parse(PROJECT).unwrap());
    common::save_as(&mut untitled, &file).unwrap();
    drop(untitled);
    let hold = Arc::new((Mutex::new(false), Condvar::new()));
    let kernel = |journals: &Path, hold: Option<Arc<(Mutex<bool>, Condvar)>>| {
        Kernel::new(KernelConfig {
            journal_root: Some(journals.to_path_buf()),
            journal: JournalConfig {
                hold_checkpoints: hold,
                ..Default::default()
            },
            ..Default::default()
        })
    };
    let prepare = |k: &mut Kernel, path: &Path| match k
        .shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(path.into()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::RecoveryRequired { ticket, recovery } => (ticket, recovery),
        other => panic!("{other:?}"),
    };
    let recover = |k: &mut Kernel, ticket: String| {
        let reply = k
            .shell(ShellCall::DecideRecovery {
                ticket,
                decision: RecoveryDecision::Recover,
            })
            .unwrap();
        assert!(matches!(reply, ShellReply::Opened { .. }), "{reply:?}");
    };
    let journals = dir.0.join("journals");
    let mut k = kernel(&journals, None);
    k.shell(ShellCall::Open {
        id: "P".into(),
        source: SessionSource::File(file.clone()),
        electron_profile: None,
    })
    .unwrap();
    handle(&mut k, rename("one"));
    handle(&mut k, rename("two"));
    k.durability_barrier().unwrap();
    drop(k); // a crash: the session stays

    let mut k = kernel(&journals, Some(hold.clone()));
    let (ticket, _) = prepare(&mut k, &file);
    *hold.0.lock().unwrap() = true;
    recover(&mut k, ticket);
    handle(&mut k, rename("three"));
    k.journal_sync().unwrap().sync().unwrap();
    let exited = dir.0.join("after-quit-now");
    copy_tree(&journals, &exited.join("journals"));
    std::fs::copy(&file, exited.join("project.pmv")).unwrap();
    *hold.0.lock().unwrap() = false;
    hold.1.notify_all();
    drop(k);

    let mut k = kernel(&exited.join("journals"), None);
    let (ticket, recovery) = prepare(&mut k, &exited.join("project.pmv"));
    assert!(
        matches!(recovery, RecoveryResult::Recovered { truncated: false }),
        "{recovery:?}"
    );
    recover(&mut k, ticket);
    let store = k.project("P").unwrap().store();
    assert_eq!(layer_name(&store.snapshot().document), "three");
    assert_eq!(
        store.history().labels(),
        ["Rename one", "Rename two", "Rename three"]
    );
}

mod save_points {
    use super::*;
    use pm_kernel::journal::SaveFault;
    use pm_kernel::session::{
        RecoveryDecision, RecoveryResult, SessionHandle, SessionSource, ShellCall, ShellReply,
    };
    use std::sync::{Arc, Mutex};

    fn file_project(dir: &TempDir) -> PathBuf {
        let file = dir.0.join("project.pmv");
        let mut untitled = pm_kernel::DocumentStore::new_untitled(parse(PROJECT).unwrap());
        common::save_as(&mut untitled, &file).unwrap();
        file
    }

    fn kernel(journals: &Path, fault: Option<Arc<Mutex<Option<SaveFault>>>>) -> Kernel {
        Kernel::new(KernelConfig {
            journal_root: Some(journals.to_path_buf()),
            journal: JournalConfig {
                fail_save_point: fault,
                ..Default::default()
            },
            ..Default::default()
        })
    }

    fn open(k: &mut Kernel, file: &Path) -> SessionHandle {
        match k
            .shell(ShellCall::Open {
                id: "P".into(),
                source: SessionSource::File(file.into()),
                electron_profile: None,
            })
            .unwrap()
        {
            ShellReply::Opened { session, .. } => session,
            other => panic!("{other:?}"),
        }
    }

    /// Opens the file after a crash and returns what recovery offers, and
    /// the project once recovered in place.
    fn reopen(journals: &Path, file: &Path) -> (RecoveryResult, Kernel) {
        let mut k = kernel(journals, None);
        let reply = k
            .shell(ShellCall::PrepareOpen {
                id: "P".into(),
                source: SessionSource::File(file.into()),
                electron_profile: None,
            })
            .unwrap();
        let ShellReply::RecoveryRequired { ticket, recovery } = reply else {
            panic!("{reply:?}");
        };
        if matches!(recovery, RecoveryResult::Recovered { .. }) {
            let reply = k
                .shell(ShellCall::DecideRecovery {
                    ticket,
                    decision: RecoveryDecision::Recover,
                })
                .unwrap();
            assert!(matches!(reply, ShellReply::Opened { .. }), "{reply:?}");
        }
        (recovery, k)
    }

    /// A save whose save point cannot be made durable does not publish: the
    /// person is told, the file keeps its bytes, and every edit (before the
    /// failed save and after it) recovers onto that file. A torn save point
    /// is cut back so it never stands between those edits; one that cannot
    /// be cut moves the journal to a new segment and checkpoint.
    #[test]
    fn a_save_point_that_cannot_be_made_durable_stops_the_save() {
        for fault in [
            SaveFault::TornWrite,
            SaveFault::SyncFails,
            SaveFault::TornWriteUntruncatable,
        ] {
            for asynchronous in [true, false] {
                let dir = TempDir::new("save-point-fault");
                let file = file_project(&dir);
                let before = std::fs::read(&file).unwrap();
                let journals = dir.0.join("journals");
                let inject = Arc::new(Mutex::new(None));
                let mut k = kernel(&journals, Some(inject.clone()));
                let session = open(&mut k, &file);
                handle(&mut k, rename("before save"));
                *inject.lock().unwrap() = Some(fault);
                let error = if asynchronous {
                    // The app's path: frozen on the actor, written on a worker.
                    k.prepare_session_save(session.clone(), None, false)
                        .err()
                        .unwrap_or_else(|| panic!("{fault:?}: the save was allowed to publish"))
                } else {
                    k.shell(ShellCall::Save(session.clone())).unwrap_err()
                };
                assert!(
                    error.to_string().contains("Not saved"),
                    "{fault:?}: {error}"
                );
                assert_eq!(std::fs::read(&file).unwrap(), before, "{fault:?}");
                handle(&mut k, rename("after the failed save"));
                // A later save goes ahead once the journal can record it;
                // only the attempt failed.
                k.durability_barrier().unwrap();
                drop(k); // a crash

                let (recovery, k) = reopen(&journals, &file);
                assert!(
                    matches!(recovery, RecoveryResult::Recovered { truncated: false }),
                    "{fault:?} {asynchronous}: {recovery:?}"
                );
                let store = k.project("P").unwrap().store();
                assert_eq!(
                    layer_name(&store.snapshot().document),
                    "after the failed save",
                    "{fault:?}"
                );
                assert_eq!(
                    store.history().labels(),
                    ["Rename before save", "Rename after the failed save"],
                    "{fault:?}"
                );
            }
        }
    }

    /// The person can save again once the journal records save points.
    #[test]
    fn a_save_after_a_failed_save_point_publishes() {
        let dir = TempDir::new("save-point-retry");
        let file = file_project(&dir);
        let inject = Arc::new(Mutex::new(Some(SaveFault::TornWrite)));
        let mut k = kernel(&dir.0.join("journals"), Some(inject));
        let session = open(&mut k, &file);
        handle(&mut k, rename("saved"));
        assert!(k.shell(ShellCall::Save(session.clone())).is_err());
        k.shell(ShellCall::Save(session)).unwrap();
        let saved = common::open_store(&file);
        assert_eq!(layer_name(&saved.snapshot().document), "saved");
    }

    /// Another writer installs the same document with its own undo history
    /// while this session's save runs, so the save cannot publish. After a
    /// crash the file holds the save point's document but not its history:
    /// the recovered work is offered as a copy, never adopted over the other
    /// writer's history.
    #[test]
    fn a_same_document_with_another_history_is_not_adopted() {
        let dir = TempDir::new("save-point-other-writer");
        let file = file_project(&dir);
        let other = dir.0.join("other.pmv");
        std::fs::copy(&file, &other).unwrap();
        let journals = dir.0.join("journals");
        let mut k = kernel(&journals, None);
        let session = open(&mut k, &file);
        handle(&mut k, rename("A"));
        let save = k.prepare_session_save(session, None, false).unwrap();

        // The other writer: the same rename under its own label.
        let mut w = Kernel::new(KernelConfig::default());
        let other_session = open(&mut w, &other);
        let reply = w
            .handle(
                "P",
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: Some("Their rename".into()),
                        body: WriteBody::Commands(vec![Command::SetLayerName {
                            layer_id: "layer0".into(),
                            name: "A".into(),
                        }]),
                        expected_revision: None,
                    },
                ),
            )
            .unwrap();
        assert!(reply.reply.is_some());
        w.shell(ShellCall::Save(other_session)).unwrap();
        drop(w);
        std::fs::rename(&other, &file).unwrap();

        let completion = save.run();
        assert!(
            k.finish_session_save(completion).is_err(),
            "the save published over the other writer"
        );
        handle(&mut k, rename("B"));
        k.durability_barrier().unwrap();
        drop(k); // a crash

        let theirs = common::open_store(&file);
        assert_eq!(layer_name(&theirs.snapshot().document), "A");
        assert_eq!(theirs.history().labels(), ["Their rename"]);
        let (recovery, _) = reopen(&journals, &file);
        assert!(
            matches!(recovery, RecoveryResult::OfferCopy { .. }),
            "{recovery:?}"
        );
    }

    /// The empty staging file a save cut short by Quit Now left next to the
    /// project goes when the project is next opened or saved; anything that
    /// is not exactly such a file of this project stays (a non-empty one, a
    /// near miss of the name, another project's, a link).
    #[test]
    fn an_abandoned_empty_staging_file_goes_on_open_and_save() {
        let dir = TempDir::new("abandoned-temp");
        let file = file_project(&dir);
        let abandoned = || {
            let path = pm_kernel::project_io::temporary_sibling(&file);
            std::fs::write(&path, b"").unwrap();
            path
        };
        let opened_left = abandoned();
        let kept = [
            (
                pm_kernel::project_io::temporary_sibling(&file),
                &b"partial"[..],
            ),
            (dir.0.join(".project.pmv.not-a-uuid.tmp"), b""),
            (
                pm_kernel::project_io::temporary_sibling(&dir.0.join("other.pmv")),
                b"",
            ),
            (
                dir.0
                    .join(".project.pmv.6f1c2a3b-4d5e-4f60-8a7b-1c2d3e4f5a6b.tmp.bak"),
                b"",
            ),
        ];
        for (path, bytes) in &kept {
            std::fs::write(path, bytes).unwrap();
        }
        let target = dir.0.join("elsewhere");
        std::fs::write(&target, b"").unwrap();
        let link = pm_kernel::project_io::temporary_sibling(&file);
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let mut k = kernel(&dir.0.join("journals"), None);
        let session = open(&mut k, &file);
        assert!(!opened_left.exists(), "left by an earlier save");
        let saved_left = abandoned();
        handle(&mut k, rename("saved"));
        k.shell(ShellCall::Save(session)).unwrap();
        assert!(!saved_left.exists(), "left before this save");
        for (path, _) in &kept {
            assert!(path.exists(), "{path:?} is not ours to remove");
        }
        assert!(link.symlink_metadata().is_ok() && target.exists());
    }

    /// Rewrites the first line of `path` (a segment head or checkpoint
    /// metadata) through `edit`, keeping a segment head's checksum valid.
    fn rewrite_head(path: &Path, checksummed: bool, edit: impl Fn(&mut pm_model::json::Map)) {
        let text = std::fs::read_to_string(path).unwrap();
        let (head, rest) = text.split_once('\n').unwrap();
        let json = if checksummed {
            head.split_once(' ').unwrap().1
        } else {
            head
        };
        let Value::Object(mut m) = parse(json).unwrap() else {
            panic!("head is not an object");
        };
        edit(&mut m);
        let json = pm_model::json::to_canonical_string(&Value::Object(m));
        let head = if checksummed {
            let sum = &pm_model::hash::sha256_hex(json.as_bytes())[..16];
            format!("{sum} {json}")
        } else {
            json
        };
        std::fs::write(path, format!("{head}\n{rest}")).unwrap();
    }

    fn session_dir(journals: &Path) -> PathBuf {
        journal::sessions(&journals.join("P")).pop().unwrap()
    }

    fn crashed_session(dir: &TempDir, checkpoint_revisions: u64) -> (PathBuf, PathBuf) {
        let file = file_project(dir);
        let journals = dir.0.join("journals");
        let mut k = Kernel::new(config(&journals, checkpoint_revisions));
        open(&mut k, &file);
        handle(&mut k, rename("one"));
        handle(&mut k, rename("two"));
        k.durability_barrier().unwrap();
        drop(k);
        (file, journals)
    }

    /// A journal a newer Powermove wrote (a newer version, with a record
    /// this build does not know) is refused as such, and its files are
    /// kept; it does not fail as a broken journal.
    #[test]
    fn a_newer_journal_is_refused_as_made_by_a_newer_powermove() {
        let dir = TempDir::new("journal-newer");
        let (file, journals) = crashed_session(&dir, 1_000);
        let session = session_dir(&journals);
        let segment = session.join("journal-00000000.log");
        rewrite_head(&segment, true, |m| {
            m.insert("version", Value::Number(f64::from(journal::FORMAT + 1)));
        });
        let mut text = std::fs::read_to_string(&segment).unwrap();
        let future = r#"{"t":"future","v":1}"#;
        let sum = &pm_model::hash::sha256_hex(future.as_bytes())[..16];
        text.push_str(&format!("{sum} {future}\n"));
        std::fs::write(&segment, text).unwrap();
        let (recovery, _) = reopen(&journals, &file);
        assert_eq!(
            recovery,
            RecoveryResult::Unrecoverable {
                reason: journal::NEWER_FORMAT.into()
            }
        );
        assert!(segment.exists(), "the journal is kept");

        // A newer checkpoint is refused the same way.
        let dir = TempDir::new("journal-newer-checkpoint");
        let (file, journals) = crashed_session(&dir, 1);
        let session = session_dir(&journals);
        let mut checkpoints: Vec<PathBuf> = std::fs::read_dir(&session)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                let name = p.file_name().unwrap().to_string_lossy();
                name.starts_with("checkpoint-") && name.ends_with(".json")
            })
            .collect();
        checkpoints.sort();
        let checkpoint = checkpoints.pop().unwrap();
        rewrite_head(&checkpoint, false, |m| {
            m.insert("version", Value::Number(f64::from(journal::FORMAT + 1)));
        });
        let (recovery, _) = reopen(&journals, &file);
        assert_eq!(
            recovery,
            RecoveryResult::Unrecoverable {
                reason: journal::NEWER_FORMAT.into()
            }
        );
    }

    /// Journals written before save points (version 1) still recover.
    #[test]
    fn a_version_1_journal_still_recovers() {
        for checkpoint_revisions in [1_000, 1] {
            let dir = TempDir::new("journal-v1");
            let (file, journals) = crashed_session(&dir, checkpoint_revisions);
            let session = session_dir(&journals);
            for entry in std::fs::read_dir(&session).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let checksummed = name.starts_with("journal-");
                if checksummed || name.starts_with("checkpoint-") {
                    rewrite_head(&path, checksummed, |m| {
                        m.insert("version", Value::Number(1.0));
                        m.remove("bases");
                    });
                }
            }
            let (recovery, k) = reopen(&journals, &file);
            assert!(
                matches!(recovery, RecoveryResult::Recovered { truncated: false }),
                "{recovery:?}"
            );
            let store = k.project("P").unwrap().store();
            assert_eq!(layer_name(&store.snapshot().document), "two");
            assert_eq!(store.history().labels(), ["Rename one", "Rename two"]);
        }
    }
}
