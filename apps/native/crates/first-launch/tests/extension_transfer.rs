use pm_first_launch::{Config, extension_transfer::*};
use std::{fs, path::Path};
fn fixture() -> (tempfile::TempDir, Config) {
    let t = tempfile::tempdir().unwrap();
    let c = Config {
        electron_profile: t.path().join("electron"),
        native_profile: t.path().join("old-native"),
    };
    fs::create_dir_all(c.electron_profile.join("extensions/effect")).unwrap();
    fs::create_dir_all(c.native_profile.join("extensions/youtube-import")).unwrap();
    fs::write(
        c.electron_profile.join("extensions/effect/manifest.json"),
        r#"{"id":"effect","version":"1"}"#,
    )
    .unwrap();
    fs::write(
        c.electron_profile.join("extensions-provenance.json"),
        r#"{"effect":{"trusted":true}}"#,
    )
    .unwrap();
    fs::write(
        c.native_profile
            .join("extensions/youtube-import/manifest.json"),
        "native",
    )
    .unwrap();
    (t, c)
}

fn snapshot(root: &Path, scoped: bool) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(
        root: &Path,
        dir: &Path,
        scoped: bool,
        out: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(root).unwrap();
            if scoped
                && [
                    "extensions",
                    "extensions-provenance.json",
                    "extension-import",
                    "first-launch.lock",
                ]
                .iter()
                .any(|name| rel.starts_with(name))
            {
                continue;
            }
            if p.is_dir() {
                out.insert(rel.into(), vec![]);
                visit(root, &p, scoped, out);
            } else {
                out.insert(rel.into(), fs::read(&p).unwrap());
            }
        }
    }
    let mut out = Default::default();
    visit(root, root, scoped, &mut out);
    out
}

fn check(_: &Path, _: &serde_json::Value) -> Result<String, String> {
    Ok("Activated in test realm".into())
}
#[test]
fn existing_profile_import_and_write_scope() {
    let (_t, c) = fixture();
    let mut protected = vec![];
    for p in [
        "store/projects.json",
        "store/project.test.json",
        "store/projectHistory.test.json",
        "store/settings.json",
        "store/workspaces.json",
        "claude-runtime/.credentials.json",
        "codex-runtime/auth.json",
        "session.json",
    ] {
        let path = c.native_profile.join(p);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("protected {p}")).unwrap();
        protected.push((path.clone(), fs::read(path).unwrap()));
    }
    let before = snapshot(&c.native_profile, true);
    let source_before = snapshot(&c.electron_profile, false);
    let mut events = vec![];
    let out = run(&c, |p| events.push(p), check).unwrap();
    assert_eq!(before, snapshot(&c.native_profile, true));
    assert_eq!(source_before, snapshot(&c.electron_profile, false));
    assert_eq!(out.brought_in, 1);
    assert_eq!(out.could_not_load, 0);
    assert_eq!(events.last().unwrap().done, events.last().unwrap().total);
    for (p, bytes) in protected {
        assert_eq!(fs::read(p).unwrap(), bytes);
    }
    assert!(!c.native_profile.join("migration.json").exists());
    assert_eq!(
        fs::read(c.native_profile.join("extensions-provenance.json")).unwrap(),
        fs::read(c.electron_profile.join("extensions-provenance.json")).unwrap()
    );
    let allowed = [
        "extensions",
        "extensions-provenance.json",
        "extension-import",
        "first-launch.lock",
        "store",
        "claude-runtime",
        "codex-runtime",
        "session.json",
    ];
    for p in fs::read_dir(&c.native_profile).unwrap() {
        assert!(allowed.contains(&p.unwrap().file_name().to_str().unwrap()));
    }
}
#[test]
fn declined_then_no_repeat_offer_and_settings_reset() {
    let (_t, c) = fixture();
    assert!(scan(&c).unwrap().should_offer());
    set_declined(&c.native_profile, true).unwrap();
    assert!(declined(&c.native_profile).unwrap());
    assert!(!scan(&c).unwrap().should_offer());
    set_declined(&c.native_profile, false).unwrap();
    assert!(scan(&c).unwrap().should_offer());
}
#[test]
fn rerun_only_new_or_changed_files_and_retains_native_edits() {
    let (_t, c) = fixture();
    run(&c, |_| {}, check).unwrap();
    let path = c.native_profile.join("extensions/effect/manifest.json");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    assert!(!scan(&c).unwrap().has_new_or_changed());
    let out = run(&c, |_| {}, check).unwrap();
    assert_eq!(out.brought_in, 0);
    assert_eq!(out.already_present, 1);
    assert_eq!(modified, fs::metadata(&path).unwrap().modified().unwrap());
    fs::write(
        c.electron_profile.join("extensions/effect/manifest.json"),
        r#"{"id":"effect","version":"2"}"#,
    )
    .unwrap();
    assert_eq!(scan(&c).unwrap().changed, 1);
    assert_eq!(run(&c, |_| {}, check).unwrap().brought_in, 1);
    fs::write(&path, "native edit").unwrap();
    assert!(
        scan(&c)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.status == ScanStatus::Skipped && e.reason.contains("your copy is kept"))
    );
    run(&c, |_| {}, check).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "native edit");
}
#[test]
fn existing_youtube_conflict_never_overwritten() {
    let (_t, c) = fixture();
    fs::create_dir_all(c.electron_profile.join("extensions/youtube-import")).unwrap();
    fs::write(
        c.electron_profile
            .join("extensions/youtube-import/manifest.json"),
        r#"{"id":"youtube-import"}"#,
    )
    .unwrap();
    let out = run(&c, |_| {}, check).unwrap();
    assert!(
        out.entries
            .iter()
            .any(|e| e.id == "youtube-import" && e.outcome == TransferOutcome::Skipped)
    );
    assert_eq!(
        fs::read_to_string(
            c.native_profile
                .join("extensions/youtube-import/manifest.json")
        )
        .unwrap(),
        "native"
    );
}
#[test]
fn missing_or_unreadable_source_has_reason_and_run_writes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let (_t, c) = fixture();
    let ext = c.electron_profile.join("extensions");
    fs::set_permissions(&ext, fs::Permissions::from_mode(0)).unwrap();
    assert!(scan(&c).unwrap().source_issue.is_some());
    assert!(run(&c, |_| {}, check).is_err());
    assert!(!c.native_profile.join("extension-import").exists());
    fs::set_permissions(&ext, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(&c.electron_profile).unwrap();
    assert!(scan(&c).unwrap().source_issue.is_some());
    assert!(run(&c, |_| {}, check).is_err());
}
#[test]
fn skips_empty_folder_and_returns_real_load_error() {
    let (_t, c) = fixture();
    fs::create_dir_all(c.electron_profile.join("extensions/.staging")).unwrap();
    assert!(
        scan(&c)
            .unwrap()
            .entries
            .iter()
            .all(|e| !e.id.starts_with('.'))
    );
    fs::create_dir_all(c.electron_profile.join("extensions/figma-layer-import")).unwrap();
    let out = run(
        &c,
        |_| {},
        |_, _| Err("requires radial-directional-displacement".into()),
    )
    .unwrap();
    assert_eq!(out.could_not_load, 1);
    assert!(
        out.entries
            .iter()
            .any(|e| e.reason.contains("empty or incomplete"))
    );
    assert!(out.entries.iter().any(|e| {
        e.outcome == TransferOutcome::CouldNotLoad
            && e.reason
                .contains("requires radial-directional-displacement")
    }));
    let out = run(&c, |_| {}, check).unwrap();
    assert_eq!(out.already_present, 1);
}
#[test]
fn overlapping_profiles_and_destination_links_are_refused() {
    let t = tempfile::tempdir().unwrap();
    assert!(
        scan(&Config {
            electron_profile: t.path().into(),
            native_profile: t.path().into()
        })
        .is_err()
    );
    let (_t, c) = fixture();
    fs::remove_dir_all(c.native_profile.join("extensions")).unwrap();
    std::os::unix::fs::symlink(
        c.electron_profile.join("extensions"),
        c.native_profile.join("extensions"),
    )
    .unwrap();
    assert!(run(&c, |_| {}, check).is_err());
    assert!(!c.native_profile.join("extension-import").exists());
}

