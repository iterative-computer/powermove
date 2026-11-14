//! Later launches on synthetic profiles: everything the original app changed
//! comes in, native work stays, both sides of a conflict are kept.
mod support;
use pm_first_launch::{
    Config, ConflictKind, DeltaPrepared, Event, Launch, Outcome, State, Step, default_paths, delta,
    delta_if_changed, delta_media, delta_store, detect, onboarding_completed, report, run,
    surfaces::migration::ReportKind,
};
use pm_kernel::{
    Kernel, KernelConfig, LibraryStore,
    locks::OpenDecision,
    persistence::{DiskFormat, LibraryWriter},
    session::{ShellCall, ShellReply},
};
use pm_model::json::parse;
use serde_json::{Value, json};
use std::{fs, path::Path};
use support::*;
use tempfile::tempdir;

fn ready(root: &Path) -> Config {
    let config = profile(root);
    assert!(matches!(
        run(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));
    config
}
fn state(config: &Config) -> State {
    read(&config.native_profile.join("migration.json"))
}
fn library(config: &Config) -> LibraryStore {
    LibraryStore::open(&config.native_profile)
}
fn store(config: &Config, name: &str) -> std::path::PathBuf {
    config.electron_profile.join("store").join(name)
}
/// Edit a project in the original app, the way its save rewrites the slot.
fn edit_source(config: &Config, name: &str) {
    let path = store(config, "project.p.json");
    let mut v: Value = read(&path);
    v["proj"]["name"] = name.into();
    write(&path, &v);
}
fn source_project(config: &Config, id: &str, name: &str) {
    write(
        &store(config, &format!("project.{id}.json")),
        &json!({"v":"1.2.0","proj":{"id":id,"name":name,"w":10,"h":10,"fps":30,"dur":1,"layers":[],"comps":{}}}),
    );
    let mut rows: Value = read(&store(config, "projects.json"));
    rows.as_array_mut()
        .unwrap()
        .push(json!({"id":id,"name":name}));
    write(&store(config, "projects.json"), &rows);
}
/// A native edit, as a kernel save would write it.
fn edit_native(config: &Config, id: &str, name: &str) {
    let writer = LibraryWriter::open(&config.native_profile, DiskFormat::V1_2, None).unwrap();
    let p = writer.store().load(id).unwrap().unwrap();
    let mut project = p.project.clone();
    project
        .as_object_mut()
        .unwrap()
        .insert("name", pm_model::json::Value::from(name));
    writer
        .save_project(
            id,
            project,
            p.history.unwrap(),
            p.workspace.unwrap_or_else(|| parse("{}").unwrap()),
            p.session.unwrap_or_else(|| parse("{}").unwrap()),
        )
        .unwrap();
}
fn name_of(config: &Config, id: &str) -> String {
    library(config)
        .load(id)
        .unwrap()
        .unwrap()
        .project
        .get("name")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
fn local_storage(config: &Config) -> Value {
    read(&config.native_profile.join("local-storage.json"))
}

#[test]
fn every_change_in_the_original_app_comes_in_unrenamed_and_the_source_stays() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    let cp = checkpoint(&config);
    let id = cp.projects["p"].clone();
    edit_source(&config, "Edited in the original app");
    source_project(&config, "added", "New in the original app");
    write(&store(&config, "theme.json"), &json!("light"));
    fs::remove_file(store(&config, "ext.synthetic.json")).unwrap();
    fs::write(
        config.electron_profile.join("extensions/synthetic/main.ts"),
        "// updated synthetic extension\n",
    )
    .unwrap();
    write(
        &config
            .electron_profile
            .join("extensions/later/manifest.json"),
        &json!({"id":"later","version":"2.0.0"}),
    );
    fs::write(
        config
            .electron_profile
            .join("Agent Workspaces/synthetic/artifacts/result.txt"),
        b"later synthetic result",
    )
    .unwrap();
    media_db(&config.electron_profile, true);
    let key = [b"_app://powermove\0".as_slice(), &[1], b"pm.later"].concat();
    fs::remove_dir_all(config.electron_profile.join("Local Storage/leveldb")).unwrap();
    database(
        &config.electron_profile.join("Local Storage/leveldb"),
        "leveldb.BytewiseComparator",
        &[(key, [&[1], b"later".as_slice()].concat())],
    );
    let source = files(&config.electron_profile);
    let original = files(&config.native_profile.join("first-launch-checkpoint"));

    assert!(matches!(
        delta(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));

    assert_eq!(files(&config.electron_profile), source);
    assert_eq!(
        files(&config.native_profile.join("first-launch-checkpoint")),
        original
    );
    assert_eq!(name_of(&config, &id), "Edited in the original app");
    let added = state(&config).receipts["added"].native.clone();
    assert_eq!(name_of(&config, &added), "New in the original app");
    assert_eq!(
        library(&config).read("theme").unwrap().unwrap().as_str(),
        Some("light")
    );
    assert!(library(&config).read("ext.synthetic").unwrap().is_none());
    let shared = config.native_profile.join("extensions-shared");
    assert_eq!(
        fs::read_to_string(shared.join("synthetic/main.ts")).unwrap(),
        "// updated synthetic extension\n"
    );
    assert!(shared.join("later/manifest.json").is_file());
    assert!(
        fs::File::options()
            .write(true)
            .open(shared.join("synthetic/main.ts"))
            .is_err(),
        "shared extension copies stay sealed after a merge"
    );
    assert!(!config.native_profile.join("Agent Workspaces").exists());
    let entries = local_storage(&config);
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_eq!(entries[0]["key"], "pm.later");
    assert_eq!(entries[0]["value"], "later");
    assert!(state(&config).conflicts.is_empty());

    let mut kernel = Kernel::new(KernelConfig::default());
    assert!(matches!(
        kernel
            .shell(ShellCall::AttachLibrary {
                profile: config.native_profile.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .unwrap(),
        ShellReply::Library(OpenDecision::Writable)
    ));
    assert!(kernel.state().extensions.contains_key("later"));
    assert_eq!(
        kernel
            .state()
            .settings
            .get("theme")
            .and_then(|v| v.as_str()),
        Some("light")
    );
}

#[test]
fn native_work_stays_and_a_change_on_both_sides_keeps_both() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    let id = checkpoint(&config).projects["p"].clone();
    edit_native(&config, &id, "Edited in Native");
    edit_source(&config, "Edited in the original app");
    fs::write(
        config.native_profile.join("store/theme.json"),
        "\"native theme\"",
    )
    .unwrap();
    write(&store(&config, "theme.json"), &json!("original theme"));
    write(
        &store(&config, "ext.synthetic.json"),
        &json!({"count":8,"unknown":true}),
    );
    let native_ext = config.native_profile.join("store/ext.synthetic.json");
    fs::write(&native_ext, br#"{"count":7,"unknown":true,"nativeOnly":1}"#).unwrap();
    let artifact = "Agent Workspaces/synthetic/artifacts/result.txt";
    fs::create_dir_all(config.native_profile.join(artifact).parent().unwrap()).unwrap();
    fs::write(config.native_profile.join(artifact), b"native artifact").unwrap();
    fs::write(config.electron_profile.join(artifact), b"original artifact").unwrap();

    delta(&config, |_| Ok(())).unwrap();

    assert_eq!(name_of(&config, &id), "Edited in Native");
    let copies = library(&config).projects().unwrap();
    let kept = copies
        .iter()
        .find(|row| row.id != id)
        .expect("the original's version is another library project");
    assert_eq!(name_of(&config, &kept.id), "Edited in the original app");
    assert_eq!(
        library(&config).read("theme").unwrap().unwrap().as_str(),
        Some("native theme")
    );
    // Extension objects merge one member at a time.
    let merged = library(&config).read("ext.synthetic").unwrap().unwrap();
    assert_eq!(merged.get("count").unwrap().as_f64(), Some(8.));
    assert_eq!(merged.get("nativeOnly").unwrap().as_f64(), Some(1.));
    assert_eq!(
        fs::read(config.native_profile.join(artifact)).unwrap(),
        b"native artifact"
    );
    let conflicts = state(&config).conflicts;
    assert!(conflicts.iter().any(|c| c.kind == ConflictKind::Project));
    assert!(
        conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::Setting && c.id == "theme")
    );
    assert!(conflicts.iter().any(|c| c.kind == ConflictKind::File));
    let base = state(&config).delta_base.unwrap();
    let delta_dir = config.native_profile.join("first-launch-deltas").join(base);
    assert_eq!(
        fs::read(delta_dir.join("profile").join(artifact)).unwrap(),
        b"original artifact"
    );
    let rows = report::load(&config.native_profile).unwrap();
    assert_eq!(
        rows.sections()[&ReportKind::Conflicts].len(),
        conflicts.len()
    );

    // A later launch with nothing new does not repeat any of it.
    let before = library(&config).projects().unwrap().len();
    delta(&config, |_| Ok(())).unwrap();
    assert_eq!(library(&config).projects().unwrap().len(), before);
    assert_eq!(state(&config).conflicts, conflicts);
}

#[test]
fn trash_follows_only_while_the_copy_is_untouched_and_nothing_is_ever_purged() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    source_project(&config, "keep", "Removed later");
    run(&config, |_| Ok(())).unwrap();
    let ids = checkpoint(&config).projects;
    let rows: Value = read(&store(&config, "projects.json"));
    write(
        &store(&config, "projectTrash.json"),
        &json!([rows[0].clone()]),
    );
    write(&store(&config, "projects.json"), &json!([rows[1].clone()]));
    delta(&config, |_| Ok(())).unwrap();
    let trashed = library(&config).trash().unwrap();
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].id, ids["p"]);

    // Permanently deleted there: library work is additive, so it stays here.
    write(&store(&config, "projects.json"), &json!([]));
    fs::remove_file(store(&config, "project.keep.json")).unwrap();
    delta(&config, |_| Ok(())).unwrap();
    assert_eq!(name_of(&config, &ids["keep"]), "Removed later");
}

