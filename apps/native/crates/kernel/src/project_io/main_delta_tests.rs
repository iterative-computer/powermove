use super::*;
use crate::{DocumentStore, library::decode_disk_value};
use pm_model::json::{parse, to_canonical_string};
use std::sync::atomic::{AtomicU64, Ordering};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "pm-delta-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> Value {
    parse(include_str!("fixtures/main-document.json")).unwrap()
}

/// Backups stay bounded: versions past the budgets are deleted, as the old
/// app pruned after each save, never the newest; generations earlier builds
/// archived go too; recovery copies and unknown files stay.
#[test]
fn backup_budgets_keep_the_newest_generations_and_delete_older_ones() {
    let dir = Scratch::new();
    let policy = BackupPolicy {
        directory: dir.path("backups"),
        max_versions: 2,
        max_bytes: 1 << 20,
        min_versions: 1,
    };
    std::fs::create_dir(&policy.directory).unwrap();
    let named = |day: u32| {
        policy
            .directory
            .join(format!("2026-10-{day:02}T23-38-05-123Z.pmv"))
    };
    for day in 1..=5 {
        std::fs::write(named(day), format!("generation {day}")).unwrap();
    }
    let recovery = policy
        .directory
        .join("2026-10-01T23-38-05-123Z.pmv.recovery.pmv");
    std::fs::write(&recovery, b"uncertain publication").unwrap();
    let archive = policy.directory.join("archived");
    std::fs::create_dir(&archive).unwrap();
    std::fs::write(archive.join("2026-09-01T00-00-00-000Z.pmv"), b"old").unwrap();
    std::fs::write(
        archive.join(format!("2026-09-02T00-00-00-000Z.pmv.{UUID}")),
        b"old",
    )
    .unwrap();
    durable::retain(&policy, None, &Default::default()).unwrap();
    assert_eq!(
        durable::list_backups(&policy).unwrap(),
        vec![named(5), named(4)]
    );
    assert_eq!(std::fs::read(named(5)).unwrap(), b"generation 5");
    assert!(recovery.exists(), "a recovery copy is not a generation");
    assert!(!archive.exists(), "archived generations are gone");
    // Over the byte budget, the newest is still kept.
    let tight = BackupPolicy {
        max_bytes: 1,
        ..policy.clone()
    };
    durable::retain(&tight, None, &Default::default()).unwrap();
    assert_eq!(durable::list_backups(&tight).unwrap(), vec![named(5)]);
    // Unknown files in an old archive stay, and so does the folder.
    std::fs::create_dir(&archive).unwrap();
    std::fs::write(archive.join("notes.txt"), b"mine").unwrap();
    durable::retain(&policy, None, &Default::default()).unwrap();
    assert_eq!(std::fs::read(archive.join("notes.txt")).unwrap(), b"mine");
}

#[test]
fn a_full_disk_reclaims_archived_generations_oldest_first_and_the_save_lands() {
    let dir = Scratch::new();
    let root = dir.path("backups");
    let policy = BackupPolicy::new(root.join("this"));
    let archive = policy.directory.join("archived");
    let other = root.join("other/archived");
    std::fs::create_dir_all(&archive).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let active = policy.directory.join("2026-01-01T00-00-00-000Z.pmv");
    let oldest = other.join("2026-02-01T00-00-00-000Z.pmv");
    let older = archive.join("2026-03-01T00-00-00-000Z.pmv");
    let newest = archive.join(format!("2026-04-01T00-00-00-000Z.pmv-1.{UUID}"));
    let unnamed = archive.join("notes.pmv");
    for file in [&active, &oldest, &older, &newest, &unnamed] {
        std::fs::write(file, b"0123456789").unwrap();
    }
    assert_eq!(durable::reclaim_archived(&policy, 11), 20);
    assert!(!oldest.exists() && !older.exists());
    assert!(newest.exists(), "only what was needed");
    assert!(
        active.exists() && unnamed.exists(),
        "active and unknown stay"
    );

    let path = dir.path("project.pmv");
    let saved = |fault| {
        let mut store = DocumentStore::new_untitled(fixture().get("proj").unwrap().clone());
        store.prepare_save(
            SaveTarget::SaveAs {
                path: path.clone(),
                replace: true,
            },
            SaveOptions {
                backups: Some(policy.clone()),
                fault: Some(fault),
                ..Default::default()
            },
        )
    };
    // Nothing left to reclaim would not retry: the space error comes back.
    let failed = saved(WriteFault::DiskFullAfter(1)).unwrap().run();
    assert!(
        failed.as_ref().is_err_and(|e| e.is_disk_full()),
        "{:?}",
        failed.err()
    );
    assert!(!newest.exists(), "the failed save reclaimed the rest");
    std::fs::write(&newest, b"0123456789").unwrap();
    saved(WriteFault::DiskFullOnce(1)).unwrap().run().unwrap();
    assert!(path.exists(), "the retry published the file");
    assert!(!newest.exists());
}

