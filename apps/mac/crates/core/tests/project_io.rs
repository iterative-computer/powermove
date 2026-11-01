//! Open/save round trips over real fixtures, and what a failed save leaves.

mod common;

use common::*;
use pm_mac_core::project_io::save::{SaveLayout, VerifyLevel, WriteFault};
use pm_mac_core::project_io::{
    BackupPolicy, MediaEntry, MediaSource, OpenOptions, SaveOptions, SaveTarget, list_backups,
    open_project,
};
use pm_mac_core::{Command, DocumentStore, Origin, ProjectIoError};
use pm_model::canonical::{canonical_project, project_of};
use pm_model::container::{ContainerFormat, ReadOptions, read_container};
use pm_model::json::{Value, parse, to_canonical_string};

fn canonical_of_file(path: &std::path::Path) -> Value {
    let file = std::fs::File::open(path).unwrap();
    let c = read_container(&file, ReadOptions { verify_media: true }).unwrap();
    canonical_project(project_of(&c.document).clone())
}

fn media_bytes(path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let bytes = read(path);
    let c = pm_model::container::read_container_bytes(&bytes, ReadOptions { verify_media: true }).unwrap();
    c.media
        .iter()
        .map(|r| (r.id.to_string_lossy(), c.media_bytes(&bytes, r).unwrap().to_vec()))
        .collect()
}

