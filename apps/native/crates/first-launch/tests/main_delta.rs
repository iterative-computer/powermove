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
    ] {
        assert!(cp.files.contains_key(std::path::Path::new(path)), "{path}");
        assert_eq!(
            fs::read(config.electron_profile.join(path)).unwrap(),
            fs::read(config.native_profile.join("legacy-main").join(path)).unwrap()
        );
    }
    for excluded in [
        "recovery",
        "backups",
        "Agent Workspaces",
        "transcription-cache",
    ] {
        assert!(!cp.files.keys().any(|p| p.starts_with(excluded)));
        assert!(!config.native_profile.join(excluded).exists());
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
fn size_mismatch_is_offline_and_checksum_verification_is_deferred_until_adoption() {
    let t = private_tempdir();
    let config = main_profile(&t.path().canonicalize().unwrap());
    let records: Value = serde_json::from_str(include_str!("fixtures/main-media.json")).unwrap();
    let record = &records[0];
    let path = config
        .electron_profile
        .join("Media Store")
        .join(record["record"]["file"].as_str().unwrap());
    fs::write(&path, b"corrupt").unwrap();
    let before = files(&config.electron_profile);
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));
    assert_eq!(before, files(&config.electron_profile));
    assert!(
        pm_native_migrate::linked_summary(&config.native_profile.join("Media Store"))
            .unwrap()
            .total_items
            < 3
    );
}

#[test]
fn unindexed_payloads_are_left_in_source_and_unknown_index_metadata_is_refused() {
    let t = private_tempdir();
    let config = main_profile(&t.path().canonicalize().unwrap());
    let path = config.electron_profile.join("Media Store/unindexed.bin");
    fs::write(&path, b"synthetic unrecognized media").unwrap();
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));
    assert!(
        !checkpoint(&config)
            .files
            .contains_key(std::path::Path::new("Media Store/unindexed.bin"))
    );
    assert_eq!(fs::read(path).unwrap(), b"synthetic unrecognized media");
    let other = private_tempdir();
    let config = main_profile(&other.path().canonicalize().unwrap());
    let unknown = config
        .electron_profile
        .join("Media Store/Index/future-record.json");
    fs::write(&unknown, b"synthetic unrecognized metadata").unwrap();
    assert!(run(&config, |_| Ok(())).is_err());
    assert_eq!(
        fs::read(
            config
                .native_profile
                .join("first-launch-checkpoint/profile/Media Store/Index/future-record.json")
        )
        .unwrap(),
        fs::read(unknown).unwrap()
    );
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
    assert_eq!(pass["diskRecords"]["media:proxy"]["source"], "linked");
}

/// Metadata-only links do not open guarded source folders or ask for read access.
#[test]
fn guarded_main_source_links_are_published_without_opening_the_original() {
    use pm_first_launch::{
        Outcome,
        source_access::{self, Choice, PROMPT_TIMEOUT},
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
    let outcome = run(&config, |_| Ok(())).unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::Complete {
                blocked_owners: 0,
                not_reached: 0,
                ..
            }
        ),
        "{outcome:?}"
    );
    let state: pm_first_launch::State = read(&config.native_profile.join("migration.json"));
    assert!(state.folder_outcomes.is_empty());
    assert!(
        pm_first_launch::waiting_folders(&config)
            .unwrap()
            .is_empty()
    );
    let summary =
        pm_native_migrate::linked_summary(&config.native_profile.join("Media Store")).unwrap();
    assert_eq!(summary.folders[&downloads].items, 1);
    assert_eq!(fs::read(&source).unwrap(), b"synthetic original");
}

