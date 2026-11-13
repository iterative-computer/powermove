//! Slice 2: durable library writes, lock policy, state and shell workflows.
mod common;
use pm_kernel::{
    library::{DYNAMIC_PREFIXES, LibraryStore, STATIC_KEYS},
    locks::*,
    persistence::{DiskFormat, LibraryWriter, encode_disk_value},
    session::*,
    state::*,
    *,
};
use pm_model::json::{Value, parse, to_canonical_string};
use std::{
    fs,
    path::Path,
    process::{Command as Process, Stdio},
    time::Duration,
};
const PROJECT: &str = r##"{"id":"P","name":"Fixture","compId":"C1","compName":"Main","w":200,"h":100,"fps":30,"dur":2,"bg":"#000000","assets":{},"layers":[{"id":"layer0","type":"shape","name":"Layer","from":0,"dur":2,"on":true,"p":{"opacity":{"v":100,"kf":[],"expr":null}}}],"comps":{},"future":{"kept":true}}"##;
fn value() -> Value {
    parse(PROJECT).unwrap()
}
fn writer(path: &Path) -> LibraryWriter {
    LibraryWriter::open(path, DiskFormat::V1_2, None).unwrap_or_else(|e| panic!("{e:?}"))
}
fn kernel(path: &Path) -> Kernel {
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(path.join("journals")),
        trash: Some(common::trash(&path.join("Trash"))),
        ..Default::default()
    });
    assert!(matches!(
        k.shell(ShellCall::AttachLibrary {
            profile: path.into(),
            format: DiskFormat::V1_2,
            electron_profile: None
        })
        .unwrap(),
        ShellReply::Library(OpenDecision::Writable)
    ));
    k
}
fn open(k: &mut Kernel, id: &str, source: SessionSource) -> SessionHandle {
    match k
        .shell(ShellCall::Open {
            id: id.into(),
            source,
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        _ => panic!(),
    }
}
fn edit(k: &mut Kernel, id: &str, n: f64) {
    let a = k
        .handle(
            id,
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Opacity".into()),
                    body: WriteBody::Commands(vec![Command::SetStaticOpacity {
                        layer_id: "layer0".into(),
                        value: n,
                    }]),
                    expected_revision: None,
                },
            ),
        )
        .unwrap()
        .reply
        .unwrap();
    assert!(matches!(a.reply, Reply::Applied { changed: true, .. }));
}
fn state(ops: Vec<StateOp>) -> StateTransaction {
    StateTransaction {
        ops,
        ..Default::default()
    }
}
fn setting(key: &str, value: Option<Value>) -> StateOp {
    StateOp::Setting {
        key: key.into(),
        value,
    }
}

