use anyhow::Result;
use pm_first_launch::media::{FileMedia, ResidentSources, SourceAccess};
use pm_kernel::{
    DocumentStore,
    project_io::{OpenOptions, SaveOptions, SaveTarget, open_project},
};
use pm_model::json::parse;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixture(root: &Path, linked: bool) -> (PathBuf, PathBuf) {
    let store = root.join("Media Store");
    fs::create_dir_all(store.join("Index")).unwrap();
    let hash = format!("{:x}", Sha256::digest(b"synthetic media"));
    let payload = if linked {
        root.join("original.mov")
    } else {
        store.join(format!("{hash}.mov"))
    };
    fs::write(&payload, b"synthetic media").unwrap();
    let mut record = json!({"key":"media:clip","revision":"r1","type":"video/quicktime",
        "size":15,"sha256":hash,"mtime":1,"future":{"retain":true}});
    if linked {
        record["sourcePath"] = json!(payload);
    } else {
        record["file"] = json!(payload.file_name().unwrap().to_str().unwrap());
    }
    fs::write(
        store
            .join("Index")
            .join(format!("{:x}.json", Sha256::digest(b"media:clip"))),
        record.to_string(),
    )
    .unwrap();
    (store, payload)
}

#[test]
fn admitted_flat_files_and_links_reach_real_kernel_save_and_portable_reopen() {
    for linked in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (store, payload) = fixture(&root, linked);
        let before = fs::read(&payload).unwrap();
        let mut adapter = FileMedia::open(&store, ResidentSources).unwrap();
        let admitted = adapter.resolve("media:clip").unwrap().unwrap();
        assert_eq!(admitted.record().raw["future"]["retain"], true);
        assert_eq!(admitted.read_range(2, 4).unwrap(), b"nthe");
        let project = parse(
            &json!({"id":"p","compId":"c","w":1920,"h":1080,"layers":[],"assets":{"clip":{"id":"clip",
            "type":"video","storageKey":"media:clip","sourcePath":payload}},
            "comps":{"c":{"id":"c","w":1920,"h":1080,"layers":[]}}})
            .to_string(),
        )
        .unwrap();
        let mut document = DocumentStore::new_untitled(project);
        admitted.attach("clip", document.media_mut()).unwrap();
        assert_eq!(document.media().is_linked(&"clip".into()), linked);
        for portable in [false, true] {
            let output = root.join(format!("save-{portable}.pmv"));
            let receipt = document
                .prepare_save(
                    SaveTarget::SaveAs {
                        path: output.clone(),
                        replace: false,
                    },
                    SaveOptions {
                        portable,
                        ..Default::default()
                    },
                )
                .unwrap()
                .run()
                .unwrap();
            assert!(receipt.unavailable_links.is_empty());
            let reopened = open_project(&output, OpenOptions::default()).unwrap();
            assert_eq!(reopened.file.media.len(), usize::from(!linked || portable));
            if !portable {
                document.finish_save(receipt).unwrap();
            }
        }
        assert_eq!(fs::read(&payload).unwrap(), before);
    }
}
struct Offline;
impl SourceAccess for Offline {
    fn resident(&mut self, _: &Path) -> Result<bool> {
        Ok(false)
    }
}
#[test]
fn metadata_and_offline_sources_do_not_read_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let (store, payload) = fixture(&root, true);
    fs::write(&payload, b"different bytes").unwrap();
    let mut adapter = FileMedia::open(&store, Offline).unwrap();
    assert!(adapter.metadata("media:clip").unwrap().is_some());
    assert!(adapter.resolve("media:clip").unwrap().is_none());
    fs::remove_file(payload).unwrap();
    assert!(adapter.metadata("media:clip").unwrap().is_some());
    assert!(adapter.resolve("media:clip").unwrap().is_none());
    assert!(adapter.metadata("media:absent").unwrap().is_none());
}
#[test]
fn changed_same_length_bytes_and_unbounded_ranges_never_pass() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let (store, payload) = fixture(&root, true);
    let mut adapter = FileMedia::open(&store, ResidentSources).unwrap();
    let admitted = adapter.resolve("media:clip").unwrap().unwrap();
    assert!(admitted.read_range(u64::MAX, 1).is_err());
    assert!(admitted.read_range(0, 4 * 1024 * 1024 + 1).is_err());
    assert!(admitted.read_range(15, 1).is_err());
    assert!(admitted.read_range(15, 0).unwrap().is_empty());
    fs::write(&payload, b"synthetic MEDIA").unwrap();
    assert!(admitted.read_range(0, 15).is_err());
    assert!(adapter.resolve("media:clip").is_err());
}
#[test]
fn source_and_index_symlinks_are_refused_with_source_bytes_retained() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let (store, payload) = fixture(&root, true);
    let target = root.join("retained.mov");
    fs::rename(&payload, &target).unwrap();
    symlink(&target, &payload).unwrap();
    let mut adapter = FileMedia::open(&store, ResidentSources).unwrap();
    assert!(adapter.resolve("media:clip").is_err());
    fs::rename(store.join("Index"), store.join("RetainedIndex")).unwrap();
    symlink(store.join("RetainedIndex"), store.join("Index")).unwrap();
    assert!(adapter.metadata("media:clip").is_err());
    assert_eq!(fs::read(target).unwrap(), b"synthetic media");
}

#[test]
fn metadata_corruption_wrong_keys_and_oversized_records_remain_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let (store, payload) = fixture(&root, true);
    let adapter = FileMedia::open(&store, ResidentSources).unwrap();
    let index = store
        .join("Index")
        .join(format!("{:x}.json", Sha256::digest(b"media:clip")));
    let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&index).unwrap()).unwrap();
    raw["key"] = json!("media:different");
    fs::write(&index, raw.to_string()).unwrap();
    assert!(adapter.metadata("media:clip").is_err());
    fs::write(&index, b"{invalid JSON").unwrap();
    assert!(adapter.metadata("media:clip").is_err());
    fs::write(&index, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(adapter.metadata("media:clip").is_err());
    assert_eq!(fs::read(payload).unwrap(), b"synthetic media");
}