#[cfg(unix)]
#[test]
fn backup_retention_never_follows_a_linked_archive() {
    let dir = Scratch::new();
    let policy = BackupPolicy::new(dir.path("backups"));
    std::fs::create_dir(&policy.directory).unwrap();
    let kept = policy.directory.join("2026-10-31T23-38-05-123Z.pmv");
    std::fs::write(&kept, b"authored generation").unwrap();
    let other = dir.path("other");
    std::fs::create_dir(&other).unwrap();
    let outside = other.join("2026-01-01T00-00-00-000Z.pmv");
    std::fs::write(&outside, b"not ours").unwrap();
    std::os::unix::fs::symlink(&other, policy.directory.join("archived")).unwrap();
    durable::retain(&policy, None, &Default::default()).unwrap();
    assert_eq!(std::fs::read(kept).unwrap(), b"authored generation");
    assert_eq!(std::fs::read(outside).unwrap(), b"not ours");
}
/// The suffix earlier builds put on an archived generation whose name was
/// already taken (`random_uuid`).
const UUID: &str = "0f8e2c1a-3b4d-4e5f-9a6b-7c8d9e0f1a2b";

/// Archive cleanup deletes only the exact names the app wrote there; a
/// timestamp-prefixed recovery copy, conflict copy or note survives both
/// retention and a full-disk reclaim.
#[test]
fn archive_cleanup_deletes_only_the_exact_names_the_app_wrote() {
    let dir = Scratch::new();
    let policy = BackupPolicy::new(dir.path("backups").join("this"));
    let archive = policy.directory.join("archived");
    std::fs::create_dir_all(&archive).unwrap();
    let stamp = "2026-09-01T00-00-00-000Z";
    let ours = [
        format!("{stamp}.pmv"),
        format!("{stamp}.pmv-2"),
        format!("{stamp}.pmv.{UUID}"),
        format!("{stamp}.pmv-2.{UUID}"),
    ];
    let theirs = [
        format!("{stamp}.pmv.recovery.pmv"),
        format!("{stamp}.pmv.conflict.pmv"),
        format!("{stamp}.pmv-2.notes"),
        format!("{stamp}.pmvx"),
        // Not a v4 UUID (version 1), so not a name `random_uuid` made.
        format!("{stamp}.pmv.0f8e2c1a-3b4d-1e5f-9a6b-7c8d9e0f1a2b"),
        format!("{stamp}.pmv.{UUID}.pmv"),
        format!("{stamp}.pmv-.{UUID}"),
    ];
    let write_all = || {
        for name in ours.iter().chain(&theirs) {
            std::fs::write(archive.join(name), name).unwrap();
        }
    };
    write_all();
    durable::retain(&policy, None, &Default::default()).unwrap();
    for name in &ours {
        assert!(
            !archive.join(name).exists(),
            "{name} is an archived generation"
        );
    }
    for name in &theirs {
        assert_eq!(
            std::fs::read(archive.join(name)).unwrap(),
            name.as_bytes(),
            "{name} is not ours"
        );
    }
    write_all();
    let ours_bytes: u64 = ours.iter().map(|n| n.len() as u64).sum();
    assert_eq!(durable::reclaim_archived(&policy, u64::MAX), ours_bytes);
    for name in &theirs {
        assert!(archive.join(name).exists(), "{name} survives a reclaim");
    }
}

