mod support;
use pm_first_launch::{Outcome, run};
use pm_kernel::LibraryStore;
use serde_json::{Value, json};
use std::fs;
use support::*;

fn main_profile(root: &std::path::Path) -> pm_first_launch::Config {
    let config = profile(root);
    fs::remove_dir_all(config.electron_profile.join("IndexedDB")).unwrap();
    let records: Value = serde_json::from_str(include_str!("fixtures/main-media.json")).unwrap();
    for entry in records.as_array().unwrap() {
        let record = &entry["record"];
        let store = config.electron_profile.join("Media Store");
        fs::create_dir_all(store.join("Index")).unwrap();
        fs::write(
            store.join(record["file"].as_str().unwrap()),
            entry["payload"].as_str().unwrap(),
        )
        .unwrap();
        write(
            &store.join("Index").join(entry["index"].as_str().unwrap()),
            record,
        );
    }
    write(
        &config.electron_profile.join("storage-upgrade-v1.json"),
        &json!({"version":1,"origins":{"app://powermove":{"steps":["recovery","browser"],"beforeBytes":123,"future":true}}}),
    );
    write(
        &config.electron_profile.join("recovery/files.json"),
        &json!({"futureRecovery":true}),
    );
    write(
        &config.electron_profile.join("backups/p/uncertain.json"),
        &json!({"keep":"recovery bytes"}),
    );
    write(
        &config.electron_profile.join("project-files.json"),
        &json!({"p":{"path":"/synthetic/offline.pmv","hash":"retain"}}),
    );
    write(
        &config
            .electron_profile
            .join("project-files.json.pending-synthetic.json"),
        &json!({"projectId":"p","futurePublication":true}),
    );
    let workspace = json!({"id":"custom","name":"Custom","agentDockMigration":3,
        "layout":{"docks":[{"id":"left","panels":["assets"]},
        {"id":"center","panels":[{"id":"viewer","flex":true},{"id":"timeline","size":319}]},
        {"id":"right","panels":["inspector"]}]},"future":true});
    write(
        &config
            .electron_profile
            .join("store/projectWorkspace.p.json"),
        &workspace,
    );
    write(
        &config.electron_profile.join("store/workspaces.json"),
        &json!([workspace]),
    );
    config
}

#[test]
fn main_disk_media_receipts_recovery_and_project_workspaces_migrate_without_browser_storage() {
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let config = main_profile(&root);
    let before = files(&config.electron_profile);
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
    let cp = checkpoint(&config);
    for path in [
        "storage-upgrade-v1.json",
        "project-files.json",
        "project-files.json.pending-synthetic.json",
        "recovery/files.json",
        "backups/p/uncertain.json",
    ] {
        assert!(cp.files.contains_key(std::path::Path::new(path)), "{path}");
        assert_eq!(
            fs::read(config.electron_profile.join(path)).unwrap(),
            fs::read(config.native_profile.join("legacy-main").join(path)).unwrap()
        );
    }
    let state: Value = read(&config.native_profile.join("Media Store/migration.json"));
    let pass = state["passes"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(pass["diskRecords"].as_object().unwrap().len(), 3);
    assert!(
        pass["closure"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["status"] == "proxy-only")
    );
    let store = LibraryStore::open(&config.native_profile);
    let project = store.load(&cp.projects["p"]).unwrap().unwrap();
    let ws = project.workspace.unwrap();
    assert_eq!(ws.get("agentDockMigration").unwrap().as_f64(), Some(5.0));
    assert_eq!(ws.get("future"), Some(&pm_model::Value::Bool(true)));
    assert!(pm_model::json::to_canonical_string(&ws).contains("bottom"));
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::Complete {
            projects: 1,
            blocked_owners: 0,
            ..
        }
    ));
}

#[test]
fn checksum_failure_keeps_source_and_checkpoint_recoverable_and_retries_exact_bytes() {
    let t = private_tempdir();
    let config = main_profile(&t.path().canonicalize().unwrap());
    let records: Value = serde_json::from_str(include_str!("fixtures/main-media.json")).unwrap();
    let path = config
        .electron_profile
        .join("Media Store")
        .join(records[0]["record"]["file"].as_str().unwrap());
    fs::write(&path, b"corrupt").unwrap();
    let before = files(&config.electron_profile);
    assert!(run(&config, |_| Ok(())).is_err());
    assert_eq!(before, files(&config.electron_profile));
    assert!(
        config
            .native_profile
            .join("first-launch-checkpoint/checkpoint.json")
            .exists()
    );
    assert!(!config.native_profile.join("Media Store/keys").exists());
    // The immutable checkpoint is retained; changing the source later must not
    // make a previously captured corrupt payload look verified.
    fs::write(&path, records[0]["payload"].as_str().unwrap()).unwrap();
    assert!(run(&config, |_| Ok(())).is_err());
}

