//! Cmd-S with nothing unsaved writes nothing when the file is still the one
//! the session last read or wrote, and still does what a save does for the
//! person: says it saved and retires the recovery journal. An unrecorded
//! write, or a file changed on disk, makes it write as before. Synthetic
//! fixtures only.
mod common;
use common::TempDir;
use pm_kernel::{journal, persistence::DiskFormat, session::*, *};
use pm_model::Value;
use pm_model::ops::Op;
use pm_model::patch::{Patch, PathPart};
use std::os::unix::fs::MetadataExt;
use std::{fs, path::Path, time::SystemTime};

fn value() -> Value {
    pm_model::json::parse(r##"{"id":"p","name":"Base","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}"##).unwrap()
}
fn config(root: &Path) -> KernelConfig {
    KernelConfig {
        journal_root: Some(root.join("journals")),
        backup_root: None,
        journal: journal::JournalConfig {
            checkpoint_revisions: 1,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn kernel(root: &Path) -> Kernel {
    let mut k = Kernel::new(config(root));
    k.shell(ShellCall::AttachLibrary {
        profile: root.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    k
}
fn opened(reply: ShellReply) -> SessionHandle {
    match reply {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}
/// A new project saved as `project.pmv`.
fn fixture(root: &TempDir) -> (Kernel, SessionHandle) {
    let mut k = kernel(&root.0);
    let s = opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::New(value()),
            electron_profile: None,
        })
        .unwrap(),
    );
    k.shell(ShellCall::SaveAs {
        session: s.clone(),
        path: root.path("project.pmv"),
        replace: false,
    })
    .unwrap();
    (k, s)
}
fn call(k: &mut Kernel, c: impl Into<Call>) -> Reply {
    k.handle("p", Request::new(Caller::native(), c))
        .unwrap()
        .reply
        .unwrap()
        .reply
}
fn rename_call(name: &str) -> Call {
    Call::Transact {
        label: Some("Rename".into()),
        body: vec![
            CompCommand::Set {
                patch: Box::new(pm_kernel::commands::comps::CompPatch {
                    name: Some(name.into()),
                    ..Default::default()
                }),
            }
            .into(),
        ]
        .into(),
        expected_revision: None,
    }
}
fn rename(k: &mut Kernel, name: &str) {
    let reply = call(k, rename_call(name));
    assert!(
        matches!(reply, Reply::Applied { changed: true, .. }),
        "{reply:?}"
    );
}
fn save(k: &mut Kernel, s: &SessionHandle) -> ShellReply {
    let reply = k.shell(ShellCall::Save(s.clone())).unwrap();
    assert!(matches!(reply, ShellReply::Saved { .. }), "{reply:?}");
    reply
}
fn dirty(k: &Kernel) -> bool {
    k.project("p").unwrap().store().needs_save()
}

/// What identifies the file on disk: its bytes, inode, size and times.
#[derive(Debug, PartialEq)]
struct OnDisk {
    bytes: Vec<u8>,
    inode: u64,
    modified: SystemTime,
    changed: (i64, i64),
}
fn on_disk(path: &Path) -> OnDisk {
    let meta = fs::symlink_metadata(path).unwrap();
    OnDisk {
        bytes: fs::read(path).unwrap(),
        inode: meta.ino(),
        modified: meta.modified().unwrap(),
        changed: (meta.ctime(), meta.ctime_nsec()),
    }
}

#[test]
fn a_clean_save_leaves_the_file_untouched() {
    let root = TempDir::new("clean-save");
    let (mut k, s) = fixture(&root);
    let path = root.path("project.pmv");
    assert!(!dirty(&k));
    let before = on_disk(&path);
    for _ in 0..3 {
        save(&mut k, &s);
        assert!(!dirty(&k));
        assert_eq!(on_disk(&path), before, "a clean Cmd-S rewrote the file");
    }
    // An edit is written, and the save after it is clean again.
    rename(&mut k, "Edited");
    assert!(dirty(&k));
    save(&mut k, &s);
    let written = on_disk(&path);
    assert_ne!(written.bytes, before.bytes);
    save(&mut k, &s);
    assert_eq!(on_disk(&path), written);
    // Reopened, the file holds the edit.
    k.shell(ShellCall::Close(s)).unwrap();
    let s = opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::File(path.clone()),
            electron_profile: None,
        })
        .unwrap(),
    );
    let name = k.project("p").unwrap().store().root().unwrap().get("name");
    assert_eq!(name.map(|n| n.to_value()), Some(Value::from("Edited")));
    // Clean on reopen: Cmd-S writes nothing.
    save(&mut k, &s);
    assert_eq!(on_disk(&path), written);
}

/// An extension's raw write is recorded in no undo step; the project is
/// unsaved all the same, and Cmd-S writes it.
#[test]
fn an_outside_write_then_save_writes() {
    let root = TempDir::new("clean-save-outside");
    let (mut k, s) = fixture(&root);
    let path = root.path("project.pmv");
    let before = on_disk(&path);
    let reply = call(
        &mut k,
        Call::DirectWrite {
            ops: vec![Op::Leaf(Patch::set(
                vec![PathPart::from("name")],
                Value::from("Written outside"),
            ))],
        },
    );
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
    assert!(
        k.project("p")
            .unwrap()
            .store()
            .history()
            .labels()
            .is_empty(),
        "the write is in no undo step"
    );
    assert!(dirty(&k));
    save(&mut k, &s);
    assert!(!dirty(&k));
    let after = on_disk(&path);
    assert_ne!(after.bytes, before.bytes, "the outside write was not saved");
    assert!(String::from_utf8_lossy(&after.bytes).contains("Written outside"));
}

/// A file touched or replaced by anything else since the session read or
/// wrote it is written by the next Cmd-S, through the same handling as a
/// save with changes.
#[test]
fn a_file_changed_on_disk_then_save_writes() {
    // Touched: same bytes, a new modified time.
    let root = TempDir::new("clean-save-touched");
    let (mut k, s) = fixture(&root);
    let path = root.path("project.pmv");
    let before = on_disk(&path);
    let file = fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(before.modified - std::time::Duration::from_secs(60))
        .unwrap();
    drop(file);
    let touched = on_disk(&path);
    assert_ne!(touched.modified, before.modified);
    let ShellReply::Saved { displaced, .. } = save(&mut k, &s) else {
        unreachable!()
    };
    let after = on_disk(&path);
    assert_ne!(
        after.modified, touched.modified,
        "a touched file was not written"
    );
    assert_ne!(after.inode, touched.inode, "the save publishes a new file");
    assert_eq!(displaced, None);
    // Written now: the next clean Cmd-S writes nothing.
    save(&mut k, &s);
    assert_eq!(on_disk(&path), after);

    // Moved: the session follows it to its new name, and writes it there.
    let moved = root.path("moved.pmv");
    fs::rename(&path, &moved).unwrap();
    let at_new_name = on_disk(&moved);
    save(&mut k, &s);
    assert_ne!(
        on_disk(&moved).inode,
        at_new_name.inode,
        "a moved file was not written"
    );
    fs::rename(&moved, &path).unwrap();
    save(&mut k, &s);
    let after = on_disk(&path);
    save(&mut k, &s);
    assert_eq!(on_disk(&path), after);

    // Replaced by another program: the save is refused as any save over a
    // version the session never read is, and the work goes to a recovery
    // copy in the library; the other program's file stays as it is.
    let other = root.path("other.pmv");
    fs::write(&other, b"not this session's file").unwrap();
    fs::rename(&other, &path).unwrap();
    let replaced = on_disk(&path);
    let ShellReply::Saved { recovery_copy, .. } = save(&mut k, &s) else {
        unreachable!()
    };
    assert!(recovery_copy.is_some(), "a replaced file was written over");
    assert_eq!(on_disk(&path), replaced, "the other file stays as it is");
}

/// A session that undid and redid back to the saved state (the same steps
/// the file holds) has journal records; a skipped Cmd-S retires that
/// journal session as a written save does (a fresh one, based on the file,
/// takes its place), and a crash after it offers no recovery. Without the
/// save the session with those records is still there.
#[test]
fn a_skipped_save_retires_the_journal() {
    for save_it in [false, true] {
        let root = TempDir::new("clean-save-journal");
        let (mut k, s) = fixture(&root);
        let path = root.path("project.pmv");
        let journals = root.path("journals/p");
        rename(&mut k, "Edited");
        save(&mut k, &s);
        call(&mut k, HistoryCall::Undo);
        call(&mut k, HistoryCall::Redo);
        assert!(!dirty(&k));
        k.durability_barrier().unwrap();
        let with_records = journal::sessions(&journals);
        assert!(!with_records.is_empty());
        let before = on_disk(&path);
        if save_it {
            save(&mut k, &s);
            assert_eq!(on_disk(&path), before, "the clean save wrote");
        }
        k.durability_barrier().unwrap();
        let now = journal::sessions(&journals);
        for old in &with_records {
            assert_eq!(
                now.contains(old),
                !save_it,
                "saved: {save_it}: {old:?} in {now:?}"
            );
        }
        drop(k); // a crash: no quit, no close
        let mut k = kernel(&root.0);
        let reply = k
            .shell(ShellCall::PrepareOpen {
                id: "p".into(),
                source: SessionSource::File(path.clone()),
                electron_profile: None,
            })
            .unwrap();
        assert!(
            matches!(reply, ShellReply::Opened { .. }),
            "saved: {save_it}: {reply:?}"
        );
    }
}

/// The app's Cmd-S: the actor prepares the save, a worker runs it, the actor
/// finishes it. Clean, it writes nothing, says it saved, and retires the
/// journal records of an edit undone and redone back to the saved state.
#[test]
fn the_apps_clean_save_writes_nothing_and_retires_the_journal() {
    let root = TempDir::new("clean-save-actor");
    let path = root.path("project.pmv");
    {
        let (mut k, s) = fixture(&root);
        k.shell(ShellCall::Close(s)).unwrap();
    }
    let actor = KernelActor::spawn(config(&root.0));
    let k = actor.handle();
    let s = match k
        .shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::File(path.clone()),
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    };
    let request = |c: Call| {
        k.call("p", Request::new(Caller::native(), c))
            .wait()
            .unwrap()
            .unwrap();
    };
    request(rename_call("Edited"));
    let plan = k.prepare_save(s.clone(), None).wait().unwrap().unwrap();
    let completion = plan.run();
    k.finish_save(completion).wait().unwrap().unwrap();
    request(HistoryCall::Undo.into());
    request(HistoryCall::Redo.into());
    let before = on_disk(&path);
    for _ in 0..3 {
        let plan = k.prepare_save(s.clone(), None).wait().unwrap().unwrap();
        let completion = plan.run();
        let reply = k.finish_save(completion).wait().unwrap().unwrap();
        assert!(matches!(reply, ShellReply::Saved { .. }), "{reply:?}");
        assert_eq!(on_disk(&path), before, "a clean Cmd-S rewrote the file");
    }
    k.sync_journals().unwrap();
    drop(k);
    drop(actor); // a crash: no quit, no close
    let mut k = kernel(&root.0);
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "p".into(),
            source: SessionSource::File(path),
            electron_profile: None,
        })
        .unwrap();
    assert!(matches!(reply, ShellReply::Opened { .. }), "{reply:?}");
}