#[test]
fn a_native_trash_survives_a_later_edit_in_the_original_app() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    let id = checkpoint(&config).projects["p"].clone();
    let writer = LibraryWriter::open(&config.native_profile, DiskFormat::V1_2, None).unwrap();
    assert!(writer.trash(&id).unwrap());
    drop(writer);
    edit_source(&config, "Edited in the original app");
    delta(&config, |_| Ok(())).unwrap();
    assert_eq!(library(&config).trash().unwrap()[0].id, id);
    assert!(library(&config).projects().unwrap().is_empty());
    assert_eq!(name_of(&config, &id), "Edited in the original app");
}

#[test]
fn a_project_edited_then_trashed_in_the_original_app_is_trashed_here() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    let id = checkpoint(&config).projects["p"].clone();
    edit_source(&config, "Edited, then trashed");
    let rows: Value = read(&store(&config, "projects.json"));
    write(&store(&config, "projectTrash.json"), &json!(rows));
    write(&store(&config, "projects.json"), &json!([]));
    delta(&config, |_| Ok(())).unwrap();
    assert_eq!(library(&config).trash().unwrap()[0].id, id);
    assert_eq!(name_of(&config, &id), "Edited, then trashed");
}

fn local_storage_key(config: &Config, value: &str) {
    let key = [b"_app://powermove\0".as_slice(), &[1], b"pm.later"].concat();
    fs::remove_dir_all(config.electron_profile.join("Local Storage/leveldb")).unwrap();
    database(
        &config.electron_profile.join("Local Storage/leveldb"),
        "leveldb.BytewiseComparator",
        &[(key, [&[1], value.as_bytes()].concat())],
    );
}
fn delta_folders(config: &Config) -> Vec<String> {
    let Ok(entries) = fs::read_dir(config.native_profile.join("first-launch-deltas")) else {
        return vec![];
    };
    entries
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn chromium_rewriting_its_files_costs_one_capture_and_keeps_nothing() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    let base = state(&config).delta_base;
    // The original app ran: same content, new files.
    let log = config.electron_profile.join("Local Storage/leveldb/LOG");
    fs::write(&log, b"opened").unwrap();
    assert!(matches!(
        delta_if_changed(&config, |_| Ok(())).unwrap(),
        DeltaPrepared::MediaPending
    ));
    delta_media(&config, |_| Ok(())).unwrap();
    assert_eq!(state(&config).delta_base, base);
    assert_eq!(delta_folders(&config), Vec::<String>::new());
    assert!(matches!(
        delta_if_changed(&config, |_| panic!("merged source")).unwrap(),
        DeltaPrepared::Unchanged
    ));
}