/// A project folder in the backup root that is a link to somewhere else is
/// refused by listing, retention, a reclaim from another project and a save
/// (which never writes or prunes through it).
#[cfg(unix)]
#[test]
fn backups_never_follow_a_linked_project_folder() {
    let dir = Scratch::new();
    let root = dir.path("backups");
    let outside = dir.path("outside");
    std::fs::create_dir_all(outside.join("archived")).unwrap();
    std::fs::create_dir(&root).unwrap();
    let named = |day: u32| format!("2026-10-{day:02}T00-00-00-000Z.pmv");
    for day in 1..=4 {
        std::fs::write(outside.join(named(day)), b"not ours").unwrap();
        std::fs::write(outside.join("archived").join(named(day)), b"not ours").unwrap();
    }
    std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
    let linked = BackupPolicy {
        directory: root.join("linked"),
        max_versions: 1,
        max_bytes: 1 << 20,
        min_versions: 1,
    };
    assert!(
        durable::retain(&linked, None, &Default::default()).is_err(),
        "a linked project folder is refused"
    );
    assert!(durable::list_backups(&linked).is_err());
    let sibling = BackupPolicy::new(root.join("this"));
    std::fs::create_dir(&sibling.directory).unwrap();
    assert_eq!(durable::reclaim_archived(&sibling, u64::MAX), 0);

    let path = dir.path("project.pmv");
    let mut store = DocumentStore::new_untitled(fixture().get("proj").unwrap().clone());
    let options = || SaveOptions {
        backups: Some(linked.clone()),
        ..Default::default()
    };
    save(&mut store, &path, options());
    let before = std::fs::read(&path).unwrap();
    let replacing = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: path.clone(),
                replace: true,
            },
            options(),
        )
        .unwrap()
        .run();
    assert!(
        replacing.is_err(),
        "no backup through a link, so no replace"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    for day in 1..=4 {
        assert_eq!(
            std::fs::read(outside.join(named(day))).unwrap(),
            b"not ours"
        );
        let archived = outside.join("archived").join(named(day));
        assert_eq!(std::fs::read(archived).unwrap(), b"not ours");
    }
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        5,
        "nothing written outside"
    );
}

/// Retention prunes in save order, not wall-clock order: after the clock
/// moved backward (or an earlier build stamped from a clock that ran ahead),
/// the version a save just replaced is the newest backup and survives.
#[test]
fn a_clock_that_moved_backward_never_prunes_the_backup_just_written() {
    let dir = Scratch::new();
    let policy = BackupPolicy {
        directory: dir.path("backups").join("p"),
        max_versions: 2,
        max_bytes: 1 << 30,
        min_versions: 2,
    };
    std::fs::create_dir_all(&policy.directory).unwrap();
    let future = |n: u32| {
        policy
            .directory
            .join(format!("2031-01-01T00-00-0{n}-000Z.pmv"))
    };
    for n in 1..=2 {
        std::fs::write(future(n), format!("future {n}")).unwrap();
    }
    let path = dir.path("project.pmv");
    let mut store = DocumentStore::new_untitled(fixture().get("proj").unwrap().clone());
    let options = || SaveOptions {
        backups: Some(policy.clone()),
        ..Default::default()
    };
    save(&mut store, &path, options());
    let replaced = std::fs::read(&path).unwrap();
    save(&mut store, &path, options());
    let kept = durable::list_backups(&policy).unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(
        std::fs::read(&kept[0]).unwrap(),
        replaced,
        "the replaced version is kept, newest"
    );
    assert_eq!(kept[1], future(2));
    assert!(
        kept[0].file_name() > future(2).file_name(),
        "stamped after the newest kept"
    );
    assert!(!future(1).exists(), "the oldest in save order is pruned");
}