/// A profile written by the shipped app at current main (FileStore through
/// main's Projects/takes, gzip disk envelopes, separate and combined session
/// history, journal, and a storage-upgrade checkpoint paused at its browser
/// stage; `pm-kernel/tests/main_current/generate.mjs`) migrates without a
/// changed source byte, keeps the checkpoint exactly, and its project loads
/// with every field of main's document.
#[test]
fn current_main_profile_records_migrate_with_every_project_field_and_the_checkpoint_kept() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kernel/tests/fixtures/main-current");
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let config = profile(&root);
    let store = config.electron_profile.join("store");
    for entry in fs::read_dir(fixtures.join("profile/store")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "projects.json" {
            fs::copy(entry.path(), store.join(entry.file_name())).unwrap();
        }
    }
    // Both registries' projects: the synthetic base profile's and main's.
    let mut registry: Value = read(&store.join("projects.json"));
    let main: Value = read(&fixtures.join("profile/store/projects.json"));
    registry
        .as_array_mut()
        .unwrap()
        .extend(main.as_array().unwrap().iter().cloned());
    write(&store.join("projects.json"), &registry);
    let checkpoint_bytes = fs::read(fixtures.join("profile/storage-upgrade-v1.json")).unwrap();
    fs::write(
        config.electron_profile.join("storage-upgrade-v1.json"),
        &checkpoint_bytes,
    )
    .unwrap();

    let before = files(&config.electron_profile);
    let result = run(&config, |_| Ok(())).unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
                projects: 2,
                blocked_owners: 0,
                ..
            }
        ),
        "{result:?}"
    );
    assert_eq!(
        before,
        files(&config.electron_profile),
        "source profile changed"
    );
    assert_eq!(
        fs::read(
            config
                .native_profile
                .join("legacy-main/storage-upgrade-v1.json")
        )
        .unwrap(),
        checkpoint_bytes,
        "the storage-upgrade checkpoint must be kept exactly"
    );

    // Main's project: the gzip slot decoded, its journal not newer than the
    // slot, and the same document main saved to its project file.
    let saved = {
        let file = fs::File::open(fixtures.join("current.pmv")).unwrap();
        pm_model::container::read_container(
            &file,
            pm_model::container::ReadOptions { verify_media: true },
        )
        .unwrap()
        .document
    };
    let cp = checkpoint(&config);
    let library = LibraryStore::open(&config.native_profile);
    let project = library.load(&cp.projects["P-1"]).unwrap().unwrap();
    // The library keys the project by its migrated id (the checkpoint maps
    // main's id to it); every other field is main's.
    assert_eq!(
        project.project.get("id").and_then(pm_model::Value::as_str),
        Some(cp.projects["P-1"].as_str())
    );
    let mut changed = vec![];
    diff_paths(
        &project.project,
        saved.get("proj").unwrap(),
        "proj",
        &mut changed,
    );
    assert_eq!(changed, ["proj.id"], "migration changed main's project");
    let history = project.history.expect("main's session history");
    let history = pm_model::json::to_canonical_string(&history);
    assert!(history.contains("Renamed then undone"), "{history}");
    let workspace = project.workspace.expect("main's project workspace");
    assert_eq!(
        workspace
            .get("futureWorkspaceField")
            .and_then(pm_model::Value::as_f64),
        Some(1.0)
    );
    // Takes (gzip) carry over decodable; a second run is idempotent.
    let takes = pm_model::json::parse(
        &fs::read_to_string(config.native_profile.join("store/takes.json")).unwrap(),
    )
    .unwrap();
    let takes = pm_kernel::library::decode_disk_value(takes, 768 << 20).unwrap();
    assert!(pm_model::json::to_canonical_string(&takes).contains("Before review"));
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { projects: 2, .. }
    ));
}