#[test]
fn verified_main_index_does_not_admit_unindexed_or_unknown_payloads() {
    for relative in ["unindexed.bin", "Index/future-record.json"] {
        let t = private_tempdir();
        let config = main_profile(&t.path().canonicalize().unwrap());
        let path = config.electron_profile.join("Media Store").join(relative);
        fs::write(&path, b"synthetic unrecognized media").unwrap();
        let source = files(&config.electron_profile);
        let error = run(&config, |_| Ok(())).unwrap_err();
        assert_eq!(
            error.downcast_ref::<pm_first_launch::MigrationRequirement>(),
            Some(&pm_first_launch::MigrationRequirement::MainFileMediaAdapter)
        );
        assert_eq!(source, files(&config.electron_profile));
        assert_eq!(
            fs::read(
                config
                    .native_profile
                    .join("first-launch-checkpoint/profile/Media Store")
                    .join(relative)
            )
            .unwrap(),
            b"synthetic unrecognized media"
        );
        assert!(!config.native_profile.join("Media Store/keys").exists());
        assert!(run(&config, |_| Ok(())).is_err());
    }
}

#[test]
fn checkpoint_restore_is_isolated_retains_interruption_and_refuses_corruption() {
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let config = main_profile(&root);
    run(&config, |_| Ok(())).unwrap();
    let source = config.native_profile.join("first-launch-checkpoint");
    let before = files(&source);
    let original = files(&config.electron_profile);
    let dest = root.join("recovered-checkpoint");
    assert!(
        pm_first_launch::recovery::restore_checkpoint(&source, &dest, || anyhow::bail!(
            "synthetic crash"
        ))
        .is_err()
    );
    let partial = root.join(".recovered-checkpoint.checkpoint-restoring");
    assert!(partial.is_dir());
    let retained = files(&partial);
    pm_first_launch::recovery::restore_checkpoint(&source, &dest, || Ok(())).unwrap();
    assert_eq!(files(&source), before);
    assert_eq!(files(&dest), before);
    assert_eq!(files(&config.electron_profile), original);
    assert!(
        fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with(".recovered-checkpoint.checkpoint-restoring")
                && e.path() != partial
                && files(&e.path()) == retained)
    );
    assert!(pm_first_launch::recovery::restore_checkpoint(&source, &dest, || Ok(())).is_err());
    assert!(
        pm_first_launch::recovery::restore_checkpoint(
            &source,
            &config.electron_profile.join("unsafe"),
            || Ok(())
        )
        .is_err()
    );
    let path = dest.join("profile/store/project.p.json");
    fs::write(&path, b"changed").unwrap();
    assert!(
        pm_first_launch::recovery::restore_checkpoint(&dest, &root.join("refused"), || Ok(()))
            .is_err()
    );
    assert_eq!(files(&config.electron_profile), original);
}

#[test]
fn checkpoint_recovery_refuses_foreign_stage_linked_parent_and_destination_collision() {
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let config = main_profile(&root);
    run(&config, |_| Ok(())).unwrap();
    let source = config.native_profile.join("first-launch-checkpoint");
    let before = files(&source);
    let dest = root.join("foreign");
    let stage = root.join(".foreign.checkpoint-restoring");
    fs::create_dir(&stage).unwrap();
    fs::write(stage.join("unrelated"), b"keep").unwrap();
    assert!(pm_first_launch::recovery::restore_checkpoint(&source, &dest, || Ok(())).is_err());
    assert_eq!(fs::read(stage.join("unrelated")).unwrap(), b"keep");
    let alias = root.join("alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    assert!(
        pm_first_launch::recovery::restore_checkpoint(&source, &alias.join("linked"), || Ok(()))
            .is_err()
    );
    assert!(!root.join("linked").exists());
    let collision = root.join("collision");
    let mut once = false;
    assert!(
        pm_first_launch::recovery::restore_checkpoint(&source, &collision, || {
            if !once {
                fs::create_dir(&collision)?;
                fs::write(collision.join("foreign"), b"keep")?;
                once = true;
            }
            Ok(())
        })
        .is_err()
    );
    assert_eq!(fs::read(collision.join("foreign")).unwrap(), b"keep");
    assert_eq!(files(&source), before);
}

fn private_tempdir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

