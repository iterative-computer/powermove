mod support;
use pm_first_launch::{ConflictKind, DeltaPrepared, Outcome, State, delta, delta_if_changed, run};
use serde_json::json;
use std::fs;
use support::*;

#[test]
fn fresh_disabled_store_and_delta_preserve_native_edits() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile(temp.path());
    let source = config.electron_profile.join("extensions/synthetic");
    write(
        &source.join("manifest.json"),
        &json!({"id":"synthetic","name":"Synthetic effect","version":"1.0.0","apiVersion":1,"entry":"main.ts"}),
    );
    write(
        &source.join(".powermove-store.json"),
        &json!({"repoId":"synthetic-repo","releaseId":"synthetic-release"}),
    );
    write(
        &config.electron_profile.join("extensions-provenance.json"),
        &json!({"synthetic":{"trusted":false}}),
    );
    write(
        &config.electron_profile.join("store/extensions.json"),
        &json!({"synthetic":false}),
    );
    fs::create_dir_all(source.join("node_modules/cache")).unwrap();
    fs::write(source.join("node_modules/cache/ignored"), "cache").unwrap();
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));
    let active = config.native_profile.join("extensions/synthetic");
    assert!(active.join("main.ts").is_file());
    assert!(!active.join("node_modules").exists());
    assert!(active.join(".powermove-store.json").is_file());
    let provenance: serde_json::Value =
        read(&config.native_profile.join("extensions-provenance.json"));
    assert_eq!(provenance["synthetic"]["trusted"], false);
    let enabled: serde_json::Value = read(&config.native_profile.join("store/extensions.json"));
    assert_eq!(enabled["synthetic"], false);
    fs::write(source.join("main.ts"), "// electron update").unwrap();
    delta(&config, |_| Ok(())).unwrap();
    assert_eq!(
        fs::read_to_string(active.join("main.ts")).unwrap(),
        "// electron update"
    );
    fs::write(active.join("main.ts"), "// native edit").unwrap();
    fs::write(source.join("main.ts"), "// another electron update").unwrap();
    delta(&config, |_| Ok(())).unwrap();
    assert_eq!(
        fs::read_to_string(active.join("main.ts")).unwrap(),
        "// native edit"
    );
    let state: State = read(&config.native_profile.join("migration.json"));
    assert!(
        state
            .conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::File && c.id == "extensions/synthetic")
    );
    fs::remove_dir_all(&source).unwrap();
    delta(&config, |_| Ok(())).unwrap();
    assert!(active.join("main.ts").is_file());
}

#[test]
fn old_migration_receipt_repairs_even_when_source_stamp_is_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile(temp.path());
    run(&config, |_| Ok(())).unwrap();
    fs::remove_dir_all(config.native_profile.join("extensions")).unwrap();
    let path = config.native_profile.join("migration.json");
    let mut state: serde_json::Value = read(&path);
    for key in [
        "extensions_imported",
        "extension_receipts",
        "extension_provenance",
    ] {
        state.as_object_mut().unwrap().remove(key);
    }
    write(&path, &state);
    assert!(matches!(
        delta_if_changed(&config, |_| Ok(())).unwrap(),
        DeltaPrepared::MediaPending
    ));
    assert!(
        config
            .native_profile
            .join("extensions/synthetic/main.ts")
            .is_file()
    );
}