#[test]
fn debounce_and_flush_keep_latest_value_and_no_temp_files() {
    let dir = common::TempDir::new("debounce");
    let w = writer(&dir.0);
    w.set("theme", Value::from("one")).unwrap();
    w.set("theme", Value::from("two")).unwrap();
    assert!(!dir.path("store/theme.json").exists());
    w.flush().unwrap();
    assert_eq!(w.store().read("theme").unwrap(), Some(Value::from("two")));
    assert!(
        !fs::read_dir(dir.path("store")).unwrap().any(|f| f
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp-"))
    );
    w.delete("theme").unwrap();
    w.flush().unwrap();
    assert!(w.store().read("theme").unwrap().is_none());
}
#[test]
fn debounce_eventually_writes_without_a_flush() {
    let dir = common::TempDir::new("timer");
    let w = writer(&dir.0);
    w.set("theme", Value::from("ready")).unwrap();
    let start = std::time::Instant::now();
    while !dir.path("store/theme.json").exists() {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(w.store().read("theme").unwrap(), Some(Value::from("ready")));
}
#[test]
fn failed_write_is_retained_for_retry() {
    let dir = common::TempDir::new("failed");
    let w = writer(&dir.0);
    fs::create_dir(dir.path("store/theme.json")).unwrap();
    w.set("theme", Value::from("kept")).unwrap();
    assert!(w.flush().is_err());
    fs::remove_dir(dir.path("store/theme.json")).unwrap();
    w.flush().unwrap();
    assert_eq!(w.store().read("theme").unwrap(), Some(Value::from("kept")));
    assert!(w.set("project../bad", Value::Null).is_err());
}
#[test]
fn gzip_threshold_and_plain_v12_are_explicit() {
    for n in [32765, 32766, 32767, 40000] {
        let v = Value::from("a".repeat(n));
        let plain = encode_disk_value(&v, DiskFormat::V1_2).unwrap();
        assert_eq!(parse(std::str::from_utf8(&plain).unwrap()).unwrap(), v);
        let encoded = encode_disk_value(&v, DiskFormat::V1_3).unwrap();
        let disk = parse(std::str::from_utf8(&encoded).unwrap()).unwrap();
        assert_eq!(disk.get("__powermoveDiskValue").is_some(), n + 2 >= 32768);
        assert_eq!(
            pm_kernel::library::decode_disk_value(disk, 1 << 20).unwrap(),
            v
        );
    }
}
#[test]
fn legacy_partial_state_writes_keep_history_even_when_debounced() {
    let dir = common::TempDir::new("legacy-state");
    let w = writer(&dir.0);
    w.set(
        "projectState.P",
        parse(r#"{"time":1,"history":{"version":1,"entries":[],"index":-1}}"#).unwrap(),
    )
    .unwrap();
    w.set("projectState.P", parse(r#"{"time":2}"#).unwrap())
        .unwrap();
    w.flush().unwrap();
    assert!(
        w.store()
            .read("projectState.P")
            .unwrap()
            .unwrap()
            .get("history")
            .is_some()
    );
}
#[test]
fn interrupted_bundle_is_replayed_after_reopening_the_library() {
    let dir = common::TempDir::new("intent");
    fs::create_dir(dir.path("store")).unwrap();
    fs::write(dir.path("store/library.pending"),r#"[{"key":"theme","deleted":false,"value":"restored"},{"key":"previewResolution","deleted":true}]"#).unwrap();
    fs::write(dir.path("store/previewResolution.json"), r#""half""#).unwrap();
    let w = writer(&dir.0);
    assert_eq!(
        w.store().read("theme").unwrap(),
        Some(Value::from("restored"))
    );
    assert!(!dir.path("store/library.pending").exists());
    assert!(w.store().read("previewResolution").unwrap().is_none());
}
#[test]
fn bad_bundle_is_not_partially_applied() {
    let dir = common::TempDir::new("bad-intent");
    fs::create_dir(dir.path("store")).unwrap();
    fs::write(
        dir.path("store/library.pending"),
        r#"[{"key":"theme","deleted":false,"value":"bad"},{"key":"../bad","deleted":true}]"#,
    )
    .unwrap();
    assert!(LibraryWriter::open(&dir.0, DiskFormat::V1_2, None).is_err());
    assert!(!dir.path("store/theme.json").exists());
    assert!(dir.path("store/library.pending").exists());
}
#[test]
fn create_edit_save_close_reopen_and_undo_after_rollback() {
    let dir = common::TempDir::new("lifecycle");
    let mut k = kernel(&dir.0);
    let h = open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 40.0);
    k.handle(
        "P",
        Request::new(
            Caller::native(),
            WorkspaceCall::Mutate {
                next: parse(r#"{"id":"design","name":"Design","layout":{"docks":[]},"future":17}"#)
                    .unwrap(),
                threw: None,
            },
        ),
    )
    .unwrap();
    let mut ui = k.state().ui["P"].clone();
    ui.timeline = parse(r#"{"pps":140,"scrollT":2,"graph":true}"#).unwrap();
    ui.transport = parse(r#"{"time":1.25,"playing":false}"#).unwrap();
    k.state_transaction(state(vec![StateOp::Ui {
        project: "P".into(),
        next: ui,
    }]))
    .unwrap();
    k.shell(ShellCall::Save(h.clone())).unwrap();
    assert!(!k.project("P").unwrap().store().is_dirty());
    let disk = LibraryStore::open(&dir.0);
    let p = disk.load("P").unwrap().unwrap();
    assert!(p.project.get("future").is_some());
    assert_eq!(
        p.workspace.as_ref().unwrap().get("future"),
        Some(&Value::Number(17.0))
    );
    let mut rollback = Kernel::new(KernelConfig::default());
    rollback.open("P", OpenSource::Library(p)).unwrap();
    assert!(matches!(
        rollback
            .handle("P", Request::new(Caller::native(), HistoryCall::Undo))
            .unwrap()
            .reply
            .unwrap()
            .reply,
        Reply::Bool(true)
    ));
    assert_eq!(
        common::opacity_of(rollback.project("P").unwrap().store(), "layer0"),
        Some(100.0)
    );
    k.shell(ShellCall::Close(h.clone())).unwrap();
    assert!(k.shell(ShellCall::Save(h)).is_err());
    drop(k);
    let mut k = kernel(&dir.0);
    open(
        &mut k,
        "P",
        SessionSource::Library {
            profile: dir.0.clone(),
            project: "P".into(),
        },
    );
    assert_eq!(
        k.state().ui["P"].transport.get("time"),
        Some(&Value::Number(1.25))
    );
    assert_eq!(
        common::opacity_of(k.project("P").unwrap().store(), "layer0"),
        Some(40.0)
    );
}
#[test]
fn duplicate_trash_restore_and_recents_keep_slots_and_history() {
    let dir = common::TempDir::new("trash");
    let mut k = kernel(&dir.0);
    let h = open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 50.0);
    k.shell(ShellCall::Close(h)).unwrap();
    k.shell(ShellCall::Duplicate {
        project: "P".into(),
        id: "copy".into(),
        name: "Copy".into(),
    })
    .unwrap();
    k.shell(ShellCall::Trash("P".into())).unwrap();
    let store = LibraryStore::open(&dir.0);
    assert_eq!(store.projects().unwrap().len(), 1);
    assert_eq!(store.trash().unwrap().len(), 1);
    assert!(store.load("P").unwrap().unwrap().history.is_some());
    k.shell(ShellCall::Restore("P".into())).unwrap();
    assert_eq!(store.projects().unwrap().len(), 2);
    assert!(store.trash().unwrap().is_empty());
    let ShellReply::Recent(rows) = k.shell(ShellCall::Recent).unwrap() else {
        panic!()
    };
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|r| r.get("w").is_some() && r.get("h").is_some())
    );
}
#[test]
fn save_as_pmv_and_changed_file_fall_back_to_a_recovery_slot() {
    let dir = common::TempDir::new("file-save");
    let mut k = kernel(&dir.0);
    let h = open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 60.0);
    let file = dir.path("export.pmv");
    k.shell(ShellCall::SaveAs {
        session: h.clone(),
        path: file.clone(),
        replace: false,
    })
    .unwrap();
    assert!(
        !dir.path("export.pmvlock").exists(),
        "nothing is written beside the project"
    );
    assert!(
        lock_project(&k.lock_root(), &file).is_err(),
        "the saved file is locked"
    );
    let mut old = common::open_store(&file);
    assert_eq!(common::opacity_of(&old, "layer0"), Some(60.0));
    old.undo().unwrap();
    assert_eq!(common::opacity_of(&old, "layer0"), Some(100.0));
    edit(&mut k, "P", 30.0);
    fs::write(&file, b"changed elsewhere").unwrap();
    let ShellReply::Saved {
        recovery_copy: Some(copy),
        ..
    } = k.shell(ShellCall::Save(h)).unwrap()
    else {
        panic!()
    };
    assert_eq!(fs::read(file).unwrap(), b"changed elsewhere");
    assert!(LibraryStore::open(&dir.0).load(&copy).unwrap().is_some());
}
#[test]
fn recovery_base_mismatch_offers_a_copy_and_keeps_current_source() {
    let dir = common::TempDir::new("mismatch");
    let mut k = kernel(&dir.0);
    open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 25.0);
    k.sync_all();
    drop(k);
    {
        let w = writer(&dir.0);
        let mut p = w.store().load("P").unwrap().unwrap();
        if let Value::Object(m) = &mut p.project {
            m.insert("name", Value::from("Other writer"));
        }
        w.save_project(
            "P",
            p.project,
            p.history.unwrap(),
            p.workspace.unwrap_or(Value::Null),
            p.session.unwrap_or(Value::Null),
        )
        .unwrap();
    }
    let mut k = kernel(&dir.0);
    let ShellReply::Opened {
        recovery: RecoveryResult::OfferCopy { project: copy, .. },
        ..
    } = k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::Library {
                profile: dir.0.clone(),
                project: "P".into(),
            },
            electron_profile: None,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        k.project("P")
            .unwrap()
            .store()
            .snapshot()
            .document
            .to_value()
            .unwrap()
            .get("name")
            .and_then(Value::as_str),
        Some("Other writer")
    );
    let p = LibraryStore::open(&dir.0).load(&copy).unwrap().unwrap();
    let mut recovered = Kernel::new(KernelConfig::default());
    recovered.open("copy", OpenSource::Library(p)).unwrap();
    assert_eq!(
        common::opacity_of(recovered.project("copy").unwrap().store(), "layer0"),
        Some(25.0)
    );
}
#[test]
fn successful_save_rebases_the_journal_and_next_crash_recovers_new_edits() {
    let dir = common::TempDir::new("rebase");
    let mut k = kernel(&dir.0);
    let h = open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 80.0);
    let old = k.base_hash("P").unwrap().to_owned();
    k.shell(ShellCall::Save(h)).unwrap();
    assert_ne!(k.base_hash("P").unwrap(), old);
    edit(&mut k, "P", 10.0);
    k.sync_all();
    drop(k);
    let mut k = kernel(&dir.0);
    let ShellReply::Opened { recovery, .. } = k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::Library {
                profile: dir.0.clone(),
                project: "P".into(),
            },
            electron_profile: None,
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(recovery, RecoveryResult::Recovered { .. }));
    assert_eq!(
        common::opacity_of(k.project("P").unwrap().store(), "layer0"),
        Some(10.0)
    );
}
#[test]
fn a_read_only_folder_is_a_copy_decision_and_a_syncing_folder_is_not() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::TempDir::new("policy");
    for path in [
        "Library/Mobile Documents/com~apple~CloudDocs/movie.pmv",
        "Dropbox (Personal)/movie.pmv",
        "OneDrive-Work/movie.pmv",
    ] {
        assert_eq!(open_decision(&dir.path(path), None), OpenDecision::Writable);
    }
    let ro = dir.path("readonly");
    fs::create_dir(&ro).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    assert_eq!(
        open_decision(&ro.join("p.pmv"), None),
        OpenDecision::AsCopy(CopyReason::ReadOnly)
    );
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
}
fn open_file(k: &mut Kernel, file: &Path) -> (SessionHandle, OpenDecision) {
    match k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::File(file.into()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened {
            session, decision, ..
        } => (session, decision),
        _ => panic!(),
    }
}
/// A project in a folder a sync client keeps (by name, or marked with a
/// file-provider attribute) opens in place: Save writes the file, the next
/// open holds the edit, and no library copy is made. A version the client
/// lands underneath between open and save is never overwritten: the save
/// keeps the file as the client left it and the work goes to a recovery
/// copy.
#[test]
fn a_project_in_a_syncing_folder_opens_and_saves_in_place() {
    let dir = common::TempDir::new("syncing");
    let folder = dir.path("Dropbox/job");
    fs::create_dir_all(&folder).unwrap();
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let p = std::ffi::CString::new(folder.as_os_str().as_bytes()).unwrap();
        let name = std::ffi::CString::new("com.apple.file-provider.synthetic").unwrap();
        assert_eq!(
            unsafe { libc::setxattr(p.as_ptr(), name.as_ptr(), b"1".as_ptr().cast(), 1, 0, 0) },
            0
        );
    }
    let file = folder.join("cut.pmv");
    fs::write(&file, PROJECT).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::Writable);
    edit(&mut k, "P", 20.0);
    assert!(matches!(
        k.shell(ShellCall::Save(session.clone())).unwrap(),
        ShellReply::Saved {
            recovery_copy: None,
            ..
        }
    ));
    assert_eq!(
        common::opacity_of(&common::open_store(&file), "layer0"),
        Some(20.0),
        "Save wrote the file in the syncing folder"
    );
    k.shell(ShellCall::Close(session)).unwrap();
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::Writable);
    assert_eq!(
        common::opacity_of(k.project("P").unwrap().store(), "layer0"),
        Some(20.0),
        "the next open starts from the saved work"
    );
    let rows = LibraryStore::open(&dir.path("native")).projects().unwrap();
    assert!(
        rows.iter().all(|row| !row.name.ends_with("— Native")),
        "no copy was made: {:?}",
        rows.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
    // Save As into the syncing folder works too.
    let beside = folder.join("cut 2.pmv");
    k.shell(ShellCall::SaveAs {
        session: session.clone(),
        path: beside.clone(),
        replace: false,
    })
    .unwrap();
    assert_eq!(
        common::opacity_of(&common::open_store(&beside), "layer0"),
        Some(20.0)
    );
    // The sync client lands another Mac's version (a new file at the path).
    edit(&mut k, "P", 55.0);
    let landed = folder.join(".landing");
    fs::write(&landed, PROJECT).unwrap();
    fs::rename(&landed, &beside).unwrap();
    let theirs = fs::read(&beside).unwrap();
    let ShellReply::Saved {
        recovery_copy: Some(copy),
        ..
    } = k.shell(ShellCall::Save(session)).unwrap()
    else {
        panic!("a save over a version landed underneath must not succeed in place")
    };
    assert_eq!(fs::read(&beside).unwrap(), theirs, "their version is kept");
    let kept = LibraryStore::open(&dir.path("native"))
        .load(&copy)
        .unwrap()
        .expect("the work is kept as a recovery copy");
    assert!(to_canonical_string(&kept.project).contains("55"));
}
/// A file that cannot be opened in place (read-only here) opens as a copy,
/// and opens that same copy every time: the work done in it is there, and
/// the library holds one copy however often the file is opened.
#[test]
fn a_file_that_must_open_as_a_copy_reopens_the_same_copy() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::TempDir::new("same-copy");
    let file = dir.path("original.pmv");
    fs::write(&file, PROJECT).unwrap();
    let bytes = fs::read(&file).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
    let mut k = kernel(&dir.path("native"));
    let copies = |dir: &common::TempDir| {
        LibraryStore::open(&dir.path("native"))
            .projects()
            .unwrap()
            .into_iter()
            .filter(|row| row.name.ends_with("— Native"))
            .count()
    };
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::AsCopy(CopyReason::ReadOnly));
    edit(&mut k, "P", 20.0);
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(copies(&dir), 1);
    for opened in 2..=3 {
        let (session, decision) = open_file(&mut k, &file);
        assert_eq!(
            decision,
            OpenDecision::AsCopy(CopyReason::ReadOnly),
            "open {opened} still says it is a copy, and why"
        );
        assert_eq!(
            common::opacity_of(k.project("P").unwrap().store(), "layer0"),
            Some(20.0),
            "open {opened} is the copy with the work in it"
        );
        let meta = k.session_metadata("P").unwrap().metadata;
        assert_eq!(
            meta.get("copyOf")
                .and_then(|c| c.get("path"))
                .and_then(Value::as_str),
            file.to_str()
        );
        k.shell(ShellCall::Close(session)).unwrap();
        assert_eq!(copies(&dir), 1, "open {opened} made another copy");
    }
    assert_eq!(fs::read(&file).unwrap(), bytes, "the file is untouched");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
}
fn copy_slot(k: &Kernel) -> String {
    match k.session_metadata("P").unwrap().destination {
        SessionDestination::Library(slot) => slot,
        other => panic!("{other:?}"),
    }
}
fn copies_of(k: &mut Kernel, session: &SessionHandle) -> Vec<Value> {
    match k.shell(ShellCall::CopiesOf(session.clone())).unwrap() {
        ShellReply::Copies(rows) => rows,
        _ => panic!(),
    }
}
/// A copy belongs to the file it was made from, not to the name that file
/// had. Read-only file A opens as a copy and is edited there; then another
/// file B is put at A's name. Opening B shows B, as a copy of its own, and
/// says once that an earlier copy exists for that name. A's copy keeps A's
/// work, and A back at the name opens it again.
#[test]
fn a_different_file_at_a_copied_files_name_gets_a_copy_of_its_own() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::TempDir::new("copy-identity");
    let file = dir.path("cut.pmv");
    let put = |bytes: &str| {
        let _ = fs::remove_file(&file);
        fs::write(&file, bytes).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
    };
    let other = PROJECT.replace("Fixture", "Another");
    put(PROJECT);
    let mut k = kernel(&dir.path("native"));
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::AsCopy(CopyReason::ReadOnly));
    let first = copy_slot(&k);
    edit(&mut k, "P", 20.0);
    k.shell(ShellCall::Close(session)).unwrap();

    put(&other);
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::AsCopy(CopyReason::ReadOnly));
    let store = k.project("P").unwrap().store();
    assert_eq!(
        common::opacity_of(store, "layer0"),
        Some(100.0),
        "the file that is there now opened, not the copy of the one before"
    );
    let second = copy_slot(&k);
    assert_ne!(second, first, "a copy of its own");
    let rows = copies_of(&mut k, &session);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].get("id").and_then(Value::as_str), Some(&*first));
    assert_eq!(rows[0].get("different"), Some(&Value::from(true)));
    assert_eq!(rows[0].get("file").and_then(Value::as_str), Some("cut.pmv"));
    assert!(copies_of(&mut k, &session).is_empty(), "said once");
    edit(&mut k, "P", 70.0);
    k.shell(ShellCall::Close(session)).unwrap();
    // Each file opens its own copy from then on.
    let (session, _) = open_file(&mut k, &file);
    assert_eq!(copy_slot(&k), second);
    assert_eq!(
        common::opacity_of(k.project("P").unwrap().store(), "layer0"),
        Some(70.0)
    );
    k.shell(ShellCall::Close(session)).unwrap();
    put(PROJECT);
    let (session, _) = open_file(&mut k, &file);
    assert_eq!(copy_slot(&k), first);
    assert_eq!(
        common::opacity_of(k.project("P").unwrap().store(), "layer0"),
        Some(20.0),
        "the first file's work is still in its copy"
    );
    k.shell(ShellCall::Close(session)).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
}
/// The same for a link: the name is a symbolic link to a read-only file, and
/// the link is pointed at another file. The copy made through the link was
/// of the first file; the second opens on its own.
#[test]
fn a_link_pointed_at_another_file_does_not_open_the_first_files_copy() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = common::TempDir::new("copy-link");
    fs::create_dir_all(dir.path("store")).unwrap();
    let (a, b) = (dir.path("store/a.pmv"), dir.path("store/b.pmv"));
    fs::write(&a, PROJECT).unwrap();
    fs::write(&b, PROJECT.replace("Fixture", "Another")).unwrap();
    for file in [&a, &b] {
        fs::set_permissions(file, fs::Permissions::from_mode(0o444)).unwrap();
    }
    let link = dir.path("cut.pmv");
    symlink(&a, &link).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, decision) = open_file(&mut k, &link);
    assert_eq!(decision, OpenDecision::AsCopy(CopyReason::ReadOnly));
    let first = copy_slot(&k);
    edit(&mut k, "P", 20.0);
    k.shell(ShellCall::Close(session)).unwrap();
    fs::remove_file(&link).unwrap();
    symlink(&b, &link).unwrap();
    let (session, _) = open_file(&mut k, &link);
    assert_ne!(copy_slot(&k), first);
    assert_eq!(
        common::opacity_of(k.project("P").unwrap().store(), "layer0"),
        Some(100.0)
    );
    assert_eq!(copies_of(&mut k, &session).len(), 1);
    k.shell(ShellCall::Close(session)).unwrap();
    for file in [&a, &b] {
        fs::set_permissions(file, fs::Permissions::from_mode(0o644)).unwrap();
    }
}
/// A file that opens as a copy because the original app is running opens
/// that same copy every time, like any other copy: the copy is this
/// library's own project, and what kept the file from opening in place is
/// not applied to it again (which made a copy of the copy on every open).
#[test]
fn a_copy_made_while_the_original_app_runs_is_reopened_not_copied_again() {
    use std::os::unix::fs::symlink;
    let dir = common::TempDir::new("copy-electron");
    let electron = dir.path("electron");
    fs::create_dir_all(&electron).unwrap();
    let mut hostname = [0i8; 256];
    unsafe {
        libc::gethostname(hostname.as_mut_ptr(), 255);
    };
    let hostname = unsafe { std::ffi::CStr::from_ptr(hostname.as_ptr()) }.to_string_lossy();
    symlink(
        format!("{hostname}-{}", std::process::id()),
        electron.join("SingletonLock"),
    )
    .unwrap();
    let file = dir.path("cut.pmv");
    fs::write(&file, PROJECT).unwrap();
    let mut k = kernel(&dir.path("native"));
    let open = |k: &mut Kernel| match k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: Some(electron.clone()),
        })
        .unwrap()
    {
        ShellReply::Opened {
            session, decision, ..
        } => (session, decision),
        _ => panic!(),
    };
    let (session, decision) = open(&mut k);
    assert_eq!(decision, OpenDecision::AsCopy(CopyReason::OlderElectron));
    let first = copy_slot(&k);
    edit(&mut k, "P", 20.0);
    k.shell(ShellCall::Close(session)).unwrap();
    for _ in 0..2 {
        let (session, decision) = open(&mut k);
        assert_eq!(decision, OpenDecision::AsCopy(CopyReason::OlderElectron));
        assert_eq!(copy_slot(&k), first, "the same copy, not a copy of it");
        assert_eq!(
            common::opacity_of(k.project("P").unwrap().store(), "layer0"),
            Some(20.0)
        );
        k.shell(ShellCall::Close(session)).unwrap();
    }
}
/// A file that opens in place again is told about the copies that hold work
/// for it: the copy an earlier build made of a file in a syncing folder
/// (named "<project> — Native", nothing recorded) and a copy recorded for
/// the file. Each is reported once, the file's entry remembers it across a
/// close, and the copies themselves are untouched.
#[test]
fn a_file_opened_in_place_is_told_once_about_its_stranded_copies() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::TempDir::new("stranded");
    let file = dir.path("Dropbox/cut.pmv");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, PROJECT).unwrap();
    let native = dir.path("native");
    // Two copies made while the file could not open in place.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
    let mut k = kernel(&native);
    let (session, _) = open_file(&mut k, &file);
    edit(&mut k, "P", 20.0);
    k.shell(ShellCall::Close(session)).unwrap();
    drop(k);
    let copy = |native: &Path| {
        let rows = LibraryStore::open(native).projects().unwrap();
        let row = rows.iter().find(|r| r.name.ends_with("— Native")).unwrap();
        (row.id.clone(), row.name.clone())
    };
    let (recorded, name) = copy(&native);
    // The same copy as an earlier build left it: nothing says whose it is.
    let store = LibraryStore::open(&native);
    let project = store.load(&recorded).unwrap().unwrap();
    let w = writer(&native);
    let mut meta = project
        .session
        .clone()
        .unwrap()
        .as_object()
        .cloned()
        .unwrap();
    meta.remove("copyOf");
    w.save_project(
        "copy-earlier",
        project.project.clone(),
        Value::Null,
        Value::Null,
        Value::Object(meta),
    )
    .unwrap();
    w.flush().unwrap();
    drop(w);
    let before = fs::read(native.join(format!("project.{recorded}.json"))).ok();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    let mut k = kernel(&native);
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::Writable);
    let copies = |k: &mut Kernel, session: &SessionHandle| match k
        .shell(ShellCall::CopiesOf(session.clone()))
        .unwrap()
    {
        ShellReply::Copies(rows) => rows
            .iter()
            .map(|r| r.get("id").and_then(Value::as_str).unwrap().to_owned())
            .collect::<Vec<_>>(),
        _ => panic!(),
    };
    let mut told = copies(&mut k, &session);
    told.sort();
    let mut expected = vec!["copy-earlier".to_owned(), recorded.clone()];
    expected.sort();
    assert_eq!(told, expected);
    assert!(copies(&mut k, &session).is_empty(), "told once");
    k.shell(ShellCall::Close(session)).unwrap();
    let (session, _) = open_file(&mut k, &file);
    assert!(
        copies(&mut k, &session).is_empty(),
        "the file's entry remembers across a close"
    );
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(copy(&native).1, name);
    assert_eq!(
        fs::read(native.join(format!("project.{recorded}.json"))).ok(),
        before,
        "the copy is untouched"
    );
}
fn rename_comp(k: &mut Kernel, id: &str, name: &str) {
    use pm_kernel::commands::comps::{CompCommand, CompPatch};
    k.handle(
        id,
        Request::new(
            Caller::native(),
            Call::Transact {
                label: Some("Rename".into()),
                body: WriteBody::Commands(vec![
                    CompCommand::Set {
                        patch: Box::new(CompPatch {
                            name: Some(name.into()),
                            ..Default::default()
                        }),
                    }
                    .into(),
                ]),
                expected_revision: None,
            },
        ),
    )
    .unwrap();
}
fn saved_name(file: &Path) -> Option<String> {
    let opened = open_project(file, OpenOptions::default()).unwrap();
    opened
        .document
        .root()
        .and_then(|r| r.get("name"))
        .map(|n| n.to_value())
        .and_then(|n| n.as_str().map(str::to_owned))
}
/// The open project's file (with embedded media) is renamed, then moved to
/// another folder, in Finder: Save keeps writing that file where it is now,
/// by the app's own save route. Then it is deleted: Save cannot write it,
/// but the work and every embedded item are kept (a recovery copy), and Save
/// As writes a complete file from what the session holds.
#[test]
fn a_file_renamed_moved_or_deleted_while_open_can_still_be_saved() {
    let dir = common::TempDir::new("follow");
    fs::create_dir_all(dir.path("job")).unwrap();
    fs::create_dir_all(dir.path("archive")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::Writable);
    let media = k.project("P").unwrap().store().media().len();
    assert!(media > 0);
    let save = |k: &mut Kernel| {
        let plan = k.prepare_session_save(session.clone(), None, false)?;
        k.finish_session_save(plan.run())
    };
    let destination = |k: &Kernel| match k.session_metadata("P").unwrap().destination {
        SessionDestination::File(path) => path,
        other => panic!("{other:?}"),
    };
    // Renamed in place.
    let renamed = dir.path("job/cut v2.pmv");
    fs::rename(&file, &renamed).unwrap();
    rename_comp(&mut k, "P", "One");
    save(&mut k).expect("a renamed file is still saved");
    assert_eq!(saved_name(&renamed).as_deref(), Some("One"));
    assert!(!file.exists(), "nothing is written back at the old name");
    assert_eq!(
        destination(&k).canonicalize().unwrap(),
        renamed.canonicalize().unwrap()
    );
    // Moved to another folder.
    let moved = dir.path("archive/cut v2.pmv");
    fs::rename(&renamed, &moved).unwrap();
    rename_comp(&mut k, "P", "Two");
    save(&mut k).expect("a moved file is still saved");
    assert_eq!(saved_name(&moved).as_deref(), Some("Two"));
    let saved = open_project(&moved, OpenOptions::default()).unwrap();
    assert_eq!(
        saved.file.media.len(),
        media,
        "its embedded media went with it"
    );
    // Deleted: no file to write, nothing lost, and Save As works.
    fs::remove_file(&moved).unwrap();
    rename_comp(&mut k, "P", "Three");
    assert!(
        save(&mut k).is_err(),
        "a file that is gone cannot be saved in place"
    );
    let rescued = dir.path("rescued.pmv");
    let plan = k
        .prepare_session_save(session.clone(), Some(rescued.clone()), false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("Save As works from what the session holds");
    assert_eq!(saved_name(&rescued).as_deref(), Some("Three"));
    assert_eq!(
        open_project(&rescued, OpenOptions::default())
            .unwrap()
            .file
            .media
            .len(),
        media,
        "every embedded item was written from the held file"
    );
}
/// Hooks for a disk without the atomic exchange: HFS+ (it has an exclusive
/// rename), exFAT and a network share (neither that nor hard links), a
/// volume with hard links only, and a FAT32 volume whose driver answers the
/// exchange with success and only renames.
fn disks_without_an_exchange() -> [(&'static str, pm_kernel::project_io::PublishHooks); 4] {
    use pm_kernel::project_io::PublishHooks;
    [
        (
            "no exchange",
            PublishHooks {
                no_exchange: true,
                ..Default::default()
            },
        ),
        (
            "no exchange, no exclusive rename",
            PublishHooks {
                no_exchange: true,
                no_exclusive: true,
                ..Default::default()
            },
        ),
        (
            "no exchange, hard links but no exclusive rename",
            PublishHooks {
                no_exchange: true,
                links_only: true,
                ..Default::default()
            },
        ),
        (
            "an exchange that only renames",
            PublishHooks {
                exchange_renames: true,
                ..Default::default()
            },
        ),
    ]
}
/// The private files a save keeps beside `file` on purpose: the version it
/// replaced, retained under the save's private name until a later save has
/// read it again, and that save's record, which says so.
fn retained(file: &Path) -> Vec<std::path::PathBuf> {
    let dir = file.parent().unwrap();
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let record = entry.unwrap().path();
        let name = record.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with(&private_prefix(file)) && name.ends_with(".save")) {
            continue;
        }
        let text = fs::read_to_string(&record).unwrap();
        let kept: Vec<_> = text
            .lines()
            .filter_map(|line| parse(line.split_once(' ')?.1).ok())
            .filter(|step| step.get("step").and_then(Value::as_str) == Some("retained"))
            .filter_map(|step| Some(dir.join(step.get("name")?.as_str()?)))
            .filter(|path| path.exists())
            .collect();
        // A record also stays while it has something to tell the person.
        let tells = text.contains(r#""step":"kept""#) || text.contains(r#""step":"restored""#);
        if !kept.is_empty() || tells {
            found.extend(kept);
            found.push(record);
        }
    }
    found.sort();
    found.dedup();
    found
}
/// What the files retained beside `file` hold.
fn retained_versions(file: &Path) -> Vec<Vec<u8>> {
    retained(file)
        .into_iter()
        .filter(|p| !p.to_string_lossy().ends_with(".save"))
        .map(|p| fs::read(p).unwrap())
        .collect()
}
/// The version a save replaced and retained beside `file`: its bytes.
fn retained_version(file: &Path) -> Vec<u8> {
    let mut kept = retained_versions(file);
    assert_eq!(kept.len(), 1, "one replaced version is retained");
    kept.remove(0)
}
/// The names a save may leave beside `file` and must not: its staging file,
/// a file it moved aside, or a record, other than what it retains on
/// purpose ([`retained`]).
fn private_leftovers(file: &Path) -> Vec<String> {
    let retained = retained(file);
    fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !retained.contains(p))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|n| {
            n.starts_with(&private_prefix(file))
                && [".tmp", ".aside", ".partial", ".gone", ".save"]
                    .iter()
                    .any(|e| n.ends_with(e))
        })
        .collect()
}
/// What the private names a save uses beside `file` begin with.
fn private_prefix(file: &Path) -> String {
    format!(".{}.", file.file_name().unwrap().to_string_lossy())
}
/// A kernel with a lock folder of its own, as a build that does not share
/// the user's has.
fn kernel_with_locks(path: &Path, locks: &Path) -> Kernel {
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(path.join("journals")),
        trash: Some(common::trash(&path.join("Trash"))),
        lock_root: Some(locks.into()),
        ..Default::default()
    });
    k.shell(ShellCall::AttachLibrary {
        profile: path.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    k
}
/// A kernel that keeps the versions its saves replace, as the app does.
fn kernel_with_backups(path: &Path) -> Kernel {
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(path.join("journals")),
        trash: Some(common::trash(&path.join("Trash"))),
        backup_root: Some(path.join("backups")),
        ..Default::default()
    });
    k.shell(ShellCall::AttachLibrary {
        profile: path.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    k
}
/// The version a save replaced is read and found to be the one the session
/// had; a hash taken then says nothing about a moment later. Another program
/// that still has that file open (or a sync client that swaps its private
/// name) can write work into it afterwards. So the replaced file is never
/// removed by the save that replaced it: it is retained, and the next save
/// reads it again before it goes. Work written into it meanwhile is found
/// then and kept beside the project under a plain name the save reports.
#[test]
fn work_written_into_the_replaced_file_after_it_was_read_is_never_removed() {
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (disk, hooks) in disks {
        let dir = common::TempDir::new("retained");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            // Written through the file itself, as a program that had it
            // open would: the same file, new bytes.
            after_judged: Some(std::sync::Arc::new(|replaced| {
                fs::write(replaced, b"unread work").unwrap()
            })),
            ..hooks.clone()
        });
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        k.finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("{disk}: {e}"));
        assert_eq!(saved_name(&file).as_deref(), Some("One"), "{disk}");
        assert_eq!(
            retained_version(&file),
            b"unread work",
            "{disk}: the replaced file is still there, with what was written into it"
        );
        let one = fs::read(&file).unwrap();
        // The next save reads the retained file again before pruning it.
        k.set_publish_hooks(hooks);
        rename_comp(&mut k, "P", "Two");
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let ShellReply::Saved {
            displaced: Some(kept),
            ..
        } = k
            .finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("{disk}: {e}"))
        else {
            panic!("{disk}: the save says a changed version was kept")
        };
        assert_eq!(fs::read(&kept).unwrap(), b"unread work", "{disk}");
        assert_eq!(
            visible_siblings(&file),
            std::slice::from_ref(&kept),
            "{disk}"
        );
        assert_eq!(saved_name(&file).as_deref(), Some("Two"), "{disk}");
        let now = retained_versions(&file);
        assert!(
            now.contains(&one),
            "{disk}: and the version this save replaced is retained in its turn"
        );
        // Where a file can only be given a name by copying it, the file
        // that was written into stays too until it was read once more.
        assert!(
            now.iter().all(|v| *v == one || v == b"unread work"),
            "{disk}"
        );
        assert_eq!(private_leftovers(&file), [""; 0], "{disk}");
        let two = fs::read(&file).unwrap();
        rename_comp(&mut k, "P", "Three");
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let saved = k.finish_session_save(plan.run()).unwrap();
        assert!(
            matches!(
                saved,
                ShellReply::Saved {
                    displaced: None,
                    ..
                }
            ),
            "{disk}: {saved:?}"
        );
        assert_eq!(retained_version(&file), two, "{disk}");
        assert_eq!(fs::read(&kept).unwrap(), b"unread work", "{disk}");
        assert_eq!(visible_siblings(&file), [kept], "{disk}");
        assert_eq!(private_leftovers(&file), [""; 0], "{disk}");
    }
}
/// Writes `bytes` into the file `path` through a descriptor, as a program
/// that kept it open would: the same file, new contents.
fn write_through(path: &Path, bytes: &[u8]) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
    file.set_len(bytes.len() as u64).unwrap();
}
/// A hook for the instant after a file was last read and right before it
/// is disposed of: work lands in it then. Returns what was written, in
/// order.
fn work_before_disposal() -> (
    pm_kernel::project_io::PublishHook,
    std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
) {
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = written.clone();
    let hook: pm_kernel::project_io::PublishHook = std::sync::Arc::new(move |path: &Path| {
        let mut log = log.lock().unwrap();
        let work = format!("work written at the last moment {}", log.len()).into_bytes();
        write_through(path, &work);
        log.push(work);
    });
    (hook, written)
}
/// A Trash hook that counts its calls and moves nothing.
fn counted_trash() -> (
    pm_kernel::project_io::TrashHook,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let hook = pm_kernel::project_io::TrashHook(std::sync::Arc::new(move |_: &Path| {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(std::io::Error::other("not in this test"))
    }));
    (hook, calls)
}
/// Saves `n` times with a new name each time.
fn save_times(k: &mut Kernel, session: &SessionHandle, n: usize, disk: &str) {
    for i in 0..n {
        rename_comp(k, "P", &format!("Save {i}"));
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        k.finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("{disk}: {e}"));
    }
}
/// A version a save no longer needs (the one an earlier save replaced and
/// retained, beside the project or in the backups) is read again as it
/// goes, and when it holds exactly what its record says it is removed in
/// that same call. Nothing reaches the Trash and nothing is left beside the
/// project: a professional who saves every minute does not fill their
/// Trash with hidden versions.
#[test]
fn a_version_unchanged_when_it_goes_is_removed_and_nothing_reaches_the_trash() {
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (disk, hooks) in disks {
        for backups in [false, true] {
            let dir = common::TempDir::new("disposal-unchanged");
            fs::create_dir_all(dir.path("job")).unwrap();
            let file = dir.path("job/cut.pmv");
            fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
            let native = dir.path("native");
            let mut k = if backups {
                kernel_with_backups(&native)
            } else {
                kernel(&native)
            };
            let (trash, calls) = counted_trash();
            let disposed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let seen = disposed.clone();
            k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
                trash: Some(trash),
                before_disposal: Some(std::sync::Arc::new(move |_: &Path| {
                    seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })),
                ..hooks.clone()
            });
            let (session, _) = open_file(&mut k, &file);
            // Enough saves for the backups to prune a version this project's
            // saves moved there.
            save_times(&mut k, &session, if backups { 14 } else { 3 }, disk);
            let order = std::sync::atomic::Ordering::SeqCst;
            assert!(disposed.load(order) > 0, "{disk} {backups}: something went");
            assert_eq!(
                calls.load(order),
                0,
                "{disk} {backups}: the Trash is never asked"
            );
            assert_eq!(
                visible_siblings(&file),
                [""; 0].map(Path::new),
                "{disk} {backups}"
            );
            assert_eq!(private_leftovers(&file), [""; 0], "{disk} {backups}");
        }
    }
}
/// A version that was written into after it was retained (a program that
/// still had it open, a sync client) is found changed when it is to go: it
/// is never removed. It is kept beside the project under a plain name and
/// the person is told; nothing goes to the Trash.
#[test]
fn a_version_changed_before_it_goes_is_kept_beside_the_project_and_told() {
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (disk, hooks) in disks {
        for backups in [false, true] {
            let dir = common::TempDir::new("disposal-changed");
            fs::create_dir_all(dir.path("job")).unwrap();
            let file = dir.path("job/cut.pmv");
            fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
            let native = dir.path("native");
            let mut k = if backups {
                kernel_with_backups(&native)
            } else {
                kernel(&native)
            };
            let (trash, calls) = counted_trash();
            let (hook, written) = work_before_disposal();
            k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
                trash: Some(trash),
                before_disposal: Some(hook),
                ..hooks.clone()
            });
            let (session, _) = open_file(&mut k, &file);
            save_times(&mut k, &session, if backups { 14 } else { 3 }, disk);
            let written = written.lock().unwrap().clone();
            assert!(!written.is_empty(), "{disk} {backups}");
            let kept: Vec<_> = visible_siblings(&file)
                .iter()
                .map(|p| fs::read(p).unwrap())
                .collect();
            for work in &written {
                assert!(kept.contains(work), "{disk} {backups}: kept beside");
            }
            let told = kept_beside(&mut k, &session);
            for work in &written {
                assert!(
                    told.iter().any(|(_, path)| path
                        .as_ref()
                        .is_some_and(|p| fs::read(p).is_ok_and(|b| b == *work))),
                    "{disk} {backups}: and told: {told:?}"
                );
            }
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{disk} {backups}: nothing to the Trash"
            );
        }
    }
}
/// Recovery removes files too: the cut-short copy it takes back from under
/// the project's name, and the save's own staged file once the path holds a
/// copy of it. Each is read again as it goes; written into before that, it
/// is never removed: it stays in the project's folder for the person.
#[test]
fn files_recovery_finds_changed_when_they_go_are_kept() {
    for case in ["partial copy", "staged duplicate"] {
        let dir = common::TempDir::new("recovery-disposal");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        if case == "partial copy" {
            assert!(killed(&native, &file, "no-exclusive", "mid-copy", "save"));
        } else {
            assert!(killed(&native, &file, "no-exclusive", "installed", "save"));
            // Cut short before the staged file went: it is still there,
            // and the path holds a copy of it.
            let record = private_file(&file, ".save");
            fs::copy(&file, record.with_extension("tmp")).unwrap();
        }
        let mut k = kernel(&native);
        let (trash, calls) = counted_trash();
        let (hook, written) = work_before_disposal();
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            trash: Some(trash),
            before_disposal: Some(hook),
            ..child_disk("no-exclusive")
        });
        let _ = k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        });
        let written = written.lock().unwrap().clone();
        assert_eq!(written.len(), 1, "{case}");
        let in_folder: Vec<_> = fs::read_dir(dir.path("job"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .map(|p| fs::read(p).unwrap())
            .collect();
        assert!(
            in_folder.contains(&written[0]),
            "{case}: in the project's folder, under a name the person finds"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "{case}");
    }
}
/// Recovery gives the finished file of a save that was cut short a name the
/// person finds; on a volume where that can only be done by copying it, the
/// file itself is still under its private name afterwards. Work written
/// into that file after the copy read it (a sync client, a program that had
/// it open) is not in the copy: the file is read again as it goes, and a
/// changed one is never unlinked. It is kept beside the project and told.
#[test]
fn a_recovered_files_original_changed_after_it_was_copied_is_never_removed_unread() {
    let dir = common::TempDir::new("recovered-source");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let native = dir.path("native");
    assert!(killed(&native, &file, "no-exclusive", "between", "save"));
    let staged = private_file(&file, ".tmp");
    let mut k = kernel(&native);
    let source = staged.clone();
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        // Half way through writing the copy that gets the "(recovered …)"
        // name: every byte of the staged file was read by then.
        mid_copy: Some(std::sync::Arc::new(move |to: &Path| {
            if to.to_string_lossy().contains("(recovered") {
                write_through(&source, b"work written after the copy read the file");
            }
        })),
        ..child_disk("no-exclusive")
    });
    let (session, _) = open_file(&mut k, &file);
    let everything: Vec<_> = fs::read_dir(dir.path("job"))
        .unwrap()
        .map(|e| fs::read(e.unwrap().path()).unwrap())
        .collect();
    assert!(
        everything
            .iter()
            .any(|bytes| bytes == b"work written after the copy read the file"),
        "what was written into the staged file is still in the project's folder"
    );
    let told = kept_beside(&mut k, &session);
    assert!(
        told.iter().any(|(_, path)| path.as_ref().is_some_and(|p| {
            fs::read(p).is_ok_and(|b| b == b"work written after the copy read the file")
        })),
        "and the person is told where: {told:?}"
    );
    assert!(
        visible_siblings(&file)
            .iter()
            .filter(|p| p.to_string_lossy().contains("(recovered"))
            .any(|p| saved_name(p).as_deref() == Some("One")),
        "the recovered save is there too"
    );
}
/// A version a save no longer needs is read and found unchanged, and at
/// that instant another writer puts a file of its own under the private
/// name the version had (a sync client replacing it). What is unlinked is
/// the file that was read, never whatever the name holds by then: the file
/// that landed is kept beside the project and told.
#[test]
fn a_file_that_lands_under_a_versions_name_as_it_goes_is_never_removed() {
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (disk, hooks) in disks {
        let dir = common::TempDir::new("disposal-landing");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (trash, calls) = counted_trash();
        let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            trash: Some(trash),
            before_unlink: Some(std::sync::Arc::new(move |name: &Path| {
                if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    land(name, b"a file that landed under the private name");
                }
            })),
            ..hooks.clone()
        });
        let (session, _) = open_file(&mut k, &file);
        save_times(&mut k, &session, 3, disk);
        let kept: Vec<_> = visible_siblings(&file)
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect();
        assert!(
            kept.iter()
                .any(|b| b == b"a file that landed under the private name"),
            "{disk}: kept beside the project"
        );
        let told = kept_beside(&mut k, &session);
        assert!(
            told.iter().any(|(_, path)| path.as_ref().is_some_and(|p| {
                fs::read(p).is_ok_and(|b| b == b"a file that landed under the private name")
            })),
            "{disk}: and told: {told:?}"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "{disk}");
    }
}
/// The app is killed between capturing a version it is disposing of and
/// unlinking it: the version sits under the capture name the save's record
/// names. The next open judges it as it would have been judged then:
/// unchanged, it goes; written into since, it is kept beside the project
/// and told. Nothing stays hidden.
#[test]
fn a_disposal_cut_short_is_finished_by_the_next_open() {
    for changed in [false, true] {
        let dir = common::TempDir::new("disposal-cut-short");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        assert!(killed(&native, &file, "no-exclusive", "installed", "save"));
        // Cut short before the staged file went: the path holds a copy.
        let record = private_file(&file, ".save");
        fs::copy(&file, record.with_extension("tmp")).unwrap();
        assert!(killed(&native, &file, "no-exclusive", "unlink", "open"));
        let captured = private_file(&file, ".gone");
        if changed {
            write_through(&captured, b"work written into the captured file");
        }
        let mut k = kernel(&native);
        k.set_publish_hooks(child_disk("no-exclusive"));
        let (session, _) = open_file(&mut k, &file);
        assert_eq!(saved_name(&file).as_deref(), Some("One"), "{changed}");
        let kept: Vec<_> = visible_siblings(&file)
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect();
        let told = kept_beside(&mut k, &session);
        if changed {
            assert_eq!(kept, [b"work written into the captured file".to_vec()]);
            assert!(
                told.iter().any(|(_, path)| path
                    .as_ref()
                    .is_some_and(|p| visible_siblings(&file).contains(p))),
                "{told:?}"
            );
        } else {
            assert_eq!(kept, [[0u8; 0]; 0], "nothing to keep");
            assert!(told.iter().all(|(_, path)| path.is_none()), "{told:?}");
        }
        assert_eq!(private_leftovers(&file), [""; 0], "{changed}");
    }
}
/// Recovery finds a file changed when it is to go and cannot keep it beside
/// the project (every name it could be given is taken, as when the folder
/// or its disk refuses): it goes to the Trash, or stays hidden where it
/// is. Either way the person is told, by the save's record, until they have
/// seen it, and the Error Log says where the file is.
#[test]
fn a_changed_file_that_cannot_be_kept_beside_the_project_is_still_told() {
    for trash in [true, false] {
        let dir = common::TempDir::new("stray");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        assert!(killed(&native, &file, "no-exclusive", "installed", "save"));
        let record = private_file(&file, ".save");
        fs::copy(&file, record.with_extension("tmp")).unwrap();
        let mut k = kernel(&native);
        let (hook, written) = work_before_disposal();
        let (no_trash, _) = counted_trash();
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            before_disposal: Some(hook),
            before_install: Some(std::sync::Arc::new(|to: &Path| {
                if to.to_string_lossy().contains("(other version") {
                    land(to, b"in the way");
                }
            })),
            trash: (!trash).then_some(no_trash),
            ..child_disk("no-exclusive")
        });
        let (session, _) = open_file(&mut k, &file);
        let work = written.lock().unwrap()[0].clone();
        let told = kept_beside(&mut k, &session);
        let why = if trash { "trashed" } else { "hidden" };
        assert!(told.iter().any(|(w, _)| w == why), "{why}: {told:?}");
        if trash {
            assert!(common::trashed(&native.join("Trash")).contains(&work));
        } else {
            assert!(
                told.iter().any(|(_, path)| path
                    .as_ref()
                    .is_some_and(|p| fs::read(p).is_ok_and(|b| b == work))),
                "the notice says where the hidden file is: {told:?}"
            );
        }
        let log = k.take_log();
        assert!(
            log.iter().any(|l| l.contains("could not be kept")),
            "{why}: {log:?}"
        );
        // Not seen yet: the next launch tells it again.
        k.shell(ShellCall::Close(session)).unwrap();
        drop(k);
        let mut k = kernel(&native);
        k.set_publish_hooks(child_disk("no-exclusive"));
        let (session, _) = open_file(&mut k, &file);
        let told = kept_beside(&mut k, &session);
        assert!(
            told.iter().any(|(w, path)| w == why
                || path
                    .as_ref()
                    .is_some_and(|p| fs::read(p).is_ok_and(|b| b == work))),
            "{why}: told again: {told:?}"
        );
        seen(&mut k, &session, &told);
        assert_eq!(kept_beside(&mut k, &session), [], "{why}: seen");
    }
}
/// A save that only appends to the project file writes a file whose first
/// bytes are the whole previous file. On a volume where the new file can
/// only be copied to its name, the name must never hold those bytes alone
/// while the copy runs: a sync client that replicates the file at that
/// instant would hand another computer a file that opens as the previous
/// version, with nothing to say a save was under way. Whatever is under the
/// name before the copy is complete does not open as a project.
#[test]
fn a_file_being_copied_to_its_name_never_opens_as_the_previous_version() {
    let dir = common::TempDir::new("copy-prefix");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let previous = fs::read(&file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    let replicas = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let seen = replicas.clone();
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        copy_stops_at: Some((
            previous.len() as u64,
            std::sync::Arc::new(move |name: &Path| {
                seen.lock().unwrap().push(fs::read(name).unwrap());
            }),
        )),
        ..child_disk("no-exclusive")
    });
    let plan = k.prepare_session_save(session, None, false).unwrap();
    k.finish_session_save(plan.run()).unwrap();
    assert_eq!(saved_name(&file).as_deref(), Some("One"));
    let saved = fs::read(&file).unwrap();
    assert!(
        saved.starts_with(&previous),
        "the save appended to the previous file (the case this test is about)"
    );
    let replicas = replicas.lock().unwrap().clone();
    assert_eq!(replicas.len(), 1, "the copy passed that length once");
    assert_eq!(replicas[0].len(), previous.len());
    let replica = dir.path("replica.pmv");
    fs::write(&replica, &replicas[0]).unwrap();
    assert!(
        open_project(&replica, OpenOptions::default()).is_err(),
        "what the name held then is not a project another computer could open"
    );
}
/// Every private file beside `file` (staged, moved aside, retained): what
/// each holds.
fn private_versions(file: &Path) -> Vec<Vec<u8>> {
    fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            name.starts_with('.') && (name.ends_with(".aside") || name.ends_with(".tmp"))
        })
        .map(|p| fs::read(p).unwrap())
        .collect()
}
/// The order of a save: install, confirm (the path holds the new file and
/// the folder synced), record that, and only then look at the version that
/// was replaced. Nothing is done with the old version while the new one is
/// not confirmed in place.
#[test]
fn the_replaced_version_is_looked_at_only_after_the_new_one_is_confirmed() {
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (disk, hooks) in disks {
        let dir = common::TempDir::new("order");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let (steps, judged, syncs) = (order.clone(), order.clone(), order.clone());
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            step: Some(std::sync::Arc::new(move |step, _| {
                steps.lock().unwrap().push(step.to_owned())
            })),
            after_judged: Some(std::sync::Arc::new(move |_| {
                judged.lock().unwrap().push("replaced version read".into())
            })),
            folder_sync: Some(std::sync::Arc::new(move || {
                syncs.lock().unwrap().push("folder synced".into());
                (None, None)
            })),
            ..hooks
        });
        let plan = k.prepare_session_save(session, None, false).unwrap();
        k.finish_session_save(plan.run()).unwrap();
        let order = order.lock().unwrap().clone();
        let at = |what: &str| order.iter().rposition(|o| o == what);
        let installed = at("installed").unwrap_or_else(|| panic!("{disk}: {order:?}"));
        assert!(
            order[..installed]
                .iter()
                .rev()
                .take(1)
                .eq(["folder synced"]),
            "{disk}: the folder is synced right before the step is recorded: {order:?}"
        );
        assert!(
            at("replaced version read").is_some_and(|read| read > installed),
            "{disk}: {order:?}"
        );
    }
}
/// Another writer lands a different, valid version at the project's path
/// after the new file was confirmed in place, while the version it replaced
/// is being judged and retained. The save must not certify that version
/// with this save's hash: the save fails, this save's file is kept beside
/// the project (written from the descriptor the save holds of it), the
/// arrived version stays under the name, and after Close and a reopen the
/// saved work is still there to recover and is told.
#[test]
fn a_version_that_lands_while_the_replaced_one_is_retained_is_never_taken_for_the_save() {
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (disk, hooks) in disks {
        let dir = common::TempDir::new("late-arrival");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let other = fs::read(common::ts_fixture("save-as.pmv")).unwrap();
        let native = dir.path("native");
        let mut k = kernel(&native);
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (landing, project) = (other.clone(), file.clone());
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            after_judged: Some(std::sync::Arc::new(move |_| {
                if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    land(&project, &landing);
                }
            })),
            ..hooks.clone()
        });
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let failed = k
            .finish_session_save(plan.run())
            .expect_err(&format!("{disk}: another version holds the name"));
        assert_eq!(
            fs::read(&file).unwrap(),
            other,
            "{disk}: the version that landed stays"
        );
        let ours: Vec<_> = visible_siblings(&file)
            .into_iter()
            .filter(|p| saved_name(p).as_deref() == Some("One"))
            .collect();
        assert_eq!(ours.len(), 1, "{disk}: {failed}: this save is kept beside");
        k.set_publish_hooks(hooks.clone());
        let ShellReply::Closed { recovery_copy } = k.shell(ShellCall::Close(session)).unwrap()
        else {
            panic!()
        };
        assert!(
            recovery_copy.is_some() || live_sessions(&native, "P") > 0,
            "{disk}: Close does not forget the work"
        );
        drop(k);
        let mut k = kernel(&native);
        k.set_publish_hooks(hooks);
        let (session, _) = open_file(&mut k, &file);
        assert_eq!(
            saved_name(&ours[0]).as_deref(),
            Some("One"),
            "{disk}: still there"
        );
        assert!(
            kept_beside(&mut k, &session)
                .iter()
                .any(|(_, path)| path.as_ref() == Some(&ours[0])),
            "{disk}: and told at the next open"
        );
    }
}
/// Another program writes its own version into the very file the save just
/// installed (through the name, the same file) while the save is still
/// judging and retaining the version it replaced. The file under the name
/// is the installed one by its number, but no longer by its bytes: the save
/// must not certify it with its own hash. The save fails, what was written
/// stays under the name untouched, and the session's work stays unsaved and
/// journalled, so Close and a reopen still have it.
#[test]
fn work_written_into_the_installed_file_before_the_save_ends_is_never_taken_for_the_save() {
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (disk, hooks) in disks {
        let dir = common::TempDir::new("written-into");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let other = fs::read(common::ts_fixture("save-as.pmv")).unwrap();
        let native = dir.path("native");
        let mut k = kernel(&native);
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (written, project) = (other.clone(), file.clone());
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            after_judged: Some(std::sync::Arc::new(move |_| {
                if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    write_through(&project, &written);
                }
            })),
            ..hooks.clone()
        });
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let saved = k.finish_session_save(plan.run());
        assert!(
            saved.is_err(),
            "{disk}: the file under the name is not this save's: {saved:?}"
        );
        assert_eq!(
            fs::read(&file).unwrap(),
            other,
            "{disk}: what was written stays, untouched"
        );
        k.set_publish_hooks(hooks.clone());
        let ShellReply::Closed { recovery_copy } = k.shell(ShellCall::Close(session)).unwrap()
        else {
            panic!()
        };
        assert!(
            recovery_copy.is_some() || live_sessions(&native, "P") > 0,
            "{disk}: Close does not forget the work"
        );
        assert_eq!(
            fs::read(&file).unwrap(),
            other,
            "{disk}: nor replace the file"
        );
    }
}
/// A folder sync that really fails fails the save. "This volume has no
/// folder sync" is believed only from the two answers that mean it, only
/// when the folder gave that answer as the save began, and the plain sync
/// asked for instead is not ignored: an I/O error from it is an error. The
/// replaced version is not retired, the record stays, and the journal with
/// the work is not forgotten.
#[test]
fn a_folder_sync_that_fails_fails_the_save_and_keeps_the_old_version_and_the_journal() {
    type Answers = fn(usize) -> (Option<i32>, Option<i32>);
    let cases: [(&str, Answers); 2] = [
        // No full sync for folders here (EINVAL), the plain one works: until
        // the sync after the install fails with an I/O error.
        ("the plain sync fails", |call| match call {
            0 => (Some(22), None),
            _ => (Some(22), Some(5)),
        }),
        // The folder synced as the save began, and then says it cannot.
        ("a folder that could be synced cannot", |call| match call {
            0 => (None, None),
            _ => (Some(45), None),
        }),
    ];
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (case, answers) in cases {
        for (disk, hooks) in disks.clone() {
            let dir = common::TempDir::new("sync-fails");
            fs::create_dir_all(dir.path("job")).unwrap();
            let file = dir.path("job/cut.pmv");
            fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
            let original = fs::read(&file).unwrap();
            let native = dir.path("native");
            let mut k = kernel(&native);
            let (session, _) = open_file(&mut k, &file);
            rename_comp(&mut k, "P", "One");
            let calls = std::sync::atomic::AtomicUsize::new(0);
            k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
                folder_sync: Some(std::sync::Arc::new(move || {
                    answers(calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
                })),
                ..hooks
            });
            let plan = k
                .prepare_session_save(session.clone(), None, false)
                .unwrap();
            let failed = k
                .finish_session_save(plan.run())
                .expect_err("the folder did not sync");
            assert!(
                failed.to_string().contains("save finalization failed"),
                "{case}, {disk}: {failed}"
            );
            assert!(
                private_versions(&file).contains(&original),
                "{case}, {disk}: the version it replaced is still beside the project"
            );
            assert!(
                private_leftovers(&file)
                    .iter()
                    .any(|n| n.ends_with(".save")),
                "{case}, {disk}: and the save's record with it"
            );
            assert!(
                live_sessions(&native, "P") > 0,
                "{case}, {disk}: the journal still holds the work"
            );
        }
    }
}
/// A network share has no way to prove that a rename survives a power cut
/// (measured: the full sync of a folder is "not supported" there; the plain
/// one answers, and the server decides). A save to such a volume stands,
/// but the app does not act as if it were proven: a copy of the saved
/// version is kept on this machine and the journal is not deleted at close,
/// until a later save's copy takes its place. If something else is
/// read back (the server lost the save), the person is told where the copy
/// is.
#[test]
fn a_save_the_volume_cannot_prove_durable_keeps_a_local_copy_until_it_is_read_back() {
    for lost in [false, true] {
        let dir = common::TempDir::new("unproven");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let original = fs::read(&file).unwrap();
        let native = dir.path("native");
        let share = || pm_kernel::project_io::PublishHooks {
            no_exchange: true,
            no_exclusive: true,
            folder_sync: Some(std::sync::Arc::new(|| (Some(45), None))),
            ..Default::default()
        };
        let mut k = kernel(&native);
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        k.set_publish_hooks(share());
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        k.finish_session_save(plan.run())
            .expect("the save stands as far as the volume can tell");
        let saved = fs::read(&file).unwrap();
        let local_copies = || -> Vec<std::path::PathBuf> {
            fs::read_dir(native.join("unconfirmed/P"))
                .map(|d| d.map(|e| e.unwrap().path()).collect())
                .unwrap_or_default()
        };
        let kept = local_copies();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            fs::read(&kept[0]).unwrap(),
            saved,
            "the saved version, on this machine"
        );
        k.shell(ShellCall::Close(session)).unwrap();
        assert!(
            live_sessions(&native, "P") > 0,
            "the journal is not deleted while the save is unproven"
        );
        drop(k);
        if lost {
            // The server lost the rename: the old version is what is there.
            fs::write(&file, &original).unwrap();
        }
        let mut k = kernel(&native);
        k.set_publish_hooks(share());
        let session = match k
            .shell(ShellCall::PrepareOpen {
                id: "P".into(),
                source: SessionSource::File(file.clone()),
                electron_profile: None,
            })
            .unwrap()
        {
            ShellReply::Opened { session, .. } => session,
            ShellReply::RecoveryRequired { ticket, .. } => {
                assert!(lost, "a save that arrived opens without a question");
                decide(&mut k, ticket, RecoveryDecision::Discard)
            }
            other => panic!("{other:?}"),
        };
        let told = kept_beside(&mut k, &session);
        if lost {
            assert_eq!(told, [("unconfirmed".to_owned(), Some(kept[0].clone()))]);
            assert_eq!(saved_name(&kept[0]).as_deref(), Some("One"));
        } else {
            assert_eq!(told, [], "read back: nothing to tell");
            assert_eq!(
                local_copies(),
                kept,
                "and the copy stays: a read back proves nothing on a share"
            );
        }
    }
}
/// The folder fails to sync once, right after a save installed its file:
/// that save is reported failed (its file is in place, not proven durable).
/// Once the folder syncs again, the next Save in place goes through: it
/// syncs the folder, adopts the save that was installed (its file is still
/// the one under the name), and writes the edits made since. No close and
/// reopen, no Save As.
#[test]
fn a_save_whose_folder_failed_to_sync_once_does_not_block_the_next_save() {
    let mut disks = vec![("an atomic exchange", Default::default())];
    disks.extend(disks_without_an_exchange());
    for (disk, hooks) in disks {
        let dir = common::TempDir::new("finalize-once");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        let mut k = kernel(&native);
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        let calls = std::sync::atomic::AtomicUsize::new(0);
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            folder_sync: Some(std::sync::Arc::new(move || {
                match calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                    1 => (Some(5), None),
                    _ => (None, None),
                }
            })),
            ..hooks.clone()
        });
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        k.finish_session_save(plan.run())
            .expect_err(&format!("{disk}: the folder did not sync"));
        k.set_publish_hooks(hooks.clone());
        rename_comp(&mut k, "P", "Two");
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap_or_else(|e| panic!("{disk}: {e}"));
        k.finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("{disk}: Save in place goes through: {e}"));
        assert_eq!(saved_name(&file).as_deref(), Some("Two"), "{disk}");
        assert!(
            !k.project("P").unwrap().store().is_dirty(),
            "{disk}: everything is saved"
        );
        rename_comp(&mut k, "P", "Three");
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        k.finish_session_save(plan.run()).unwrap();
        assert_eq!(saved_name(&file).as_deref(), Some("Three"), "{disk}");
    }
}
/// Save As writes the new file, and the folder fails to sync once right
/// after. The save is reported failed, but the file is there with the work
/// in it. The retry adopts all of it together: the session is that file's
/// from then on (its name, its lock), not the file it was opened from.
/// Another copy of the app cannot open the new file for writing meanwhile,
/// and the original is never written.
#[test]
fn a_save_as_whose_folder_failed_to_sync_once_is_adopted_with_its_file_and_its_lock() {
    for again in ["in place", "save as"] {
        let dir = common::TempDir::new("finalize-save-as");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let original = fs::read(&file).unwrap();
        let target = dir.path("job/second.pmv");
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        let calls = std::sync::atomic::AtomicUsize::new(0);
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            folder_sync: Some(std::sync::Arc::new(move || {
                match calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                    1 => (Some(5), None),
                    _ => (None, None),
                }
            })),
            ..Default::default()
        });
        let plan = k
            .prepare_session_save(session.clone(), Some(target.clone()), false)
            .unwrap();
        k.finish_session_save(plan.run())
            .expect_err("the folder did not sync");
        assert_eq!(saved_name(&target).as_deref(), Some("One"), "{again}");
        k.set_publish_hooks(Default::default());
        rename_comp(&mut k, "P", "Two");
        let path = (again == "save as").then(|| target.clone());
        let plan = k
            .prepare_session_save(session.clone(), path, true)
            .unwrap_or_else(|e| panic!("{again}: {e}"));
        k.finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("{again}: {e}"));
        assert_eq!(saved_name(&target).as_deref(), Some("Two"), "{again}");
        assert_eq!(fs::read(&file).unwrap(), original, "{again}: untouched");
        assert_eq!(
            k.session_metadata("P").unwrap().destination,
            SessionDestination::File(target.clone()),
            "{again}: the session is the new file's"
        );
        // Its lock is held: another copy of the app gets a copy, and the
        // file the session left is free again.
        let mut second = kernel(&dir.path("second-profile"));
        let (_, decision) = open_file(&mut second, &target);
        assert_eq!(
            decision,
            OpenDecision::AsCopy(CopyReason::Locked),
            "{again}"
        );
        let mut third = kernel(&dir.path("third-profile"));
        let (_, decision) = open_file(&mut third, &file);
        assert_eq!(decision, OpenDecision::Writable, "{again}");
        rename_comp(&mut k, "P", "Three");
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        k.finish_session_save(plan.run()).unwrap();
        assert_eq!(saved_name(&target).as_deref(), Some("Three"), "{again}");
        assert_eq!(fs::read(&file).unwrap(), original, "{again}");
    }
}
/// On a network share no save can be proven durable from this Mac: the
/// server alone decides when a rename survives. So the local copy of the
/// last save there is its checkpoint. A save whose local copy cannot be
/// made fails before anything is published (the file and the journal keep
/// everything). Reading the file back later is no proof (it can come from a
/// cache), so the copy outlives an open that reads the save back and the
/// Close after it: when the server later turns out to have lost the save,
/// the copy still holds it and the person is told where it is.
#[test]
fn an_unproven_save_keeps_its_local_copy_until_a_later_save_takes_its_place() {
    let dir = common::TempDir::new("unproven-kept");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let native = dir.path("native");
    let share = || pm_kernel::project_io::PublishHooks {
        no_exchange: true,
        no_exclusive: true,
        folder_sync: Some(std::sync::Arc::new(|| (Some(45), None))),
        ..Default::default()
    };
    let mut k = kernel(&native);
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    k.set_publish_hooks(share());
    // The local copy cannot be made: the save does not happen.
    fs::create_dir_all(native.join("unconfirmed")).unwrap();
    fs::write(native.join("unconfirmed/P"), b"in the way").unwrap();
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect_err("no local copy, no save");
    assert_eq!(fs::read(&file).unwrap(), original, "nothing was published");
    assert!(
        live_sessions(&native, "P") > 0,
        "the journal keeps the work"
    );
    fs::remove_file(native.join("unconfirmed/P")).unwrap();
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("now it can be kept");
    k.shell(ShellCall::Close(session)).unwrap();
    // Read back: as far as this Mac can tell, it arrived. Closed again.
    let (session, _) = open_file(&mut k, &file);
    assert_eq!(kept_beside(&mut k, &session), []);
    k.shell(ShellCall::Close(session)).unwrap();
    drop(k);
    // The server had not kept it after all.
    fs::write(&file, &original).unwrap();
    let mut k = kernel(&native);
    let session = match k
        .shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        ShellReply::RecoveryRequired { ticket, .. } => {
            decide(&mut k, ticket, RecoveryDecision::Discard)
        }
        other => panic!("{other:?}"),
    };
    let told = kept_beside(&mut k, &session);
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0].0, "unconfirmed");
    assert_eq!(
        saved_name(told[0].1.as_ref().unwrap()).as_deref(),
        Some("One"),
        "the save is still on this Mac"
    );
}
/// Recovery's moves are made durable before the open goes on: a folder
/// sync that fails after them fails the open (and says so) rather than
/// opening as if they would survive a power cut.
#[test]
fn a_recovery_whose_folder_cannot_be_synced_does_not_open_as_if_it_could() {
    let dir = common::TempDir::new("recovery-sync");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let native = dir.path("native");
    assert!(killed_between_renames(&native, &file, false));
    let mut k = kernel(&native);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        no_exchange: true,
        // The first sync (as recovery begins) works; the one after its
        // moves fails with an I/O error.
        folder_sync: Some(std::sync::Arc::new(move || {
            match calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                0 => (None, None),
                _ => (Some(5), None),
            }
        })),
        ..Default::default()
    });
    let failed = k
        .shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .expect_err("the moves could not be made durable");
    assert!(failed.to_string().contains("sync"), "{failed}");
    k.set_publish_hooks(Default::default());
    open_file(&mut k, &file);
}
/// What a volume does is proven by every save, not once per run of the
/// app: a drive can be unplugged and another mounted in its place under the
/// same name, device number and type. Here the volume exchanges files for
/// the first save and afterwards answers the exchange with success while
/// only renaming (which destroys whatever is at the destination). The next
/// save proves the exchange again, finds it does not hold, and takes the
/// route that moves the file aside: the previous version is retained and a
/// version that had just landed is kept.
#[test]
fn what_a_volume_does_is_proven_by_every_save_not_remembered_from_an_earlier_one() {
    let dir = common::TempDir::new("remounted");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let weak = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        weak_exchange: Some(weak.clone()),
        ..Default::default()
    });
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("a save by the atomic exchange");
    let one = fs::read(&file).unwrap();
    // The same mount point, device and type; another volume's behaviour.
    weak.store(true, std::sync::atomic::Ordering::SeqCst);
    rename_comp(&mut k, "P", "Two");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("the save takes the other route");
    assert_eq!(saved_name(&file).as_deref(), Some("Two"));
    assert_eq!(
        retained_version(&file),
        one,
        "the version it replaced was moved aside and retained, not renamed over"
    );
    // And a version that lands right before the file is taken is kept.
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        weak_exchange: Some(weak),
        before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
        ..Default::default()
    });
    rename_comp(&mut k, "P", "Three");
    let plan = k.prepare_session_save(session, None, false).unwrap();
    let ShellReply::Saved {
        displaced: Some(kept),
        ..
    } = k.finish_session_save(plan.run()).unwrap()
    else {
        panic!("the save says another version was kept")
    };
    assert_eq!(fs::read(&kept).unwrap(), b"version B");
    assert_eq!(saved_name(&file).as_deref(), Some("Three"));
}
/// Two copies of the app on one Mac (the shipped app and a development
/// build, or two profiles) never write one project file: the lock that
/// excludes them is the user's, not a profile's. The second opens the
/// file as a copy.
#[test]
fn another_profile_on_this_mac_is_excluded_from_a_file_that_is_open() {
    let dir = common::TempDir::new("two-profiles");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let mut shipped = kernel(&dir.path("profile-a"));
    let mut development = kernel(&dir.path("profile-b"));
    assert_eq!(shipped.lock_root(), development.lock_root());
    assert_eq!(shipped.lock_root(), user_lock_root());
    let (_held, first) = open_file(&mut shipped, &file);
    assert_eq!(first, OpenDecision::Writable);
    let (_, second) = open_file(&mut development, &file);
    assert_eq!(second, OpenDecision::AsCopy(CopyReason::Locked));
}
/// A save that is going on in another copy of the app is not a save that
/// was cut short. Even a copy that does not share the lock (so nothing
/// stopped it from getting this far) moves none of that save's files while
/// the process that began it is alive: the open is refused in plain words,
/// and the save finishes undisturbed.
#[test]
fn a_save_in_progress_in_another_live_process_is_never_recovered() {
    let dir = common::TempDir::new("live-owner");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let go_on = dir.path("go-on");
    let mut saving = Process::new(std::env::current_exe().unwrap())
        .args(["--exact", "save_child_process"])
        .env("PM_KERNEL_TEST_SAVE_PROFILE", dir.path("profile-a"))
        .env("PM_KERNEL_TEST_SAVE_LOCKS", dir.path("locks-a"))
        .env("PM_KERNEL_TEST_SAVE_FILE", &file)
        .env("PM_KERNEL_TEST_SAVE_DISK", "no-exchange")
        .env("PM_KERNEL_TEST_SAVE_KILL", "between")
        .env("PM_KERNEL_TEST_SAVE_PAUSE", &go_on)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let reached = go_on.with_extension("reached");
    for _ in 0..1500 {
        if reached.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(reached.exists(), "the save is between its two renames");
    assert!(!file.exists());
    let before = tree(&dir.path("job"));
    let mut other = kernel_with_locks(&dir.path("profile-b"), &dir.path("locks-b"));
    let refused = other
        .shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .expect_err("the project is being saved right now");
    assert!(
        refused.to_string().contains("being saved right now"),
        "{refused}"
    );
    let ShellReply::Leftovers(rows) = other.shell(ShellCall::Leftovers(file.clone())).unwrap()
    else {
        panic!()
    };
    assert!(
        rows.iter()
            .all(|r| r.get("why").and_then(Value::as_str) == Some("active")),
        "{rows:?}"
    );
    let chosen = rows[0].get("path").and_then(Value::as_str).unwrap();
    other
        .shell(ShellCall::UseLeftover {
            path: file.clone(),
            leftover: chosen.into(),
            unproven: false,
        })
        .expect_err("its files are not for anyone to choose from");
    assert_eq!(tree(&dir.path("job")), before, "nothing was moved");
    fs::write(&go_on, b"").unwrap();
    assert_eq!(saving.wait().unwrap().code(), Some(3), "the save finished");
    assert_eq!(saved_name(&file).as_deref(), Some("One"));
    assert_eq!(visible_siblings(&file), [""; 0].map(Path::new));
    assert_eq!(private_leftovers(&file), [""; 0]);
}
/// A save begun on another computer and not finished may be a crash or may
/// still be going on: its files reach this Mac on a volume both write, or
/// in a folder on this Mac's own disk that a sync client fills from the
/// other computer while it is still saving. Nothing here can tell which,
/// and what recovery moved would sync back into that save. So it is never
/// settled unasked, wherever the folder is: the open says which computer,
/// which user and since when, nothing is moved, and the person chooses.
#[test]
fn a_save_begun_on_another_computer_is_never_settled_unasked() {
    {
        let dir = common::TempDir::new("elsewhere");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let original = fs::read(&file).unwrap();
        let killed = Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "save_child_process"])
            .env("PM_KERNEL_TEST_SAVE_PROFILE", dir.path("their-profile"))
            .env("PM_KERNEL_TEST_SAVE_FILE", &file)
            .env("PM_KERNEL_TEST_SAVE_DISK", "no-exclusive")
            .env("PM_KERNEL_TEST_SAVE_KILL", "between")
            .env("PM_KERNEL_TEST_SAVE_HOST", "studio-mac-2")
            .stdout(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(killed.status.code(), Some(0));
        let before = tree(&dir.path("job"));
        let mut k = kernel(&dir.path("my-profile"));
        // A folder on this Mac's own disk (the test's): no flag says
        // that another computer writes it.
        k.set_publish_hooks(child_disk("no-exclusive"));
        let opened = k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        });
        let refused = opened.expect_err("another computer's save").to_string();
        assert!(
            refused.contains("studio-mac-2") && refused.contains("Nothing was moved"),
            "{refused}"
        );
        assert_eq!(tree(&dir.path("job")), before, "nothing was moved");
        let ShellReply::Leftovers(rows) = k.shell(ShellCall::Leftovers(file.clone())).unwrap()
        else {
            panic!()
        };
        let text = |row: &Value, key: &str| row.get(key).and_then(Value::as_str).map(str::to_owned);
        assert_eq!(rows.len(), 2, "{rows:?}");
        for row in &rows {
            assert_eq!(text(row, "why").as_deref(), Some("elsewhere"));
            assert_eq!(text(row, "computer").as_deref(), Some("studio-mac-2"));
            assert!(
                row.get("at")
                    .and_then(Value::as_f64)
                    .is_some_and(|at| at > 0.0)
            );
        }
        // The person chooses the last saved version.
        let previous = rows
            .iter()
            .find(|r| text(r, "role").as_deref() == Some("previous"))
            .and_then(|r| text(r, "path"))
            .unwrap();
        k.shell(ShellCall::UseLeftover {
            path: file.clone(),
            leftover: previous.into(),
            unproven: false,
        })
        .unwrap();
        assert_eq!(fs::read(&file).unwrap(), original);
        let kept = visible_siblings(&file);
        assert_eq!(kept.len(), 1, "their unfinished save is kept: {kept:?}");
        assert_eq!(saved_name(&kept[0]).as_deref(), Some("One"));
    }
}
/// A coordinator as the app supplies one: grants every write (or none),
/// and notes what the project file held when the work began and ended.
struct Coordinator {
    grants: bool,
    seen: std::sync::Mutex<Vec<Coordinated>>,
}
/// A coordinated write: of which file, and the name of the composition in
/// it when the work began and when it ended (`None`: no file there).
type Coordinated = (std::path::PathBuf, Option<String>, Option<String>);
impl pm_kernel::project_io::WriteCoordinator for Coordinator {
    fn coordinate(&self, path: &Path, work: &mut dyn FnMut(bool)) {
        let holds = |path: &Path| path.exists().then(|| saved_name(path)).flatten();
        let before = holds(path);
        work(self.grants);
        let entry = (path.to_owned(), before, holds(path));
        self.seen.lock().unwrap().push(entry);
    }
}
fn coordinated_kernel(path: &Path, grants: bool) -> (Kernel, std::sync::Arc<Coordinator>) {
    let coordinator = std::sync::Arc::new(Coordinator {
        grants,
        seen: Default::default(),
    });
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(path.join("journals")),
        trash: Some(common::trash(&path.join("Trash"))),
        write_coordinator: Some(pm_kernel::project_io::SharedWriteCoordinator(
            coordinator.clone(),
        )),
        ..Default::default()
    });
    k.shell(ShellCall::AttachLibrary {
        profile: path.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    (k, coordinator)
}
/// A coordinator under which the coordinated write of one file waits (a
/// share that stalls, a program that does not answer) until it is released.
struct Stalled {
    file: std::path::PathBuf,
    armed: std::sync::atomic::AtomicBool,
    waiting: std::sync::atomic::AtomicBool,
    release: std::sync::atomic::AtomicBool,
}
impl pm_kernel::project_io::WriteCoordinator for Stalled {
    fn coordinate(&self, path: &Path, work: &mut dyn FnMut(bool)) {
        use std::sync::atomic::Ordering::SeqCst;
        if path == self.file && self.armed.load(SeqCst) {
            self.waiting.store(true, SeqCst);
            while !self.release.load(SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        work(true)
    }
}
/// A kernel on its own thread, as the app runs it, with a library at
/// `profile` and a coordinator that can stall the writes of `file`.
fn stalling_actor(profile: &Path, file: &Path) -> (KernelActor, std::sync::Arc<Stalled>) {
    let stalled = std::sync::Arc::new(Stalled {
        file: file.to_owned(),
        armed: false.into(),
        waiting: false.into(),
        release: false.into(),
    });
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(profile.join("journals")),
        trash: Some(common::trash(&profile.join("Trash"))),
        write_coordinator: Some(pm_kernel::project_io::SharedWriteCoordinator(
            stalled.clone(),
        )),
        ..Default::default()
    });
    actor
        .handle()
        .shell(ShellCall::AttachLibrary {
            profile: profile.into(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    (actor, stalled)
}
fn open_on(k: &KernelHandle, id: &str, file: &Path) -> SessionHandle {
    match k
        .shell(ShellCall::Open {
            id: id.into(),
            source: SessionSource::File(file.into()),
            electron_profile: None,
        })
        .wait()
    {
        Some(Ok(ShellReply::Opened { session, .. })) => session,
        other => panic!("{other:?}"),
    }
}
fn rename_on(k: &KernelHandle, id: &str, name: &str) -> Pending<Result<Ack, KernelFailure>> {
    use pm_kernel::commands::comps::{CompCommand, CompPatch};
    k.call(
        id,
        Request::new(
            Caller::native(),
            Call::Transact {
                label: Some("Rename".into()),
                body: WriteBody::Commands(vec![
                    CompCommand::Set {
                        patch: Box::new(CompPatch {
                            name: Some(name.into()),
                            ..Default::default()
                        }),
                    }
                    .into(),
                ]),
                expected_revision: None,
            },
        ),
    )
}
/// Whether `pending` is answered within `limit`.
fn answered<T>(pending: &Pending<T>, limit: Duration) -> Option<T> {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if let Some(value) = pending.try_take() {
            return Some(value);
        }
        if std::time::Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
/// Opening a project whose last save was cut short recovers it: files are
/// read whole, hashed and moved, on whatever disk the project is on. While
/// that waits (a large project, a share that stalls) the kernel goes on
/// answering every other project: recovery runs on a worker, and the
/// kernel only adopts its result.
#[test]
fn a_recovery_waiting_on_its_disk_does_not_hold_up_the_other_projects() {
    use std::sync::atomic::Ordering::SeqCst;
    let dir = common::TempDir::new("stalled-recovery");
    fs::create_dir_all(dir.path("job")).unwrap();
    let (file, other) = (dir.path("job/cut.pmv"), dir.path("job/other.pmv"));
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    fs::copy(common::ts_fixture("fresh.pmv"), &other).unwrap();
    let original = fs::read(&file).unwrap();
    let native = dir.path("native");
    assert!(killed(&native, &file, "no-exchange", "between", "save"));
    let (actor, stalled) = stalling_actor(&native, &file);
    let k = actor.handle();
    open_on(&k, "Q", &other);
    stalled.armed.store(true, SeqCst);
    let opening = k.shell(ShellCall::Open {
        id: "P".into(),
        source: SessionSource::File(file.clone()),
        electron_profile: None,
    });
    let reached = (0..2000).any(|_| {
        std::thread::sleep(Duration::from_millis(5));
        stalled.waiting.load(SeqCst)
    });
    // The other project is worked on while the recovery waits.
    let edit = rename_on(&k, "Q", "Still answered");
    let edited = answered(&edit, Duration::from_secs(5));
    stalled.release.store(true, SeqCst);
    let opened = opening.wait();
    assert!(reached, "the recovery reached its coordinated write");
    assert!(
        matches!(edited, Some(Ok(_))),
        "the other project's edit was answered while the recovery waited"
    );
    assert!(
        matches!(opened, Some(Ok(ShellReply::Opened { .. }))),
        "{opened:?}"
    );
    assert_eq!(
        fs::read(&file).unwrap(),
        original,
        "and the project is recovered"
    );
    actor.shutdown();
}
/// The same for a save: before it writes, a save settles what an earlier
/// save of the file left (here another copy of the app, with a lock folder
/// of its own, was killed in the middle of saving it). That runs on a
/// worker too; the kernel answers the other projects meanwhile, and the
/// save then goes through.
#[test]
fn a_save_that_must_first_recover_does_not_hold_up_the_other_projects() {
    use std::sync::atomic::Ordering::SeqCst;
    let dir = common::TempDir::new("stalled-save");
    fs::create_dir_all(dir.path("job")).unwrap();
    let (file, other) = (dir.path("job/cut.pmv"), dir.path("job/other.pmv"));
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    fs::copy(common::ts_fixture("fresh.pmv"), &other).unwrap();
    let native = dir.path("native");
    let (actor, stalled) = stalling_actor(&native, &file);
    let k = actor.handle();
    let session = open_on(&k, "P", &file);
    open_on(&k, "Q", &other);
    let killed = Process::new(std::env::current_exe().unwrap())
        .args(["--exact", "save_child_process"])
        .env("PM_KERNEL_TEST_SAVE_PROFILE", dir.path("their-profile"))
        .env("PM_KERNEL_TEST_SAVE_LOCKS", dir.path("their-locks"))
        .env("PM_KERNEL_TEST_SAVE_FILE", &file)
        .env("PM_KERNEL_TEST_SAVE_DISK", "no-exchange")
        .env("PM_KERNEL_TEST_SAVE_KILL", "between")
        .stdout(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(killed.status.code(), Some(0));
    rename_on(&k, "P", "Mine").wait().unwrap().unwrap();
    stalled.armed.store(true, SeqCst);
    let preparing = k.prepare_save(session, None);
    let reached = (0..2000).any(|_| {
        std::thread::sleep(Duration::from_millis(5));
        stalled.waiting.load(SeqCst)
    });
    let edit = rename_on(&k, "Q", "Still answered");
    let edited = answered(&edit, Duration::from_secs(5));
    stalled.release.store(true, SeqCst);
    let prepared = preparing.wait();
    assert!(reached, "the save's recovery reached its coordinated write");
    assert!(
        matches!(edited, Some(Ok(_))),
        "the other project's edit was answered while the save's recovery waited"
    );
    let save = prepared.unwrap().unwrap_or_else(|e| panic!("{e}"));
    k.finish_save(save.run())
        .wait()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(saved_name(&file).as_deref(), Some("Mine"));
    assert!(
        visible_siblings(&file)
            .iter()
            .any(|p| saved_name(p).as_deref() == Some("One")),
        "the save that was cut short is kept beside"
    );
    actor.shutdown();
}
/// A save's publication runs inside a coordinated write of the project
/// file (other programs that take part in file coordination wait for it,
/// and are asked to write their own changes first), and so do the moves a
/// recovery makes at the next open. When the system does not grant one in
/// time the save and the recovery go ahead all the same: nobody is kept
/// from saving by another program that will not answer.
#[test]
fn a_save_and_a_recovery_run_inside_a_coordinated_write_or_go_ahead_without() {
    for grants in [true, false] {
        let dir = common::TempDir::new("coordinated");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        let (mut k, coordinator) = coordinated_kernel(&native, grants);
        let (session, _) = open_file(&mut k, &file);
        assert_eq!(
            *coordinator.seen.lock().unwrap(),
            [],
            "an open with nothing to settle asks for nothing"
        );
        let before = saved_name(&file);
        rename_comp(&mut k, "P", "One");
        let plan = k.prepare_session_save(session, None, false).unwrap();
        k.finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("grants {grants}: {e}"));
        assert_eq!(
            *coordinator.seen.lock().unwrap(),
            [(file.clone(), before, Some("One".to_owned()))],
            "grants {grants}: the file was replaced inside the coordinated write"
        );
        drop(k);
        // The moves of a recovery, too.
        let crashed = dir.path("job/crashed.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &crashed).unwrap();
        let original = saved_name(&crashed);
        let crash_profile = dir.path("native-crash");
        assert!(killed_between_renames(&crash_profile, &crashed, false));
        let (mut k, coordinator) = coordinated_kernel(&crash_profile, grants);
        prepare(&mut k, "P", SessionSource::File(crashed.clone()));
        assert_eq!(
            *coordinator.seen.lock().unwrap(),
            [(crashed.clone(), None, original)],
            "grants {grants}: the last saved version was put back inside one"
        );
    }
}
/// A save keeps the version it replaced until the next save stands and has
/// read it again: one previous version beside the project, never a pile.
#[test]
fn a_replaced_version_is_retained_until_the_next_save_has_read_it_again() {
    for (disk, hooks) in disks_without_an_exchange() {
        let dir = common::TempDir::new("retained-plain");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut previous = fs::read(&file).unwrap();
        let mut k = kernel(&dir.path("native"));
        k.set_publish_hooks(hooks);
        let (session, _) = open_file(&mut k, &file);
        for name in ["One", "Two", "Three"] {
            rename_comp(&mut k, "P", name);
            let plan = k
                .prepare_session_save(session.clone(), None, false)
                .unwrap();
            let saved = k.finish_session_save(plan.run()).unwrap();
            assert!(
                matches!(
                    saved,
                    ShellReply::Saved {
                        displaced: None,
                        ..
                    }
                ),
                "{disk}: {saved:?}"
            );
            assert_eq!(retained_version(&file), previous, "{disk} {name}");
            assert_eq!(retained(&file).len(), 2, "{disk} {name}: it and its record");
            assert_eq!(private_leftovers(&file), [""; 0], "{disk} {name}");
            assert_eq!(visible_siblings(&file), [""; 0].map(Path::new), "{disk}");
            previous = fs::read(&file).unwrap();
        }
    }
}
/// Where the app's backups are on the project's volume, the replaced file
/// itself becomes the backup of record (in place of the copy made before the
/// save), so nothing stays beside the project, and work written into it
/// after it was read is in the backups. When that backup's turn to be
/// pruned comes it is read again: changed, it is not deleted but kept
/// beside the project.
#[test]
fn the_replaced_file_becomes_the_backup_and_a_changed_one_is_never_pruned() {
    let dir = common::TempDir::new("retained-backups");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let native = dir.path("native");
    let mut k = kernel_with_backups(&native);
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        after_judged: Some(std::sync::Arc::new(|replaced| {
            fs::write(replaced, b"unread work").unwrap()
        })),
        ..Default::default()
    });
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run()).unwrap();
    let backups = || -> Vec<Vec<u8>> {
        fs::read_dir(native.join("backups/P"))
            .unwrap()
            .map(|e| fs::read(e.unwrap().path()).unwrap())
            .collect()
    };
    assert_eq!(
        backups(),
        [b"unread work".to_vec()],
        "the one backup of that save is the replaced file itself"
    );
    assert_eq!(retained(&file), [""; 0].map(Path::new));
    assert_eq!(private_leftovers(&file), [""; 0]);
    k.set_publish_hooks(Default::default());
    let mut told = None;
    for n in 0..12 {
        assert_eq!(visible_siblings(&file), [""; 0].map(Path::new), "save {n}");
        rename_comp(&mut k, "P", &format!("Next {n}"));
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        if let ShellReply::Saved {
            displaced: Some(kept),
            ..
        } = k.finish_session_save(plan.run()).unwrap()
        {
            told = Some(kept);
        }
    }
    let kept = told.expect("the save that would have pruned it says it was kept");
    assert_eq!(fs::read(&kept).unwrap(), b"unread work");
    assert_eq!(visible_siblings(&file), [kept]);
    assert_eq!(backups().len(), 12, "the others are pruned as before");
    assert!(!backups().contains(&b"unread work".to_vec()));
}
/// Save works on a disk without the atomic exchange (an exFAT or HFS+ drive,
/// a FAT32 card, a network share), by the app's save route and by the
/// kernel's own: the file holds the new work under its own name, the next
/// save is an ordinary one, and nothing is left beside it. Such a disk used
/// to refuse every save of an existing file, so each one needed a new name.
#[test]
fn a_save_replaces_the_file_on_a_disk_without_an_atomic_exchange() {
    for (disk, hooks) in disks_without_an_exchange() {
        let dir = common::TempDir::new("no-exchange");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        k.set_publish_hooks(hooks);
        let (session, decision) = open_file(&mut k, &file);
        assert_eq!(decision, OpenDecision::Writable, "{disk}");
        let media = k.project("P").unwrap().store().media().len();
        for name in ["One", "Two"] {
            rename_comp(&mut k, "P", name);
            let plan = k
                .prepare_session_save(session.clone(), None, false)
                .unwrap();
            let saved = k
                .finish_session_save(plan.run())
                .unwrap_or_else(|e| panic!("{disk}: Save writes the file in place: {e}"));
            assert!(
                matches!(
                    saved,
                    ShellReply::Saved {
                        displaced: None,
                        recovery_copy: None,
                        ..
                    }
                ),
                "{disk}: {saved:?}"
            );
            assert_eq!(saved_name(&file).as_deref(), Some(name), "{disk}");
        }
        rename_comp(&mut k, "P", "Three");
        let saved = k.shell(ShellCall::Save(session)).unwrap();
        assert!(
            matches!(
                saved,
                ShellReply::Saved {
                    displaced: None,
                    recovery_copy: None,
                    ..
                }
            ),
            "{disk}: the kernel's own save writes the file too: {saved:?}"
        );
        assert_eq!(saved_name(&file).as_deref(), Some("Three"), "{disk}");
        assert_eq!(
            open_project(&file, OpenOptions::default())
                .unwrap()
                .file
                .media
                .len(),
            media,
            "{disk}: with all its media"
        );
        assert_eq!(private_leftovers(&file), [""; 0], "{disk}");
        assert_eq!(visible_siblings(&file), [""; 0].map(Path::new), "{disk}");
    }
}
/// Gives the file at `path` a new file number with the same bytes, as a
/// network share does to a file that is renamed (its numbers follow names).
fn renumber(path: &Path) {
    let copy = path.with_extension("renumbered");
    fs::copy(path, &copy).unwrap();
    fs::rename(&copy, path).unwrap();
}
/// A network share (measured: an SMB share mounted on macOS 27) has no
/// exchange and no exclusive rename, and numbers files by name: the file
/// moved aside has another number than it had at the path, and the installed
/// file another than it had while staged. Save works there all the same. The
/// file moved aside is known by its bytes (it is not kept as "another
/// version" on every save), and the installed file by its content.
#[test]
fn a_save_stands_on_a_share_that_renumbers_a_file_when_it_is_renamed() {
    let dir = common::TempDir::new("renumbering-share");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let mut k = kernel(&dir.path("native"));
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        no_exchange: true,
        no_exclusive: true,
        between_renames: Some(std::sync::Arc::new(|path| {
            let aside = fs::read_dir(path.parent().unwrap())
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.extension().is_some_and(|e| e == "aside"))
                .expect("the file moved aside");
            renumber(&aside);
        })),
        after_exchange: Some(std::sync::Arc::new(renumber)),
        ..Default::default()
    });
    let (session, _) = open_file(&mut k, &file);
    for name in ["One", "Two"] {
        rename_comp(&mut k, "P", name);
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let saved = k
            .finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("Save writes the file in place: {e}"));
        assert!(
            matches!(
                saved,
                ShellReply::Saved {
                    displaced: None,
                    recovery_copy: None,
                    ..
                }
            ),
            "{saved:?}"
        );
        assert_eq!(saved_name(&file).as_deref(), Some(name));
        assert_eq!(visible_siblings(&file), [""; 0].map(Path::new));
        assert_eq!(private_leftovers(&file), [""; 0]);
    }
}
/// On such a disk the file is moved aside and the new one installed at the
/// empty path. A version that lands at the path after the save's last check
/// is what gets moved aside: it is read, found not to be the version the
/// save could replace, and kept beside the project under a plain name the
/// save reports. Nothing unread is removed.
#[test]
fn a_version_that_lands_before_the_file_is_moved_aside_is_kept_beside_the_project() {
    for (disk, hooks) in disks_without_an_exchange() {
        let dir = common::TempDir::new("aside-b");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
            ..hooks
        });
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let ShellReply::Saved {
            displaced: Some(kept),
            ..
        } = k
            .finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("{disk}: the save is published: {e}"))
        else {
            panic!("{disk}: the save says another version was kept")
        };
        assert_eq!(saved_name(&file).as_deref(), Some("One"), "{disk}");
        assert_eq!(fs::read(&kept).unwrap(), b"version B", "{disk}");
        assert_eq!(visible_siblings(&file), std::slice::from_ref(&kept));
        assert!(
            kept.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("cut (other version "),
            "{disk}: {kept:?}"
        );
        assert_eq!(private_leftovers(&file), [""; 0], "{disk}");
    }
}
/// A version that lands at the empty path between the two renames is never
/// replaced: the install refuses a taken name. That version stays where it
/// landed, this save's finished file is kept beside it under a plain name
/// the failure reports, and the work is still open for Save As.
#[test]
fn a_version_that_lands_between_the_two_renames_is_never_replaced() {
    for (disk, hooks) in disks_without_an_exchange() {
        let dir = common::TempDir::new("aside-c");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            between_renames: Some(std::sync::Arc::new(|path| {
                assert!(!path.exists(), "the path is empty between the renames");
                land(path, b"version C")
            })),
            ..hooks.clone()
        });
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let failed = k
            .finish_session_save(plan.run())
            .expect_err("the path was taken");
        assert_eq!(fs::read(&file).unwrap(), b"version C", "{disk}: C stays");
        let kept = visible_siblings(&file);
        assert_eq!(kept.len(), 1, "{disk}: {kept:?}");
        assert!(
            kept[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("cut (recovered "),
            "{disk}: {kept:?}"
        );
        assert_eq!(saved_name(&kept[0]).as_deref(), Some("One"), "{disk}");
        assert_eq!(
            pm_kernel::project_io::kept_beside(&failed.to_string()).as_deref(),
            Some(kept[0].as_path()),
            "{disk}: the failure names where the save is: {failed}"
        );
        assert_eq!(private_leftovers(&file), [""; 0], "{disk}");
        // The work is still open and Save As writes it.
        k.set_publish_hooks(hooks);
        let rescued = dir.path("job/mine.pmv");
        let plan = k
            .prepare_session_save(session.clone(), Some(rescued.clone()), false)
            .unwrap();
        k.finish_session_save(plan.run()).unwrap();
        assert_eq!(saved_name(&rescued).as_deref(), Some("One"), "{disk}");
        assert_eq!(fs::read(&file).unwrap(), b"version C", "{disk}");
    }
}
/// Lands `bytes` under the first name `wanted` accepts, once, at the instant
/// right before a file is placed under that name: after every look a save or
/// a recovery takes at the name, before the call that takes it. Says whether
/// it landed.
fn lands_at_the_install(
    hooks: pm_kernel::project_io::PublishHooks,
    wanted: impl Fn(&Path) -> bool + Send + Sync + 'static,
    bytes: Vec<u8>,
) -> (
    pm_kernel::project_io::PublishHooks,
    std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
) {
    let landed = std::sync::Arc::new(std::sync::Mutex::new(None));
    let seen = landed.clone();
    let hooks = pm_kernel::project_io::PublishHooks {
        before_install: Some(std::sync::Arc::new(move |to: &Path| {
            let mut seen = seen.lock().unwrap();
            if seen.is_none() && wanted(to) {
                assert!(!to.exists(), "the name was looked at and found free");
                land(to, &bytes);
                *seen = Some(to.to_owned());
            }
        })),
        ..hooks
    };
    (hooks, landed)
}
/// "Look that the name is free, then rename" is no way to install a file: a
/// version that lands after the look and before the rename would be replaced
/// unread, however short that is. On a disk with no rename that refuses a
/// taken name (exFAT, a network share) the install still never replaces: the
/// version that landed at that instant stays under the project's name, and
/// this save's finished file is kept beside it.
#[test]
fn a_version_that_lands_at_the_instant_of_the_install_is_never_replaced() {
    for (disk, hooks) in disks_without_an_exchange() {
        let dir = common::TempDir::new("aside-d");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        let project = file.clone();
        let (hooks, landed) =
            lands_at_the_install(hooks, move |to| to == project, b"version D".to_vec());
        k.set_publish_hooks(hooks);
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let failed = k
            .finish_session_save(plan.run())
            .expect_err("the name was taken at the instant of the install");
        assert!(landed.lock().unwrap().is_some(), "{disk}: D landed");
        assert_eq!(
            fs::read(&file).unwrap(),
            b"version D",
            "{disk}: the version that landed is still there: {failed}"
        );
        let kept = visible_siblings(&file);
        assert_eq!(kept.len(), 1, "{disk}: {kept:?}");
        assert_eq!(saved_name(&kept[0]).as_deref(), Some("One"), "{disk}");
        assert_eq!(
            pm_kernel::project_io::kept_beside(&failed.to_string()).as_deref(),
            Some(kept[0].as_path()),
            "{disk}: the failure names where the save is: {failed}"
        );
    }
}
/// Where a name can only be taken by creating it and copying into it, a copy
/// that fails half way (a full disk) is taken back: the project's name
/// holds the last saved version again, whole, the work is still open, and
/// the next save writes it.
#[test]
fn a_copy_that_fails_while_installing_leaves_the_last_saved_version_under_the_name() {
    let dir = common::TempDir::new("aside-g");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    let armed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        no_exchange: true,
        no_exclusive: true,
        copy_fails: Some(armed.clone()),
        ..Default::default()
    });
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let failed = k
        .finish_session_save(plan.run())
        .expect_err("the copy failed");
    assert!(!armed.load(std::sync::atomic::Ordering::SeqCst), "{failed}");
    assert_eq!(
        fs::read(&file).unwrap(),
        original,
        "the last saved version is under the project's name, whole: {failed}"
    );
    assert_eq!(visible_siblings(&file), [""; 0].map(Path::new));
    assert_eq!(private_leftovers(&file), [""; 0]);
    let plan = k.prepare_session_save(session, None, false).unwrap();
    k.finish_session_save(plan.run())
        .expect("the next save writes the file");
    assert_eq!(saved_name(&file).as_deref(), Some("One"));
}
/// The names kept files get ("… (other version …)", "… (recovered …)") are
/// taken the same way: a file that lands under one of them at the instant
/// it is taken is never replaced; the kept file gets the next name.
#[test]
fn a_file_that_lands_under_the_name_a_kept_version_is_given_is_never_replaced() {
    for (disk, hooks) in disks_without_an_exchange() {
        let dir = common::TempDir::new("aside-e");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        let (hooks, landed) = lands_at_the_install(
            pm_kernel::project_io::PublishHooks {
                before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
                ..hooks
            },
            |to| {
                to.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("cut (other version "))
            },
            b"somebody else's file".to_vec(),
        );
        k.set_publish_hooks(hooks);
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let ShellReply::Saved {
            displaced: Some(kept),
            ..
        } = k
            .finish_session_save(plan.run())
            .unwrap_or_else(|e| panic!("{disk}: the save is published: {e}"))
        else {
            panic!("{disk}: the save says another version was kept")
        };
        let arrival = landed.lock().unwrap().clone().expect("a file landed");
        assert_eq!(
            fs::read(&arrival).unwrap(),
            b"somebody else's file",
            "{disk}: the file that landed under that name is untouched"
        );
        assert_ne!(kept, arrival, "{disk}");
        assert_eq!(fs::read(&kept).unwrap(), b"version B", "{disk}");
        assert_eq!(saved_name(&file).as_deref(), Some("One"), "{disk}");
    }
}
/// The same at the next open after a save was killed between its two
/// renames: the last saved version is put back under the project's name
/// only if the name is still free at the instant it is taken. A version
/// that lands right then stays; the last saved version and the unfinished
/// save are both kept beside it.
#[test]
fn a_version_that_lands_while_the_last_saved_version_is_put_back_is_never_replaced() {
    for (disk, hooks) in disks_without_an_exchange() {
        let dir = common::TempDir::new("aside-f");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let original = fs::read(&file).unwrap();
        // Another version of the project, as another computer would save it.
        let elsewhere = dir.path("elsewhere/cut.pmv");
        fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        fs::copy(&file, &elsewhere).unwrap();
        {
            let mut other = kernel(&dir.path("other"));
            let (session, _) = open_file(&mut other, &elsewhere);
            rename_comp(&mut other, "P", "Elsewhere");
            other.shell(ShellCall::Save(session)).unwrap();
        }
        let version_d = fs::read(&elsewhere).unwrap();
        let native = dir.path("native");
        assert!(killed_between_renames(&native, &file, false));
        assert!(!file.exists(), "{disk}: killed with the name empty");
        let mut k = kernel(&native);
        let project = file.clone();
        let (hooks, landed) =
            lands_at_the_install(hooks, move |to| to == project, version_d.clone());
        k.set_publish_hooks(hooks);
        let _ = k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        });
        assert!(landed.lock().unwrap().is_some(), "{disk}: D landed");
        assert_eq!(
            fs::read(&file).unwrap(),
            version_d,
            "{disk}: the version that landed is still under the project's name"
        );
        let beside: Vec<_> = visible_siblings(&file)
            .into_iter()
            .map(|p| fs::read(p).unwrap())
            .collect();
        assert!(
            beside.contains(&original),
            "{disk}: the last saved version is kept beside it"
        );
        assert_eq!(beside.len(), 2, "{disk}: and the unfinished save");
    }
}
/// What the kernel has to tell about files kept beside the session's
/// project file: (why, where).
fn kept_beside(
    k: &mut Kernel,
    session: &SessionHandle,
) -> Vec<(String, Option<std::path::PathBuf>)> {
    let ShellReply::Kept(rows) = k.shell(ShellCall::KeptBeside(session.clone())).unwrap() else {
        panic!()
    };
    rows.iter()
        .map(|row| {
            (
                row.get("why").and_then(Value::as_str).unwrap().to_owned(),
                row.get("path").and_then(Value::as_str).map(Into::into),
            )
        })
        .collect()
}
/// The person saw the notices `rows` ([`kept_beside`]).
fn seen(k: &mut Kernel, session: &SessionHandle, rows: &[(String, Option<std::path::PathBuf>)]) {
    k.shell(ShellCall::KeptSeen {
        session: session.clone(),
        paths: rows.iter().filter_map(|(_, path)| path.clone()).collect(),
        restored: rows.iter().any(|(why, _)| why == "restored"),
        other: true,
    })
    .unwrap();
}
/// A notice about a kept file is written into the save's record before the
/// file is kept, stays there until the person has seen it, and is about the
/// project it happened to. So: asking does not forget it; a crash right
/// after the save does not lose it; every kept file is told, also when the
/// save itself failed; and it is not told to another project later put
/// under the same name.
#[test]
fn a_kept_version_is_told_until_it_was_seen_whatever_happens_in_between() {
    let dir = common::TempDir::new("notices");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let native = dir.path("native");
    let mut k = kernel(&native);
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
        ..Default::default()
    });
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let ShellReply::Saved {
        displaced: Some(kept),
        ..
    } = k.finish_session_save(plan.run()).unwrap()
    else {
        panic!()
    };
    let told = kept_beside(&mut k, &session);
    assert_eq!(told, [("arrived".to_owned(), Some(kept.clone()))]);
    assert_eq!(
        kept_beside(&mut k, &session),
        told,
        "asking forgets nothing"
    );
    // Nor does saving again before it was seen.
    k.set_publish_hooks(Default::default());
    rename_comp(&mut k, "P", "Two");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run()).unwrap();
    assert_eq!(kept_beside(&mut k, &session), told, "still this project's");
    // Not seen yet, and the app goes away: the next open still says it.
    k.set_publish_hooks(Default::default());
    k.shell(ShellCall::Close(session)).unwrap();
    drop(k);
    let mut k = kernel(&native);
    let (session, _) = open_file(&mut k, &file);
    assert_eq!(kept_beside(&mut k, &session), told);
    // Another project put under that name is told there is a kept file,
    // and that it is from an earlier save under the name.
    let mine = fs::read(&file).unwrap();
    let elsewhere = dir.path("elsewhere/cut.pmv");
    fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    fs::copy(common::ts_fixture("fresh.pmv"), &elsewhere).unwrap();
    {
        let mut other = kernel(&dir.path("other"));
        let (session, _) = open_file(&mut other, &elsewhere);
        rename_comp(&mut other, "P", "Elsewhere");
        other.shell(ShellCall::Save(session)).unwrap();
    }
    k.shell(ShellCall::Close(session)).unwrap();
    fs::copy(&elsewhere, &file).unwrap();
    let ShellReply::Opened { session, .. } = k
        .shell(ShellCall::Open {
            id: "Q".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        kept_beside(&mut k, &session),
        [("earlier".to_owned(), Some(kept.clone()))],
        "told as a file an earlier save under this name kept, not as this project's"
    );
    k.shell(ShellCall::Close(session)).unwrap();
    // The project it is about, again: told, seen, and then not told again.
    fs::write(&file, &mine).unwrap();
    let (session, _) = open_file(&mut k, &file);
    assert_eq!(kept_beside(&mut k, &session), told);
    seen(&mut k, &session, &told);
    assert_eq!(kept_beside(&mut k, &session), []);
    assert_eq!(private_leftovers(&file), [""; 0]);
    assert_eq!(
        fs::read(&kept).unwrap(),
        b"version B",
        "the file itself stays"
    );
}
/// A save kept another version beside the project, and before anyone was
/// told another program legitimately changed the project file (an editor
/// on another computer, synced in). The file under the name is no longer
/// one the save's record knows, but the kept file is still there and still
/// untold: the person is told about it, as a file an earlier save kept,
/// until they have seen it.
#[test]
fn a_kept_version_is_still_told_after_another_program_changed_the_project() {
    let dir = common::TempDir::new("notice-edited");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let native = dir.path("native");
    let mut k = kernel(&native);
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
        ..Default::default()
    });
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let ShellReply::Saved {
        displaced: Some(kept),
        ..
    } = k.finish_session_save(plan.run()).unwrap()
    else {
        panic!()
    };
    k.set_publish_hooks(Default::default());
    k.shell(ShellCall::Close(session)).unwrap();
    drop(k);
    // Edited elsewhere before the notice was ever shown.
    write_through(&file, &fs::read(common::ts_fixture("save-as.pmv")).unwrap());
    let mut k = kernel(&native);
    let (session, _) = open_file(&mut k, &file);
    let told = kept_beside(&mut k, &session);
    assert_eq!(told, [("earlier".to_owned(), Some(kept.clone()))]);
    // And a save by this app does not make it forget.
    rename_comp(&mut k, "P", "Two");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run()).unwrap();
    assert_eq!(kept_beside(&mut k, &session).len(), 1, "still told");
    let told = kept_beside(&mut k, &session);
    seen(&mut k, &session, &told);
    assert_eq!(kept_beside(&mut k, &session), []);
    assert_eq!(fs::read(&kept).unwrap(), b"version B");
}
/// A save that dies right after it kept another version (before anything
/// could reply or write a notice elsewhere) has already recorded it: the
/// next open tells.
#[test]
fn a_crash_right_after_a_version_was_kept_does_not_lose_the_notice() {
    for disk in ["real", "no-exchange", "no-exclusive"] {
        let dir = common::TempDir::new("notice-crash");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        let crashed = Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "save_child_process"])
            .env("PM_KERNEL_TEST_SAVE_PROFILE", &native)
            .env("PM_KERNEL_TEST_SAVE_FILE", &file)
            .env("PM_KERNEL_TEST_SAVE_DISK", disk)
            .env("PM_KERNEL_TEST_SAVE_KILL", "closed")
            .env("PM_KERNEL_TEST_SAVE_LAND", "1")
            .stdout(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(crashed.status.code(), Some(0), "{disk}");
        assert_eq!(saved_name(&file).as_deref(), Some("One"), "{disk}");
        let kept = visible_siblings(&file);
        assert_eq!(kept.len(), 1, "{disk}: {kept:?}");
        let mut k = kernel(&native);
        k.set_publish_hooks(child_disk(disk));
        let session = match k
            .shell(ShellCall::PrepareOpen {
                id: "P".into(),
                source: SessionSource::File(file.clone()),
                electron_profile: None,
            })
            .unwrap()
        {
            ShellReply::Opened { session, .. } => session,
            ShellReply::RecoveryRequired { ticket, .. } => {
                decide(&mut k, ticket, RecoveryDecision::Discard)
            }
            other => panic!("{disk}: {other:?}"),
        };
        assert_eq!(
            kept_beside(&mut k, &session),
            [("arrived".to_owned(), Some(kept[0].clone()))],
            "{disk}"
        );
    }
}
/// Two versions arrive during one save: one before the file is moved aside
/// (it is what gets moved, and is kept), one between the two renames (it
/// takes the name; this save's file is kept beside it and the save fails).
/// The failure names this save's file. The other kept file is told as well:
/// both are in the save's record.
#[test]
fn every_file_a_failed_save_kept_is_told_not_only_the_one_its_error_names() {
    for (disk, hooks) in disks_without_an_exchange() {
        let dir = common::TempDir::new("notices-both");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version X"))),
            between_renames: Some(std::sync::Arc::new(|path| land(path, b"version Y"))),
            ..hooks
        });
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let failed = k
            .finish_session_save(plan.run())
            .expect_err("Y took the name");
        let ours = pm_kernel::project_io::kept_beside(&failed.to_string()).expect("named");
        assert_eq!(fs::read(&file).unwrap(), b"version Y", "{disk}");
        let told = kept_beside(&mut k, &session);
        assert_eq!(told.len(), 2, "{disk}: {told:?}");
        assert!(
            told.contains(&("unfinished".to_owned(), Some(ours))),
            "{disk}: {told:?}"
        );
        let (_, other) = told
            .iter()
            .find(|(why, _)| why == "arrived")
            .expect("X is told");
        assert_eq!(
            fs::read(other.as_ref().unwrap()).unwrap(),
            b"version X",
            "{disk}"
        );
    }
}
/// A save nobody is shown the result of (the one a close does on its own
/// when a window goes at quit, or a compaction of every open project) finds
/// another version at the project's path. That version is
/// kept beside the project, as in any save, but there is no window to say
/// so in: the project's entry remembers it, and the next open of the
/// project says it, once.
#[test]
fn a_version_kept_by_a_save_at_quit_or_close_is_told_at_the_next_open() {
    for at_quit in [true, false] {
        let dir = common::TempDir::new("kept-untold");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        let mut k = kernel(&native);
        let (session, _) = open_file(&mut k, &file);
        rename_comp(&mut k, "P", "One");
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
            ..Default::default()
        });
        if at_quit {
            // Every open project written where it belongs, then the end.
            k.shell(ShellCall::CompactAll).unwrap();
            k.shell(ShellCall::ReleaseLibrary).unwrap();
        } else {
            k.shell(ShellCall::Close(session)).unwrap();
        }
        drop(k);
        assert_eq!(saved_name(&file).as_deref(), Some("One"), "the save stands");
        let kept = visible_siblings(&file);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(fs::read(&kept[0]).unwrap(), b"version B");
        // The next launch: opened the way the app opens it.
        let mut k = kernel(&native);
        let ShellReply::Opened { session, .. } = k
            .shell(ShellCall::PrepareOpen {
                id: "P".into(),
                source: SessionSource::File(file.clone()),
                electron_profile: None,
            })
            .unwrap()
        else {
            panic!("a saved project opens without a question")
        };
        let told = kept_beside(&mut k, &session);
        assert_eq!(
            told,
            [("arrived".to_owned(), Some(kept[0].clone()))],
            "at_quit {at_quit}"
        );
        seen(&mut k, &session, &told);
        assert_eq!(kept_beside(&mut k, &session), [], "told until seen");
        // Told, and closed: the next open has nothing to say.
        k.shell(ShellCall::Close(session)).unwrap();
        let (session, _) = open_file(&mut k, &file);
        assert_eq!(kept_beside(&mut k, &session), []);
    }
}
/// The disk a child process pretends to save on (`PM_KERNEL_TEST_SAVE_DISK`).
fn child_disk(disk: &str) -> pm_kernel::project_io::PublishHooks {
    use pm_kernel::project_io::PublishHooks;
    match disk {
        // The volume decides the route.
        "real" => PublishHooks::default(),
        "no-exchange" => PublishHooks {
            no_exchange: true,
            ..Default::default()
        },
        "no-exclusive" => PublishHooks {
            no_exchange: true,
            no_exclusive: true,
            ..Default::default()
        },
        "links-only" => PublishHooks {
            no_exchange: true,
            links_only: true,
            ..Default::default()
        },
        other => panic!("{other}"),
    }
}
/// A child process that opens a project and saves it (or only opens it:
/// `PM_KERNEL_TEST_SAVE_ACTION=open`) and is killed at a point of the save
/// or of the recovery at open: `between` (between the two renames),
/// `mid-copy`, `unlink` (a version was captured and read, and is about to
/// be unlinked), `withdrawn` (a file was moved off a public name and is not
/// judged yet), or the name of a step of the save's record, optionally
/// `step#n` for its n-th occurrence. Killed: nothing after that instant
/// runs, not even a destructor. Exits 3 when the point was never reached.
#[test]
fn save_child_process() {
    let (Some(profile), Some(file)) = (
        std::env::var_os("PM_KERNEL_TEST_SAVE_PROFILE"),
        std::env::var_os("PM_KERNEL_TEST_SAVE_FILE"),
    ) else {
        return;
    };
    let var = |name: &str| std::env::var(name).ok();
    let kill = var("PM_KERNEL_TEST_SAVE_KILL").unwrap_or_else(|| "between".into());
    let (step, nth) = match kill.split_once('#') {
        Some((step, nth)) => (step.to_owned(), nth.parse::<usize>().unwrap()),
        None => (kill.clone(), 1),
    };
    let seen = std::sync::atomic::AtomicUsize::new(0);
    let land_at = var("PM_KERNEL_TEST_SAVE_LAND_AT");
    // Paused, not killed: says it got there, waits to be told to go on.
    let die: pm_kernel::project_io::PublishHook =
        match std::env::var_os("PM_KERNEL_TEST_SAVE_PAUSE").map(std::path::PathBuf::from) {
            Some(go_on) => std::sync::Arc::new(move |_| {
                fs::write(go_on.with_extension("reached"), b"").unwrap();
                while !go_on.exists() {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }),
            None => std::sync::Arc::new(|_| unsafe { libc::_exit(0) }),
        };
    let hooks = pm_kernel::project_io::PublishHooks {
        between_renames: (kill == "between").then(|| die.clone()),
        mid_copy: (kill == "mid-copy").then(|| die.clone()),
        before_unlink: (kill == "unlink").then(|| die.clone()),
        after_withdrawn: (kill == "withdrawn").then(|| die.clone()),
        step: Some(std::sync::Arc::new(move |reached, path| {
            // Another program's file lands at the project's name right as
            // this step is about to be taken.
            if land_at.as_deref() == Some(reached) {
                land(path, b"version B");
            }
            if reached == step && seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 == nth
            {
                unsafe { libc::_exit(0) }
            }
        })),
        owner_host: var("PM_KERNEL_TEST_SAVE_HOST"),
        before_exchange: var("PM_KERNEL_TEST_SAVE_LAND").map(
            |_| -> pm_kernel::project_io::PublishHook {
                std::sync::Arc::new(|path| land(path, b"version B"))
            },
        ),
        ..child_disk(&var("PM_KERNEL_TEST_SAVE_DISK").unwrap_or_else(|| "no-exchange".into()))
    };
    // A build that does not share the user's lock folder has its own. A
    // child that is killed never removes its lock file, so without one it
    // locks beside its own profile, never in the user's lock folder.
    let locks = std::env::var_os("PM_KERNEL_TEST_SAVE_LOCKS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(&profile).with_extension("child-locks"));
    let mut k = kernel_with_locks(Path::new(&profile), &locks);
    if var("PM_KERNEL_TEST_SAVE_ACTION").as_deref() == Some("open") {
        k.set_publish_hooks(hooks);
        let _ = k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.into()),
            electron_profile: None,
        });
        std::process::exit(3);
    }
    let (session, _) = open_file(&mut k, Path::new(&file));
    rename_comp(&mut k, "P", "One");
    k.set_publish_hooks(hooks);
    let plan = k.prepare_session_save(session, None, false).unwrap();
    k.finish_session_save(plan.run()).unwrap();
    std::process::exit(3);
}
/// Runs [`save_child_process`] with the profile `native` on `file`. True
/// when it was killed at `kill`; false when it never got there.
fn killed(native: &Path, file: &Path, disk: &str, kill: &str, action: &str) -> bool {
    let mut child = Process::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "save_child_process"])
        .env("PM_KERNEL_TEST_SAVE_PROFILE", native)
        .env("PM_KERNEL_TEST_SAVE_FILE", file)
        .env("PM_KERNEL_TEST_SAVE_DISK", disk)
        .env("PM_KERNEL_TEST_SAVE_KILL", kill)
        .env("PM_KERNEL_TEST_SAVE_ACTION", action)
        .stdout(Stdio::null());
    let killed = child.output().unwrap();
    assert!(
        matches!(killed.status.code(), Some(0 | 3)),
        "{disk} {kill} {action}: {}",
        String::from_utf8_lossy(&killed.stderr)
    );
    killed.status.code() == Some(0)
}
/// Opens `file` in a child process ([`save_child_process`]) in which another
/// program's file lands under the project's name as recovery reaches the
/// step `land_at`, and which is killed right after recovery moved a file off
/// a public name. True when it got there.
fn killed_landing(native: &Path, file: &Path, disk: &str, land_at: &str) -> bool {
    let mut child = Process::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "save_child_process"])
        .env("PM_KERNEL_TEST_SAVE_PROFILE", native)
        .env("PM_KERNEL_TEST_SAVE_FILE", file)
        .env("PM_KERNEL_TEST_SAVE_DISK", disk)
        .env("PM_KERNEL_TEST_SAVE_KILL", "withdrawn")
        .env("PM_KERNEL_TEST_SAVE_LAND_AT", land_at)
        .env("PM_KERNEL_TEST_SAVE_ACTION", "open")
        .stdout(Stdio::null());
    let killed = child.output().unwrap();
    assert!(
        matches!(killed.status.code(), Some(0 | 3)),
        "{disk} {land_at}: {}",
        String::from_utf8_lossy(&killed.stderr)
    );
    killed.status.code() == Some(0)
}
/// Opens `file` in a child process with the profile `native`, renames its
/// composition to "One", saves, and kills the child between the save's two
/// renames. False when the save had no such moment (an atomic exchange, on
/// a `real` volume that has one).
fn killed_between_renames(native: &Path, file: &Path, real: bool) -> bool {
    let disk = if real { "real" } else { "no-exchange" };
    killed(native, file, disk, "between", "save")
}
/// The app is killed (or the disk pulled) between the two renames: the
/// project's name holds nothing and the last saved version sits beside it
/// under a private name. The next open, by the entry or by the file's name,
/// puts that version back under the project's name and offers the unsaved
/// work from the journal, so the person has both the old and the new.
#[test]
fn a_save_killed_between_its_two_renames_leaves_the_last_saved_version_for_the_next_open() {
    for by_entry in [false, true] {
        let dir = common::TempDir::new("aside-killed");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let original = fs::read(&file).unwrap();
        let native = dir.path("native");
        assert!(killed_between_renames(&native, &file, false));
        assert!(!file.exists(), "killed with the name empty");
        let aside: Vec<_> = private_leftovers(&file)
            .into_iter()
            .filter(|n| n.ends_with(".aside"))
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(
            fs::read(dir.path("job").join(&aside[0])).unwrap(),
            original,
            "the last saved version, whole, under its private name"
        );
        let mut k = kernel(&native);
        let source = if by_entry {
            SessionSource::Library {
                profile: native.clone(),
                project: "P".into(),
            }
        } else {
            SessionSource::File(file.clone())
        };
        let (ticket, recovery) = prepare(&mut k, "P", source);
        assert!(
            matches!(recovery, RecoveryResult::Recovered { truncated: false }),
            "{recovery:?}"
        );
        assert_eq!(
            fs::read(&file).unwrap(),
            original,
            "the last saved version is under the project's name again"
        );
        let session = decide(&mut k, ticket, RecoveryDecision::Recover);
        let name = k.project("P").unwrap().store().root().unwrap().get("name");
        assert_eq!(
            name.map(|n| n.to_value()).as_ref().and_then(Value::as_str),
            Some("One"),
            "the work being saved is recovered"
        );
        // The file that save had finished is not left hidden: it holds the
        // same work, under a plain name.
        assert_eq!(private_leftovers(&file), [""; 0]);
        let unfinished = visible_siblings(&file);
        assert_eq!(unfinished.len(), 1, "{unfinished:?}");
        assert!(
            unfinished[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("cut (recovered "),
            "{unfinished:?}"
        );
        assert_eq!(saved_name(&unfinished[0]).as_deref(), Some("One"));
        // The person is told once what was found and where it is.
        let told = kept_beside(&mut k, &session);
        assert_eq!(
            told,
            [
                ("restored".to_owned(), None),
                ("unfinished".to_owned(), Some(unfinished[0].clone()))
            ]
        );
        seen(&mut k, &session, &told);
        assert_eq!(kept_beside(&mut k, &session), []);
        // And it saves: the session is an ordinary one again.
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            no_exchange: true,
            ..Default::default()
        });
        let plan = k.prepare_session_save(session, None, false).unwrap();
        k.finish_session_save(plan.run()).unwrap();
        assert_eq!(saved_name(&file).as_deref(), Some("One"));
    }
}
/// The files a save's record names, found beside `file` by extension.
fn private_file(file: &Path, extension: &str) -> std::path::PathBuf {
    let found: Vec<_> = private_leftovers(file)
        .into_iter()
        .filter(|n| n.ends_with(extension))
        .collect();
    assert_eq!(found.len(), 1, "{extension}: {found:?}");
    file.parent().unwrap().join(&found[0])
}
/// A save on a volume that can only copy a file to its name was cut short
/// mid-copy, and then the incomplete file at the project's name was not the
/// one this save wrote: a sync client changed it, or it is a fresh file with
/// the same first bytes (no mark of this save). Recovery never takes such a
/// file back, so it is not lost; the open cannot parse it, and the person is
/// offered the versions the save's record proves (the last saved one and the
/// one being saved). Choosing one opens it; the file that was at the name is
/// kept beside it.
#[test]
fn an_incomplete_file_at_the_name_never_traps_the_open_when_whole_versions_exist() {
    for case in [
        "changed by a sync client",
        "a copy without this save's mark",
    ] {
        let dir = common::TempDir::new("trapped-open");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let original = saved_name(&file);
        let native = dir.path("native");
        assert!(killed(&native, &file, "no-exclusive", "mid-copy", "save"));
        let mut stray = fs::read(&file).unwrap();
        if case == "changed by a sync client" {
            stray[10] ^= 0xff;
        }
        land(&file, &stray);
        let mut k = kernel(&native);
        k.set_publish_hooks(child_disk("no-exclusive"));
        k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .expect_err(&format!("{case}: the file at the name is not a project"));
        let holding = |bytes: &[u8]| {
            fs::read_dir(dir.path("job"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .any(|p| fs::read(&p).is_ok_and(|b| b == bytes))
        };
        assert!(holding(&stray), "{case}: the file at the name is not lost");
        let ShellReply::Leftovers(rows) = k.shell(ShellCall::Leftovers(file.clone())).unwrap()
        else {
            panic!()
        };
        let text = |row: &Value, key: &str| row.get(key).and_then(Value::as_str).map(str::to_owned);
        let offered = |role: &str| {
            rows.iter()
                .find(|r| text(r, "role").as_deref() == Some(role))
                .and_then(|r| text(r, "path"))
                .map(std::path::PathBuf::from)
        };
        let saving = offered("saving").unwrap_or_else(|| panic!("{case}: {rows:?}"));
        assert_eq!(saved_name(&saving).as_deref(), Some("One"), "{case}");
        let previous = offered("previous").unwrap_or_else(|| panic!("{case}: {rows:?}"));
        assert_eq!(saved_name(&previous), original, "{case}");
        k.shell(ShellCall::UseLeftover {
            path: file.clone(),
            leftover: saving,
            unproven: false,
        })
        .unwrap_or_else(|e| panic!("{case}: {e}"));
        assert_eq!(saved_name(&file).as_deref(), Some("One"), "{case}");
        assert!(holding(&stray), "{case}: kept beside");
        assert!(
            visible_siblings(&file)
                .iter()
                .any(|p| fs::read(p).unwrap() == stray),
            "{case}: under a name the person finds"
        );
        let (session, _) = open_file(&mut k, &file);
        let _ = session;
    }
}
/// The file under the project's name does not open, and of the two files
/// the save's record names one is damaged too (an interrupted sync, another
/// writer). Only a file read now and found to hold exactly what the record
/// says is offered as a version, and only such a file can be made the
/// project unasked for more: the damaged one is listed as not identified
/// and is never installed in place of the whole version that is there.
#[test]
fn a_file_that_is_not_what_its_record_says_is_never_offered_or_installed_as_a_version() {
    let dir = common::TempDir::new("unproven-offer");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let native = dir.path("native");
    assert!(killed(&native, &file, "no-exclusive", "mid-copy", "save"));
    // The incomplete copy was changed (so it is not taken back), and the
    // staged file is damaged.
    land(&file, b"not a project");
    let staged = private_file(&file, ".tmp");
    write_through(&staged, b"damaged while it was synced");
    let mut k = kernel(&native);
    k.set_publish_hooks(child_disk("no-exclusive"));
    k.shell(ShellCall::PrepareOpen {
        id: "P".into(),
        source: SessionSource::File(file.clone()),
        electron_profile: None,
    })
    .expect_err("the file at the name is not a project");
    let ShellReply::Leftovers(rows) = k.shell(ShellCall::Leftovers(file.clone())).unwrap() else {
        panic!()
    };
    let text = |row: &Value, key: &str| row.get(key).and_then(Value::as_str).map(str::to_owned);
    let proven: Vec<_> = rows
        .iter()
        .filter(|r| r.get("proven") == Some(&Value::Bool(true)))
        .collect();
    // The last saved version, under its private name and (recovery kept
    // it beside the unreadable file) under a plain one.
    assert!(!proven.is_empty(), "a whole version is offered: {rows:?}");
    for row in &proven {
        assert_eq!(text(row, "role").as_deref(), Some("previous"), "{rows:?}");
        assert_eq!(fs::read(text(row, "path").unwrap()).unwrap(), original);
    }
    let previous = std::path::PathBuf::from(text(proven[0], "path").unwrap());
    let damaged = rows
        .iter()
        .find(|r| text(r, "path").as_deref() == staged.to_str())
        .unwrap_or_else(|| panic!("the damaged file is listed: {rows:?}"));
    assert_eq!(damaged.get("proven"), Some(&Value::Bool(false)));
    let before = tree(&dir.path("job"));
    k.shell(ShellCall::UseLeftover {
        path: file.clone(),
        leftover: staged.clone(),
        unproven: false,
    })
    .expect_err("a file that is not what its record says is not a version to open");
    assert_eq!(tree(&dir.path("job")), before, "nothing was moved");
    k.shell(ShellCall::UseLeftover {
        path: file.clone(),
        leftover: previous,
        unproven: false,
    })
    .unwrap();
    assert_eq!(fs::read(&file).unwrap(), original);
    let kept: Vec<_> = visible_siblings(&file)
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect();
    assert!(kept.contains(&b"not a project".to_vec()), "{kept:?}");
    assert!(
        kept.contains(&b"damaged while it was synced".to_vec()),
        "the damaged file is kept for the person too: {kept:?}"
    );
}
/// A crash in the middle of writing a record line leaves a torn tail. What
/// recovery appends after it must still be read: the record is cut back
/// to its last whole line first. Here the last saved version is put back,
/// and that is told at the next open.
#[test]
fn recovery_after_a_torn_record_line_is_not_lost_behind_it() {
    let dir = common::TempDir::new("torn-tail");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let native = dir.path("native");
    assert!(killed_between_renames(&native, &file, false));
    let record = private_file(&file, ".save");
    let mut log = fs::OpenOptions::new().append(true).open(&record).unwrap();
    std::io::Write::write_all(&mut log, br#"0123456789abcdef {"step":"ins"#).unwrap();
    drop(log);
    let mut k = kernel(&native);
    let (ticket, _) = prepare(&mut k, "P", SessionSource::File(file.clone()));
    assert_eq!(fs::read(&file).unwrap(), original, "put back");
    let session = decide(&mut k, ticket, RecoveryDecision::Recover);
    assert!(
        kept_beside(&mut k, &session)
            .iter()
            .any(|(why, _)| why == "restored"),
        "and told"
    );
}
/// A record is read only if every name in it is a name of its own save
/// beside the project: a single file name, never a path. A record whose
/// step names a file elsewhere (a crafted or damaged line that still carries
/// a valid checksum) is not acted on at all, so recovery cannot withdraw or
/// remove a file outside the project's folder, here one whose bytes begin
/// the staged file's.
#[test]
fn a_record_that_names_a_file_outside_the_project_folder_is_never_acted_on() {
    let dir = common::TempDir::new("crafted-record");
    fs::create_dir_all(dir.path("outside")).unwrap();
    let victim = dir.path("outside/victim.pmv");
    for to in [
        "../outside/victim.pmv".to_owned(),
        victim.display().to_string(),
    ] {
        let job = dir.path("job");
        let _ = fs::remove_dir_all(&job);
        fs::create_dir_all(&job).unwrap();
        let file = job.join("cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        let _ = fs::remove_dir_all(&native);
        assert!(killed(&native, &file, "no-exchange", "install", "save"));
        let staged = private_file(&file, ".tmp");
        let bytes = fs::read(&staged).unwrap();
        fs::write(&victim, &bytes[..bytes.len() / 2]).unwrap();
        let mut step = pm_model::json::Map::new();
        step.insert("step", Value::from("keep"));
        step.insert(
            "from",
            Value::from(staged.file_name().unwrap().to_string_lossy().into_owned()),
        );
        step.insert("to", Value::from(to.as_str()));
        step.insert("why", Value::from("other"));
        let text = to_canonical_string(&Value::Object(step));
        let sum = &pm_model::hash::sha256_hex(text.as_bytes())[..16];
        let record = private_file(&file, ".save");
        let mut log = fs::OpenOptions::new().append(true).open(&record).unwrap();
        std::io::Write::write_all(&mut log, format!("{sum} {text}\n").as_bytes()).unwrap();
        drop(log);
        let outside = tree(&dir.path("outside"));
        let mut k = kernel(&native);
        let _ = k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        });
        assert!(
            tree(&dir.path("outside")) == outside,
            "{to}: the file outside and its folder are untouched"
        );
        assert!(staged.exists(), "{to}: the record's own files stay too");
    }
}
/// Recovery goes by a save's record, never by a file's name or date. A file
/// that merely has the name a save gives the file it moves aside (left by
/// something else, or corrupt) is not named by any record: it is never made
/// the project and never touched, however new it is, and the Error Log says
/// it is there. The file the record does name goes back under the project's
/// name.
#[test]
fn a_file_no_record_names_is_never_made_the_project() {
    let dir = common::TempDir::new("foreign-aside");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let native = dir.path("native");
    assert!(killed_between_renames(&native, &file, false));
    // Newer than the file the save moved aside, and not a project.
    let foreign = dir.path("job/.cut.pmv.0f8fad5b-d9cb-469f-a165-70867728950e.aside");
    fs::write(&foreign, b"not this project").unwrap();
    let mut k = kernel(&native);
    let (ticket, _) = prepare(&mut k, "P", SessionSource::File(file.clone()));
    assert_eq!(
        fs::read(&file).unwrap(),
        original,
        "the version the record names is the project again"
    );
    assert_eq!(fs::read(&foreign).unwrap(), b"not this project");
    let log = k.take_log();
    assert!(
        log.iter().any(|l| l.contains(&*foreign.to_string_lossy())),
        "the Error Log names the file that was left: {log:?}"
    );
    decide(&mut k, ticket, RecoveryDecision::Recover);
    drop(k);
    // With no record at all, a file of that name is nothing to recover from.
    let alone = dir.path("alone/cut.pmv");
    fs::create_dir_all(alone.parent().unwrap()).unwrap();
    let foreign = dir.path("alone/.cut.pmv.0f8fad5b-d9cb-469f-a165-70867728950e.aside");
    fs::write(&foreign, &original).unwrap();
    let mut k = kernel(&dir.path("native-alone"));
    k.shell(ShellCall::PrepareOpen {
        id: "P".into(),
        source: SessionSource::File(alone.clone()),
        electron_profile: None,
    })
    .expect_err("nothing is under that name");
    assert!(!alone.exists());
    assert_eq!(fs::read(&foreign).unwrap(), original);
}
/// The file a record names is believed only once its bytes were read and
/// are what the record says. When the file moved aside holds something else
/// (it changed, or it was another version that had just landed), recovery
/// cannot tell which file is the project: it moves nothing, the open says
/// so in plain words, and the person chooses. The file they choose takes the
/// project's name; the other gets a name they can find.
#[test]
fn files_that_do_not_match_their_record_are_left_for_the_person_to_choose() {
    for choice in ["previous", "saving"] {
        let dir = common::TempDir::new("undecided");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let native = dir.path("native");
        assert!(killed_between_renames(&native, &file, false));
        let aside = private_file(&file, ".aside");
        // Another version of the project, as another computer would save it.
        let elsewhere = dir.path("elsewhere/cut.pmv");
        fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        fs::copy(&aside, &elsewhere).unwrap();
        {
            let mut other = kernel(&dir.path("other"));
            let (session, _) = open_file(&mut other, &elsewhere);
            rename_comp(&mut other, "P", "Elsewhere");
            other.shell(ShellCall::Save(session)).unwrap();
        }
        fs::copy(&elsewhere, &aside).unwrap();
        let before = tree(&dir.path("job"));
        let mut k = kernel(&native);
        let refused = k
            .shell(ShellCall::PrepareOpen {
                id: "P".into(),
                source: SessionSource::File(file.clone()),
                electron_profile: None,
            })
            .expect_err("which file is the project cannot be told");
        assert!(
            refused.to_string().contains("Nothing was moved"),
            "{refused}"
        );
        assert_eq!(tree(&dir.path("job")), before, "nothing was moved");
        let ShellReply::Leftovers(rows) = k.shell(ShellCall::Leftovers(file.clone())).unwrap()
        else {
            panic!()
        };
        let text = |row: &Value, key: &str| row.get(key).and_then(Value::as_str).map(str::to_owned);
        let roles: Vec<_> = rows.iter().map(|r| text(r, "role").unwrap()).collect();
        assert_eq!(roles, ["previous", "saving"], "{rows:?}");
        assert!(
            rows.iter()
                .all(|r| text(r, "why").as_deref() == Some("unknown"))
        );
        assert!(
            rows.iter().all(|r| text(r, "computer").is_some()),
            "{rows:?}"
        );
        assert_eq!(tree(&dir.path("job")), before, "asking moves nothing");
        let chosen = rows
            .iter()
            .find(|r| text(r, "role").as_deref() == Some(choice))
            .and_then(|r| text(r, "path"))
            .unwrap();
        // Told that neither could be identified, the person chooses one.
        k.shell(ShellCall::UseLeftover {
            path: file.clone(),
            leftover: chosen.into(),
            unproven: true,
        })
        .unwrap();
        let (mine, other) = if choice == "previous" {
            ("Elsewhere", "One")
        } else {
            ("One", "Elsewhere")
        };
        assert_eq!(saved_name(&file).as_deref(), Some(mine), "{choice}");
        let kept = visible_siblings(&file);
        assert_eq!(kept.len(), 1, "{choice}: {kept:?}");
        assert_eq!(saved_name(&kept[0]).as_deref(), Some(other), "{choice}");
        assert_eq!(private_leftovers(&file), [""; 0], "{choice}");
        k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .expect("the project opens");
    }
}
/// Two saves of one file that were both cut short: neither record is acted
/// on. Nothing is moved and the person chooses.
#[test]
fn two_records_of_one_file_are_never_settled_unasked() {
    let dir = common::TempDir::new("two-records");
    let native = dir.path("native");
    for job in ["job", "other-job"] {
        fs::create_dir_all(dir.path(job)).unwrap();
        fs::copy(
            common::ts_fixture("fresh.pmv"),
            dir.path(job).join("cut.pmv"),
        )
        .unwrap();
    }
    let file = dir.path("job/cut.pmv");
    assert!(killed_between_renames(&native, &file, false));
    // The second: a save of a file of that name cut short before it moved
    // anything, its record and staged file brought beside the first.
    let second = dir.path("other-job/cut.pmv");
    assert!(killed(
        &dir.path("native-2"),
        &second,
        "no-exchange",
        "aside",
        "save"
    ));
    for name in private_leftovers(&second) {
        fs::rename(
            dir.path("other-job").join(&name),
            dir.path("job").join(&name),
        )
        .unwrap();
    }
    let before = tree(&dir.path("job"));
    let mut k = kernel(&native);
    k.shell(ShellCall::PrepareOpen {
        id: "P".into(),
        source: SessionSource::File(file.clone()),
        electron_profile: None,
    })
    .expect_err("two saves were cut short");
    assert_eq!(tree(&dir.path("job")), before, "nothing was moved");
    let ShellReply::Leftovers(rows) = k.shell(ShellCall::Leftovers(file.clone())).unwrap() else {
        panic!()
    };
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(
        rows.iter()
            .all(|r| r.get("why").and_then(Value::as_str) == Some("several")),
        "{rows:?}"
    );
}
/// A save killed at any of its steps, on each kind of disk, is settled by
/// the next open from its record: the project's name holds a whole version
/// (the last saved one, or the new one once it was installed), the work
/// being saved is in it or offered from the journal, and nothing is left
/// under a private name.
#[test]
fn a_save_killed_at_any_step_is_settled_at_the_next_open() {
    let cases = [
        ("real", &["begin", "exchanged", "closed"][..]),
        ("no-exchange", &["begin", "aside", "install", "closed"]),
        ("links-only", &["aside", "install", "closed"]),
        ("no-exclusive", &["aside", "install", "mid-copy", "closed"]),
    ];
    for (disk, kills) in cases {
        for kill in kills {
            let dir = common::TempDir::new("killed-step");
            fs::create_dir_all(dir.path("job")).unwrap();
            let file = dir.path("job/cut.pmv");
            fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
            let original = fs::read(&file).unwrap();
            let native = dir.path("native");
            assert!(killed(&native, &file, disk, kill, "save"), "{disk} {kill}");
            let mut k = kernel(&native);
            k.set_publish_hooks(child_disk(disk));
            let opened = k
                .shell(ShellCall::PrepareOpen {
                    id: "P".into(),
                    source: SessionSource::File(file.clone()),
                    electron_profile: None,
                })
                .unwrap_or_else(|e| panic!("{disk} {kill}: {e}"));
            let on_disk = fs::read(&file).unwrap();
            let session = match opened {
                ShellReply::RecoveryRequired { ticket, .. } => {
                    assert_eq!(on_disk, original, "{disk} {kill}: the last saved version");
                    decide(&mut k, ticket, RecoveryDecision::Recover)
                }
                ShellReply::Opened { session, .. } => {
                    assert_eq!(
                        saved_name(&file).as_deref(),
                        Some("One"),
                        "{disk} {kill}: the save was installed"
                    );
                    session
                }
                other => panic!("{disk} {kill}: {other:?}"),
            };
            let name = k.project("P").unwrap().store().root().unwrap().get("name");
            assert_eq!(
                name.map(|n| n.to_value()).as_ref().and_then(Value::as_str),
                Some("One"),
                "{disk} {kill}: the work being saved is there"
            );
            assert_eq!(private_leftovers(&file), [""; 0], "{disk} {kill}");
            for kept in visible_siblings(&file) {
                assert_eq!(
                    saved_name(&kept).as_deref(),
                    Some("One"),
                    "{disk} {kill}: only the unfinished save is kept beside: {kept:?}"
                );
            }
            let plan = k.prepare_session_save(session, None, false).unwrap();
            k.finish_session_save(plan.run())
                .unwrap_or_else(|e| panic!("{disk} {kill}: the next save: {e}"));
            assert_eq!(saved_name(&file).as_deref(), Some("One"), "{disk} {kill}");
            assert_eq!(private_leftovers(&file), [""; 0], "{disk} {kill}");
        }
    }
}
/// A second crash, in the middle of the recovery itself, loses nothing and
/// strands nothing: the record stays until everything it names is settled,
/// each of recovery's moves is recorded before it is made, and the open
/// after that carries on from where it stopped.
#[test]
fn a_second_crash_during_recovery_is_carried_on_by_the_next_open() {
    for disk in ["no-exchange", "no-exclusive"] {
        for second in ["restore", "restored", "keep", "mid-copy", "closed"] {
            if second == "mid-copy" && disk != "no-exclusive" {
                continue;
            }
            let dir = common::TempDir::new("killed-recovery");
            fs::create_dir_all(dir.path("job")).unwrap();
            let file = dir.path("job/cut.pmv");
            fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
            let original = fs::read(&file).unwrap();
            let native = dir.path("native");
            assert!(killed(&native, &file, disk, "between", "save"), "{disk}");
            assert!(
                killed(&native, &file, disk, second, "open"),
                "{disk}: recovery killed at {second}"
            );
            let mut k = kernel(&native);
            k.set_publish_hooks(child_disk(disk));
            let (ticket, recovery) = prepare(&mut k, "P", SessionSource::File(file.clone()));
            assert!(
                matches!(recovery, RecoveryResult::Recovered { truncated: false }),
                "{disk} {second}: {recovery:?}"
            );
            assert_eq!(fs::read(&file).unwrap(), original, "{disk} {second}");
            assert_eq!(
                private_leftovers(&file),
                [""; 0],
                "{disk} {second}: nothing is stranded under a private name"
            );
            let kept = visible_siblings(&file);
            assert_eq!(kept.len(), 1, "{disk} {second}: {kept:?}");
            assert_eq!(
                saved_name(&kept[0]).as_deref(),
                Some("One"),
                "{disk} {second}"
            );
            decide(&mut k, ticket, RecoveryDecision::Recover);
        }
    }
}
/// A save is cut short in the middle of copying its file to the name, and
/// the recovery that takes that incomplete copy back is itself cut short
/// right after it moved the copy off the name. The name it was moved to is
/// one the save's record names, written there before the move: the next
/// open finds it, judges it and finishes. Nothing is left under a private
/// name no record knows.
#[test]
fn a_second_crash_while_an_incomplete_copy_is_taken_back_leaves_nothing_unrecorded() {
    let dir = common::TempDir::new("withdraw-crash");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let native = dir.path("native");
    assert!(killed(&native, &file, "no-exclusive", "mid-copy", "save"));
    assert!(killed(&native, &file, "no-exclusive", "unlink", "open"));
    let mut k = kernel(&native);
    k.set_publish_hooks(child_disk("no-exclusive"));
    let _ = open_file(&mut k, &file);
    assert_eq!(fs::read(&file).unwrap(), original);
    assert_eq!(private_leftovers(&file), [""; 0]);
    let kept = visible_siblings(&file);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(saved_name(&kept[0]).as_deref(), Some("One"));
}
/// Another program puts a file of its own under the project's name at the
/// instant recovery moves the incomplete copy off it. What recovery then
/// holds is not the copy: it is never removed. It is kept beside the
/// project and told, also when the app is killed right then.
#[test]
fn a_file_that_lands_as_an_incomplete_copy_is_taken_back_is_kept_and_told() {
    for crash in [false, true] {
        let dir = common::TempDir::new("withdraw-landing");
        fs::create_dir_all(dir.path("job")).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let original = fs::read(&file).unwrap();
        let native = dir.path("native");
        assert!(killed(&native, &file, "no-exclusive", "mid-copy", "save"));
        if crash {
            // Killed with the landed file already moved to the record's
            // name and nothing judged yet.
            assert!(killed_landing(&native, &file, "no-exclusive", "withdraw"));
        }
        let mut k = kernel(&native);
        let project = file.clone();
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            step: (!crash).then(|| -> pm_kernel::project_io::StepHook {
                std::sync::Arc::new(move |step, _| {
                    if step == "withdraw" {
                        land(&project, b"version B");
                    }
                })
            }),
            ..child_disk("no-exclusive")
        });
        let (session, _) = open_file(&mut k, &file);
        assert_eq!(fs::read(&file).unwrap(), original, "{crash}");
        let landed: Vec<_> = visible_siblings(&file)
            .into_iter()
            .filter(|p| fs::read(p).unwrap() == b"version B")
            .collect();
        assert_eq!(landed.len(), 1, "{crash}: kept beside the project");
        let told = kept_beside(&mut k, &session);
        assert!(
            told.iter()
                .any(|(_, path)| path.as_ref() == Some(&landed[0])),
            "{crash}: and told: {told:?}"
        );
        assert_eq!(private_leftovers(&file), [""; 0], "{crash}");
    }
}
/// Puts `bytes` at `path` the way a sync client lands a version: written
/// beside it, then renamed over it.
fn land(path: &Path, bytes: &[u8]) {
    let staged = path.with_extension("landing");
    fs::write(&staged, bytes).unwrap();
    fs::rename(&staged, path).unwrap();
}
fn visible_siblings(file: &Path) -> Vec<std::path::PathBuf> {
    let mut found: Vec<_> = fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p != file && p.extension().is_some_and(|e| e == "pmv"))
        .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .collect();
    found.sort();
    found
}
/// Another version lands at the project's path after the save's last check
/// and before the exchange. The exchange hands it back; it is not the
/// version the save could replace. It is kept beside the project under a
/// plain name the save reports, the save itself stands, and the session
/// carries on saving normally.
#[test]
fn a_version_that_arrives_during_a_save_is_kept_beside_the_project() {
    let dir = common::TempDir::new("exchange-b");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
        ..Default::default()
    });
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let ShellReply::Saved {
        displaced: Some(kept),
        ..
    } = k
        .finish_session_save(plan.run())
        .expect("the save is published")
    else {
        panic!("the save says another version was kept")
    };
    assert_eq!(saved_name(&file).as_deref(), Some("One"));
    assert_eq!(fs::read(&kept).unwrap(), b"version B");
    assert_eq!(
        visible_siblings(&file),
        std::slice::from_ref(&kept),
        "beside the project"
    );
    assert!(
        kept.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cut (other version "),
        "{kept:?}"
    );
    // Nothing is stuck: the next save is an ordinary one.
    k.set_publish_hooks(Default::default());
    rename_comp(&mut k, "P", "Two");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    assert!(matches!(
        k.finish_session_save(plan.run()).unwrap(),
        ShellReply::Saved {
            displaced: None,
            ..
        }
    ));
    assert_eq!(saved_name(&file).as_deref(), Some("Two"));
    assert_eq!(fs::read(&kept).unwrap(), b"version B");
}
/// Version B lands before the exchange and version C right after it. The
/// save never puts B back: nothing can put a file back only if the path
/// still holds ours, and a put-back that lost that race took C, unread, from
/// the path it arrived at. C stays where it landed, B is kept beside it
/// under the name the failed save reports, and the work is still open.
#[test]
fn a_second_version_arriving_after_the_exchange_is_never_displaced() {
    let dir = common::TempDir::new("exchange-b-c");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
        after_exchange: Some(std::sync::Arc::new(|path| land(path, b"version C"))),
        ..Default::default()
    });
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let failed = k
        .finish_session_save(plan.run())
        .expect_err("our file is no longer at the path");
    assert_eq!(
        fs::read(&file).unwrap(),
        b"version C",
        "C is where it landed"
    );
    let kept = visible_siblings(&file);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(fs::read(&kept[0]).unwrap(), b"version B");
    assert!(
        failed.to_string().contains(&*kept[0].to_string_lossy()),
        "the failure names where B is: {failed}"
    );
    // The work is still open and Save As writes it.
    k.set_publish_hooks(Default::default());
    let rescued = dir.path("job/mine.pmv");
    let plan = k
        .prepare_session_save(session.clone(), Some(rescued.clone()), false)
        .unwrap();
    k.finish_session_save(plan.run()).unwrap();
    assert_eq!(saved_name(&rescued).as_deref(), Some("One"));
    assert_eq!(fs::read(&file).unwrap(), b"version C");
}
/// The file the exchange hands back cannot be read (a placeholder a sync
/// client has not downloaded, a failing disk): it cannot be judged, so it is
/// kept, beside the project under a plain name the save reports, never
/// under a hidden one with the save reported as failed.
#[test]
fn a_displaced_file_that_cannot_be_read_is_kept_beside_the_project() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::TempDir::new("exchange-unreadable");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        before_exchange: Some(std::sync::Arc::new(|path| {
            let staged = path.with_extension("landing");
            fs::write(&staged, b"cannot be read").unwrap();
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o000)).unwrap();
            fs::rename(&staged, path).unwrap();
        })),
        ..Default::default()
    });
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let ShellReply::Saved {
        displaced: Some(kept),
        ..
    } = k
        .finish_session_save(plan.run())
        .expect("the save is published")
    else {
        panic!("the save says another version was kept")
    };
    assert_eq!(saved_name(&file).as_deref(), Some("One"));
    assert_eq!(visible_siblings(&file), std::slice::from_ref(&kept));
    let meta = fs::metadata(&kept).unwrap();
    assert_eq!(
        (meta.len(), meta.permissions().mode() & 0o777),
        (14, 0),
        "the file itself, as it arrived"
    );
    fs::set_permissions(&kept, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(&kept).unwrap(), b"cannot be read");
}
/// A disk with neither an exclusive rename nor hard links (exFAT) had no way
/// at all to create a project file: the first Save and every Save As failed
/// with "Operation not supported", and with Save refused there too the work
/// could not be written to that disk by any route. A new file is created
/// there by exclusive creation of its name; a name that is taken is never
/// written over unasked.
#[test]
fn a_new_file_is_created_on_a_disk_without_exclusive_rename_or_links() {
    let dir = common::TempDir::new("no-exclusive");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let mut k = kernel(&dir.path("native"));
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        no_exchange: true,
        no_exclusive: true,
        ..Default::default()
    });
    let (session, _) = open_file(&mut k, &file);
    let media = k.project("P").unwrap().store().media().len();
    assert!(media > 0);
    rename_comp(&mut k, "P", "One");
    // A name that is taken is not written over unless the person said so.
    let taken = dir.path("job/other.pmv");
    fs::write(&taken, &original).unwrap();
    let plan = k
        .prepare_session_save(session.clone(), Some(taken.clone()), false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect_err("a file that is there is never written over unasked");
    assert_eq!(fs::read(&taken).unwrap(), original);
    let created = dir.path("job/cut 2.pmv");
    let plan = k
        .prepare_session_save(session.clone(), Some(created.clone()), false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("Save As creates a new file");
    assert_eq!(saved_name(&created).as_deref(), Some("One"));
    assert_eq!(
        open_project(&created, OpenOptions::default())
            .unwrap()
            .file
            .media
            .len(),
        media,
        "with all its media"
    );
    let left: Vec<_> = fs::read_dir(dir.path("job"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(left.is_empty(), "no staging file is left: {left:?}");
}
/// Saves on real volumes, by the app's save route: `PM_TEST_VOLUMES` names
/// folders (one per volume, separated by `:`), for example the mount points
/// of scratch disk images of each filesystem and of a network share. On
/// each: Save writes the file in place twice over; Save As creates a new
/// file; a version that lands before the file is taken, between the two
/// renames, and at the instant of the install is kept; a save killed at
/// each step its record has is settled by the next open; and so is a
/// recovery that is itself killed. One line per volume says what each case
/// did there (`n/a`: that volume's route has no such moment).
#[test]
#[ignore = "needs PM_TEST_VOLUMES"]
fn saves_on_real_volumes_replace_the_file_in_place() {
    let volumes = std::env::var("PM_TEST_VOLUMES").expect("PM_TEST_VOLUMES");
    for volume in volumes.split(':').filter(|v| !v.is_empty()) {
        let started = std::time::Instant::now();
        let dir = common::TempDir::new("volume");
        let folder = Path::new(volume).join(format!("pm-save-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let mut cells: Vec<String> = Vec::new();
        let file = folder.join("cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, decision) = open_file(&mut k, &file);
        assert_eq!(decision, OpenDecision::Writable, "{volume}");
        // A volume other machines write keeps its lock beside the file, and
        // a dead owner's lock is believed there for 30 seconds.
        let shared = project_lock_path(&file).exists();
        let save = |k: &mut Kernel, to: Option<&Path>| {
            let plan = k
                .prepare_session_save(session.clone(), to.map(Into::into), false)
                .unwrap();
            k.finish_session_save(plan.run())
        };
        for name in ["One", "Two"] {
            rename_comp(&mut k, "P", name);
            save(&mut k, None).unwrap_or_else(|e| panic!("{volume}: Save in place: {e}"));
            assert_eq!(saved_name(&file).as_deref(), Some(name), "{volume}");
        }
        cells.push("save in place x2: ok".into());
        let rescued = folder.join("cut 2.pmv");
        save(&mut k, Some(&rescued)).unwrap_or_else(|e| panic!("{volume}: Save As: {e}"));
        assert_eq!(saved_name(&rescued).as_deref(), Some("Two"), "{volume}");
        cells.push("save as: ok".into());
        // Save As moved the session: the landings are on the file it has now.
        rename_comp(&mut k, "P", "Three");
        k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
            before_exchange: Some(std::sync::Arc::new(|path| land(path, b"version B"))),
            ..Default::default()
        });
        let ShellReply::Saved {
            displaced: Some(kept),
            ..
        } = save(&mut k, None)
            .unwrap_or_else(|e| panic!("{volume}: a save with a version landing: {e}"))
        else {
            panic!("{volume}: the landed version is reported")
        };
        assert_eq!(saved_name(&rescued).as_deref(), Some("Three"), "{volume}");
        assert_eq!(fs::read(&kept).unwrap(), b"version B", "{volume}");
        cells.push("lands before the move: kept, save stands".into());
        // Between the renames, and at the instant of the install: the
        // version that landed stays, this save's file is kept beside it.
        for (when, bytes) in [
            ("between the renames", "version C"),
            ("at the install", "version D"),
        ] {
            // The session carries on from whatever the name holds now.
            drop(k);
            let fresh = folder.join(format!("landing {when}.pmv"));
            fs::copy(common::ts_fixture("fresh.pmv"), &fresh).unwrap();
            k = kernel(&dir.path(&format!("native {when}")));
            let (landing, _) = open_file(&mut k, &fresh);
            rename_comp(&mut k, "P", "Four");
            let project = fresh.clone();
            let (hooks, landed) = if when == "between the renames" {
                let landed = std::sync::Arc::new(std::sync::Mutex::new(None));
                let seen = landed.clone();
                (
                    pm_kernel::project_io::PublishHooks {
                        between_renames: Some(std::sync::Arc::new(move |path: &Path| {
                            land(path, bytes.as_bytes());
                            *seen.lock().unwrap() = Some(path.to_owned());
                        })),
                        ..Default::default()
                    },
                    landed,
                )
            } else {
                lands_at_the_install(
                    Default::default(),
                    move |to| to == project,
                    bytes.as_bytes().to_vec(),
                )
            };
            k.set_publish_hooks(hooks);
            let plan = k.prepare_session_save(landing, None, false).unwrap();
            let outcome = k.finish_session_save(plan.run());
            cells.push(if landed.lock().unwrap().is_none() {
                outcome.unwrap_or_else(|e| panic!("{volume}: {when}: {e}"));
                assert_eq!(saved_name(&fresh).as_deref(), Some("Four"), "{volume}");
                format!("lands {when}: n/a")
            } else {
                let e = outcome.expect_err("the name was taken");
                assert_eq!(
                    fs::read(&fresh).unwrap(),
                    bytes.as_bytes(),
                    "{volume} {when}"
                );
                let ours = pm_kernel::project_io::kept_beside(&e.to_string())
                    .unwrap_or_else(|| panic!("{volume} {when}: {e}"));
                assert_eq!(
                    saved_name(&ours).as_deref(),
                    Some("Four"),
                    "{volume} {when}"
                );
                format!("lands {when}: it stays, ours kept beside")
            });
        }
        drop(k);
        // Installed and confirmed; while the version it replaced is judged
        // and retained, another valid version lands at the name: it is
        // never taken for this save, ours is kept beside it.
        {
            let late = folder.join("late arrival.pmv");
            fs::copy(common::ts_fixture("fresh.pmv"), &late).unwrap();
            let mut k = kernel(&dir.path("native late"));
            let (session, _) = open_file(&mut k, &late);
            rename_comp(&mut k, "P", "Five");
            let other = fs::read(common::ts_fixture("save-as.pmv")).unwrap();
            let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (landing, project) = (other.clone(), late.clone());
            k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
                after_judged: Some(std::sync::Arc::new(move |_| {
                    if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        land(&project, &landing);
                    }
                })),
                ..Default::default()
            });
            let plan = k.prepare_session_save(session, None, false).unwrap();
            let e = k
                .finish_session_save(plan.run())
                .expect_err("another version holds the name");
            assert_eq!(fs::read(&late).unwrap(), other, "{volume}");
            let ours = pm_kernel::project_io::kept_beside(&e.to_string())
                .unwrap_or_else(|| panic!("{volume}: {e}"));
            assert_eq!(saved_name(&ours).as_deref(), Some("Five"), "{volume}");
            cells.push(
                "lands while the replaced one is retained: it stays, ours kept beside".into(),
            );
        }
        // The folder fails to sync once right after the install: the next
        // Save in place goes through.
        {
            let once = folder.join("sync once.pmv");
            fs::copy(common::ts_fixture("fresh.pmv"), &once).unwrap();
            let mut k = kernel(&dir.path("native sync once"));
            let (session, _) = open_file(&mut k, &once);
            rename_comp(&mut k, "P", "Six");
            let calls = std::sync::atomic::AtomicUsize::new(0);
            k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
                folder_sync: Some(std::sync::Arc::new(move || {
                    match calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                        1 => (Some(5), None),
                        _ => (None, None),
                    }
                })),
                ..Default::default()
            });
            let plan = k
                .prepare_session_save(session.clone(), None, false)
                .unwrap();
            k.finish_session_save(plan.run())
                .expect_err("the folder did not sync");
            k.set_publish_hooks(Default::default());
            rename_comp(&mut k, "P", "Seven");
            let plan = k
                .prepare_session_save(session, None, false)
                .unwrap_or_else(|e| panic!("{volume}: {e}"));
            k.finish_session_save(plan.run())
                .unwrap_or_else(|e| panic!("{volume}: the next save: {e}"));
            assert_eq!(saved_name(&once).as_deref(), Some("Seven"), "{volume}");
            cells.push("folder sync fails once: next save in place ok".into());
        }
        // A save killed at each step of its record. All the kills first:
        // on a share one wait then covers every dead owner's lock.
        let wait = || {
            if shared {
                std::thread::sleep(Duration::from_secs(31));
            }
        };
        let case = |name: String| {
            let crashed = folder.join(format!("{name}.pmv"));
            fs::copy(common::ts_fixture("fresh.pmv"), &crashed).unwrap();
            let original = fs::read(&crashed).unwrap();
            (crashed, dir.path(&format!("native-{name}")), original)
        };
        let mut killed_at = Vec::new();
        for step in [
            "begin",
            "aside",
            "install",
            "mid-copy",
            "exchanged",
            "installed",
            "closed",
        ] {
            let (crashed, native, original) = case(format!("killed {step}"));
            if killed(&native, &crashed, "real", step, "save") {
                killed_at.push((step, crashed, native, original));
            } else {
                cells.push(format!("killed at {step}: n/a"));
            }
        }
        // Killed mid-copy, and then the incomplete file at the name is
        // changed (a sync client): it is never taken back, and the open
        // offers the versions the record proves.
        let changed = killed_at
            .iter()
            .any(|(step, ..)| *step == "mid-copy")
            .then(|| {
                let (crashed, native, _) = case("changed mid-copy".into());
                assert!(killed(&native, &crashed, "real", "mid-copy", "save"));
                let mut stray = fs::read(&crashed).unwrap();
                stray[10] ^= 0xff;
                land(&crashed, &stray);
                (crashed, native, stray)
            });
        // And a recovery that is itself killed, after a save killed between
        // its renames (or, where a volume exchanges, right after that).
        let first = if killed_at.iter().any(|(step, ..)| *step == "aside") {
            "between"
        } else {
            "exchanged"
        };
        let mut twice = Vec::new();
        for second in ["restore", "restored", "keep", "mid-copy", "closed"] {
            let (crashed, native, original) = case(format!("twice {second}"));
            assert!(killed(&native, &crashed, "real", first, "save"), "{volume}");
            twice.push((second, crashed, native, original));
        }
        wait();
        // What the next open finds and does: a whole version under the name,
        // the work there or offered, nothing under a private name, and Save.
        let reopen = |crashed: &Path, native: &Path, original: &[u8], what: &str| {
            let mut k = kernel(native);
            let opened = k
                .shell(ShellCall::PrepareOpen {
                    id: "P".into(),
                    source: SessionSource::File(crashed.to_owned()),
                    electron_profile: None,
                })
                .unwrap_or_else(|e| panic!("{volume}: {what}: {e}"));
            let session = match opened {
                ShellReply::RecoveryRequired { ticket, .. } => {
                    assert_eq!(fs::read(crashed).unwrap(), original, "{volume}: {what}");
                    decide(&mut k, ticket, RecoveryDecision::Recover)
                }
                ShellReply::Opened { session, .. } => {
                    assert_eq!(
                        saved_name(crashed).as_deref(),
                        Some("One"),
                        "{volume}: {what}"
                    );
                    session
                }
                other => panic!("{volume}: {what}: {other:?}"),
            };
            let name = k.project("P").unwrap().store().root().unwrap().get("name");
            assert_eq!(
                name.map(|n| n.to_value()).as_ref().and_then(Value::as_str),
                Some("One"),
                "{volume}: {what}: the work being saved is there"
            );
            assert_eq!(private_leftovers(crashed), [""; 0], "{volume}: {what}");
            let plan = k
                .prepare_session_save(session.clone(), None, false)
                .unwrap();
            k.finish_session_save(plan.run())
                .unwrap_or_else(|e| panic!("{volume}: {what}: the next save: {e}"));
            assert_eq!(
                saved_name(crashed).as_deref(),
                Some("One"),
                "{volume}: {what}"
            );
            assert_eq!(private_leftovers(crashed), [""; 0], "{volume}: {what}");
            k.shell(ShellCall::Close(session)).unwrap();
        };
        for (step, crashed, native, original) in &killed_at {
            reopen(crashed, native, original, &format!("killed at {step}"));
            cells.push(format!("killed at {step}: reopened ok"));
        }
        match &changed {
            None => cells.push("changed partial copy: n/a".into()),
            Some((crashed, native, stray)) => {
                let mut k = kernel(native);
                k.shell(ShellCall::PrepareOpen {
                    id: "P".into(),
                    source: SessionSource::File(crashed.clone()),
                    electron_profile: None,
                })
                .expect_err("not a project at the name");
                let ShellReply::Leftovers(rows) =
                    k.shell(ShellCall::Leftovers(crashed.clone())).unwrap()
                else {
                    panic!()
                };
                let saving = rows
                    .iter()
                    .find(|r| r.get("role").and_then(Value::as_str) == Some("saving"))
                    .and_then(|r| r.get("path").and_then(Value::as_str))
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| panic!("{volume}: {rows:?}"));
                k.shell(ShellCall::UseLeftover {
                    path: crashed.clone(),
                    leftover: saving,
                    unproven: false,
                })
                .unwrap_or_else(|e| panic!("{volume}: {e}"));
                assert_eq!(saved_name(crashed).as_deref(), Some("One"), "{volume}");
                assert!(
                    visible_siblings(crashed)
                        .iter()
                        .any(|p| fs::read(p).is_ok_and(|b| b == *stray)),
                    "{volume}: the changed file is kept"
                );
                cells.push("changed partial copy: offered, chosen, changed file kept".into());
            }
        }
        let mut recovering = Vec::new();
        for (second, crashed, native, original) in twice {
            let reached = killed(&native, &crashed, "real", second, "open");
            recovering.push((second, reached, crashed, native, original));
        }
        wait();
        for (second, reached, crashed, native, original) in &recovering {
            let what = format!("killed at {first}, recovery killed at {second}");
            reopen(crashed, native, original, &what);
            cells.push(if *reached {
                format!("recovery killed at {second}: reopened ok")
            } else {
                format!("recovery killed at {second}: n/a")
            });
        }
        eprintln!(
            "VOLUME {volume} ({} s): {}",
            started.elapsed().as_secs(),
            cells.join("; ")
        );
        let _ = fs::remove_dir_all(&folder);
    }
}
/// Save follows a file the person renamed or moved, but only to a place
/// people keep documents. Moved into an application bundle, or into a
/// hidden folder, the file is not written there on the quiet: Save does not
/// happen (the sheet with Save As), the moved file keeps its bytes, and
/// Save As writes the work where the person chooses. An ordinary folder is
/// still followed, dots in its name or not.
#[test]
fn a_file_moved_into_a_bundle_or_hidden_folder_is_not_saved_into_silently() {
    for place in ["Tool.app/Contents/Resources", ".cache/items"] {
        let dir = common::TempDir::new("follow-boundary");
        fs::create_dir_all(dir.path("job")).unwrap();
        fs::create_dir_all(dir.path(place)).unwrap();
        let file = dir.path("job/cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, _) = open_file(&mut k, &file);
        let media = k.project("P").unwrap().store().media().len();
        let moved = dir.path(place).join("cut.pmv");
        fs::rename(&file, &moved).unwrap();
        let bytes = fs::read(&moved).unwrap();
        rename_comp(&mut k, "P", "One");
        let saved = k
            .prepare_session_save(session.clone(), None, false)
            .and_then(|plan| k.finish_session_save(plan.run()));
        assert!(saved.is_err(), "{place}: not followed there");
        assert_eq!(fs::read(&moved).unwrap(), bytes, "{place}: left as it was");
        let chosen = dir.path("job/cut 2.pmv");
        let plan = k
            .prepare_session_save(session.clone(), Some(chosen.clone()), false)
            .unwrap();
        k.finish_session_save(plan.run())
            .expect("Save As writes where the person chooses");
        assert_eq!(saved_name(&chosen).as_deref(), Some("One"));
        assert_eq!(
            open_project(&chosen, OpenOptions::default())
                .unwrap()
                .file
                .media
                .len(),
            media
        );
    }
    // An ordinary folder with a dot in its name is a place like any other.
    let dir = common::TempDir::new("follow-dotted");
    fs::create_dir_all(dir.path("job")).unwrap();
    fs::create_dir_all(dir.path("2026.11 final v1.2")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, _) = open_file(&mut k, &file);
    let moved = dir.path("2026.11 final v1.2/cut.pmv");
    fs::rename(&file, &moved).unwrap();
    rename_comp(&mut k, "P", "One");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("an ordinary move is followed");
    assert_eq!(saved_name(&moved).as_deref(), Some("One"));
}
/// The open file (with embedded media) is renamed, a save is prepared, and
/// the file is renamed again before the save's worker runs. That save fails
/// (the name it was prepared for is gone). The proof that the held file
/// still has its bytes must read it from the start each time: read from
/// wherever the last proof left the descriptor, it hashed nothing, the
/// unchanged file counted as changed, and neither Save nor Save As could
/// write its media again.
#[test]
fn a_file_renamed_again_before_its_save_runs_can_still_be_saved() {
    let dir = common::TempDir::new("renamed-twice");
    fs::create_dir_all(dir.path("job")).unwrap();
    let first = dir.path("job/a.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &first).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, _) = open_file(&mut k, &first);
    let media = k.project("P").unwrap().store().media().len();
    assert!(media > 0);
    let second = dir.path("job/b.pmv");
    fs::rename(&first, &second).unwrap();
    rename_comp(&mut k, "P", "One");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let third = dir.path("job/c.pmv");
    fs::rename(&second, &third).unwrap();
    k.finish_session_save(plan.run())
        .expect_err("the name the save was prepared for is gone");
    // Save As writes every embedded item from the held file.
    let rescued = dir.path("job/rescued.pmv");
    let plan = k
        .prepare_session_save(session.clone(), Some(rescued.clone()), false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("Save As works from the held file");
    assert_eq!(saved_name(&rescued).as_deref(), Some("One"));
    assert_eq!(
        open_project(&rescued, OpenOptions::default())
            .unwrap()
            .file
            .media
            .len(),
        media
    );
}
/// The same two renames, then Save again: it follows the file to the name
/// it has now.
#[test]
fn a_save_follows_a_file_renamed_again_before_the_save_ran() {
    let dir = common::TempDir::new("renamed-twice-follow");
    fs::create_dir_all(dir.path("job")).unwrap();
    let first = dir.path("job/a.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &first).unwrap();
    let mut k = kernel(&dir.path("native"));
    let (session, _) = open_file(&mut k, &first);
    let media = k.project("P").unwrap().store().media().len();
    let second = dir.path("job/b.pmv");
    fs::rename(&first, &second).unwrap();
    rename_comp(&mut k, "P", "One");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let third = dir.path("job/c.pmv");
    fs::rename(&second, &third).unwrap();
    k.finish_session_save(plan.run())
        .expect_err("the name the save was prepared for is gone");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("the next save follows the file to its new name");
    assert_eq!(saved_name(&third).as_deref(), Some("One"));
    assert_eq!(
        open_project(&third, OpenOptions::default())
            .unwrap()
            .file
            .media
            .len(),
        media
    );
    assert!(!first.exists() && !second.exists());
}
/// Unsaved work is found by the file, not by its path: the file is renamed
/// while it has unsaved changes and the app ends without a save; opening the
/// renamed file (a different session id, as its path gives) recovers them.
#[test]
fn a_renamed_file_is_offered_its_unsaved_work() {
    let dir = common::TempDir::new("recover-renamed");
    let file = dir.path("cut.pmv");
    fs::write(&file, PROJECT).unwrap();
    let native = dir.path("native");
    let mut k = kernel(&native);
    let (session, _) = open_file(&mut k, &file);
    // Saved once, so the file is the app's own container and has an entry.
    k.shell(ShellCall::Save(session)).unwrap();
    edit(&mut k, "P", 10.0);
    // The app ends with the edit only in the journal.
    drop(k);
    let renamed = dir.path("cut final.pmv");
    fs::rename(&file, &renamed).unwrap();
    let mut k = kernel(&native);
    let ShellReply::Opened { recovery, .. } = k
        .shell(ShellCall::Open {
            id: "Q".into(),
            source: SessionSource::File(renamed.clone()),
            electron_profile: None,
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(
        matches!(recovery, RecoveryResult::Recovered { .. }),
        "{recovery:?}"
    );
    assert_eq!(
        common::opacity_of(k.project("Q").unwrap().store(), "layer0"),
        Some(10.0)
    );
}
/// Every file under `dir`, with its bytes.
fn tree(dir: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut found = std::collections::BTreeMap::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else {
                found.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    found
}
/// A project file saved by the app at `file`, closed.
fn saved_file(profile: &Path, file: &Path) {
    fs::write(file, PROJECT).unwrap();
    let mut k = kernel(profile);
    let h = open(&mut k, "seed", SessionSource::File(file.into()));
    edit(&mut k, "seed", 90.0);
    k.shell(ShellCall::Save(h.clone())).unwrap();
    k.shell(ShellCall::Close(h)).unwrap();
}
fn legacy_open(k: &mut Kernel, id: &str, file: &Path) -> (SessionHandle, RecoveryResult) {
    match k
        .shell(ShellCall::Open {
            id: id.into(),
            source: SessionSource::File(file.into()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened {
            session, recovery, ..
        } => (session, recovery),
        other => panic!("{other:?}"),
    }
}
/// Equal bytes do not make two files one. A and its duplicate B each have
/// unsaved work of their own; A is deleted; C is another duplicate of what
/// A had saved. B recovers B's work. C is never handed A's: an open that
/// asks nobody recovers nothing and moves nothing, an open that can ask
/// offers it as work from A's name, declining leaves A's journal byte for
/// byte, and only Recover opens it in C.
#[test]
fn unsaved_work_is_never_handed_to_an_identical_duplicate_unasked() {
    let dir = common::TempDir::new("journal-identity");
    let native = dir.path("native");
    let (a, b, c) = (dir.path("a.pmv"), dir.path("b.pmv"), dir.path("c.pmv"));
    saved_file(&native, &a);
    fs::copy(&a, &b).unwrap();
    fs::copy(&a, &c).unwrap();
    crash_with_edit(&native, "A", &a, 20.0);
    crash_with_edit(&native, "B", &b, 30.0);
    fs::remove_file(&a).unwrap();
    let journal = native.join("journals/A");
    let left = tree(&journal);
    assert!(!left.is_empty());
    let mut k = kernel(&native);
    // B has its own.
    let (ticket, offer) = prepare(&mut k, "B", SessionSource::File(b.clone()));
    assert!(
        matches!(offer, RecoveryResult::Recovered { .. }),
        "{offer:?}"
    );
    let session = decide(&mut k, ticket, RecoveryDecision::Recover);
    assert_eq!(
        common::opacity_of(k.project("B").unwrap().store(), "layer0"),
        Some(30.0)
    );
    k.shell(ShellCall::DeferClose(session)).unwrap();
    // Nobody to ask: nothing recovered, nothing moved.
    let (session, recovery) = legacy_open(&mut k, "C", &c);
    assert_eq!(recovery, RecoveryResult::None);
    assert_eq!(
        common::opacity_of(k.project("C").unwrap().store(), "layer0"),
        Some(90.0),
        "the duplicate opens as it was saved"
    );
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(
        tree(&journal),
        left,
        "the other file's journal is untouched"
    );
    // Asked: offered under the other file's name; declined, it stays.
    let (ticket, offer) = prepare(&mut k, "C", SessionSource::File(c.clone()));
    assert_eq!(
        offer,
        RecoveryResult::FromOtherFile {
            name: "a.pmv".into(),
            truncated: false
        }
    );
    assert_eq!(tree(&journal), left, "offering consumes nothing");
    let session = decide(&mut k, ticket, RecoveryDecision::Discard);
    assert_eq!(
        common::opacity_of(k.project("C").unwrap().store(), "layer0"),
        Some(90.0)
    );
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(tree(&journal), left, "declining leaves it where it is");
    // Accepted: the work opens here.
    let (ticket, offer) = prepare(&mut k, "C", SessionSource::File(c.clone()));
    assert!(matches!(offer, RecoveryResult::FromOtherFile { .. }));
    decide(&mut k, ticket, RecoveryDecision::Recover);
    assert_eq!(
        common::opacity_of(k.project("C").unwrap().store(), "layer0"),
        Some(20.0)
    );
}
/// A file with unsaved work is deleted and restored from a backup under
/// another name: the same bytes in a new file. Its unsaved work is offered
/// as work from the old name; accepted, it opens, saves into the restored
/// file, and only then is the old journal retired.
#[test]
fn a_file_restored_from_a_backup_is_offered_its_unsaved_work() {
    let dir = common::TempDir::new("journal-restore");
    let native = dir.path("native");
    let file = dir.path("cut.pmv");
    saved_file(&native, &file);
    let backup = fs::read(&file).unwrap();
    crash_with_edit(&native, "A", &file, 20.0);
    fs::remove_file(&file).unwrap();
    let restored = dir.path("cut restored.pmv");
    fs::write(&restored, &backup).unwrap();
    let journal = native.join("journals/A");
    let left = tree(&journal);
    let mut k = kernel(&native);
    let (ticket, offer) = prepare(&mut k, "R", SessionSource::File(restored.clone()));
    assert_eq!(
        offer,
        RecoveryResult::FromOtherFile {
            name: "cut.pmv".into(),
            truncated: false
        }
    );
    assert_eq!(tree(&journal), left);
    let session = decide(&mut k, ticket, RecoveryDecision::Recover);
    assert_eq!(
        common::opacity_of(k.project("R").unwrap().store(), "layer0"),
        Some(20.0)
    );
    k.shell(ShellCall::Save(session.clone())).unwrap();
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(
        common::opacity_of(&common::open_store(&restored), "layer0"),
        Some(20.0)
    );
    assert_eq!(live_sessions(&native, "A"), 0, "taken once it was chosen");
}
/// A journal left by a project that is not open and cannot be folded into
/// the library (here its library slot is unreadable) never fails a quit: the
/// quit's compaction succeeds, the library is released, and the journal is
/// kept as it is for the next open of that project.
#[test]
fn a_leftover_journal_that_cannot_be_folded_never_fails_a_quit() {
    fn find(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
        fs::read_dir(dir).ok()?.flatten().find_map(|e| {
            let path = e.path();
            if path.is_dir() {
                find(&path, name)
            } else {
                (e.file_name().to_string_lossy().starts_with(name)).then_some(path)
            }
        })
    }
    let dir = common::TempDir::new("quit-leftover");
    let native = dir.path("native");
    let mut k = kernel(&native);
    let h = open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 40.0);
    // The app ends without closing it: its journal holds the edit.
    drop(h);
    drop(k);
    let journal = native.join("journals/P");
    let before = pm_kernel::journal::sessions(&journal);
    assert!(!before.is_empty());
    // Its slot can no longer be read.
    let slot = find(&native, "project.P.").expect("the project's slot");
    fs::write(&slot, b"{ not json").unwrap();
    let mut k = kernel(&native);
    let reply = k
        .shell(ShellCall::PrepareQuit)
        .expect("a quit is not refused for a project that is not open");
    assert!(matches!(reply, ShellReply::Compacted { .. }));
    k.shell(ShellCall::ReleaseLibrary).unwrap();
    assert_eq!(
        pm_kernel::journal::sessions(&journal),
        before,
        "the journal is kept for the next open"
    );
}
/// A closed project's slot reads fine, but the journal that holds its
/// unsaved work cannot be replayed by this build (here: written by a newer
/// one). A quit does not open that project: opening it would have ignored
/// the journal, saved the older slot and removed the journal as "saved".
/// Slot and journal are byte for byte what they were, for the next open to
/// put to the person.
#[test]
fn a_quit_leaves_a_journal_it_cannot_recover_exactly_as_it_is() {
    let dir = common::TempDir::new("quit-unrecoverable");
    let native = dir.path("native");
    let mut k = kernel(&native);
    open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 40.0);
    k.sync_all();
    // The app ends without closing it: its journal holds the edit.
    drop(k);
    let journal = native.join("journals/P");
    let session = pm_kernel::journal::sessions(&journal).pop().unwrap();
    let segment = session.join("journal-00000000.log");
    let text = fs::read_to_string(&segment).unwrap();
    let (first, rest) = text.split_once('\n').unwrap();
    let mut head = parse(first.split_once(' ').unwrap().1).unwrap();
    head.as_object_mut().unwrap().insert(
        "version",
        Value::Number(f64::from(pm_kernel::journal::FORMAT + 1)),
    );
    let head = to_canonical_string(&head);
    let sum = &pm_model::hash::sha256_hex(head.as_bytes())[..16];
    fs::write(&segment, format!("{sum} {head}\n{rest}")).unwrap();
    let slot = native.join("store/project.P.json");
    let stored = fs::read(&slot).expect("the project's slot");
    let before = tree(&journal);

    let mut k = kernel(&native);
    assert!(matches!(
        k.shell(ShellCall::PrepareQuit).unwrap(),
        ShellReply::Compacted { .. }
    ));
    k.shell(ShellCall::ReleaseLibrary).unwrap();
    drop(k);
    assert_eq!(tree(&journal), before, "the journal's bytes are untouched");
    assert_eq!(
        fs::read(&slot).unwrap(),
        stored,
        "the slot is not rewritten"
    );
    // The next open is told, and still nothing is thrown away.
    let mut k = kernel(&native);
    let (_, offer) = prepare(
        &mut k,
        "P",
        SessionSource::Library {
            profile: native.clone(),
            project: "P".into(),
        },
    );
    assert!(
        matches!(offer, RecoveryResult::Unrecoverable { .. }),
        "{offer:?}"
    );
    assert_eq!(tree(&journal), before);
}
#[test]
fn a_crashed_owner_is_taken_over_at_once_and_a_live_one_never() {
    let dir = common::TempDir::new("stale");
    let path = dir.path("p.pmvlock");
    let dead = 2_000_000_000u32;
    assert!(!pid_alive(dead));
    // A live pid with a fresh heartbeat owns the lock (an unflocked writer).
    fs::write(
        &path,
        format!(
            "{{\"pid\":{},\"heartbeat\":{}}}",
            std::process::id(),
            now_ms()
        ),
    )
    .unwrap();
    assert_eq!(
        FileLock::acquire(&path).err().map(|e| e.kind()),
        Some(std::io::ErrorKind::WouldBlock)
    );
    // Killed a moment ago: the flock is free and the pid is gone. No wait.
    fs::write(
        &path,
        format!("{{\"pid\":{dead},\"heartbeat\":{}}}", now_ms()),
    )
    .unwrap();
    let lock = FileLock::acquire(&path).unwrap();
    let record = parse(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        record.get("pid"),
        Some(&Value::Number(std::process::id() as f64))
    );
    // Held: a second claim, even from this process, waits.
    assert!(FileLock::acquire(&path).is_err());
    drop(lock);
    // A reused pid whose heartbeat stopped 30 s ago is stale too.
    fs::write(
        &path,
        format!("{{\"pid\":{},\"heartbeat\":0}}", std::process::id()),
    )
    .unwrap();
    drop(FileLock::acquire(&path).unwrap());
    assert!(FileLock::acquire(&path).is_ok());
}
/// A lock record that was never written or was torn (a crash, a full disk
/// between create and write) does not refuse the project or the library for
/// good: on a local volume the flock proves no owner lives, so the open is
/// writable and the record is rewritten.
#[test]
fn an_empty_or_torn_lock_record_on_a_local_volume_does_not_refuse_the_open() {
    for torn in [&b""[..], b"{\"pid\":4"] {
        let dir = common::TempDir::new("torn-lock");
        let file = dir.path("cut.pmv");
        fs::write(&file, PROJECT).unwrap();
        fs::write(project_lock_path(&file), torn).unwrap();
        let native = dir.path("native");
        fs::create_dir_all(&native).unwrap();
        // The library's own lock, torn the same way.
        let mut k = kernel(&native);
        let h = open(&mut k, "seed", SessionSource::New(value()));
        k.shell(ShellCall::Close(h)).unwrap();
        drop(k);
        let library_lock = {
            fn find(dir: &Path) -> Option<std::path::PathBuf> {
                fs::read_dir(dir).ok()?.flatten().find_map(|e| {
                    let path = e.path();
                    if path.is_dir() {
                        find(&path)
                    } else {
                        (e.file_name() == "library.lock").then_some(path)
                    }
                })
            }
            find(&native).expect("the library lock")
        };
        fs::write(&library_lock, torn).unwrap();
        let mut k = kernel(&native);
        let (session, decision) = open_file(&mut k, &file);
        assert_eq!(decision, OpenDecision::Writable, "{torn:?}");
        edit(&mut k, "P", 30.0);
        k.shell(ShellCall::Save(session)).unwrap();
        assert_eq!(
            common::opacity_of(&common::open_store(&file), "layer0"),
            Some(30.0)
        );
    }
}
#[test]
fn torn_record_lock_recovers_without_stealing_a_live_flock() {
    // FileLock::acquire explicitly exercises record mode; lock_project's
    // local-user flock has no owner JSON and is covered independently below.
    let dir = common::TempDir::new("bad-record-lock");
    let path = dir.path("lock");
    for bytes in [b"broken".as_slice(), b"", b"{\"pid\":", b"\xff"] {
        fs::write(&path, bytes).unwrap();
        fs::write(owner_record_path(&path), bytes).unwrap();
        let lock = FileLock::acquire(&path).unwrap();
        assert!(FileLock::acquire(&path).is_err(), "live flock must win over torn metadata");
        let owner = parse(&fs::read_to_string(owner_record_path(&path)).unwrap()).unwrap();
        assert_eq!(owner.get("pid"), Some(&Value::Number(std::process::id() as f64)));
        drop(lock);
        drop(FileLock::acquire(&path).unwrap());
    }
}
#[test]
fn fresh_legacy_owner_is_protected_when_heartbeats_share_a_millisecond() {
    let dir = common::TempDir::new("legacy-owner-tie");
    let path = dir.path("library.lock");
    let heartbeat = now_ms();
    fs::write(&path, format!("{{\"pid\":{},\"heartbeat\":{heartbeat}}}", std::process::id())).unwrap();
    fs::write(owner_record_path(&path), format!("{{\"pid\":0,\"heartbeat\":{heartbeat}}}")).unwrap();
    assert_eq!(FileLock::acquire(&path).err().map(|e| e.kind()), Some(std::io::ErrorKind::WouldBlock));
}
#[test]
fn a_fresh_atomic_owner_is_protected_from_a_same_millisecond_legacy_release() {
    let dir = common::TempDir::new("atomic-owner-tie");
    let path = dir.path("library.lock");
    let heartbeat = now_ms();
    fs::write(&path, format!("{{\"pid\":0,\"heartbeat\":{heartbeat}}}")).unwrap();
    fs::write(owner_record_path(&path), format!("{{\"pid\":{},\"heartbeat\":{heartbeat}}}", std::process::id())).unwrap();
    assert_eq!(FileLock::acquire(&path).err().map(|e| e.kind()), Some(std::io::ErrorKind::WouldBlock));
}
#[test]
fn lock_child_process() {
    let Some(path) = std::env::var_os("PM_KERNEL_TEST_LOCK") else {
        return;
    };
    let expected = std::env::var("PM_KERNEL_TEST_LOCK_EXPECT").unwrap();
    // A project file's lock when its lock folder is given, else a record lock.
    let result = match std::env::var_os("PM_KERNEL_TEST_LOCK_ROOT") {
        Some(root) => lock_project(Path::new(&root), Path::new(&path)),
        None => FileLock::acquire(Path::new(&path)),
    };
    assert_eq!(result.is_ok(), expected == "acquired");
}
fn child_project_lock(root: &Path, file: &Path, expected: &str) {
    let result = Process::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_child_process"])
        .env("PM_KERNEL_TEST_LOCK", file)
        .env("PM_KERNEL_TEST_LOCK_ROOT", root)
        .env("PM_KERNEL_TEST_LOCK_EXPECT", expected)
        .stdout(Stdio::null())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
/// A project on a volume only this Mac writes is locked in a lock folder
/// (the user's; here one of the test's own, to count what is in it), not
/// beside the file. Nothing is written into the project's folder, so a sync
/// client has no lock file to upload or to replace; replacing a file that
/// sits where the lock used to be changes nothing. Another process is
/// excluded while the project is open, across a save (which replaces the
/// file's inode), and until it closes; then the lock leaves no file behind.
#[test]
fn a_project_lock_is_in_a_lock_folder_and_cannot_be_replaced_underneath() {
    let dir = common::TempDir::new("local-lock");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::write(&file, PROJECT).unwrap();
    let mut k = kernel_with_locks(&dir.path("native"), &dir.path("locks"));
    let root = k.lock_root();
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::Writable);
    let beside = |dir: &common::TempDir| {
        let mut names: Vec<_> = fs::read_dir(dir.path("job"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(
        beside(&dir),
        ["cut.pmv"],
        "only the project is in its folder"
    );
    child_project_lock(&root, &file, "blocked");
    // What a sync client could do to a lock file beside the project.
    land(&project_lock_path(&file), b"");
    child_project_lock(&root, &file, "blocked");
    fs::remove_file(project_lock_path(&file)).unwrap();
    edit(&mut k, "P", 20.0);
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    k.finish_session_save(plan.run()).unwrap();
    assert!(
        !project_lock_path(&file).exists() && private_leftovers(&file).is_empty(),
        "a save leaves no lock beside the project either: {:?}",
        beside(&dir)
    );
    child_project_lock(&root, &file, "blocked");
    k.shell(ShellCall::Close(session)).unwrap();
    child_project_lock(&root, &file, "acquired");
    assert_eq!(
        fs::read_dir(&root).unwrap().count(),
        0,
        "a released lock leaves no file"
    );
}
fn child_lock(path: &Path, expected: &str) {
    let result = Process::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_child_process"])
        .env("PM_KERNEL_TEST_LOCK", path)
        .env("PM_KERNEL_TEST_LOCK_EXPECT", expected)
        .stdout(Stdio::null())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn two_processes_contend_for_library_and_project_locks() {
    let dir = common::TempDir::new("two-processes");
    let w = writer(&dir.0);
    let library_lock = dir.path("store/library.lock");
    child_lock(&library_lock, "blocked");
    drop(w);
    child_lock(&library_lock, "acquired");
    let project_lock = dir.path("movie.pmvlock");
    let lock = FileLock::acquire(&project_lock).unwrap();
    child_lock(&project_lock, "blocked");
    drop(lock);
    child_lock(&project_lock, "acquired");
}
#[test]
fn older_electron_singleton_is_detected_without_writing_its_profile() {
    use std::os::unix::fs::symlink;
    let dir = common::TempDir::new("electron");
    let mut hostname = [0i8; 256];
    unsafe {
        libc::gethostname(hostname.as_mut_ptr(), 255);
    };
    let hostname = unsafe { std::ffi::CStr::from_ptr(hostname.as_ptr()) }.to_string_lossy();
    symlink(
        format!("{hostname}-{}", std::process::id()),
        dir.path("SingletonLock"),
    )
    .unwrap();
    assert!(older_electron_running(&dir.0));
    assert_eq!(
        open_decision(&dir.path("p.pmv"), Some(&dir.0)),
        OpenDecision::AsCopy(CopyReason::OlderElectron)
    );
    fs::remove_file(dir.path("SingletonLock")).unwrap();
    symlink(format!("{hostname}-2000000000"), dir.path("SingletonLock")).unwrap();
    assert!(!older_electron_running(&dir.0));
}
#[test]
fn a_locked_file_opens_as_a_named_copy_and_never_saves_over_the_source() {
    let dir = common::TempDir::new("locked-copy");
    let file = dir.path("original.pmv");
    fs::write(&file, PROJECT).unwrap();
    let bytes = fs::read(&file).unwrap();
    let _lock = lock_project(&user_lock_root(), &file).unwrap();
    let mut k = kernel(&dir.path("native"));
    let ShellReply::Opened {
        session, decision, ..
    } = k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(decision, OpenDecision::AsCopy(CopyReason::Locked));
    edit(&mut k, "P", 20.0);
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(fs::read(file).unwrap(), bytes);
    let rows = LibraryStore::open(&dir.path("native")).projects().unwrap();
    assert!(rows[0].name.ends_with("— Native"));
}
#[test]
fn an_external_file_opens_in_place_and_its_library_entry_keeps_its_name() {
    let dir = common::TempDir::new("in-place");
    let file = dir.path("original.pmv");
    fs::write(&file, PROJECT).unwrap();
    let mut k = kernel(&dir.path("native"));
    let ShellReply::Opened {
        session, decision, ..
    } = k
        .shell(ShellCall::Open {
            id: "F".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(decision, OpenDecision::Writable);
    assert!(
        !project_lock_path(&file).exists(),
        "no lock beside the file"
    );
    assert!(lock_project(&k.lock_root(), &file).is_err(), "held");
    edit(&mut k, "F", 20.0);
    k.shell(ShellCall::Save(session.clone())).unwrap();
    k.shell(ShellCall::Close(session)).unwrap();
    let rows = LibraryStore::open(&dir.path("native")).projects().unwrap();
    assert_eq!(rows.len(), 1, "one entry: the file itself, not a copy");
    assert_eq!(rows[0].name, "Fixture");
    let saved = String::from_utf8_lossy(&fs::read(&file).unwrap()).into_owned();
    assert!(saved.contains("Fixture"), "saved in place");
}
#[test]
fn a_clean_close_leaves_no_journal_for_quit_or_the_next_open() {
    let dir = common::TempDir::new("clean-close");
    let file = dir.path("original.pmv");
    fs::write(&file, PROJECT).unwrap();
    let mut k = kernel(&dir.path("native"));
    let open = |k: &mut Kernel| {
        let ShellReply::Opened {
            session, recovery, ..
        } = k
            .shell(ShellCall::PrepareOpen {
                id: "F".into(),
                source: SessionSource::File(file.clone()),
                electron_profile: None,
            })
            .unwrap()
        else {
            panic!("a saved project opens without a recovery question")
        };
        assert_eq!(recovery, RecoveryResult::None);
        session
    };
    let session = open(&mut k);
    edit(&mut k, "F", 20.0);
    k.shell(ShellCall::Close(session)).unwrap();
    // Quit compacts only work that is not already saved.
    let ShellReply::Compacted { projects, .. } = k.shell(ShellCall::CompactAll).unwrap() else {
        panic!()
    };
    assert_eq!(projects, 0);
    // Closing what is already saved leaves the file alone.
    let saved = fs::read(&file).unwrap();
    let modified = fs::metadata(&file).unwrap().modified().unwrap();
    let session = open(&mut k);
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(fs::read(&file).unwrap(), saved);
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), modified);
}
#[test]
fn a_project_from_before_compositions_opens_with_one_stable_composition() {
    let dir = common::TempDir::new("legacy-comp");
    let file = dir.path("legacy.pmv");
    // No compId or compName, as files saved before compositions have.
    fs::write(&file, r##"{"v":"1.0.0","proj":{"id":"Plegacy1","name":"Legacy","w":200,"h":100,"fps":30,"dur":2,"bg":"#000000","assets":{},"layers":[{"id":"L1","type":"shape","name":"Layer","from":0,"dur":2,"on":true,"p":{}}]}}"##).unwrap();
    let open = || {
        let o = pm_kernel::project_io::open_project(&file, Default::default()).unwrap();
        let comp = o.open_comp.clone().expect("an open composition");
        let root = o.document.to_value().unwrap();
        let stored = root
            .get("comps")
            .and_then(|c| c.get(&comp))
            .expect("stored under its id");
        assert_eq!(stored.get("name").and_then(Value::as_str), Some("Legacy"));
        assert_eq!(
            stored.get("layers").and_then(Value::as_array).map(Vec::len),
            Some(1)
        );
        comp
    };
    assert_eq!(
        open(),
        open(),
        "the same id on every open keeps the journal base"
    );
}
#[test]
fn foreign_electron_library_is_readonly_and_copy_identity_is_new() {
    let dir = common::TempDir::new("foreign-copy");
    let source = dir.path("electron");
    {
        let w = writer(&source);
        w.save_project("P", value(), Value::Null, Value::Null, Value::Null)
            .unwrap();
    }
    let before = fs::read(source.join("store/project.P.json")).unwrap();
    let mut k = kernel(&dir.path("native"));
    let ShellReply::Opened { decision, .. } = k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::Library {
                profile: source.clone(),
                project: "P".into(),
            },
            electron_profile: None,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(decision, OpenDecision::AsCopy(CopyReason::ForeignLibrary));
    assert_eq!(
        fs::read(source.join("store/project.P.json")).unwrap(),
        before
    );
    let rows = LibraryStore::open(&dir.path("native")).projects().unwrap();
    assert_ne!(rows[0].id, "P");
}
#[test]
fn settings_workspaces_extensions_persist_and_deletions_survive_reopen() {
    let dir = common::TempDir::new("settings");
    let mut k = kernel(&dir.0);
    let workspace=parse(r#"[{"id":"design","name":"Design","builtin":true,"layout":{"docks":[]},"controls":[{"future":1}]}]"#).unwrap();
    let ack = k
        .state_transaction(state(vec![
            setting("theme", Some(Value::from("dark"))),
            setting("workspaces", Some(workspace.clone())),
            setting("workspace", Some(Value::from("design"))),
            setting("agentModel.provider", Some(Value::from("model"))),
            StateOp::Extension(ExtensionEntry {
                id: "synthetic".into(),
                installed: true,
                enabled: false,
                version: "2.3.0".into(),
                source: parse(r#"{"kind":"local","path":"synthetic"}"#).unwrap(),
            }),
        ]))
        .unwrap();
    assert!(
        ack.changes
            .iter()
            .any(|c| matches!(c,StateChange::Setting {key,..} if key=="theme"))
    );
    k.state_transaction(state(vec![setting("theme", None)]))
        .unwrap();
    drop(k);
    let k = kernel(&dir.0);
    assert!(!k.state().settings.contains_key("theme"));
    assert_eq!(k.state().settings["workspaces"], workspace);
    assert_eq!(
        k.state().settings["agentModel.provider"],
        Value::from("model")
    );
    assert_eq!(k.state().extensions["synthetic"].version, "2.3.0");
    assert!(!k.state().extensions["synthetic"].enabled);
    assert_eq!(
        LibraryStore::open(&dir.0)
            .read("extensions")
            .unwrap()
            .unwrap()
            .get("synthetic"),
        Some(&Value::Bool(false))
    );
}
#[test]
fn state_transactions_are_atomic_versioned_and_do_not_enter_document_history() {
    let dir = common::TempDir::new("state-tx");
    let mut k = kernel(&dir.0);
    open(&mut k, "P", SessionSource::New(value()));
    let before = k.state().clone();
    let revision = k.project("P").unwrap().store().revision();
    assert!(
        k.state_transaction(state(vec![
            setting("theme", Some(Value::from("lost"))),
            setting("../bad", Some(Value::Null))
        ]))
        .is_err()
    );
    assert_eq!(k.state(), &before);
    let ack = k
        .state_transaction(state(vec![setting("theme", Some(Value::from("dark")))]))
        .unwrap();
    assert_eq!(ack.revision, before.revision + 1);
    assert!(
        k.state_transaction(StateTransaction {
            expected_revision: Some(before.revision),
            ops: vec![setting("theme", None)]
        })
        .is_err()
    );
    assert_eq!(k.project("P").unwrap().store().revision(), revision);
    assert_eq!(k.project("P").unwrap().store().history().len(), 0);
    assert!(!k.project("P").unwrap().store().is_dirty());
}
#[test]
fn registry_stacks_support_buried_disposal_top_reveal_and_owner_generation() {
    let mut k = Kernel::new(Default::default());
    let owner = Writer {
        realm: 5,
        generation: 1,
    };
    let entry = |token: &str| Registration {
        token: token.into(),
        owner,
        id: "select".into(),
        definition: Value::from(token),
    };
    k.state_transaction(state(vec![
        StateOp::Register {
            registry: "commands".into(),
            entry: entry("a"),
        },
        StateOp::Register {
            registry: "commands".into(),
            entry: entry("b"),
        },
        StateOp::Register {
            registry: "commands".into(),
            entry: entry("c"),
        },
    ]))
    .unwrap();
    k.state_transaction(state(vec![StateOp::Dispose { token: "b".into() }]))
        .unwrap();
    assert_eq!(k.state().provider("commands", "select").unwrap().token, "c");
    k.state_transaction(state(vec![StateOp::Dispose { token: "c".into() }]))
        .unwrap();
    assert_eq!(k.state().provider("commands", "select").unwrap().token, "a");
    k.state_transaction(state(vec![StateOp::DisposeOwner { owner }]))
        .unwrap();
    assert!(k.state().provider("commands", "select").is_none());
}
/// An edit is acknowledged (it is in the journal's open segment), and the
/// journal then fails in the background: its checkpoint cannot be written.
/// The kernel says so at once, without waiting for another request, and
/// says which project is not protected, so its window can raise the same
/// state as for an edit refused at the time. Another open project hears
/// nothing.
#[test]
fn a_background_journal_failure_names_its_project_at_once() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Condvar, Mutex};
    let dir = common::TempDir::new("background-durability");
    let journals = dir.path("journals");
    let hold = Arc::new((Mutex::new(false), Condvar::new()));
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(journals.clone()),
        journal: pm_kernel::journal::JournalConfig {
            checkpoint_revisions: 1,
            hold_checkpoints: Some(hold.clone()),
            ..Default::default()
        },
        ..Default::default()
    });
    let h = actor.handle();
    for id in ["P", "Q"] {
        h.open(id, OpenSource::Untitled(value()))
            .wait()
            .unwrap()
            .unwrap();
    }
    h.sync().wait();
    let events = h.subscribe();
    *hold.0.lock().unwrap() = true;
    let ack = h
        .call(
            "P",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Opacity".into()),
                    body: WriteBody::Commands(vec![Command::SetStaticOpacity {
                        layer_id: "layer0".into(),
                        value: 40.0,
                    }]),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(ack.reply, Reply::Applied { changed: true, .. }));
    assert!(ack.notice.is_none(), "the edit itself was accepted");
    // Its session folder stops taking new files, then the checkpoint runs.
    let session = pm_kernel::journal::sessions(&journals.join("P"))
        .pop()
        .unwrap();
    fs::set_permissions(&session, fs::Permissions::from_mode(0o555)).unwrap();
    *hold.0.lock().unwrap() = false;
    hold.1.notify_all();
    let mut named = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while named.is_empty() && std::time::Instant::now() < deadline {
        if let Ok(KernelEvent::Durability { project, .. }) =
            events.recv_timeout(Duration::from_millis(100))
        {
            named.push(project);
        }
    }
    fs::set_permissions(&session, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(named, ["P"], "reported without another request, by project");
}
#[test]
fn actor_publishes_per_key_events_and_ui_facts_before_ack() {
    let actor = KernelActor::spawn(Default::default());
    let h = actor.handle();
    h.open("P", OpenSource::Untitled(value()))
        .wait()
        .unwrap()
        .unwrap();
    let events = h.subscribe();
    let ui = UiState {
        tool: parse(r#"{"tool":"pen","toolShape":"rect"}"#).unwrap(),
        ..Default::default()
    };
    let ack = h
        .state_transaction(state(vec![
            setting("theme", Some(Value::from("dark"))),
            StateOp::Ui {
                project: "P".into(),
                next: ui,
            },
        ]))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(h.state().revision, ack.revision);
    assert_eq!(
        h.published("P")
            .unwrap()
            .ui_state
            .tool
            .get("tool")
            .and_then(Value::as_str),
        Some("pen")
    );
    let mut settings = 0;
    for _ in 0..ack.changes.len() {
        if let KernelEvent::StateChanged {
            revision,
            change: StateChange::Setting { key, .. },
        } = events.recv_timeout(Duration::from_secs(1)).unwrap()
        {
            assert_eq!(revision, ack.revision);
            assert_eq!(key, "theme");
            settings += 1;
        }
    }
    assert_eq!(settings, 1);
}
#[test]
fn workspace_mutations_keep_partial_writes_without_document_undo_and_emit_events() {
    let actor = KernelActor::spawn(Default::default());
    let h = actor.handle();
    h.open("P", OpenSource::Untitled(value()))
        .wait()
        .unwrap()
        .unwrap();
    let events = h.subscribe();
    let ws = parse(r#"{"id":"edited","layout":{"docks":[]}}"#).unwrap();
    let ack = h
        .call(
            "P",
            Request::new(
                Caller::native(),
                WorkspaceCall::Mutate {
                    next: ws.clone(),
                    threw: Some("callback".into()),
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    let p = h.published("P").unwrap();
    assert_eq!(p.workspace, ws);
    assert_eq!(p.history.entries, 0);
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        KernelEvent::Published { .. }
    ));
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        KernelEvent::WorkspaceChanged { .. }
    ));
}
#[test]
fn v12_keys_and_writer_code_paths_and_v13_codec_accept_our_writes() {
    let root = common::crate_dir().join("../../../..");
    for tag in ["v1.2.0", "v1.3.0"] {
        let verify = || {
            Process::new("git")
                .args([
                    "rev-parse",
                    "--verify",
                    &format!("refs/tags/{tag}^{{commit}}"),
                ])
                .current_dir(&root)
                .output()
                .expect("git is required")
                .status
                .success()
        };
        if !verify() {
            let fetched = Process::new("git")
                .args(["fetch", "origin", "tag", tag])
                .current_dir(&root)
                .output()
                .expect("git fetch is required");
            if !verify() {
                assert!(
                    !fetched.status.success(),
                    "Fetch reported success but {tag} is not a valid commit"
                );
                // Write directly: libtest captures eprintln even for a skip,
                // making its default output indistinguishable from a pass.
                use std::io::Write as _;
                writeln!(
                    std::io::stderr().lock(),
                    "SKIP tagged writer parity: {tag} unavailable after fetch: {}",
                    String::from_utf8_lossy(&fetched.stderr)
                )
                .expect("report skipped parity check");
                return;
            }
        }
    }
    // Probe the capability we need, not a version string. A missing runtime
    // is a setup failure, never a successful parity check.
    let runtime = [std::env::var_os("PM_TEST_JS_RUNTIME"), Some("node".into()), Some("bun".into())]
        .into_iter().flatten().find(|program| Process::new(program).args(["--input-type=module", "-e", "import * as m from 'node:module'; if (!m.stripTypeScriptTypes && !globalThis.Bun) process.exit(1)"]).output().is_ok_and(|o| o.status.success()))
        .expect("Tagged parity needs Node with stripTypeScriptTypes or Bun; set PM_TEST_JS_RUNTIME to its executable");
    let dir = common::TempDir::new("tag-parity");
    let v = Value::from("repeat".repeat(12000));
    fs::write(
        dir.path("plain.json"),
        encode_disk_value(&v, DiskFormat::V1_2).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.path("compressed.json"),
        encode_disk_value(&v, DiskFormat::V1_3).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.path("keys.json"),
        serde_json::to_vec(&(STATIC_KEYS.to_vec(), DYNAMIC_PREFIXES.to_vec())).unwrap(),
    )
    .unwrap();
    {
        let mut k = kernel(&dir.0);
        let h = open(&mut k, "P", SessionSource::New(value()));
        edit(&mut k, "P", 45.0);
        k.shell(ShellCall::Close(h)).unwrap();
    }
    let script = common::crate_dir().join("tests/tag_store_parity.mjs");
    let result = Process::new(runtime)
        .arg(script)
        .arg(&dir.0)
        .current_dir(common::crate_dir().join("../../../.."))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        parse(&fs::read_to_string(dir.path("electron-theme.json")).unwrap()).unwrap(),
        v
    );
    assert_eq!(
        to_canonical_string(
            &pm_kernel::library::decode_disk_value(
                parse(&fs::read_to_string(dir.path("compressed.json")).unwrap()).unwrap(),
                1 << 20
            )
            .unwrap()
        ),
        to_canonical_string(&v)
    );
}
#[test]
fn copy_journals_follow_the_copy_slot_after_a_crash() {
    let dir = common::TempDir::new("copy-crash");
    let file = dir.path("external.pmv");
    fs::write(&file, PROJECT).unwrap();
    let _lock = lock_project(&user_lock_root(), &file).unwrap();
    let profile = dir.path("native");
    let mut k = kernel(&profile);
    open(&mut k, "P", SessionSource::File(file));
    let slot = LibraryStore::open(&profile).projects().unwrap()[0]
        .id
        .clone();
    edit(&mut k, "P", 15.0);
    k.sync_all();
    drop(k);
    let mut k = kernel(&profile);
    open(
        &mut k,
        &slot,
        SessionSource::Library {
            profile: profile.clone(),
            project: slot.clone(),
        },
    );
    assert_eq!(
        common::opacity_of(k.project(&slot).unwrap().store(), "layer0"),
        Some(15.0)
    );
}
#[test]
fn embedded_media_in_a_copy_survives_removal_of_the_original_file() {
    let dir = common::TempDir::new("copy-media");
    let file = dir.path("external.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let _lock = lock_project(&user_lock_root(), &file).unwrap();
    let profile = dir.path("native");
    let mut k = kernel(&profile);
    let h = open(&mut k, "P", SessionSource::File(file.clone()));
    let count = k.project("P").unwrap().store().media().len();
    assert!(count > 0);
    k.shell(ShellCall::Close(h)).unwrap();
    drop(k);
    fs::remove_file(&file).unwrap();
    let slot = LibraryStore::open(&profile).projects().unwrap()[0]
        .id
        .clone();
    let mut k = kernel(&profile);
    let h = open(
        &mut k,
        &slot,
        SessionSource::Library {
            profile: profile.clone(),
            project: slot.clone(),
        },
    );
    assert_eq!(k.project(&slot).unwrap().store().media().len(), count);
    let exported = dir.path("independent.pmv");
    k.shell(ShellCall::SaveAs {
        session: h,
        path: exported.clone(),
        replace: false,
    })
    .unwrap();
    assert_eq!(
        open_project(&exported, OpenOptions::default())
            .unwrap()
            .file
            .media
            .len(),
        count
    );
}
#[test]
fn creating_an_existing_project_id_is_refused_without_changing_the_slot() {
    let dir = common::TempDir::new("create-collision");
    let mut k = kernel(&dir.0);
    let h = open(&mut k, "P", SessionSource::New(value()));
    k.shell(ShellCall::Close(h)).unwrap();
    let before = fs::read(dir.path("store/project.P.json")).unwrap();
    assert!(
        k.shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::New(value()),
            electron_profile: None
        })
        .is_err()
    );
    assert_eq!(fs::read(dir.path("store/project.P.json")).unwrap(), before);
}
#[test]
fn the_same_source_cannot_have_two_live_sessions() {
    let dir = common::TempDir::new("source-owner");
    let mut k = kernel(&dir.0);
    open(&mut k, "P", SessionSource::New(value()));
    assert!(matches!(
        k.shell(ShellCall::Open {
            id: "alias".into(),
            source: SessionSource::Library {
                profile: dir.0.clone(),
                project: "P".into()
            },
            electron_profile: None
        }),
        Err(KernelFailure::AlreadyOpen(_))
    ));
}
#[test]
fn compact_all_publishes_unopened_library_journals_for_rollback() {
    let dir = common::TempDir::new("compact-library");
    {
        let mut k = kernel(&dir.0);
        open(&mut k, "P", SessionSource::New(value()));
        edit(&mut k, "P", 12.0);
        k.sync_all();
    }
    let mut k = kernel(&dir.0);
    let ShellReply::Compacted { projects, .. } = k.shell(ShellCall::CompactAll).unwrap() else {
        panic!()
    };
    assert_eq!(projects, 1);
    let p = LibraryStore::open(&dir.0).load("P").unwrap().unwrap();
    let mut rollback = Kernel::new(Default::default());
    rollback.open("P", OpenSource::Library(p)).unwrap();
    assert_eq!(
        common::opacity_of(rollback.project("P").unwrap().store(), "layer0"),
        Some(12.0)
    );
}
#[test]
fn compact_all_publishes_unopened_external_file_journals() {
    let dir = common::TempDir::new("compact-file");
    let path = dir.path("source.pmv");
    fs::write(&path, PROJECT).unwrap();
    let profile = dir.path("native");
    {
        let mut k = kernel(&profile);
        open(&mut k, "P", SessionSource::File(path.clone()));
        edit(&mut k, "P", 18.0);
        k.sync_all();
    }
    let mut k = kernel(&profile);
    k.shell(ShellCall::CompactAll).unwrap();
    let store = common::open_store(&path);
    assert_eq!(common::opacity_of(&store, "layer0"), Some(18.0));
}
#[test]
fn duplicate_includes_unsaved_live_edits() {
    let dir = common::TempDir::new("duplicate-live");
    let mut k = kernel(&dir.0);
    open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 22.0);
    k.shell(ShellCall::Duplicate {
        project: "P".into(),
        id: "copy".into(),
        name: "Copy".into(),
    })
    .unwrap();
    let p = LibraryStore::open(&dir.0).load("copy").unwrap().unwrap();
    let mut copy = Kernel::new(Default::default());
    copy.open("copy", OpenSource::Library(p)).unwrap();
    assert_eq!(
        common::opacity_of(copy.project("copy").unwrap().store(), "layer0"),
        Some(22.0)
    );
}
#[cfg(target_os = "macos")]
#[test]
fn sealed_system_volume_is_detected_as_read_only_without_a_probe_write() {
    assert!(is_read_only(Path::new("/System/Library")));
}
#[test]
fn live_lock_refreshes_heartbeat_and_keeps_its_inode() {
    use std::os::unix::fs::MetadataExt;
    let dir = common::TempDir::new("heartbeat");
    let path = dir.path("p.pmvlock");
    let _lock = FileLock::acquire(&path).unwrap();
    let before = parse(&fs::read_to_string(&path).unwrap()).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    std::thread::sleep(Duration::from_millis(5200));
    let after = parse(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        after.get("heartbeat").and_then(Value::as_f64)
            > before.get("heartbeat").and_then(Value::as_f64)
    );
    assert_eq!(fs::metadata(path).unwrap().ino(), inode);
}
#[test]
fn offline_external_project_compacts_as_a_copy_and_keeps_its_slot() {
    let dir = common::TempDir::new("offline");
    let path = dir.path("source.pmv");
    fs::write(&path, PROJECT).unwrap();
    let profile = dir.path("native");
    {
        let mut k = kernel(&profile);
        open(&mut k, "P", SessionSource::File(path.clone()));
        edit(&mut k, "P", 17.0);
        k.sync_all();
    }
    fs::remove_file(&path).unwrap();
    let mut k = kernel(&profile);
    let ShellReply::Compacted {
        recovery_copies, ..
    } = k.shell(ShellCall::CompactAll).unwrap()
    else {
        panic!()
    };
    assert_eq!(recovery_copies.len(), 1);
    let reader = LibraryStore::open(&profile);
    // The entry stays, naming its file; the file was its only body.
    assert!(reader.load("P").unwrap().is_none());
    assert!(
        reader
            .read("projectMeta.P")
            .unwrap()
            .unwrap()
            .get("file")
            .and_then(|f| f.get("path"))
            .and_then(|p| p.as_str())
            .is_some_and(|p| p.ends_with("source.pmv"))
    );
    assert!(
        reader
            .read("projects")
            .unwrap()
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.get("id").and_then(|id| id.as_str()) == Some("P"))
    );
    let p = reader.load(&recovery_copies[0]).unwrap().unwrap();
    let mut copy = Kernel::new(Default::default());
    copy.open("copy", OpenSource::Library(p)).unwrap();
    assert_eq!(
        common::opacity_of(copy.project("copy").unwrap().store(), "layer0"),
        Some(17.0)
    );
}
#[test]
fn duplicated_embedded_media_has_an_independent_file() {
    let dir = common::TempDir::new("duplicate-media");
    let source = dir.path("source.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &source).unwrap();
    let profile = dir.path("native");
    let mut k = kernel(&profile);
    let h = open(&mut k, "P", SessionSource::File(source.clone()));
    let count = k.project("P").unwrap().store().media().len();
    k.shell(ShellCall::Duplicate {
        project: "P".into(),
        id: "copy".into(),
        name: "Copy".into(),
    })
    .unwrap();
    k.shell(ShellCall::Close(h)).unwrap();
    drop(k);
    fs::remove_file(source).unwrap();
    let mut k = kernel(&profile);
    let h = open(
        &mut k,
        "copy",
        SessionSource::Library {
            profile: profile.clone(),
            project: "copy".into(),
        },
    );
    assert_eq!(k.project("copy").unwrap().store().media().len(), count);
    k.shell(ShellCall::SaveAs {
        session: h,
        path: dir.path("independent.pmv"),
        replace: false,
    })
    .unwrap();
}
#[test]
fn reopening_file_from_library_recents_retains_in_place_save() {
    let dir = common::TempDir::new("recent-file");
    let source = dir.path("source.pmv");
    fs::write(&source, PROJECT).unwrap();
    let profile = dir.path("native");
    {
        let mut k = kernel(&profile);
        let h = open(&mut k, "P", SessionSource::File(source.clone()));
        k.shell(ShellCall::Close(h)).unwrap();
    }
    let mut k = kernel(&profile);
    let h = open(
        &mut k,
        "P",
        SessionSource::Library {
            profile: profile.clone(),
            project: "P".into(),
        },
    );
    edit(&mut k, "P", 34.0);
    k.shell(ShellCall::Save(h)).unwrap();
    assert_eq!(
        common::opacity_of(&common::open_store(&source), "layer0"),
        Some(34.0)
    );
}
#[test]
fn rollback_compaction_does_not_resurrect_trashed_projects() {
    let dir = common::TempDir::new("compact-trash");
    let mut k = kernel(&dir.0);
    let h = open(&mut k, "P", SessionSource::New(value()));
    edit(&mut k, "P", 31.0);
    k.shell(ShellCall::Close(h)).unwrap();
    k.shell(ShellCall::Trash("P".into())).unwrap();
    let reader = LibraryStore::open(&dir.0);
    let before = reader.trash().unwrap()[0].raw.clone();
    drop(k);
    let mut k = kernel(&dir.0);
    k.shell(ShellCall::CompactAll).unwrap();
    assert!(reader.projects().unwrap().is_empty());
    assert_eq!(reader.trash().unwrap()[0].raw, before);
}
/// Crashes `id`'s session for `file` with opacity `n` unsaved.
fn crash_with_edit(profile: &Path, id: &str, file: &Path, n: f64) {
    let mut k = kernel(profile);
    open(&mut k, id, SessionSource::File(file.into()));
    edit(&mut k, id, n);
    k.sync_all();
}
fn prepare(k: &mut Kernel, id: &str, source: SessionSource) -> (String, RecoveryResult) {
    match k
        .shell(ShellCall::PrepareOpen {
            id: id.into(),
            source,
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::RecoveryRequired { ticket, recovery } => (ticket, recovery),
        other => panic!("expected a recovery question: {other:?}"),
    }
}
fn decide(k: &mut Kernel, ticket: String, decision: RecoveryDecision) -> SessionHandle {
    match k
        .shell(ShellCall::DecideRecovery { ticket, decision })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}
fn live_sessions(profile: &Path, id: &str) -> usize {
    pm_kernel::journal::sessions(&profile.join("journals").join(id)).len()
}
#[test]
fn recovering_a_file_from_recents_recovers_it_in_place() {
    let dir = common::TempDir::new("recents-recover");
    let file = dir.path("original.pmv");
    fs::write(&file, PROJECT).unwrap();
    let profile = dir.path("native");
    crash_with_edit(&profile, "F", &file, 45.0);
    let before = fs::read(&file).unwrap();
    let mut k = kernel(&profile);
    let library = SessionSource::Library {
        profile: profile.clone(),
        project: "F".into(),
    };
    let (ticket, recovery) = prepare(&mut k, "F", library);
    assert!(
        matches!(recovery, RecoveryResult::Recovered { .. }),
        "the file is unchanged, so its journal recovers in place: {recovery:?}"
    );
    let session = decide(&mut k, ticket, RecoveryDecision::Recover);
    assert_eq!(
        common::opacity_of(k.project("F").unwrap().store(), "layer0"),
        Some(45.0)
    );
    let rows = LibraryStore::open(&profile).projects().unwrap();
    assert_eq!(rows.len(), 1, "no recovery copy: {rows:?}");
    assert!(
        lock_project(&k.lock_root(), &file).is_err(),
        "the file stays held in place"
    );
    k.shell(ShellCall::Save(session.clone())).unwrap();
    assert_eq!(
        common::opacity_of(&common::open_store(&file), "layer0"),
        Some(45.0)
    );
    assert_ne!(fs::read(&file).unwrap(), before);
}
#[test]
fn recovering_as_a_copy_retires_the_crashed_session_and_quit_leaves_the_file_alone() {
    let dir = common::TempDir::new("copy-retires");
    let file = dir.path("original.pmv");
    fs::write(&file, PROJECT).unwrap();
    let profile = dir.path("native");
    crash_with_edit(&profile, "F", &file, 45.0);
    // Someone else saves the file: the journal's base no longer matches.
    fs::write(&file, PROJECT.replace("Fixture", "Elsewhere")).unwrap();
    let before = fs::read(&file).unwrap();
    let mut k = kernel(&profile);
    let (ticket, recovery) = prepare(&mut k, "F", SessionSource::File(file.clone()));
    assert!(matches!(recovery, RecoveryResult::OfferCopy { .. }));
    let session = decide(&mut k, ticket, RecoveryDecision::Recover);
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(live_sessions(&profile, "F"), 0, "the copy holds that work");
    let ShellReply::Compacted {
        recovery_copies, ..
    } = k.shell(ShellCall::PrepareQuit).unwrap()
    else {
        panic!()
    };
    assert!(recovery_copies.is_empty(), "{recovery_copies:?}");
    assert_eq!(fs::read(&file).unwrap(), before);
    let copies = LibraryStore::open(&profile)
        .projects()
        .unwrap()
        .into_iter()
        .filter(|r| r.name.ends_with("— Recovered"))
        .count();
    assert_eq!(copies, 1);
}
#[test]
fn quit_never_writes_an_unopened_file_and_its_next_open_asks() {
    let dir = common::TempDir::new("quit-no-file-write");
    let file = dir.path("original.pmv");
    fs::write(&file, PROJECT).unwrap();
    let profile = dir.path("native");
    crash_with_edit(&profile, "F", &file, 45.0);
    let before = fs::read(&file).unwrap();
    let mut k = kernel(&profile);
    k.shell(ShellCall::PrepareQuit).unwrap();
    assert_eq!(
        fs::read(&file).unwrap(),
        before,
        "no save without the person"
    );
    let (_, recovery) = prepare(&mut k, "F", SessionSource::File(file.clone()));
    assert!(matches!(recovery, RecoveryResult::Recovered { .. }));
}
#[test]
fn dont_save_after_recovering_a_file_from_before_compositions_leaves_it_alone() {
    let dir = common::TempDir::new("legacy-dont-save");
    let file = dir.path("legacy.pmv");
    fs::write(&file, r##"{"v":"1.0.0","proj":{"id":"Plegacy1","name":"Legacy","w":200,"h":100,"fps":30,"dur":2,"bg":"#000000","assets":{},"layers":[{"id":"layer0","type":"shape","name":"Layer","from":0,"dur":2,"on":true,"p":{"opacity":{"v":100,"kf":[],"expr":null}}}]}}"##).unwrap();
    let profile = dir.path("native");
    crash_with_edit(&profile, "F", &file, 45.0);
    let before = fs::read(&file).unwrap();
    let mut k = kernel(&profile);
    let (ticket, _) = prepare(&mut k, "F", SessionSource::File(file.clone()));
    let session = decide(&mut k, ticket, RecoveryDecision::Recover);
    k.shell(ShellCall::DiscardClose(session)).unwrap();
    k.shell(ShellCall::PrepareQuit).unwrap();
    assert_eq!(fs::read(&file).unwrap(), before);
}
#[test]
fn saving_in_place_keeps_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::TempDir::new("keep-mode");
    let file = dir.path("shared.pmv");
    fs::write(&file, PROJECT).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    let mut k = kernel(&dir.path("native"));
    let session = open(&mut k, "F", SessionSource::File(file.clone()));
    edit(&mut k, "F", 30.0);
    k.shell(ShellCall::Save(session)).unwrap();
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o644
    );
}
#[test]
fn a_session_killed_unchanged_or_after_saving_reopens_without_asking() {
    let dir = common::TempDir::new("saved-then-killed");
    let path = dir.path("source.pmv");
    fs::write(&path, PROJECT).unwrap();
    let profile = dir.path("native");
    let prepare = |k: &mut Kernel| {
        k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(path.clone()),
            electron_profile: None,
        })
        .unwrap()
    };
    let checkpoints = || {
        fs::read_dir(profile.join("journals/P"))
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|s| fs::read_dir(s.path()).into_iter().flatten().flatten())
            .filter(|f| f.file_name().to_string_lossy().starts_with("checkpoint-"))
            .count()
    };
    {
        // Opened and never changed: nothing rewrites the project.
        let mut k = kernel(&profile);
        open(&mut k, "P", SessionSource::File(path.clone()));
        k.sync_all();
        assert_eq!(checkpoints(), 0);
        assert!(!profile.join("store/project.P.json").exists());
    }
    let h = {
        let mut k = kernel(&profile);
        let ShellReply::Opened { session, .. } = prepare(&mut k) else {
            panic!("an unchanged session asked to recover")
        };
        edit(&mut k, "P", 41.0);
        k.shell(ShellCall::Save(session.clone())).unwrap();
        k.sync_all();
        session
    };
    drop(h);
    {
        // Killed after saving: the file holds everything the journal does.
        let mut k = kernel(&profile);
        let ShellReply::Opened { .. } = prepare(&mut k) else {
            panic!("a saved session asked to recover")
        };
        assert_eq!(
            common::opacity_of(k.project("P").unwrap().store(), "layer0"),
            Some(41.0)
        );
        assert!(!k.project("P").unwrap().store().history().is_empty());
        edit(&mut k, "P", 12.0);
        k.sync_all();
    }
    // Killed with an unsaved edit: that is worth asking about.
    let mut k = kernel(&profile);
    assert!(matches!(
        prepare(&mut k),
        ShellReply::RecoveryRequired { .. }
    ));
}
/// A layout change (a dock's active tab switched because its saved panel
/// isn't installed here, a panel moved) is no unsaved work: a session killed
/// after only that reopens without asking and wrote no checkpoint. An edit
/// after it still asks, and recovery keeps the layout.
#[test]
fn a_session_killed_after_only_layout_changes_reopens_without_asking() {
    let dir = common::TempDir::new("layout-then-killed");
    let path = dir.path("source.pmv");
    fs::write(&path, PROJECT).unwrap();
    let profile = dir.path("native");
    let prepare = |k: &mut Kernel| {
        k.shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(path.clone()),
            electron_profile: None,
        })
        .unwrap()
    };
    let layout = |k: &mut Kernel, active: &str| {
        k.handle(
            "P",
            Request::new(
                Caller::native(),
                WorkspaceCall::Mutate {
                    next: parse(&format!(
                        r#"{{"id":"design","name":"Design","layout":{{"docks":[{{"id":"right","panels":[{{"id":"{active}","active":true}}]}}]}}}}"#
                    ))
                    .unwrap(),
                    threw: None,
                },
            ),
        )
        .unwrap();
    };
    let checkpoints = || {
        fs::read_dir(profile.join("journals/P"))
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|s| fs::read_dir(s.path()).into_iter().flatten().flatten())
            .filter(|f| f.file_name().to_string_lossy().starts_with("checkpoint-"))
            .count()
    };
    {
        let mut k = kernel(&profile);
        open(&mut k, "P", SessionSource::File(path.clone()));
        layout(&mut k, "flappy-bird.panel");
        layout(&mut k, "voiceover-recorder.panel");
        k.sync_all();
        assert_eq!(checkpoints(), 0, "a layout change rewrote the document");
    }
    {
        let mut k = kernel(&profile);
        let ShellReply::Opened { .. } = prepare(&mut k) else {
            panic!("a session with only layout changes asked to recover")
        };
        layout(&mut k, "inspector");
        edit(&mut k, "P", 23.0);
        k.sync_all();
    }
    let mut k = kernel(&profile);
    let ShellReply::RecoveryRequired { ticket, .. } = prepare(&mut k) else {
        panic!("an unsaved edit after a layout change didn't ask")
    };
    let _ = ticket;
}
#[test]
fn opening_the_entry_of_a_deleted_file_recovers_its_unsaved_work() {
    let dir = common::TempDir::new("deleted-file-open");
    let path = dir.path("source.pmv");
    fs::write(&path, PROJECT).unwrap();
    let profile = dir.path("native");
    {
        let mut k = kernel(&profile);
        open(&mut k, "P", SessionSource::File(path.clone()));
        edit(&mut k, "P", 17.0);
        k.sync_all();
    }
    fs::remove_file(&path).unwrap();
    let mut k = kernel(&profile);
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::Library {
                profile: profile.clone(),
                project: "P".into(),
            },
            electron_profile: None,
        })
        .unwrap();
    let ShellReply::Opened { session, .. } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(
        common::opacity_of(k.project(&session.project).unwrap().store(), "layer0"),
        Some(17.0)
    );
    k.shell(ShellCall::Close(session)).unwrap();
    // The work is a library project now; the entry still names its file.
    let reader = LibraryStore::open(&profile);
    assert!(
        reader
            .projects()
            .unwrap()
            .iter()
            .any(|r| r.name.ends_with("— Recovered"))
    );
    assert!(reader.read("projectMeta.P").unwrap().is_some());
}
#[test]
fn duplicating_the_entry_of_a_deleted_file_copies_its_newest_work() {
    let dir = common::TempDir::new("deleted-file-duplicate");
    let path = dir.path("source.pmv");
    fs::write(&path, PROJECT).unwrap();
    let profile = dir.path("native");
    {
        let mut k = kernel(&profile);
        open(&mut k, "P", SessionSource::File(path.clone()));
        edit(&mut k, "P", 23.0);
        k.sync_all();
    }
    fs::remove_file(&path).unwrap();
    let mut k = kernel(&profile);
    k.shell(ShellCall::Duplicate {
        project: "P".into(),
        id: "copy".into(),
        name: "Copy".into(),
    })
    .unwrap();
    let copy = LibraryStore::open(&profile).load("copy").unwrap().unwrap();
    assert!(copy.session.as_ref().and_then(|m| m.get("file")).is_none());
    let mut fresh = Kernel::new(Default::default());
    fresh.open("copy", OpenSource::Library(copy)).unwrap();
    assert_eq!(
        common::opacity_of(fresh.project("copy").unwrap().store(), "layer0"),
        Some(23.0)
    );
}
#[test]
fn an_unchanged_file_reopens_with_the_workspace_it_was_closed_with() {
    let dir = common::TempDir::new("file-workspace");
    let path = dir.path("source.pmv");
    fs::write(&path, PROJECT).unwrap();
    let profile = dir.path("native");
    let ws = parse(r#"{"id":"w1","name":"Mine","layout":{"docks":[{"id":"center","panels":[{"id":"viewer","flex":true}]}]}}"#).unwrap();
    let mut k = kernel(&profile);
    let h = open(&mut k, "P", SessionSource::File(path.clone()));
    let expected = k.project("P").unwrap().workspace().clone();
    let _ack = k
        .handle(
            "P",
            Request::new(
                Caller::native(),
                WorkspaceCall::Replace {
                    expected,
                    next: ws.clone(),
                },
            ),
        )
        .unwrap();
    // The kernel files it as the active workspace record too.
    assert_eq!(k.state().settings["workspace"].as_str(), Some("w1"));
    k.shell(ShellCall::Close(h)).unwrap();
    // A panel change is not a document change: the file is not rewritten.
    assert_eq!(fs::read_to_string(&path).unwrap(), PROJECT);
    drop(k);
    let mut k = kernel(&profile);
    let h = open(&mut k, "P", SessionSource::File(path.clone()));
    assert_eq!(k.project("P").unwrap().workspace(), &ws);
    k.shell(ShellCall::Close(h)).unwrap();
    drop(k);
    // A file changed elsewhere since brings its own workspace.
    fs::write(&path, PROJECT.replace("Fixture", "Elsewhere")).unwrap();
    let mut k = kernel(&profile);
    open(&mut k, "P", SessionSource::File(path));
    assert_ne!(k.project("P").unwrap().workspace(), &ws);
}
#[test]
fn two_projects_on_one_workspace_keep_each_others_changes() {
    let dir = common::TempDir::new("shared-workspace");
    let profile = dir.path("native");
    let mut k = kernel(&profile);
    for id in ["P", "Q"] {
        let path = dir.path(&format!("{id}.pmv"));
        fs::write(&path, PROJECT).unwrap();
        open(&mut k, id, SessionSource::File(path));
    }
    let ws =
        parse(r#"{"id":"w1","name":"Mine","layout":{"left":{"size":200},"right":{"size":300}}}"#)
            .unwrap();
    let replace = |k: &mut Kernel, id: &str, next: &pm_model::Value| {
        let expected = k.project(id).unwrap().workspace().clone();
        k.handle(
            id,
            Request::new(
                Caller::native(),
                WorkspaceCall::Replace {
                    expected,
                    next: next.clone(),
                },
            ),
        )
        .unwrap();
    };
    replace(&mut k, "P", &ws);
    replace(&mut k, "Q", &ws);
    // Each window changes its own part, neither having seen the other's.
    let with = |json: &str| parse(json).unwrap();
    replace(
        &mut k,
        "P",
        &with(r#"{"id":"w1","name":"Mine","layout":{"left":{"size":250},"right":{"size":300}}}"#),
    );
    replace(
        &mut k,
        "Q",
        &with(r#"{"id":"w1","name":"Mine","layout":{"left":{"size":200},"right":{"size":350}}}"#),
    );
    let stored = k.state().settings["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w.get("id").and_then(pm_model::Value::as_str) == Some("w1"))
        .cloned()
        .unwrap();
    assert_eq!(
        stored,
        with(r#"{"id":"w1","name":"Mine","layout":{"left":{"size":250},"right":{"size":350}}}"#)
    );
    // A stale replace (its expected value is old) changes nothing anywhere.
    let before = k.state().settings["workspaces"].clone();
    k.handle(
        "P",
        Request::new(
            Caller::native(),
            WorkspaceCall::Replace {
                expected: ws.clone(),
                next: with(r#"{"id":"w1","name":"Stale"}"#),
            },
        ),
    )
    .unwrap();
    assert_eq!(k.state().settings["workspaces"], before);
}