#[test]
fn provenance_updates_without_overwriting_unrelated_native_trust() {
    let (_t, c) = fixture();
    run(&c, |_| {}, check).unwrap();
    fs::write(
        c.electron_profile.join("extensions-provenance.json"),
        r#"{"effect":{"trusted":false}}"#,
    )
    .unwrap();
    assert_eq!(scan(&c).unwrap().changed, 1);
    run(&c, |_| {}, check).unwrap();
    let p = c.native_profile.join("extensions-provenance.json");
    let v: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    assert_eq!(v["effect"]["trusted"], false);
    fs::write(&p, r#"{"effect":{"trusted":"local"}}"#).unwrap();
    assert!(!scan(&c).unwrap().has_new_or_changed());
    run(&c, |_| {}, check).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&fs::read(p).unwrap()).unwrap();
    assert_eq!(v["effect"]["trusted"], "local");
}

#[test]
fn all_dependencies_are_copied_before_validation_and_errors_are_short() {
    let (_t, c) = fixture();
    fs::create_dir_all(c.electron_profile.join("extensions/zzz-dependency")).unwrap();
    fs::write(
        c.electron_profile
            .join("extensions/zzz-dependency/manifest.json"),
        r#"{"id":"zzz-dependency"}"#,
    )
    .unwrap();
    let out = run(
        &c,
        |_| {},
        |_, _| {
            assert!(
                c.native_profile
                    .join("extensions/zzz-dependency/manifest.json")
                    .exists()
            );
            Err("activation failed\nprivate multiline diagnostics".into())
        },
    )
    .unwrap();
    assert_eq!(out.could_not_load, 2);
    assert!(
        out.entries
            .iter()
            .all(|e| e.reason == "It was copied but couldn’t load: activation failed")
    );
}