/// "Finish Edit" seals an edit left open into a step: unsaved work, so the
/// Cmd-S after it writes. (A save is refused while the edit is open.) An
/// edit left open that changed nothing seals to no step and leaves the
/// project clean: that Cmd-S writes nothing.
#[test]
fn a_save_after_finish_edit_writes() {
    let root = TempDir::new("clean-save-sealed");
    let (mut k, s) = fixture(&root);
    let path = root.path("project.pmv");
    let before = on_disk(&path);
    let begin = |k: &mut Kernel| {
        call(
            k,
            GestureCall::Begin {
                label: "Move".into(),
            },
        )
    };
    // Changed nothing: no step, still clean, nothing written.
    begin(&mut k);
    assert!(matches!(
        call(&mut k, HistoryCall::Seal),
        Reply::Bool(false)
    ));
    assert!(!dirty(&k));
    save(&mut k, &s);
    assert_eq!(on_disk(&path), before);
    // An edit left open, then Finish Edit: written.
    begin(&mut k);
    rename(&mut k, "Moved");
    assert!(
        k.shell(ShellCall::Save(s.clone())).is_err(),
        "a save waits for the open edit"
    );
    assert!(matches!(call(&mut k, HistoryCall::Seal), Reply::Bool(true)));
    assert!(dirty(&k), "the sealed edit is unsaved work");
    save(&mut k, &s);
    assert!(!dirty(&k));
    assert_ne!(
        on_disk(&path).bytes,
        before.bytes,
        "Cmd-S after Finish Edit was skipped"
    );
    k.shell(ShellCall::Close(s)).unwrap();
    opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::File(path),
            electron_profile: None,
        })
        .unwrap(),
    );
    assert_eq!(
        k.project("p").unwrap().store().history().labels().last(),
        Some(&"Move")
    );
}

