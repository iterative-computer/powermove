mod support;
use pm_first_launch::{
    Event, Outcome, State, Step, resume_with_secret_consent, run,
    secrets::{SecretAccess, SecretStep},
};
use pm_kernel::{
    Caller, History, HistoryCall, Kernel, KernelConfig, LibraryStore, OpenSource, Request,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use support::*;
use tempfile::tempdir;

#[test]
fn generated_profile_migrates_copies_history_agents_gzip_local_storage_and_media() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    // Kernel writes the shipped gzip envelope; the coordinator must decode it.
    let huge = json!({"message":"x".repeat(40_000),"unknown":true});
    let value = pm_model::json::parse(&huge.to_string()).unwrap();
    let gzip =
        pm_kernel::persistence::encode_disk_value(&value, pm_kernel::persistence::DiskFormat::V1_3)
            .unwrap();
    assert!(String::from_utf8_lossy(&gzip).contains("__powermoveDiskValue"));
    fs::write(
        config.electron_profile.join("store/ext.synthetic.json"),
        gzip,
    )
    .unwrap();
    let mut history: Value = read(&config.electron_profile.join("store/projectHistory.p.json"));
    history["unknown"] = "synthetic padding ".repeat(4000).into();
    let encoded = pm_kernel::persistence::encode_disk_value(
        &pm_model::json::parse(&history.to_string()).unwrap(),
        pm_kernel::persistence::DiskFormat::V1_3,
    )
    .unwrap();
    assert!(String::from_utf8_lossy(&encoded).contains("__powermoveDiskValue"));
    fs::write(
        config.electron_profile.join("store/projectHistory.p.json"),
        encoded,
    )
    .unwrap();
    let before = files(&config.electron_profile);
    let mut events = Vec::new();
    let result = run(&config, |e| {
        events.push(e);
        Ok(())
    })
    .unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
                projects: 1,
                history_refusals: 0,
                blocked_owners: 0,
                ..
            }
        ),
        "{result:?}"
    );
    assert_eq!(before, files(&config.electron_profile));
    let cp = checkpoint(&config);
    assert!(cp.logical_keys.contains_key("ext.synthetic"));
    let store = LibraryStore::open(&config.native_profile);
    let copied = store.load(&cp.projects["p"]).unwrap().unwrap();
    assert_eq!(
        copied.project.get("name").unwrap().as_str(),
        Some("Synthetic project — Native")
    );
    assert!(copied.session.as_ref().unwrap().get("file").is_none());
    let imported = store.read("ext.synthetic").unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&pm_model::json::to_canonical_string(&imported)).unwrap(),
        huge
    );
    assert!(
        store
            .read(&format!("agentThreads.{}", cp.projects["p"]))
            .unwrap()
            .is_some()
    );
    assert!(store.read("agentAttachment.a").unwrap().is_some());
    let history = History::import(copied.history.as_ref().unwrap(), None).unwrap();
    assert_eq!(history.len(), 1);
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel.open("test", OpenSource::Library(copied)).unwrap();
    let result = kernel.handle(
        "test",
        Request::new(
            Caller::native(),
            pm_kernel::Call::History(HistoryCall::Undo),
        ),
    );
    assert!(result.is_ok(), "{result:?}");
    let restored = kernel
        .project("test")
        .unwrap()
        .store()
        .snapshot()
        .document
        .to_value()
        .unwrap();
    assert_eq!(
        restored
            .get("assets")
            .unwrap()
            .get("old")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str(),
        Some("undo-only")
    );

    let media: Value = read(&config.native_profile.join("Media Store/migration.json"));
    let pass = media["passes"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let closure = pass["closure"].as_array().unwrap();
    assert!(
        closure
            .iter()
            .any(|r| r["status"] == "proxy-only" && r["exportWarning"] == true)
    );
    assert!(closure.iter().any(|r| r["status"] == "already-offline"));
    assert!(closure.iter().any(|r| r["reference"]["ownerKind"] == "undo"
        && r["reference"]["legacyId"] == "undo-only"
        && r["status"] == "verified-original"));
    let local: Vec<pm_native_migrate::local_storage::Entry> =
        read(&config.native_profile.join("local-storage.json"));
    assert_eq!(local.len(), 1);
    assert_eq!(
        fs::metadata(
            config
                .native_profile
                .join("extensions-shared/synthetic/main.ts")
        )
        .unwrap()
        .permissions()
        .mode()
            & 0o222,
        0
    );
    assert!(config.native_profile.join("extensions-overlay").is_dir());
    assert!(
        config
            .native_profile
            .join("Agent Workspaces/synthetic/artifacts/result.txt")
            .is_file()
    );
    assert_eq!(events.last(), Some(&Event::Complete));
}
#[test]
fn running_electron_waits_without_creating_native_or_writing_source() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    symlink(
        "unknown-host-42",
        config.electron_profile.join("SingletonLock"),
    )
    .unwrap();
    let before = files(&config.electron_profile);
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::WaitingForElectron
    ));
    assert!(!config.native_profile.exists());
    assert_eq!(before, files(&config.electron_profile));
}
#[test]
fn re_run_preserves_beta_edits_and_deletions_and_collects_later_electron_import() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    run(&config, |_| Ok(())).unwrap();
    let cp = checkpoint(&config);
    let migration = fs::read(config.native_profile.join("migration.json")).unwrap();
    let media = fs::read(config.native_profile.join("Media Store/migration.json")).unwrap();
    fs::write(
        config.native_profile.join("store/theme.json"),
        "\"beta choice\"",
    )
    .unwrap();
    fs::remove_file(config.native_profile.join("store/ext.synthetic.json")).unwrap();
    run(&config, |_| Ok(())).unwrap();
    assert_eq!(
        migration,
        fs::read(config.native_profile.join("migration.json")).unwrap()
    );
    assert_eq!(
        media,
        fs::read(config.native_profile.join("Media Store/migration.json")).unwrap()
    );
    assert!(
        !config
            .native_profile
            .join("store/ext.synthetic.json")
            .exists()
    );
    assert_eq!(
        fs::read_to_string(config.native_profile.join("store/theme.json")).unwrap(),
        "\"beta choice\""
    );
    media_db(&config.electron_profile, true);
    run(&config, |_| Ok(())).unwrap();
    let state: Value = read(&config.native_profile.join("Media Store/migration.json"));
    assert_eq!(state["passes"].as_object().unwrap().len(), 2);
    assert_eq!(
        LibraryStore::open(&config.native_profile)
            .projects()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(cp.projects, checkpoint(&config).projects);
}
#[test]
fn each_step_resumes_and_published_checkpoint_never_follows_source_changes() {
    for stop in [
        Step::Checkpoint,
        Step::Bootstrap,
        Step::Library,
        Step::Media,
        Step::LocalStorage,
        Step::Extensions,
        Step::Secrets,
    ] {
        let t = tempdir().unwrap();
        let config = profile(t.path());
        let mut stopped = false;
        let result = run(&config, |event| {
            if event == Event::Published(stop) {
                stopped = true;
                anyhow::bail!("synthetic interruption");
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(stopped);
        fs::write(
            config.electron_profile.join("store/theme.json"),
            "\"changed later\"",
        )
        .unwrap();
        run(&config, |_| Ok(())).unwrap();
        assert_eq!(
            LibraryStore::open(&config.native_profile)
                .read("theme")
                .unwrap()
                .unwrap()
                .as_str(),
            Some("dark")
        );
        assert_eq!(
            LibraryStore::open(&config.native_profile)
                .projects()
                .unwrap()
                .len(),
            1
        );
    }
}
#[test]
fn rejects_overlap_symlink_destination_tampered_checkpoint_and_low_space() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let bad = pm_first_launch::Config {
        native_profile: config
            .electron_profile
            .join(pm_first_launch::BETA_BUNDLE_ID),
        ..config.clone()
    };
    assert!(run(&bad, |_| Ok(())).is_err());
    fs::create_dir_all(config.native_profile.parent().unwrap()).unwrap();
    symlink(&config.electron_profile, &config.native_profile).unwrap();
    assert!(run(&config, |_| Ok(())).is_err());
    fs::remove_file(&config.native_profile).unwrap();
    run(&config, |e| {
        if e == Event::Published(Step::Checkpoint) {
            anyhow::bail!("stop");
        }
        Ok(())
    })
    .unwrap_err();
    fs::write(
        config
            .native_profile
            .join("first-launch-checkpoint/profile/store/theme.json"),
        "null",
    )
    .unwrap();
    assert!(run(&config, |_| Ok(())).is_err());
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let f = fs::File::create(config.electron_profile.join("Agent State/huge")).unwrap();
    f.set_len(1_000_000_000_000).unwrap();
    assert!(format!("{:#}", run(&config, |_| Ok(())).unwrap_err()).contains("6 GiB"));
}
const VECTOR: &str = "djEw521BZIsrMJSAMb+RQ1RKlWAueRfLNPqKVz3YbvlR1wyDW+SU+AhcTt32biud/p0d";
struct SyntheticSecrets {
    reads: usize,
    values: Vec<Vec<u8>>,
    fail: bool,
}
impl SecretAccess for SyntheticSecrets {
    fn legacy_password(&mut self) -> anyhow::Result<zeroize::Zeroizing<Vec<u8>>> {
        self.reads += 1;
        Ok(zeroize::Zeroizing::new(b"S6 synthetic password".to_vec()))
    }
    fn put_native(&mut self, _: &str, plain: &[u8]) -> anyhow::Result<()> {
        if self.fail {
            anyhow::bail!("synthetic sink unavailable");
        }
        self.values.push(plain.into());
        Ok(())
    }
}
#[test]
fn secrets_require_explicit_one_time_prompt_and_never_put_plaintext_in_manifests() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    fs::create_dir_all(config.electron_profile.join("env")).unwrap();
    fs::write(
        config.electron_profile.join("env/synthetic.env"),
        format!("# synthetic\nTOKEN=enc:{VECTOR}\nLITERAL=\"enc:not-encrypted\"\n"),
    )
    .unwrap();
    use base64::Engine;
    let encrypted = base64::engine::general_purpose::STANDARD
        .decode(VECTOR)
        .unwrap();
    fs::create_dir_all(config.electron_profile.join("cloud/human")).unwrap();
    fs::write(
        config.electron_profile.join("cloud/session.bin"),
        &encrypted,
    )
    .unwrap();
    fs::write(
        config.electron_profile.join("cloud/human/synthetic.bin"),
        &encrypted,
    )
    .unwrap();
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"secret":VECTOR}),
    );
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::NeedsSecretConsent { interrupted: false }
    ));
    let mut access = SyntheticSecrets {
        reads: 0,
        values: Vec::new(),
        fail: false,
    };
    assert!(matches!(
        resume_with_secret_consent(&config, &mut access, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));
    assert_eq!(access.reads, 1);
    assert_eq!(
        access.values,
        vec![b"S6 synthetic secret; never a user credential".to_vec(); 4]
    );
    let locators: Vec<serde_json::Value> =
        read(&config.native_profile.join("secret-locators.json"));
    assert_eq!(locators.len(), 4);
    assert!(locators.iter().all(|v| v["service"]
        == format!("{} migration secrets", pm_first_launch::BETA_BUNDLE_ID)
        && v["account"].as_str().is_some_and(|s| s.len() == 64)
        && v["version"].as_str().is_some_and(|s| s.len() == 64)));
    run(&config, |_| Ok(())).unwrap();
    assert_eq!(access.reads, 1);
    for (p, bytes) in files(&config.native_profile) {
        assert!(
            !bytes
                .windows(b"S6 synthetic secret; never a user credential".len())
                .any(|b| b == b"S6 synthetic secret; never a user credential"),
            "plaintext leaked to {}",
            p.display()
        );
    }
}
#[test]
fn interrupted_secret_access_is_not_automatically_prompted_again() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"secret":VECTOR}),
    );
    run(&config, |_| Ok(())).unwrap();
    let mut access = SyntheticSecrets {
        reads: 0,
        values: Vec::new(),
        fail: true,
    };
    assert!(resume_with_secret_consent(&config, &mut access, |_| Ok(())).is_err());
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::NeedsSecretConsent { interrupted: true }
    ));
    let state: State = read(&config.native_profile.join("migration.json"));
    assert_eq!(state.secrets, SecretStep::AccessStarted);
    assert_eq!(access.reads, 1);
}
#[test]
fn kill_child() {
    let Some(root) = std::env::var_os("PM_FIRST_LAUNCH_KILL_ROOT") else {
        return;
    };
    let root = std::path::Path::new(&root);
    let config = pm_first_launch::Config {
        electron_profile: root.join("Electron"),
        native_profile: pm_first_launch::beta_profile(&root.join("Native")),
    };
    run(&config, |e| {
        if e == (Event::Progress {
            step: Step::Media,
            done: 1,
            total: 0,
        }) {
            fs::write(root.join("ready"), b"ready")?;
            loop {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    })
    .unwrap();
}
#[test]
fn sigkill_mid_media_resumes_every_durable_step() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let before = files(&config.electron_profile);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "kill_child", "--nocapture"])
        .env("PM_FIRST_LAUNCH_KILL_ROOT", t.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !t.path().join("ready").exists() {
        assert!(started.elapsed() < Duration::from_secs(20));
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    let result = child.wait().unwrap();
    assert!(!result.success());
    let state: State = read(&config.native_profile.join("migration.json"));
    assert!(state.completed.contains(&Step::Library));
    assert!(!state.completed.contains(&Step::Media));
    // Kernel policy requires BOTH a dead PID and 30s stale heartbeat. This is a
    // real elapsed-time reclaim, never editing the lock or weakening the rule.
    std::thread::sleep(Duration::from_secs(31));
    let result = run(&config, |_| Ok(())).unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
                projects: 1,
                blocked_owners: 0,
                ..
            }
        ),
        "{result:?}"
    );
    assert_eq!(before, files(&config.electron_profile));
    assert_eq!(
        LibraryStore::open(&config.native_profile)
            .projects()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn missing_slots_orphans_trash_and_rejected_history_are_retained_without_overwriting_external_files()
 {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let external = t.path().join("untouched.pmv");
    fs::write(&external, b"synthetic external bytes").unwrap();
    let mut project: Value = read(&config.electron_profile.join("store/project.p.json"));
    project["proj"]["id"] = "orphan".into();
    write(
        &config.electron_profile.join("store/project.orphan.json"),
        &project,
    );
    write(
        &config.electron_profile.join("store/projectTrash.json"),
        &json!([{"id":"orphan","name":"Trashed orphan"}]),
    );
    write(
        &config.electron_profile.join("store/projects.json"),
        &json!([{"id":"p","name":"P"},{"id":"missing","name":"Missing"}]),
    );
    write(
        &config
            .electron_profile
            .join("store/projectHistory.orphan.json"),
        &json!({"version":999,"authored":"retained"}),
    );
    let result = run(&config, |_| Ok(())).unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
                projects: 2,
                history_refusals: 1,
                missing_projects: 1,
                ..
            }
        ),
        "{result:?}"
    );
    let cp = checkpoint(&config);
    let native = LibraryStore::open(&config.native_profile);
    assert_eq!(native.projects().unwrap().len(), 1);
    assert_eq!(native.trash().unwrap()[0].id, cp.projects["orphan"]);
    let copied = native.load(&cp.projects["orphan"]).unwrap().unwrap();
    assert_eq!(
        copied.history.unwrap().get("version").unwrap().as_f64(),
        Some(999.0)
    );
    assert_eq!(fs::read(external).unwrap(), b"synthetic external bytes");
}
#[test]
fn original_lost_after_availability_checkpoint_blocks_only_its_project() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let original = t.path().join("original.mov");
    fs::write(&original, b"synthetic original").unwrap();
    let mut project: Value = read(&config.electron_profile.join("store/project.p.json"));
    project["proj"]["assets"]["proxy"]["sourcePath"] = original.to_str().unwrap().into();
    write(
        &config.electron_profile.join("store/project.p.json"),
        &project,
    );
    let result = run(&config, |e| {
        if e == Event::Published(Step::Library) {
            fs::remove_file(&original)?;
        }
        Ok(())
    })
    .unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
                projects: 1,
                blocked_owners: 1,
                ..
            }
        ),
        "{result:?}"
    );
    let state: Value = read(&config.native_profile.join("Media Store/migration.json"));
    let pass = state["passes"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(pass["blockedOwners"], json!(["p"]));
}
#[test]
fn source_linked_and_already_offline_profiles_work_without_indexeddb() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    fs::remove_dir_all(config.electron_profile.join("IndexedDB")).unwrap();
    let original = t.path().join("original.mov");
    fs::write(&original, b"synthetic original").unwrap();
    let mut project: Value = read(&config.electron_profile.join("store/project.p.json"));
    project["proj"]["assets"]["clip"]["sourcePath"] = original.to_str().unwrap().into();
    write(
        &config.electron_profile.join("store/project.p.json"),
        &project,
    );
    let result = run(&config, |_| Ok(())).unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
                projects: 1,
                blocked_owners: 0,
                ..
            }
        ),
        "{result:?}"
    );
    let state: Value = read(&config.native_profile.join("Media Store/migration.json"));
    let pass = state["passes"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert!(
        pass["closure"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["status"] == "verified-original")
    );
}

