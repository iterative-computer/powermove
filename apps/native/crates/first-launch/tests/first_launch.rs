mod support;
use pm_first_launch::{
    Event, Outcome, SHADER_CACHE, State, Step, resume_with_secret_consent, run,
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
        Some("Synthetic project")
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
        // A resumed bootstrap stays on its checkpoint. Once bootstrap is whole,
        // the same launch is a delta and brings the later change in.
        assert_eq!(
            LibraryStore::open(&config.native_profile)
                .read("theme")
                .unwrap()
                .unwrap()
                .as_str(),
            Some(if stop == Step::Secrets {
                "changed later"
            } else {
                "dark"
            })
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
        if matches!(
            e,
            Event::Progress {
                step: Step::Media,
                done: 1,
                ..
            }
        ) {
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
fn the_engines_shader_cache_written_before_the_gate_still_counts_as_fresh() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let cache = config.native_profile.join(SHADER_CACHE).join("glsl");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("program.json"), b"{}").unwrap();
    run(&config, |_| Ok(())).unwrap();
    assert_eq!(
        LibraryStore::open(&config.native_profile)
            .projects()
            .unwrap()
            .len(),
        1
    );
    // Anything else there is someone's work: still refused.
    let t = tempdir().unwrap();
    let config = profile(t.path());
    fs::create_dir_all(config.native_profile.join("notes")).unwrap();
    let error = run(&config, |_| Ok(())).unwrap_err();
    assert!(format!("{error:#}").contains("not a fresh native beta profile"));
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

#[test]
fn denied_or_locked_keychain_needs_fresh_consent_then_final_delta_without_source_changes() {
    struct Denied {
        reads: usize,
    }
    impl SecretAccess for Denied {
        fn legacy_password(&mut self) -> anyhow::Result<zeroize::Zeroizing<Vec<u8>>> {
            self.reads += 1;
            anyhow::bail!("synthetic Keychain denied or locked");
        }
        fn put_native(&mut self, _: &str, _: &[u8]) -> anyhow::Result<()> {
            panic!("denied Keychain cannot write native secrets");
        }
    }
    let t = tempdir().unwrap();
    let config = profile(&t.path().canonicalize().unwrap());
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"secret":VECTOR,"unknown":"preserved"}),
    );
    let before = files(&config.electron_profile);
    run(&config, |_| Ok(())).unwrap();
    let mut denied = Denied { reads: 0 };
    assert!(resume_with_secret_consent(&config, &mut denied, |_| Ok(())).is_err());
    for _ in 0..2 {
        assert!(matches!(
            run(&config, |_| Ok(())).unwrap(),
            Outcome::NeedsSecretConsent { interrupted: true }
        ));
    }
    assert_eq!(denied.reads, 1);
    assert!(!config.native_profile.join("secret-locators.json").exists());
    let mut consent = SyntheticSecrets {
        reads: 0,
        values: Vec::new(),
        fail: false,
    };
    resume_with_secret_consent(&config, &mut consent, |_| Ok(())).unwrap();
    assert_eq!(consent.reads, 1);
    assert_eq!(consent.values.len(), 1);
    assert_eq!(before, files(&config.electron_profile));
    media_db(&config.electron_profile, true);
    run(&config, |_| Ok(())).unwrap();
    assert_eq!(consent.reads, 1);
    let migration: Value = read(&config.native_profile.join("Media Store/migration.json"));
    assert_eq!(migration["passes"].as_object().unwrap().len(), 2);
}