/// Paths (`a.b.3.c`) at which two JSON values differ.
fn diff_paths(a: &pm_model::Value, b: &pm_model::Value, at: &str, out: &mut Vec<String>) {
    use pm_model::Value as V;
    match (a, b) {
        (V::Object(x), V::Object(y)) => {
            let mut keys: Vec<String> = x
                .keys()
                .chain(y.keys())
                .map(|k| k.to_string_lossy())
                .collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                match (x.get(k.as_str()), y.get(k.as_str())) {
                    (Some(p), Some(q)) => diff_paths(p, q, &format!("{at}.{k}"), out),
                    _ => out.push(format!("{at}.{k}")),
                }
            }
        }
        (V::Array(x), V::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                diff_paths(p, q, &format!("{at}.{i}"), out);
            }
        }
        _ if a == b => {}
        _ => out.push(at.to_owned()),
    }
}

/// A profile whose media the new engine's development builds kept in main's
/// content-addressed store (`Media Store/keys` + `objects`, the native
/// library's own layout) and nowhere else: its originals come across verified
/// and the project's assets resolve; a key whose object has other bytes stays
/// offline without blocking the copy.
#[test]
fn content_addressed_main_store_media_migrate_as_verified_originals() {
    use sha2::{Digest, Sha256};
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let config = profile(&root);
    let source = &config.electron_profile;
    let original =
        fs::read(source.join("IndexedDB/app_powermove_0.indexeddb.blob/1/00/2")).unwrap();
    fs::remove_dir_all(source.join("IndexedDB")).unwrap();
    let store = source.join("Media Store");
    let put = |key: &str, bytes: &[u8], named: &[u8]| {
        let hash = blake3::hash(named).to_hex().to_string();
        fs::create_dir_all(store.join("objects").join(&hash[..2])).unwrap();
        fs::create_dir_all(store.join("keys")).unwrap();
        fs::write(store.join("objects").join(&hash[..2]).join(&hash), bytes).unwrap();
        write(
            &store
                .join("keys")
                .join(format!("{:x}.json", Sha256::digest(key.as_bytes()))),
            &json!({"key":key,"hash":hash,"size":bytes.len(),"type":"video/quicktime",
                "revision":"synthetic-revision","at":1,"source":"stream"}),
        );
    };
    put("media:original", &original, &original);
    // Named for other bytes than it holds: not media the store vouches for.
    put("content-key", b"tampered", b"what it should be");
    let result = run(&config, |_| Ok(())).unwrap();
    assert!(
        matches!(
            result,
            Outcome::Complete {
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
        pass["diskRecords"].get("media:original").is_some(),
        "{pass}"
    );
    assert!(
        pass["diskRecords"].get("content-key").is_some(),
        "metadata linking defers checksums to adoption: {pass}"
    );
    let clip = pass["closure"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            e["reference"]["key"] == "media:original" && e["reference"]["ownerKind"] == "current"
        })
        .unwrap();
    assert_eq!(clip["status"], "verified-original", "{clip}");
    // The native library's store now holds the bytes under the asset's key.
    let key: Value = read(&config.native_profile.join(format!(
        "Media Store/keys/{:x}.json",
        Sha256::digest(b"media:original")
    )));
    assert_eq!(fs::read(key["path"].as_str().unwrap()).unwrap(), original);
    assert!(
        pm_native_migrate::adopt_linked(&config.native_profile.join("Media Store"), |_, _| Ok(()))
            .is_err(),
        "corrupt content-addressed bytes are refused at adoption"
    );
}

#[test]
fn a_leftover_content_named_payload_no_record_names_does_not_stop_the_copy() {
    let t = private_tempdir();
    let config = main_profile(&t.path().canonicalize().unwrap());
    let name = format!("{}.webm", "d".repeat(64));
    fs::write(
        config.electron_profile.join("Media Store").join(&name),
        b"leftover",
    )
    .unwrap();
    let source = files(&config.electron_profile);
    run(&config, |_| Ok(())).unwrap();
    assert_eq!(source, files(&config.electron_profile));
    // Unreferenced payloads remain solely in the original profile.
    assert!(
        !config
            .native_profile
            .join("first-launch-checkpoint/profile/Media Store")
            .join(&name)
            .exists()
    );
}