#[test]
fn kill_before_first_state_rename_resumes_only_own_bootstrap_artifacts() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    fs::create_dir_all(&config.native_profile).unwrap();
    fs::write(config.native_profile.join("first-launch.lock"), b"").unwrap();
    fs::write(
        config.native_profile.join("migration.first-launch-part"),
        b"{torn",
    )
    .unwrap();
    run(&config, |_| Ok(())).unwrap();
    assert_eq!(
        LibraryStore::open(&config.native_profile)
            .projects()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn linked_native_file_parent_cannot_create_folders_in_electron() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    run(&config, |e| {
        if e == Event::Published(Step::Checkpoint) {
            anyhow::bail!("stop");
        }
        Ok(())
    })
    .unwrap_err();
    symlink(
        &config.electron_profile,
        config.native_profile.join("Agent Workspaces"),
    )
    .unwrap();
    let before = files(&config.electron_profile);
    assert!(run(&config, |_| Ok(())).is_err());
    assert_eq!(before, files(&config.electron_profile));
}

#[test]
fn production_decrypt_path_refuses_wrong_password_version_and_padding() {
    use base64::Engine;
    let encrypted = base64::engine::general_purpose::STANDARD
        .decode(VECTOR)
        .unwrap();
    assert!(
        pm_first_launch::secrets::decrypt_mac(b"wrong synthetic password", &encrypted).is_err()
    );
    assert!(
        pm_first_launch::secrets::decrypt_mac(b"S6 synthetic password", b"v11not-mac").is_err()
    );
    assert!(
        pm_first_launch::secrets::decrypt_mac(
            b"S6 synthetic password",
            &encrypted[..encrypted.len() - 1]
        )
        .is_err()
    );
}
#[test]
fn multiple_snapshot_batches_share_one_verified_closure_pass() {
    use pm_native_migrate::{
        indexeddb,
        migration::{ClosureInput, OwnerKind, migrate_many},
        snapshot::{BlobCopy, Snapshot},
    };
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let dir = config
        .electron_profile
        .join("IndexedDB/app_powermove_0.indexeddb.leveldb");
    let blobs = config
        .electron_profile
        .join("IndexedDB/app_powermove_0.indexeddb.blob");
    let a = Snapshot::capture(&dir, Some(&blobs), &t.path().join("a"), BlobCopy::All).unwrap();
    let b = Snapshot::capture(&dir, Some(&blobs), &t.path().join("b"), BlobCopy::All).unwrap();
    let records = indexeddb::read(&a).unwrap().records;
    let closure = ClosureInput {
        inventoried: [
            OwnerKind::Current,
            OwnerKind::Trash,
            OwnerKind::Take,
            OwnerKind::Undo,
            OwnerKind::Redo,
            OwnerKind::Agent,
        ]
        .into_iter()
        .collect(),
        references: vec![],
    };
    let dest = t.path().join("media");
    let pass = migrate_many(
        &[(&a, &records[..1]), (&b, &records[1..])],
        &closure,
        &dest,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(pass.records.len(), 3);
    assert!(pass.cutover_ready);
    let before = fs::read(dest.join("migration.json")).unwrap();
    migrate_many(
        &[(&a, &records[..1]), (&b, &records[1..])],
        &closure,
        &dest,
        |_| panic!("no-op pass"),
    )
    .unwrap();
    assert_eq!(before, fs::read(dest.join("migration.json")).unwrap());
}

#[test]
fn interrupted_delta_keeps_unknown_quarantined_archives_and_beta_changes() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let unknown = config
        .electron_profile
        .join("store/future-private-shape.json");
    let quarantined = config
        .electron_profile
        .join("store/theme.json.corrupt-synthetic");
    fs::write(&unknown, b"opaque synthetic input\x00\xff").unwrap();
    fs::write(&quarantined, b"{synthetic torn JSON").unwrap();
    run(&config, |_| Ok(())).unwrap();
    let base = config.native_profile.join("first-launch-checkpoint");
    let archive = files(&base);
    assert_eq!(
        fs::read(base.join("profile/store/future-private-shape.json")).unwrap(),
        b"opaque synthetic input\x00\xff"
    );
    assert_eq!(
        fs::read(base.join("profile/store/theme.json.corrupt-synthetic")).unwrap(),
        b"{synthetic torn JSON"
    );
    // Simulate later Electron edits/removals after the immutable base is sealed.
    fs::remove_file(unknown).unwrap();
    fs::write(quarantined, b"later synthetic quarantine").unwrap();
    media_db(&config.electron_profile, true);
    let source = files(&config.electron_profile);
    let beta = config.native_profile.join("store/theme.json");
    fs::write(&beta, b"\"native authored setting\"").unwrap();
    let deleted = config.native_profile.join("store/ext.synthetic.json");
    fs::remove_file(&deleted).unwrap();
    let mut interrupted = false;
    assert!(
        run(&config, |event| {
            if matches!(
                event,
                Event::Progress {
                    step: Step::Media,
                    ..
                }
            ) && !interrupted
            {
                interrupted = true;
                anyhow::bail!("synthetic interrupted delta");
            }
            Ok(())
        })
        .is_err()
    );
    assert!(interrupted);
    assert!(
        config
            .native_profile
            .join("first-launch-deltas/pending/checkpoint.json")
            .is_file()
    );
    run(&config, |_| Ok(())).unwrap();
    assert_eq!(
        files(&base),
        archive,
        "the original 3-way base is immutable"
    );
    assert_eq!(
        files(&config.electron_profile),
        source,
        "migration never changes Electron"
    );
    assert_eq!(fs::read(beta).unwrap(), b"\"native authored setting\"");
    assert!(
        !deleted.exists(),
        "a delta must not resurrect beta tombstones"
    );
    assert!(
        !config
            .native_profile
            .join("first-launch-deltas/pending")
            .exists()
    );
    let deltas = fs::read_dir(config.native_profile.join("first-launch-deltas"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(deltas.len(), 1);
    assert_eq!(
        fs::read(
            deltas[0]
                .path()
                .join("profile/store/theme.json.corrupt-synthetic")
        )
        .unwrap(),
        b"later synthetic quarantine"
    );
    let media: Value = read(&config.native_profile.join("Media Store/migration.json"));
    assert_eq!(media["passes"].as_object().unwrap().len(), 2);
}