fn save(store: &mut DocumentStore, path: &Path, options: SaveOptions) -> Value {
    let receipt = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: path.into(),
                replace: true,
            },
            options,
        )
        .unwrap()
        .run()
        .unwrap();
    store.finish_save(receipt).unwrap();
    let f = File::open(path).unwrap();
    read_container(&f, ReadOptions { verify_media: true })
        .unwrap()
        .document
}

#[test]
fn json_pmv3_and_range_pmv4_keep_memory_workspace_unknowns_and_all_owned_media() {
    for (name, bytes) in [
        (
            "json",
            include_bytes!("fixtures/main-document.json").as_slice(),
        ),
        ("pmv3", include_bytes!("fixtures/main-pmv3.pmv").as_slice()),
        (
            "pmv4",
            include_bytes!("fixtures/main-ranges.pmv").as_slice(),
        ),
    ] {
        let dir = Scratch::new();
        let input = dir.path(name);
        std::fs::write(&input, bytes).unwrap();
        let opened = open_project(&input, OpenOptions::default()).unwrap();
        assert!(opened.history.is_some());
        let media_count = opened.file.media.len();
        assert!(media_count >= 1);
        let mut store = DocumentStore::open(opened);
        let output = dir.path("output.pmv");
        for _ in 0..2 {
            let doc = save(&mut store, &output, SaveOptions::default());
            for key in ["proj", "ws", "history", "futureEnvelope"] {
                assert_eq!(doc.get(key), fixture().get(key), "{name}: {key}");
            }
            assert_eq!(doc.get("nativeLegacyMediaArchive"), fixture().get("media"));
            if name == "pmv3" {
                let metadata = doc.get("nativePmv3Metadata").unwrap();
                assert_eq!(
                    metadata.get("futureHeader").unwrap().get("keep"),
                    Some(&Value::Bool(true))
                );
                assert!(
                    metadata.get("media").unwrap().as_array().unwrap()[0]
                        .get("futureMedia")
                        .is_some()
                );
            }
            let reopened = open_project(&output, OpenOptions::default()).unwrap();
            assert_eq!(reopened.file.media.len(), media_count);
            if name == "pmv4" {
                let index = reopened.file.envelope.container_index.as_ref().unwrap();
                assert_eq!(
                    index.get("futureIndex").unwrap().get("keep"),
                    Some(&Value::Bool(true))
                );
                assert_eq!(
                    index.get("document").unwrap().get("futureRange"),
                    Some(&Value::from(7))
                );
                assert!(
                    index
                        .get("nativeArchivedMedia")
                        .unwrap()
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|m| m.get("id").and_then(Value::as_str) == Some("orphan-range"))
                        .unwrap()
                        .get("futureMedia")
                        .is_some()
                );
            }
        }
    }
}

#[test]
fn dictionary_redo_cursor_and_input_metadata_survive_history_changes() {
    let dir = Scratch::new();
    let input = dir.path("input.json");
    std::fs::write(&input, include_bytes!("fixtures/main-document.json")).unwrap();
    let mut store = DocumentStore::open(open_project(&input, OpenOptions::default()).unwrap());
    assert!(!store.can_undo());
    assert!(store.can_redo());
    store.redo().unwrap();
    assert!(store.can_undo());
    let doc = save(&mut store, &dir.path("redo.pmv"), SaveOptions::default());
    assert_eq!(doc.get("nativeHistoryArchive"), fixture().get("history"));
    let mut reopened =
        DocumentStore::open(open_project(&dir.path("redo.pmv"), OpenOptions::default()).unwrap());
    reopened.undo().unwrap();
    let saved = save(&mut reopened, &dir.path("undo.pmv"), SaveOptions::default());
    assert_eq!(saved.get("proj"), fixture().get("proj"));
}

