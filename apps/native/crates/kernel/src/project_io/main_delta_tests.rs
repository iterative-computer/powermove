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