fn name_of(k: &Kernel) -> Option<Value> {
    let root = k.project("p").unwrap().store().root().cloned();
    root.and_then(|r| r.get("name").map(|n| n.to_value()))
}

/// Save, change the project, Undo back to the saved state, Cmd-S: the
/// document is as saved but the undone change is a redo step only memory
/// and the journal hold. The save writes it, and after a crash the reopened
/// file can Redo it (review w31 clean save, finding 1).
#[test]
fn a_save_after_undo_keeps_the_redo_step() {
    let root = TempDir::new("clean-save-redo");
    let (mut k, s) = fixture(&root);
    let path = root.path("project.pmv");
    let before = on_disk(&path);
    rename(&mut k, "Redo me");
    call(&mut k, HistoryCall::Undo);
    assert!(!dirty(&k));
    save(&mut k, &s);
    assert_ne!(
        on_disk(&path).bytes,
        before.bytes,
        "the redo step was not written"
    );
    k.durability_barrier().unwrap();
    drop(k); // a crash
    let mut k = kernel(&root.0);
    opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::File(path),
            electron_profile: None,
        })
        .unwrap(),
    );
    assert_eq!(name_of(&k), Some(Value::from("Base")));
    let redo = call(&mut k, HistoryCall::Redo);
    assert_eq!(name_of(&k), Some(Value::from("Redo me")), "{redo:?}");
}

