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
fn cloud_paths_readonly_and_symlinked_cloud_are_copy_decisions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = common::TempDir::new("policy");
    for path in [
        "Library/Mobile Documents/com~apple~CloudDocs/movie.pmv",
        "Dropbox (Personal)/movie.pmv",
        "OneDrive-Work/movie.pmv",
    ] {
        assert_eq!(
            open_decision(&dir.path(path), None),
            OpenDecision::AsCopy(CopyReason::CloudFolder)
        );
    }
    let cloud = dir.path("Dropbox");
    fs::create_dir(&cloud).unwrap();
    symlink(&cloud, dir.path("alias")).unwrap();
    assert!(is_cloud_path(&dir.path("alias/movie.pmv")));
    let ro = dir.path("readonly");
    fs::create_dir(&ro).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    assert_eq!(
        open_decision(&ro.join("p.pmv"), None),
        OpenDecision::AsCopy(CopyReason::ReadOnly)
    );
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
}
#[cfg(target_os = "macos")]
#[test]
fn file_provider_xattr_causes_copy_without_a_cloud_folder_name() {
    use std::os::unix::ffi::OsStrExt;
    let dir = common::TempDir::new("provider");
    let p = std::ffi::CString::new(dir.0.as_os_str().as_bytes()).unwrap();
    let name = std::ffi::CString::new("com.apple.file-provider.synthetic").unwrap();
    let result =
        unsafe { libc::setxattr(p.as_ptr(), name.as_ptr(), b"1".as_ptr().cast(), 1, 0, 0) };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    assert!(is_cloud_path(&dir.path("p.pmv")));
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
        format!("{{\"pid\":{},\"heartbeat\":{}}}", std::process::id(), now_ms()),
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
#[test]
fn malformed_lock_is_never_stolen() {
    let dir = common::TempDir::new("bad-lock");
    let path = dir.path("lock");
    fs::write(&path, b"broken").unwrap();
    assert!(FileLock::acquire(&path).is_err());
    assert_eq!(fs::read(path).unwrap(), b"broken");
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
    assert!(reader.load("P").unwrap().is_some());
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