#[test]
fn not_now_finishes_without_keychain_and_never_prompts_on_later_launch() {
    let t = tempdir().unwrap();
    let config = profile(&t.path().canonicalize().unwrap());
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"secret":VECTOR,"future":{"kept":true}}),
    );
    let source = files(&config.electron_profile);
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::NeedsSecretConsent { .. }
    ));
    let base = files(&config.native_profile.join("first-launch-checkpoint"));
    assert!(matches!(
        pm_first_launch::skip_secret_consent(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));
    let state: State = read(&config.native_profile.join("migration.json"));
    assert_eq!(state.secrets, SecretStep::Skipped);
    assert!(state.completed.contains(&Step::Secrets));
    assert!(state.imported_secrets.is_empty());
    assert!(!config.native_profile.join("secret-locators.json").exists());
    for _ in 0..2 {
        assert!(matches!(
            run(&config, |event| {
                assert_ne!(event, Event::SecretPromptRequired);
                Ok(())
            })
            .unwrap(),
            Outcome::Complete { .. }
        ));
    }
    assert_eq!(source, files(&config.electron_profile));
    assert_eq!(
        base,
        files(&config.native_profile.join("first-launch-checkpoint"))
    );
    let mut access = SyntheticSecrets {
        reads: 0,
        values: Vec::new(),
        fail: false,
    };
    pm_first_launch::retry_secret_consent(&config, &mut access, |_| Ok(())).unwrap();
    assert_eq!(access.reads, 1);
    assert_eq!(access.values.len(), 1);
    assert_eq!(source, files(&config.electron_profile));
}
#[test]
fn later_consent_reads_current_signout_and_keeps_old_ciphertext_archived() {
    let t = tempdir().unwrap();
    let config = profile(&t.path().canonicalize().unwrap());
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"secret":VECTOR,"future":42}),
    );
    run(&config, |_| Ok(())).unwrap();
    pm_first_launch::skip_secret_consent(&config, |_| Ok(())).unwrap();
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"future":42}),
    );
    let mut access = SyntheticSecrets {
        reads: 0,
        values: Vec::new(),
        fail: false,
    };
    pm_first_launch::retry_secret_consent(&config, &mut access, |_| Ok(())).unwrap();
    assert_eq!(access.reads, 0);
    assert!(access.values.is_empty());
    let old: Value = read(
        &config
            .native_profile
            .join("first-launch-checkpoint/profile/agent-provider.json"),
    );
    assert_eq!(old["secret"], VECTOR);
    let current: Value = read(
        &config
            .native_profile
            .join("secret-consent-retries/retry-0000000000000000/agent-provider.json"),
    );
    assert_eq!(current, json!({"future":42}));
}
#[test]
fn continue_without_them_after_denial_is_durable_even_if_completion_callback_fails() {
    let t = tempdir().unwrap();
    let config = profile(&t.path().canonicalize().unwrap());
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
    assert!(
        pm_first_launch::skip_secret_consent(&config, |event| {
            if event == Event::Published(Step::Secrets) {
                anyhow::bail!("synthetic interrupted completion");
            }
            Ok(())
        })
        .is_err()
    );
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));
    let state: State = read(&config.native_profile.join("migration.json"));
    assert_eq!(state.secrets, SecretStep::Skipped);
    assert_eq!(access.reads, 1);
}