#[test]
fn gzip_fixture_decodes_exact_document_and_enforces_declared_bounds() {
    let raw = parse(include_str!("fixtures/disk-gzip.json")).unwrap();
    assert_eq!(decode_disk_value(raw.clone(), 1 << 20).unwrap(), fixture());
    assert!(decode_disk_value(raw.clone(), 2).is_err());
    for (key, value) in [
        ("bytes", Value::from(2)),
        ("encoding", Value::from("unknown")),
        ("__powermoveDiskValue", Value::from(2)),
        ("data", Value::from("AAAA")),
    ] {
        let mut changed = raw.clone();
        changed.as_object_mut().unwrap().insert(key, value);
        assert!(decode_disk_value(changed, 1 << 20).is_err(), "{key}");
    }
    for key in [
        "project",
        "projectHistory",
        "projectState",
        "projectJournal",
        "takes",
    ] {
        let mut logical = fixture();
        logical
            .as_object_mut()
            .unwrap()
            .insert("syntheticPadding", Value::from("synthetic ".repeat(4000)));
        let encoded =
            crate::persistence::encode_disk_value(&logical, crate::persistence::DiskFormat::V1_3)
                .unwrap();
        assert!(
            std::str::from_utf8(&encoded)
                .unwrap()
                .contains("__powermoveDiskValue")
        );
        let back = decode_disk_value(
            parse(std::str::from_utf8(&encoded).unwrap()).unwrap(),
            1 << 20,
        )
        .unwrap();
        assert_eq!(back, logical, "{key}");
    }
}

#[test]
fn source_links_stay_external_on_save_as_and_portable_export_verifies_bytes() {
    let dir = Scratch::new();
    let path = dir.path("original.mov");
    let bytes = b"synthetic source";
    std::fs::write(&path, bytes).unwrap();
    let mut project = fixture().get("proj").unwrap().clone();
    project
        .as_object_mut()
        .unwrap()
        .get_mut("assets")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("linked")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("sourcePath", Value::from(path.to_str().unwrap()));
    let expected_assets = project.get("assets").cloned();
    let mut store = DocumentStore::new_untitled(project);
    store
        .media_mut()
        .insert_link(MediaEntry {
            id: "linked".into(),
            media_type: "video/quicktime".into(),
            revision: "r1".into(),
            source: MediaSource::File {
                path: path.clone(),
                length: bytes.len() as u64,
                sha256: Some(durable::hex(&Sha256::digest(bytes))),
            },
        })
        .unwrap();
    let normal = dir.path("normal.pmv");
    save(&mut store, &normal, SaveOptions::default());
    assert!(
        open_project(&normal, OpenOptions::default())
            .unwrap()
            .file
            .media
            .is_empty()
    );
    let portable = dir.path("portable.pmv");
    let receipt = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: portable.clone(),
                replace: false,
            },
            SaveOptions {
                portable: true,
                ..Default::default()
            },
        )
        .unwrap()
        .run()
        .unwrap();
    assert!(receipt.unavailable_links.is_empty());
    assert!(store.finish_save(receipt).is_err());
    assert_eq!(store.path(), Some(normal.as_path()));
    assert_eq!(
        open_project(&portable, OpenOptions::default())
            .unwrap()
            .file
            .media
            .len(),
        1
    );
    assert!(store.media().is_linked(&"linked".into()));
    std::fs::write(&path, b"changed! source!").unwrap();
    assert!(
        store
            .prepare_save(
                SaveTarget::SaveAs {
                    path: dir.path("bad.pmv"),
                    replace: false
                },
                SaveOptions {
                    portable: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .run()
            .is_err()
    );
    std::fs::remove_file(&path).unwrap();
    let receipt = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: dir.path("offline.pmv"),
                replace: false,
            },
            SaveOptions {
                portable: true,
                ..Default::default()
            },
        )
        .unwrap()
        .run()
        .unwrap();
    assert_eq!(receipt.unavailable_links, vec![JsString::from("linked")]);
    assert!(receipt.media.is_empty());
    drop(receipt);
    let saved = save(&mut store, &normal, SaveOptions::default());
    assert_eq!(
        saved.get("proj").unwrap().get("assets"),
        expected_assets.as_ref()
    );
    assert!(to_canonical_string(&saved).contains("sourcePath"));
}

