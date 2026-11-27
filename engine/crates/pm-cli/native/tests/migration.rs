use pm_native_cli::{migration, options::Options};
use serde_json::{Value, json};
use std::{ffi::OsString, fs, path::Path};
use tempfile::tempdir;
fn options(source: &Path, scratch: &Path) -> Options {
    Options::parse(
        [
            OsString::from("migrate"),
            source.as_os_str().into(),
            OsString::from("--dry-run"),
            OsString::from("--scratch"),
            scratch.as_os_str().into(),
        ],
        &mut &b""[..],
    )
    .unwrap()
}
fn profile(root: &Path) {
    fs::create_dir_all(root.join("store")).unwrap();
    fs::write(
        root.join("store/projects.json"),
        r#"[{"id":"synthetic","name":"Synthetic migration project"}]"#,
    )
    .unwrap();
    let project = json!({"id":"synthetic","name":"Synthetic migration project","w":320,"h":180,"fps":30,"dur":2,"layers":[],"assets":{},"comps":{},"markers":[]});
    fs::write(
        root.join("store/project.synthetic.json"),
        project.to_string(),
    )
    .unwrap();
    fs::write(
        root.join("store/unknown-synthetic-key.json"),
        r#"{"keep":"exact authored value"}"#,
    )
    .unwrap();
}
#[test]
fn dry_run_invokes_real_first_launch_only_on_a_verified_copy() {
    let t = tempdir().unwrap();
    let source = t.path().join("source");
    profile(&source);
    let scratch = t.path().join("dry-run");
    let before = fs::read(source.join("store/project.synthetic.json")).unwrap();
    let result = migration::dry_run(&options(&source, &scratch)).unwrap();
    assert_eq!(result["dryRun"], true);
    assert_eq!(result["sourceUnchanged"], true);
    assert_eq!(result["keychainAccessed"], false);
    assert_eq!(result["status"], "complete");
    assert_eq!(result["counts"]["projects"], 1);
    assert_eq!(
        fs::read(source.join("store/project.synthetic.json")).unwrap(),
        before
    );
    assert!(!source.join("migration.json").exists());
    assert!(
        scratch
            .join("electron-copy/store/unknown-synthetic-key.json")
            .is_file()
    );
    let native = scratch.join(pm_first_launch::BETA_BUNDLE_ID);
    let state: Value =
        serde_json::from_slice(&fs::read(native.join("migration.json")).unwrap()).unwrap();
    assert!(
        state["completed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "library")
    );
    assert!(scratch.join("cli-dry-run.json").is_file());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(scratch).unwrap().permissions().mode() & 0o777,
        0o700
    );
}
#[test]
fn migration_refuses_overlapping_and_existing_scratch_without_mutation() {
    let t = tempdir().unwrap();
    let source = t.path().join("source");
    profile(&source);
    assert!(migration::dry_run(&options(&source, &source.join("nested"))).is_err());
    assert!(!source.join("nested").exists());
    let scratch = t.path().join("existing");
    fs::create_dir(&scratch).unwrap();
    fs::write(scratch.join("authored"), b"keep").unwrap();
    assert!(migration::dry_run(&options(&source, &scratch)).is_err());
    assert_eq!(fs::read(scratch.join("authored")).unwrap(), b"keep");
}
#[test]
fn migration_refuses_symlinks_without_following_or_creating_scratch() {
    use std::os::unix::fs::symlink;
    let t = tempdir().unwrap();
    let source = t.path().join("source");
    profile(&source);
    let external = t.path().join("external");
    fs::write(&external, b"synthetic external").unwrap();
    symlink(&external, source.join("linked")).unwrap();
    let scratch = t.path().join("scratch");
    assert!(migration::dry_run(&options(&source, &scratch)).is_err());
    assert!(!scratch.exists());
    assert_eq!(fs::read(external).unwrap(), b"synthetic external");
}
#[test]
fn migration_waits_for_a_live_electron_lock_before_any_copy() {
    use std::os::unix::fs::symlink;
    let t = tempdir().unwrap();
    let source = t.path().join("source");
    profile(&source);
    symlink("unknown-host-12345", source.join("SingletonLock")).unwrap();
    let scratch = t.path().join("scratch");
    let error = migration::dry_run(&options(&source, &scratch)).unwrap_err();
    assert_eq!(error.code, "profile_busy");
    assert!(!scratch.exists());
    assert!(
        fs::symlink_metadata(source.join("SingletonLock"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