fn sorted(mut v: Vec<(String, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
    v.sort();
    v
}

/// JSON, PMV3 and PMV4 files written by the old app's encoders: saving them
/// keeps the project (unknown fields and lone surrogates included), every
/// embedded media byte, and the envelope members we do not own.
#[test]
fn container_fixtures_round_trip_through_pmv4() {
    for name in [
        "pmv4.pmv",
        "pmv4-no-media.pmv",
        "pmv4-bare-project.pmv",
        "pmv4-id-at-limit.pmv",
        "pmv4-zero-length-overlap-ok.pmv",
        "pmv3.pmv",
        "pmv3-no-media.pmv",
        "json-plain.pmv",
    ] {
        let dir = TempDir::new("roundtrip");
        let source = engine_fixture(&format!("containers/{name}"));
        let opened = open_project(&source, OpenOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let format = opened.file.format;
        let mut store = DocumentStore::open(opened);
        let out = dir.path("out.pmv");
        save_as(&mut store, &out).unwrap_or_else(|e| panic!("{name}: {e}"));

        // The production reader accepts the output with every checksum.
        let file = std::fs::File::open(&out).unwrap();
        let saved = read_container(&file, ReadOptions { verify_media: true }).unwrap();
        assert_eq!(saved.format, ContainerFormat::Pmv4, "{name}");
        // The project is the same, member for member.
        assert_eq!(
            to_canonical_string(&canonical_of_file(&out)),
            to_canonical_string(&canonical_of_file(&source)),
            "{name}: project"
        );
        // Media bytes survive. Only media of assets the project (or its
        // undo history) reaches is written, as in the old app.
        let assets = canonical_of_file(&source);
        let reachable = |id: &str| assets.get("assets").and_then(|a| a.get(id)).is_some();
        let source_media: Vec<_> = media_bytes(&source).into_iter().filter(|(id, _)| reachable(id)).collect();
        if format != ContainerFormat::Json {
            assert!(!source_media.is_empty() || name.contains("no-media"), "{name}");
            assert_eq!(sorted(media_bytes(&out)), sorted(source_media.clone()), "{name}: media");
        }
        // Envelope: the old app's members first, unknown ones kept.
        let doc = &saved.document;
        assert_eq!(doc.get("v").and_then(Value::as_str), Some("1.0.0"), "{name}");
        let source_doc = {
            let f = std::fs::File::open(&source).unwrap();
            read_container(&f, ReadOptions { verify_media: false }).unwrap().document
        };
        if source_doc.get("proj").is_some() {
            // `history: { entries: [], index: 0 }` is not replayable (no
            // version), so it is written back exactly as read.
            assert_eq!(doc.get("history"), source_doc.get("history"), "{name}: history");
        }
        // A second open of our own file works, and saving it again in place
        // appends nothing but a document.
        let mut again = open_store(&out);
        assert_eq!(again.layers(), store.layers(), "{name}");
        again
            .execute(Command::SetLayerName { layer_id: "a".into(), name: "Renamed".into() }, Origin::Interface)
            .unwrap();
        save(&mut again, SaveTarget::InPlace, SaveOptions::default()).unwrap();
        let reread = open_store(&out);
        assert_eq!(reread.layers()[0].name, "Renamed", "{name}");
        if format != ContainerFormat::Json {
            assert_eq!(sorted(media_bytes(&out)), sorted(source_media), "{name}: media after append");
        }
        assert_eq!(dir.entries(), ["out.pmv"], "{name}: no temp files");
    }
}

/// Legacy JSON with base64 media in the document: the media becomes a PMV4
/// record (only for assets the project reaches) and the member is dropped.
#[test]
fn legacy_base64_media_becomes_a_record() {
    let dir = TempDir::new("legacy");
    let src = dir.path("old.pmv");
    std::fs::write(
        &src,
        r#"{"v":1,"proj":{"id":"P","w":8,"h":8,"layers":[],"assets":{"img":{"id":"img"}}},"media":{"img":{"type":"image/png","data":"AQID"},"gone":{"type":"x","data":"AA=="}},"futureEnvelope":{"x":[1,2]}}"#,
    )
    .unwrap();
    let mut store = open_store(&src);
    assert_eq!(store.media().len(), 1);
    let out = dir.path("new.pmv");
    save_as(&mut store, &out).unwrap();
    assert_eq!(media_bytes(&out), [("img".to_owned(), vec![1, 2, 3])]);
    let file = std::fs::File::open(&out).unwrap();
    let doc = read_container(&file, ReadOptions::default()).unwrap().document;
    assert!(doc.get("media").is_none());
    assert_eq!(to_canonical_string(doc.get("futureEnvelope").unwrap()), r#"{"x":[1,2]}"#);
}

/// Every corpus project survives an untitled save and reopen unchanged.
#[test]
fn corpus_projects_round_trip() {
    let dir = TempDir::new("corpus");
    let mut count = 0;
    for entry in std::fs::read_dir(engine_fixture("corpus")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let stored = parse(&text).unwrap();
        let expected = to_canonical_string(&canonical_project(project_of(&stored).clone()));
        let mut store = DocumentStore::new_untitled(project_of(&stored).clone());
        let out = dir.path(&format!("{count}.pmv"));
        save_as(&mut store, &out).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(to_canonical_string(&canonical_of_file(&out)), expected, "{}", path.display());
        count += 1;
    }
    assert!(count >= 30, "{count}");
}

fn edited_copy(dir: &TempDir) -> (std::path::PathBuf, DocumentStore) {
    let path = dir.path("project.pmv");
    std::fs::copy(ts_fixture("fresh.pmv"), &path).unwrap();
    let mut store = open_store(&path);
    store
        .execute(Command::SetStaticOpacity { layer_id: "a".into(), value: 25.0 }, Origin::Interface)
        .unwrap();
    (path, store)
}

#[test]
fn disk_full_leaves_the_previous_file_valid() {
    for limit in [0, 3, 200, 1500] {
        let dir = TempDir::new("diskfull");
        let (path, mut store) = edited_copy(&dir);
        let before = read(&path);
        let options = SaveOptions {
            fault: Some(WriteFault::DiskFullAfter(limit)),
            ..SaveOptions::default()
        };
        let err = run_save(&mut store, SaveTarget::InPlace, options).unwrap_err();
        assert!(err.to_string().contains("No space left"), "{err}");
        assert_eq!(read(&path), before, "byte-identical after a failed save");
        assert_eq!(dir.entries(), ["project.pmv"], "temp file removed");
        assert!(store.is_dirty());
        // And the next save works.
        save(&mut store, SaveTarget::InPlace, SaveOptions::default()).unwrap();
        assert!(!store.is_dirty());
        assert_eq!(opacity_of(&open_store(&path), "a"), Some(25.0));
    }
}

#[test]
fn a_failure_before_publishing_changes_nothing() {
    let dir = TempDir::new("fail");
    let (path, mut store) = edited_copy(&dir);
    let before = read(&path);
    let options = SaveOptions {
        fault: Some(WriteFault::FailBeforePublish),
        ..SaveOptions::default()
    };
    assert!(run_save(&mut store, SaveTarget::InPlace, options).is_err());
    assert_eq!(read(&path), before);
    assert_eq!(dir.entries(), ["project.pmv"]);
    assert!(store.is_dirty());
}

#[test]
fn a_file_changed_on_disk_is_never_overwritten() {
    let dir = TempDir::new("conflict");
    let (path, mut store) = edited_copy(&dir);
    // Another app rewrites the file after we opened it.
    std::thread::sleep(std::time::Duration::from_millis(5));
    let mut other = read(&path);
    other[10] ^= 1;
    std::fs::write(&path, &other).unwrap();
    let err = run_save(&mut store, SaveTarget::InPlace, SaveOptions::default()).unwrap_err();
    assert!(matches!(err, ProjectIoError::ChangedOnDisk), "{err}");
    assert_eq!(read(&path), other);
    assert_eq!(dir.entries(), ["project.pmv"]);
    // Replaced by another file at the same path (new inode, same bytes).
    let dir2 = TempDir::new("conflict2");
    let (path2, mut store2) = edited_copy(&dir2);
    let bytes = read(&path2);
    std::fs::remove_file(&path2).unwrap();
    std::fs::write(&path2, &bytes).unwrap();
    assert!(matches!(
        run_save(&mut store2, SaveTarget::InPlace, SaveOptions::default()),
        Err(ProjectIoError::ChangedOnDisk)
    ));
}

#[test]
fn touched_but_identical_files_are_still_saved() {
    let dir = TempDir::new("touch");
    let (path, mut store) = edited_copy(&dir);
    // Same inode, same bytes, new timestamps: proven identical by re-hash.
    std::thread::sleep(std::time::Duration::from_millis(5));
    let bytes = read(&path);
    std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    save(&mut store, SaveTarget::InPlace, SaveOptions::default()).unwrap();
    assert_eq!(opacity_of(&open_store(&path), "a"), Some(25.0));
}

#[test]
fn save_as_never_overwrites_without_consent() {
    let dir = TempDir::new("exists");
    let (_, mut store) = edited_copy(&dir);
    let other = dir.path("other.pmv");
    std::fs::write(&other, b"precious").unwrap();
    let err = run_save(
        &mut store,
        SaveTarget::SaveAs { path: other.clone(), replace: false },
        SaveOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, ProjectIoError::DestinationExists(_)), "{err}");
    assert_eq!(read(&other), b"precious");
    save(&mut store, SaveTarget::SaveAs { path: other.clone(), replace: true }, SaveOptions::default()).unwrap();
    assert_eq!(opacity_of(&open_store(&other), "a"), Some(25.0));
}

#[test]
fn a_vanished_media_source_fails_the_save_cleanly() {
    let dir = TempDir::new("vanished");
    let media = dir.path("clip.bin");
    std::fs::write(&media, [7u8; 100]).unwrap();
    let mut store = DocumentStore::new_untitled(
        parse(r#"{"id":"P","w":8,"h":8,"layers":[],"assets":{"m":{"id":"m"}}}"#).unwrap(),
    );
    store.media_mut().insert(MediaEntry {
        id: "m".into(),
        media_type: "application/octet-stream".into(),
        revision: MediaEntry::fresh_revision(),
        source: MediaSource::File { path: media.clone(), length: 100, sha256: None },
    });
    std::fs::remove_file(&media).unwrap();
    let out = dir.path("out.pmv");
    assert!(save_as(&mut store, &out).is_err());
    assert!(!out.exists());
    assert!(dir.entries().is_empty(), "{:?}", dir.entries());
    // A source whose checksum no longer matches fails too.
    std::fs::write(&media, [8u8; 100]).unwrap();
    store.media_mut().insert(MediaEntry {
        id: "m".into(),
        media_type: "x".into(),
        revision: "r".into(),
        source: MediaSource::File {
            path: media.clone(),
            length: 100,
            sha256: Some("0".repeat(64)),
        },
    });
    assert!(save_as(&mut store, &out).is_err());
    assert!(!out.exists());
}

#[test]
fn edits_during_a_save_stay_dirty() {
    let dir = TempDir::new("during");
    let (path, mut store) = edited_copy(&dir);
    let plan = store.prepare_save(SaveTarget::InPlace, SaveOptions::default()).unwrap();
    assert!(store.is_saving());
    assert!(matches!(
        store.prepare_save(SaveTarget::InPlace, SaveOptions::default()),
        Err(ProjectIoError::SaveInProgress)
    ));
    let worker = std::thread::spawn(move || plan.run());
    // The user keeps editing while the worker writes revision R.
    store
        .execute(Command::SetStaticOpacity { layer_id: "a".into(), value: 60.0 }, Origin::Interface)
        .unwrap();
    let receipt = worker.join().unwrap().unwrap();
    store.finish_save(receipt).unwrap();
    assert!(!store.is_saving());
    assert!(store.is_dirty(), "the later edit is not on disk");
    assert_eq!(opacity_of(&open_store(&path), "a"), Some(25.0));
    // Undoing the later edit returns to exactly the saved state.
    store.undo().unwrap();
    assert!(!store.is_dirty());
    store.redo().unwrap();
    assert!(store.is_dirty());
    save(&mut store, SaveTarget::InPlace, SaveOptions::default()).unwrap();
    assert!(!store.is_dirty());
    assert_eq!(opacity_of(&open_store(&path), "a"), Some(60.0));
}

#[test]
fn compaction_keeps_reused_media_first_and_intact() {
    let dir = TempDir::new("compact");
    let path = dir.path("project.pmv");
    std::fs::copy(ts_fixture("append.pmv"), &path).unwrap();
    let mut store = open_store(&path);
    store
        .execute(Command::SetLayerVisible { layer_id: "b".into(), visible: false }, Origin::Interface)
        .unwrap();
    let options = SaveOptions {
        compact_min_dead_bytes: Some(0),
        verify: VerifyLevel::Full,
        ..SaveOptions::default()
    };
    let receipt = run_save(&mut store, SaveTarget::InPlace, options).unwrap();
    assert_eq!(receipt.layout, SaveLayout::Compacted);
    assert_eq!(receipt.reused_media_bytes, 3011);
    store.finish_save(receipt).unwrap();
    let bytes = read(&path);
    let c = pm_model::container::read_container_bytes(&bytes, ReadOptions { verify_media: true }).unwrap();
    // Reused records come straight after the magic, then the document.
    assert_eq!(c.media[0].offset, 5);
    assert_eq!(c.media[1].offset, 16);
    assert!(bytes.len() < read(&ts_fixture("append.pmv")).len());
    assert_eq!(c.media_bytes(&bytes, &c.media[0]).unwrap(), read(&ts_fixture("media-img.bin")));
    assert_eq!(c.media_bytes(&bytes, &c.media[1]).unwrap(), read(&ts_fixture("media-clip.bin")));
    assert_eq!(open_store(&path).layers()[1].visible, false);
}

#[test]
fn backups_keep_the_replaced_version() {
    let dir = TempDir::new("backup");
    let (path, mut store) = edited_copy(&dir);
    let original = read(&path);
    let policy = BackupPolicy::new(dir.path("Backups"));
    let options = SaveOptions { backups: Some(policy.clone()), ..SaveOptions::default() };
    let receipt = run_save(&mut store, SaveTarget::InPlace, options.clone()).unwrap();
    let backup = receipt.backup.clone().unwrap();
    store.finish_save(receipt).unwrap();
    assert_eq!(read(&backup), original);
    for value in [1.0, 2.0, 3.0] {
        store
            .execute(Command::SetStaticOpacity { layer_id: "a".into(), value }, Origin::Interface)
            .unwrap();
        save(&mut store, SaveTarget::InPlace, options.clone()).unwrap();
    }
    let small = BackupPolicy { max_versions: 2, ..policy.clone() };
    store
        .execute(Command::SetStaticOpacity { layer_id: "a".into(), value: 4.0 }, Origin::Interface)
        .unwrap();
    save(&mut store, SaveTarget::InPlace, SaveOptions { backups: Some(small), ..SaveOptions::default() }).unwrap();
    assert_eq!(list_backups(&policy).unwrap().len(), 2);
}

/// A structural edit (layer order) is saved in the old leaf-patch history
/// format and can be undone after reopening.
#[test]
fn structural_history_survives_save_and_reopen() {
    let dir = TempDir::new("structural");
    let path = dir.path("project.pmv");
    std::fs::copy(ts_fixture("fresh.pmv"), &path).unwrap();
    let mut store = open_store(&path);
    let mut project = store.snapshot().document.to_value().unwrap();
    let layers = project
        .get_mut_path(&["comps", "C1", "layers"])
        .and_then(Value::as_array_mut)
        .unwrap();
    layers.reverse();
    store.restore_project(project, "Reorder Layers", Origin::Interface).unwrap().unwrap();
    assert_eq!(store.layers().iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
    save(&mut store, SaveTarget::InPlace, SaveOptions::default()).unwrap();
    let mut reopened = open_store(&path);
    assert_eq!(reopened.history().labels(), ["Change Opacity", "Reorder Layers"]);
    reopened.undo().unwrap();
    assert_eq!(reopened.layers().iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    reopened.redo().unwrap();
    assert_eq!(reopened.layers().iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
}

trait PathMut {
    fn get_mut_path(&mut self, path: &[&str]) -> Option<&mut Value>;
}

impl PathMut for Value {
    fn get_mut_path(&mut self, path: &[&str]) -> Option<&mut Value> {
        let mut v = self;
        for k in path {
            v = v.as_object_mut()?.get_mut(k)?;
        }
        Some(v)
    }
}