#[test]
fn a_local_storage_change_alone_moves_the_base() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    local_storage_key(&config, "merged");
    assert!(matches!(
        delta_if_changed(&config, |_| Ok(())).unwrap(),
        DeltaPrepared::MediaPending
    ));
    delta_media(&config, |_| Ok(())).unwrap();
    let first = state(&config).delta_base;
    assert!(first.is_some());
    assert_eq!(delta_folders(&config).len(), 1);

    // Merged, then edited here. The original's old value is not a conflict.
    let writer = LibraryWriter::open(&config.native_profile, DiskFormat::V1_2, None).unwrap();
    let storage = pm_kernel::profile::ProfileStorage::new(&writer);
    let mut entries = storage.local_storage().unwrap();
    entries
        .iter_mut()
        .find(|e| e.key == "pm.later")
        .unwrap()
        .value = "native".into();
    storage.replace_local_storage(&entries).unwrap();
    drop(writer);
    let log = config.electron_profile.join("Local Storage/leveldb/LOG");
    fs::write(&log, b"opened").unwrap();
    delta(&config, |_| Ok(())).unwrap();
    assert_eq!(state(&config).conflicts, Default::default());
    assert_eq!(state(&config).delta_base, first);
    assert_eq!(delta_folders(&config).len(), 1);
}