#[test]
fn owned_conversion_or_pasted_bytes_survive_when_the_recorded_source_is_gone() {
    let dir = Scratch::new();
    let mut project = fixture().get("proj").unwrap().clone();
    project.as_object_mut().unwrap().insert("importQueue", parse(r#"{"folders":[{"path":"synthetic/nested","cloud":"unavailable","future":true}],"previousDocument":"retained"}"#).unwrap());
    let mut store = DocumentStore::new_untitled(project.clone());
    store.media_mut().insert(MediaEntry {
        id: "linked".into(),
        media_type: "video/webm".into(),
        revision: "converted-r1".into(),
        source: MediaSource::Memory(Arc::from(b"owned fallback".as_slice())),
    });
    let output = dir.path("fallback.pmv");
    let saved = save(&mut store, &output, SaveOptions::default());
    assert_eq!(saved.get("proj"), Some(&project));
    let opened = open_project(&output, OpenOptions::default()).unwrap();
    opened
        .file
        .media
        .get("linked")
        .unwrap()
        .copy_to(&dir.path("payload"))
        .unwrap();
    assert_eq!(
        std::fs::read(dir.path("payload")).unwrap(),
        b"owned fallback"
    );
}

#[test]
fn archived_ranges_are_checksum_verified_and_never_overlap_the_document() {
    let dir = Scratch::new();
    let input = dir.path("input.pmv");
    std::fs::write(&input, include_bytes!("fixtures/main-ranges.pmv")).unwrap();
    let mut store = DocumentStore::open(open_project(&input, OpenOptions::default()).unwrap());
    let output = dir.path("archived.pmv");
    save(&mut store, &output, SaveOptions::default());
    let mut bytes = std::fs::read(&output).unwrap();
    let container =
        pm_model::container::read_container_bytes(&bytes, ReadOptions { verify_media: true })
            .unwrap();
    assert!(!container.archived_media.is_empty());
    let archived = &container.archived_media[0];
    bytes[archived.offset as usize] ^= 1;
    std::fs::write(&output, &bytes).unwrap();
    assert!(open_project(&output, OpenOptions::default()).is_err());
    // Restore bytes, then forge a correctly checksummed index with overlapping
    // archive/document ranges. Structural validation must still reject it.
    bytes[archived.offset as usize] ^= 1;
    let footer = bytes.len() - 64;
    let index_offset =
        f64::from_le_bytes(bytes[footer + 8..footer + 16].try_into().unwrap()) as usize;
    let mut index = container.index.unwrap();
    let doc_offset = index
        .get("document")
        .unwrap()
        .get("offset")
        .unwrap()
        .clone();
    let Value::Array(records) = index
        .as_object_mut()
        .unwrap()
        .get_mut("nativeArchivedMedia")
        .unwrap()
    else {
        panic!("archive");
    };
    records[0]
        .as_object_mut()
        .unwrap()
        .insert("offset", doc_offset);
    let text = to_canonical_string(&index);
    bytes.truncate(index_offset);
    bytes.extend(text.as_bytes());
    let mut footer = [0u8; 64];
    footer[..8].copy_from_slice(b"PMV4END\n");
    footer[8..16].copy_from_slice(&(index_offset as f64).to_le_bytes());
    footer[16..20].copy_from_slice(&(text.len() as u32).to_le_bytes());
    footer[32..].copy_from_slice(&Sha256::digest(text.as_bytes()));
    bytes.extend(footer);
    assert!(
        pm_model::container::read_container_bytes(&bytes, ReadOptions { verify_media: true })
            .is_err()
    );
}