/// Undo then Redo back to the saved state, nothing else changed: the same
/// steps and the same applied count as the file holds, so Cmd-S may skip.
#[test]
fn undo_then_redo_back_to_the_saved_state_skips() {
    let root = TempDir::new("clean-save-redo-back");
    let (mut k, s) = fixture(&root);
    let path = root.path("project.pmv");
    rename(&mut k, "Edited");
    save(&mut k, &s);
    let written = on_disk(&path);
    call(&mut k, HistoryCall::Undo);
    call(&mut k, HistoryCall::Redo);
    assert!(!dirty(&k));
    save(&mut k, &s);
    assert_eq!(on_disk(&path), written);
}

fn workspace(k: &mut Kernel) -> Value {
    match call(k, WorkspaceCall::Get) {
        Reply::Workspace(w) => w,
        other => panic!("{other:?}"),
    }
}

/// Rearranged panels and nothing else: Cmd-S writes the new layout into the
/// file, so the file opened in another profile has it (review w31 clean
/// save, finding 2). Setting the same layout again skips.
#[test]
fn a_layout_change_is_written_by_cmd_s() {
    let root = TempDir::new("clean-save-layout");
    let (mut k, s) = fixture(&root);
    let path = root.path("project.pmv");
    let before = on_disk(&path);
    let expected = workspace(&mut k);
    let mut next = match &expected {
        Value::Object(map) => map.clone(),
        _ => pm_model::Map::new(),
    };
    next.insert("rearranged", Value::from(true));
    let next = Value::Object(next);
    call(
        &mut k,
        WorkspaceCall::Replace {
            expected,
            next: next.clone(),
        },
    );
    assert!(!dirty(&k), "a layout change is not an edit");
    save(&mut k, &s);
    let written = on_disk(&path);
    assert_ne!(
        written.bytes, before.bytes,
        "the new layout was not written"
    );
    // The same layout again: nothing to write.
    let current = workspace(&mut k);
    call(
        &mut k,
        WorkspaceCall::Replace {
            expected: current,
            next: next.clone(),
        },
    );
    save(&mut k, &s);
    assert_eq!(on_disk(&path), written);
    k.shell(ShellCall::Close(s)).unwrap();
    drop(k);
    // Another profile, which never saw this project: the file's layout.
    let other = TempDir::new("clean-save-layout-fresh");
    let copy = other.path("project.pmv");
    fs::copy(&path, &copy).unwrap();
    let mut k = kernel(&other.0);
    opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::File(copy),
            electron_profile: None,
        })
        .unwrap(),
    );
    assert_eq!(
        workspace(&mut k).get("rearranged"),
        Some(&Value::from(true)),
        "the fresh profile opened the old layout"
    );
}