#[test]
fn full_main_source_link_identity_migrates_without_a_sampled_fingerprint() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let config = main_profile(&root);
    let records: Value = serde_json::from_str(include_str!("fixtures/main-media.json")).unwrap();
    let entry = &records[0];
    let mut record = entry["record"].clone();
    let source = root.join("external-original.mov");
    fs::rename(
        config
            .electron_profile
            .join("Media Store")
            .join(record["file"].as_str().unwrap()),
        &source,
    )
    .unwrap();
    record.as_object_mut().unwrap().remove("file");
    record["sourcePath"] = json!(source);
    write(
        &config
            .electron_profile
            .join("Media Store/Index")
            .join(entry["index"].as_str().unwrap()),
        &record,
    );
    let mut proxy = records[1]["record"].clone();
    let proxy_source = root.join("external-proxy.mov");
    fs::rename(
        config
            .electron_profile
            .join("Media Store")
            .join(proxy["file"].as_str().unwrap()),
        &proxy_source,
    )
    .unwrap();
    proxy.as_object_mut().unwrap().remove("file");
    proxy["sourcePath"] = json!(proxy_source);
    write(
        &config
            .electron_profile
            .join("Media Store/Index")
            .join(records[1]["index"].as_str().unwrap()),
        &proxy,
    );
    let project_path = config.electron_profile.join("store/project.p.json");
    let mut project: Value = read(&project_path);
    project["proj"]["assets"]["clip"]
        .as_object_mut()
        .unwrap()
        .remove("fingerprint");
    write(&project_path, &project);
    let before = files(&config.electron_profile);
    run(&config, |_| Ok(())).unwrap();
    let manifest: Value = read(&config.native_profile.join("Media Store/migration.json"));
    let pass = manifest["passes"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let closure = pass["closure"].as_array().unwrap();
    assert!(
        closure
            .iter()
            .any(|e| e["reference"]["key"] == "media:original"
                && e["reference"]["poster"] == false
                && e["status"] == "verified-original")
    );
    assert!(
        closure
            .iter()
            .any(|e| e["reference"]["key"] == "media:proxy" && e["status"] == "proxy-only")
    );
    assert_eq!(before, files(&config.electron_profile));
    assert_eq!(fs::read(source).unwrap(), b"synthetic original");
    assert_eq!(fs::read(proxy_source).unwrap(), b"synthetic playback proxy");
    assert_eq!(
        pass["diskRecords"]["media:proxy"]["source"],
        "main-source-link"
    );
}

/// An original from main's Media Store that only lives in a folder the person
/// skipped is "not reached": offline with its source kept, listed in the
/// report, and never blocking its project.
#[test]
fn skipped_source_linked_media_is_not_reached_and_never_blocks() {
    use pm_first_launch::{
        Outcome, run_with_folder_access,
        source_access::{self, Choice, PROMPT_TIMEOUT},
        surfaces::migration::ReportKind,
    };
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let config = main_profile(&root);
    let records: Value = serde_json::from_str(include_str!("fixtures/main-media.json")).unwrap();
    let entry = &records[0];
    let mut record = entry["record"].clone();
    let downloads = root.join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let source = downloads.join("external-original.mov");
    fs::rename(
        config
            .electron_profile
            .join("Media Store")
            .join(record["file"].as_str().unwrap()),
        &source,
    )
    .unwrap();
    record.as_object_mut().unwrap().remove("file");
    record["sourcePath"] = json!(source);
    write(
        &config
            .electron_profile
            .join("Media Store/Index")
            .join(entry["index"].as_str().unwrap()),
        &record,
    );
    let _guard = source_access::install_with(
        Choice::Ask,
        Default::default(),
        vec![(downloads.clone(), "Downloads".into())],
        PROMPT_TIMEOUT,
    );
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::NeedsFolderAccess { .. }
    ));
    let outcome = run_with_folder_access(&config, Choice::Skip, |_| Ok(())).unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::Complete {
                blocked_owners: 0,
                not_reached: 1,
                ..
            }
        ),
        "{outcome:?}"
    );
    let manifest: Value = read(&config.native_profile.join("Media Store/migration.json"));
    let pass = manifest["passes"]
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
            .any(|e| e["reference"]["key"] == "media:original" && e["status"] == "not-reached"),
        "{pass:#}"
    );
    assert_eq!(pass["blockedOwners"], json!([]));
    let report = pm_first_launch::report::load(&config.native_profile).unwrap();
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.kind == ReportKind::NotReached && r.display_name == "external-original.mov"),
        "{:?}",
        report.rows
    );
    // The answer is kept per folder: a later run never asks about it again.
    let state: pm_first_launch::State = read(&config.native_profile.join("migration.json"));
    assert_eq!(
        state.folder_outcomes.get(&downloads),
        Some(&source_access::Outcome::Skipped)
    );
    assert_eq!(
        fs::read(source).unwrap(),
        b"synthetic original",
        "the original is untouched"
    );
}