#[test]
fn an_interrupted_extension_is_registered_on_resume() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    write(
        &config
            .electron_profile
            .join("extensions/later/manifest.json"),
        &json!({"id":"later","version":"2.0.0"}),
    );
    let stopped = delta(&config, |event| {
        anyhow::ensure!(event != Event::Started(Step::Extensions), "interrupted");
        Ok(())
    });
    assert!(stopped.is_err());
    delta(&config, |_| Ok(())).unwrap();
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel
        .shell(ShellCall::AttachLibrary {
            profile: config.native_profile.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .unwrap();
    assert!(kernel.state().extensions["later"].installed);
}

#[test]
fn every_interruption_resumes_without_false_conflicts_or_duplicates() {
    let stops: [fn(&Event) -> bool; 6] = [
        |e| *e == Event::Started(Step::Bootstrap),
        |e| *e == Event::Started(Step::Library),
        |e| {
            matches!(
                e,
                Event::Progress {
                    step: Step::Library,
                    ..
                }
            )
        },
        |e| *e == Event::Started(Step::LocalStorage),
        |e| *e == Event::Started(Step::Extensions),
        |e| {
            matches!(
                e,
                Event::Progress {
                    step: Step::Media,
                    ..
                }
            )
        },
    ];
    for stop in stops {
        let t = tempdir().unwrap();
        let config = ready(t.path());
        let id = checkpoint(&config).projects["p"].clone();
        edit_source(&config, "Edited in the original app");
        source_project(&config, "added", "New in the original app");
        write(&store(&config, "theme.json"), &json!("light"));
        write(
            &config
                .electron_profile
                .join("extensions/later/manifest.json"),
            &json!({"id":"later","version":"2.0.0"}),
        );
        media_db(&config.electron_profile, true);
        let mut hit = false;
        let result = delta(&config, |event| {
            if stop(&event) {
                hit = true;
                anyhow::bail!("synthetic interruption");
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(hit);
        assert!(
            config
                .native_profile
                .join("first-launch-deltas/pending")
                .exists()
        );
        delta(&config, |_| Ok(())).unwrap();
        assert!(
            !config
                .native_profile
                .join("first-launch-deltas/pending")
                .exists()
        );
        assert_eq!(name_of(&config, &id), "Edited in the original app");
        assert_eq!(library(&config).projects().unwrap().len(), 2);
        assert_eq!(
            library(&config).read("theme").unwrap().unwrap().as_str(),
            Some("light")
        );
        assert!(
            config
                .native_profile
                .join("extensions-shared/later/manifest.json")
                .is_file()
        );
        assert_eq!(state(&config).conflicts, Default::default());
    }
}

#[test]
fn an_unchanged_source_costs_nothing_and_store_and_media_run_apart() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    let bytes = fs::read(config.native_profile.join("migration.json")).unwrap();
    assert!(matches!(
        delta_if_changed(&config, |_| panic!("no events for an unchanged source")).unwrap(),
        DeltaPrepared::Unchanged
    ));
    assert_eq!(
        bytes,
        fs::read(config.native_profile.join("migration.json")).unwrap()
    );
    assert!(!config.native_profile.join("first-launch-deltas").exists());

    write(&store(&config, "theme.json"), &json!("light"));
    media_db(&config.electron_profile, true);
    assert!(matches!(
        delta_if_changed(&config, |_| Ok(())).unwrap(),
        DeltaPrepared::MediaPending
    ));
    // The library opens as soon as the store is merged; media lands behind it.
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel
        .shell(ShellCall::AttachLibrary {
            profile: config.native_profile.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .unwrap();
    assert_eq!(
        library(&config).read("theme").unwrap().unwrap().as_str(),
        Some("light")
    );
    assert!(matches!(
        delta_media(&config, |_| Ok(())).unwrap(),
        Outcome::Complete { .. }
    ));
    let media: Value = read(&config.native_profile.join("Media Store/migration.json"));
    assert_eq!(media["passes"].as_object().unwrap().len(), 2);
    drop(kernel);
    assert!(matches!(
        delta_if_changed(&config, |_| panic!("merged source")).unwrap(),
        DeltaPrepared::Unchanged
    ));
}

#[test]
fn a_running_original_app_waits_and_a_store_pass_writes_nothing() {
    let t = tempdir().unwrap();
    let config = ready(t.path());
    write(&store(&config, "theme.json"), &json!("light"));
    std::os::unix::fs::symlink(
        "another-mac-1",
        config.electron_profile.join("SingletonLock"),
    )
    .unwrap();
    assert!(matches!(
        delta_store(&config, |_| Ok(())).unwrap(),
        DeltaPrepared::WaitingForElectron
    ));
    assert_eq!(
        library(&config).read("theme").unwrap().unwrap().as_str(),
        Some("dark")
    );
    assert!(!config.native_profile.join("first-launch-deltas").exists());
}

#[test]
fn the_app_and_the_coordinator_share_one_profile_layout() {
    let support = Path::new("/synthetic/Library/Application Support");
    let config = default_paths(support);
    assert_eq!(config.electron_profile, support.join("Powermove"));
    assert_eq!(
        config.native_profile,
        support.join("com.zellzoi.powermove.native-beta")
    );

    let t = tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let fresh = default_paths(&root);
    assert_eq!(detect(&fresh), Launch::Fresh);
    fs::create_dir_all(&fresh.electron_profile).unwrap();
    assert_eq!(detect(&fresh), Launch::Migrate);

    let config = ready(&root);
    assert_eq!(detect(&config), Launch::Later);
    assert!(!onboarding_completed(&config.native_profile));
    let (mut interrupted, other) = (false, tempdir().unwrap());
    let partial = profile(other.path());
    let _ = run(&partial, |e| {
        if e == Event::Published(Step::Library) {
            interrupted = true;
            anyhow::bail!("stop")
        }
        Ok(())
    });
    assert!(interrupted);
    assert_eq!(detect(&partial), Launch::Migrate);
}

#[test]
fn a_finished_onboarding_comes_across_from_the_original_app() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    write(
        &config.electron_profile.join("onboarding-v1.json"),
        &json!({"version":1,"timeline":"layers"}),
    );
    run(&config, |_| Ok(())).unwrap();
    assert!(onboarding_completed(&config.native_profile));
}

#[test]
fn the_report_lists_what_needs_review_without_any_secret_value() {
    let t = tempdir().unwrap();
    let config = profile(t.path());
    write(
        &store(&config, "theme.json.corrupt-1"),
        &json!("synthetic quarantine"),
    );
    fs::create_dir_all(config.electron_profile.join("env")).unwrap();
    fs::write(
        config.electron_profile.join("env/synthetic.env"),
        "TOKEN=enc:AAAA\n",
    )
    .unwrap();
    let outcome = run(&config, |_| Ok(())).unwrap();
    assert!(matches!(outcome, Outcome::NeedsSecretConsent { .. }));
    pm_first_launch::skip_secret_consent(&config, |_| Ok(())).unwrap();
    let report = report::load(&config.native_profile).unwrap();
    let sections = report.sections();
    assert_eq!(sections[&ReportKind::Proxies].len(), 1);
    assert_eq!(
        sections[&ReportKind::Quarantined][0].display_name,
        "theme.json.corrupt-1"
    );
    assert_eq!(sections[&ReportKind::SignIns][0].display_name, "synthetic");
    assert!(!format!("{report:?}").contains("TOKEN"));
    assert!(!format!("{report:?}").contains("AAAA"));
}
