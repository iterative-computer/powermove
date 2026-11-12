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
    assert!(dir.path("export.pmvlock").exists());
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
/// A disk without the atomic exchange (exFAT, HFS+, most network shares)
/// cannot replace a file conditionally: checking it and then renaming over
/// it would destroy a version a sync client or another machine installed in
/// between. So Save never replaces a file there. By the app's save route the
/// file keeps its bytes and the refusal says why (the sheet with Save As);
/// Save As writes the open work to a new file; by the kernel's own route the
/// work becomes a recovery copy.
#[test]
fn a_save_never_replaces_a_file_on_a_disk_without_an_atomic_exchange() {
    let dir = common::TempDir::new("no-exchange");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let mut k = kernel(&dir.path("native"));
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        no_exchange: true,
        ..Default::default()
    });
    let (session, decision) = open_file(&mut k, &file);
    assert_eq!(decision, OpenDecision::Writable);
    rename_comp(&mut k, "P", "One");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let refused = k
        .finish_session_save(plan.run())
        .expect_err("nothing may be replaced without the exchange");
    assert!(
        refused.to_string().contains("cannot replace a file safely"),
        "{refused}"
    );
    assert_eq!(fs::read(&file).unwrap(), original, "the file is untouched");
    let staged: Vec<_> = fs::read_dir(dir.path("job"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(staged.is_empty(), "no staged file is left: {staged:?}");
    // The work is still open: Save As writes it to a new file.
    let rescued = dir.path("job/cut 2.pmv");
    let plan = k
        .prepare_session_save(session.clone(), Some(rescued.clone()), false)
        .unwrap();
    k.finish_session_save(plan.run())
        .expect("Save As creates a new file on such a disk");
    assert_eq!(saved_name(&rescued).as_deref(), Some("One"));
    // The kernel's own route (a close that saves): a recovery copy.
    rename_comp(&mut k, "P", "Two");
    let ShellReply::Saved {
        recovery_copy: Some(copy),
        ..
    } = k.shell(ShellCall::Save(session)).unwrap()
    else {
        panic!("the work goes to a recovery copy")
    };
    assert_eq!(saved_name(&rescued).as_deref(), Some("One"));
    assert_eq!(fs::read(&file).unwrap(), original);
    let kept = LibraryStore::open(&dir.path("native"))
        .load(&copy)
        .unwrap()
        .expect("the recovery copy is in the library");
    assert_eq!(
        kept.project.get("name").and_then(Value::as_str),
        Some("Two — Recovered"),
        "the copy holds the newest work"
    );
}
/// The exchange's own answer is not trusted. One filesystem driver answers
/// it with success and renames the staged file over the project instead (a
/// FAT32 volume on macOS 27), destroying whatever was there, unread. The
/// save proves the exchange on two staging files first: where it does not
/// really exchange, the project file is never touched.
#[test]
fn a_save_never_trusts_an_exchange_that_only_renames() {
    let dir = common::TempDir::new("renaming-exchange");
    fs::create_dir_all(dir.path("job")).unwrap();
    let file = dir.path("job/cut.pmv");
    fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
    let original = fs::read(&file).unwrap();
    let mut k = kernel(&dir.path("native"));
    k.set_publish_hooks(pm_kernel::project_io::PublishHooks {
        exchange_renames: true,
        ..Default::default()
    });
    let (session, _) = open_file(&mut k, &file);
    rename_comp(&mut k, "P", "One");
    let plan = k
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    let refused = k
        .finish_session_save(plan.run())
        .expect_err("nothing may be replaced by a rename");
    assert_eq!(fs::read(&file).unwrap(), original, "the file is untouched");
    assert!(
        refused.to_string().contains("cannot replace a file safely"),
        "{refused}"
    );
    let left: Vec<_> = fs::read_dir(dir.path("job"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(left.is_empty(), "no staging file is left: {left:?}");
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
/// written over.
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
    // A name that is taken is refused, whether or not replacing was allowed.
    let taken = dir.path("job/other.pmv");
    fs::write(&taken, &original).unwrap();
    for replace in [false, true] {
        let plan = k
            .prepare_session_save(session.clone(), Some(taken.clone()), replace)
            .unwrap();
        k.finish_session_save(plan.run())
            .expect_err("a file that is there is never written over");
        assert_eq!(fs::read(&taken).unwrap(), original);
    }
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
/// of scratch disk images of each filesystem. On each, Save either replaces
/// the file or refuses and leaves its bytes alone, and Save As always
/// creates a new file there.
#[test]
#[ignore = "needs PM_TEST_VOLUMES"]
fn saves_on_real_volumes_replace_safely_or_not_at_all() {
    let volumes = std::env::var("PM_TEST_VOLUMES").expect("PM_TEST_VOLUMES");
    for volume in volumes.split(':').filter(|v| !v.is_empty()) {
        let dir = common::TempDir::new("volume");
        let folder = Path::new(volume).join("pm-save-test");
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let file = folder.join("cut.pmv");
        fs::copy(common::ts_fixture("fresh.pmv"), &file).unwrap();
        let original = fs::read(&file).unwrap();
        let mut k = kernel(&dir.path("native"));
        let (session, decision) = open_file(&mut k, &file);
        assert_eq!(decision, OpenDecision::Writable, "{volume}");
        rename_comp(&mut k, "P", "One");
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        let in_place = match k.finish_session_save(plan.run()) {
            Ok(_) => {
                assert_eq!(saved_name(&file).as_deref(), Some("One"), "{volume}");
                "replaced by exchange"
            }
            Err(e) => {
                assert!(
                    e.to_string().contains("cannot replace a file safely"),
                    "{volume}: {e}"
                );
                assert_eq!(fs::read(&file).unwrap(), original, "{volume}");
                "refused, file untouched"
            }
        };
        let rescued = folder.join("cut 2.pmv");
        let plan = k
            .prepare_session_save(session.clone(), Some(rescued.clone()), false)
            .unwrap();
        let save_as = match k.finish_session_save(plan.run()) {
            Ok(_) => {
                assert_eq!(saved_name(&rescued).as_deref(), Some("One"), "{volume}");
                "created".to_owned()
            }
            Err(e) => format!("FAILED: {e}"),
        };
        eprintln!("{volume}: Save {in_place}; Save As {save_as}");
        drop(k);
        let _ = fs::remove_dir_all(&folder);
        assert_eq!(save_as, "created", "{volume}");
    }
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
fn lock_child_process() {
    let Some(path) = std::env::var_os("PM_KERNEL_TEST_LOCK") else {
        return;
    };
    let expected = std::env::var("PM_KERNEL_TEST_LOCK_EXPECT").unwrap();
    let result = FileLock::acquire(Path::new(&path));
    assert_eq!(result.is_ok(), expected == "acquired");
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
    let _lock = FileLock::acquire(&project_lock_path(&file)).unwrap();
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
    assert!(project_lock_path(&file).exists(), "held under its .pmvlock");
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
    let _lock = FileLock::acquire(&project_lock_path(&file)).unwrap();
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
    let _lock = FileLock::acquire(&project_lock_path(&file)).unwrap();
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
    let lock = parse(&fs::read_to_string(project_lock_path(&file)).unwrap()).unwrap();
    assert_eq!(
        lock.get("pid").and_then(Value::as_f64),
        Some(f64::from(std::process::id())),
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