#[test]
fn settings_retry_registers_through_attached_kernel_without_a_second_writer() {
    use pm_kernel::{
        KernelActor,
        persistence::DiskFormat,
        profile::{ProfileCall, ProfileReply},
        session::ShellCall,
    };
    let t = tempdir().unwrap();
    let config = profile(&t.path().canonicalize().unwrap());
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"secret":VECTOR}),
    );
    run(&config, |_| Ok(())).unwrap();
    pm_first_launch::skip_secret_consent(&config, |_| Ok(())).unwrap();
    let actor = KernelActor::spawn(KernelConfig::default());
    let handle = actor.handle();
    handle
        .shell(ShellCall::AttachLibrary {
            profile: config.native_profile.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    let mut access = SyntheticSecrets {
        reads: 0,
        values: Vec::new(),
        fail: false,
    };
    assert_eq!(
        pm_first_launch::retry_secret_consent_in_library(&config, &handle, &mut access, |_| Ok(()))
            .unwrap(),
        pm_first_launch::SecretRetryOutcome::Complete
    );
    let ProfileReply::Secrets(locators) = handle
        .profile(ProfileCall::SecretLocators {
            id: "agent-provider".into(),
        })
        .wait()
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(locators.len(), 1);
    assert_eq!(access.reads, 1);
    pm_first_launch::retry_secret_consent_in_library(&config, &handle, &mut access, |_| Ok(()))
        .unwrap();
    assert_eq!(access.reads, 1);
    assert_eq!(access.values.len(), 1);
}

#[test]
fn main_file_media_and_upgrade_receipt_are_preserved_and_never_reported_as_migrated() {
    let t = tempdir().unwrap();
    let config = profile(&t.path().canonicalize().unwrap());
    let record = json!({"key":"media:main-owned","revision":"main-rev","size":9,"type":"video/mp4","file":format!("{}.mp4","a".repeat(64)),"sha256":"a".repeat(64),"mtime":1,"future":{"kept":true}});
    write(
        &config
            .electron_profile
            .join(format!("Media Store/Index/{}.json", "b".repeat(64))),
        &record,
    );
    fs::write(
        config
            .electron_profile
            .join(format!("Media Store/{}.mp4", "a".repeat(64))),
        b"synthetic",
    )
    .unwrap();
    write(
        &config.electron_profile.join("storage-upgrade-v1.json"),
        &json!({"version":1,"origins":{"app://powermove":{"steps":["recovery","browser"],"beforeBytes":9}},"future":42}),
    );
    write(
        &config.electron_profile.join("store/storageSettings.json"),
        &json!({"future":{"quota":123}}),
    );
    let source = files(&config.electron_profile);
    let error = run(&config, |_| Ok(())).unwrap_err();
    assert_eq!(
        error.downcast_ref::<pm_first_launch::MigrationRequirement>(),
        Some(&pm_first_launch::MigrationRequirement::MainFileMediaAdapter)
    );
    let base = config
        .native_profile
        .join("first-launch-checkpoint/profile");
    assert_eq!(
        read::<Value>(&base.join(format!("Media Store/Index/{}.json", "b".repeat(64)))),
        record
    );
    assert_eq!(
        fs::read(base.join(format!("Media Store/{}.mp4", "a".repeat(64)))).unwrap(),
        b"synthetic"
    );
    assert_eq!(
        read::<Value>(&base.join("storage-upgrade-v1.json"))["future"],
        42
    );
    assert_eq!(
        read::<Value>(&base.join("store/storageSettings.json"))["future"]["quota"],
        123
    );
    assert_eq!(source, files(&config.electron_profile));
    assert!(!config.native_profile.join("Media Store/Index").exists());
    assert!(run(&config, |_| Ok(())).is_err());
}

#[test]
fn declining_after_partial_secret_import_retains_receipts_and_retries_only_remaining_values() {
    struct Partial {
        reads: usize,
        ids: std::collections::BTreeSet<String>,
        fail: bool,
    }
    impl SecretAccess for Partial {
        fn legacy_password(&mut self) -> anyhow::Result<zeroize::Zeroizing<Vec<u8>>> {
            self.reads += 1;
            Ok(zeroize::Zeroizing::new(b"S6 synthetic password".to_vec()))
        }
        fn put_native(&mut self, id: &str, _: &[u8]) -> anyhow::Result<()> {
            if self.fail && !self.ids.is_empty() {
                anyhow::bail!("synthetic denied second value");
            }
            assert!(
                self.ids.insert(id.into()),
                "completed receipt must prevent repeated writes"
            );
            Ok(())
        }
    }
    let t = tempdir().unwrap();
    let config = profile(&t.path().canonicalize().unwrap());
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"secret":VECTOR}),
    );
    use base64::Engine;
    fs::create_dir_all(config.electron_profile.join("cloud")).unwrap();
    fs::write(
        config.electron_profile.join("cloud/session.bin"),
        base64::engine::general_purpose::STANDARD
            .decode(VECTOR)
            .unwrap(),
    )
    .unwrap();
    let source = files(&config.electron_profile);
    run(&config, |_| Ok(())).unwrap();
    let mut access = Partial {
        reads: 0,
        ids: Default::default(),
        fail: true,
    };
    assert!(resume_with_secret_consent(&config, &mut access, |_| Ok(())).is_err());
    assert_eq!(access.ids.len(), 1);
    pm_first_launch::skip_secret_consent(&config, |_| Ok(())).unwrap();
    let state: State = read(&config.native_profile.join("migration.json"));
    assert_eq!(state.imported_secrets.len(), 1);
    assert_eq!(state.secrets, SecretStep::Skipped);
    access.fail = false;
    pm_first_launch::retry_secret_consent(&config, &mut access, |_| Ok(())).unwrap();
    assert_eq!(access.ids.len(), 2);
    assert_eq!(access.reads, 2);
    assert_eq!(source, files(&config.electron_profile));
}

#[test]
fn later_main_file_media_delta_stays_pending_and_preserves_native_edits() {
    let t = tempdir().unwrap();
    let config = profile(&t.path().canonicalize().unwrap());
    run(&config, |_| Ok(())).unwrap();
    let base = files(&config.native_profile.join("first-launch-checkpoint"));
    fs::write(
        config.native_profile.join("store/theme.json"),
        b"\"native-authored\"",
    )
    .unwrap();
    write(
        &config
            .electron_profile
            .join(format!("Media Store/Index/{}.json", "c".repeat(64))),
        &json!({"key":"main:new","sourcePath":t.path().join("unavailable.mov"),"future":true}),
    );
    let source = files(&config.electron_profile);
    let error = run(&config, |_| Ok(())).unwrap_err();
    assert_eq!(
        error.downcast_ref::<pm_first_launch::MigrationRequirement>(),
        Some(&pm_first_launch::MigrationRequirement::MainFileMediaAdapter)
    );
    assert!(
        config
            .native_profile
            .join("first-launch-deltas/pending/checkpoint.json")
            .is_file()
    );
    assert_eq!(
        files(&config.native_profile.join("first-launch-checkpoint")),
        base
    );
    assert_eq!(files(&config.electron_profile), source);
    assert_eq!(
        fs::read(config.native_profile.join("store/theme.json")).unwrap(),
        b"\"native-authored\""
    );
}

