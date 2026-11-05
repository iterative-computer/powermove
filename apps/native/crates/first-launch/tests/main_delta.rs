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
    let t = tempfile::tempdir().unwrap();
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
    let t = tempfile::tempdir().unwrap();
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
        let t = tempfile::tempdir().unwrap();
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