/// Originals in a folder macOS protects, with nothing captured to stand in.
fn protected_original(root: &std::path::Path) -> (pm_first_launch::Config, std::path::PathBuf) {
    let config = profile(root);
    fs::remove_dir_all(config.electron_profile.join("IndexedDB")).unwrap();
    let downloads = root.join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let original = downloads.join("original.mov");
    fs::write(&original, b"synthetic original").unwrap();
    let mut project: Value = read(&config.electron_profile.join("store/project.p.json"));
    project["proj"]["assets"]["clip"]["sourcePath"] = original.to_str().unwrap().into();
    write(
        &config.electron_profile.join("store/project.p.json"),
        &project,
    );
    (config, downloads)
}
fn guard(downloads: &std::path::Path) -> pm_first_launch::source_access::Scope {
    use pm_first_launch::source_access::{Choice, PROMPT_TIMEOUT, install_with};
    install_with(
        Choice::Ask,
        Default::default(),
        vec![(downloads.to_owned(), "Downloads".into())],
        PROMPT_TIMEOUT,
    )
}
#[test]
fn protected_originals_are_explained_first_and_skipped_ones_are_reported_and_kept() {
    use pm_first_launch::{run_with_folder_access, source_access::Choice};
    let t = tempdir().unwrap();
    let (config, downloads) = protected_original(t.path());
    let _guard = guard(&downloads);
    let mut events = Vec::new();
    let result = run(&config, |e| {
        events.push(e);
        Ok(())
    })
    .unwrap();
    let Outcome::NeedsFolderAccess { folders } = result else {
        panic!("{result:?}");
    };
    assert_eq!(folders, vec![("Downloads".to_owned(), 1)]);
    assert!(events.contains(&Event::FolderAccessRequired));
    assert!(events.contains(&Event::Progress {
        step: Step::Inventory,
        done: 0,
        total: 1
    }));
    let availability = config
        .native_profile
        .join("first-launch-checkpoint/availability.json");
    assert!(
        !availability.exists(),
        "nothing is decided before the person is"
    );
    // A relaunch before answering asks again; it never opens the folder.
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::NeedsFolderAccess { .. }
    ));
    let result = run_with_folder_access(&config, Choice::Skip, |_| Ok(())).unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
                projects: 1,
                blocked_owners: 0,
                not_reached: 1,
                ..
            }
        ),
        "{result:?}"
    );
    let state: State = read(&config.native_profile.join("migration.json"));
    assert_eq!(state.folder_access, Some(Choice::Skip));
    let closure: Value = read(&availability);
    let clip = closure["references"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["sourcePath"]
                .as_str()
                .is_some_and(|p| p.ends_with("original.mov"))
        })
        .unwrap();
    assert_eq!(clip["unreached"], true);
    let report = pm_first_launch::report::load(&config.native_profile).unwrap();
    assert!(report.rows.iter().any(|r| r.kind
        == pm_first_launch::surfaces::migration::ReportKind::NotReached
        && r.display_name == "original.mov"));
}
#[test]
fn allowed_protected_originals_are_copied() {
    use pm_first_launch::{run_with_folder_access, source_access::Choice};
    let t = tempdir().unwrap();
    let (config, downloads) = protected_original(t.path());
    let _guard = guard(&downloads);
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::NeedsFolderAccess { .. }
    ));
    let result = run_with_folder_access(&config, Choice::Allow, |_| Ok(())).unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
                blocked_owners: 0,
                not_reached: 0,
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
fn a_captured_payload_means_its_protected_original_is_never_opened() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    let downloads = t.path().join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let original = downloads.join("original.mov");
    fs::write(&original, b"anything").unwrap();
    let mut project: Value = read(&config.electron_profile.join("store/project.p.json"));
    project["proj"]["assets"]["clip"]["sourcePath"] = original.to_str().unwrap().into();
    write(
        &config.electron_profile.join("store/project.p.json"),
        &project,
    );
    let _guard = guard(&downloads);
    let result = run(&config, |_| Ok(())).unwrap();
    assert!(
        matches!(result, Outcome::Complete { not_reached: 0, .. }),
        "{result:?}"
    );
}
