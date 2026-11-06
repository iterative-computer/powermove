use pm_services::media_adapter::FileMedia;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;

#[test]
fn flat_and_linked_ranges_keep_unknown_metadata_and_refuse_changed_or_unsafe_sources() {
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    fs::create_dir(root.join("Index")).unwrap();
    let hash = format!("{:x}", Sha256::digest(b"abcdef"));
    let source = root.join("source.mov");
    fs::write(&source, b"abcdef").unwrap();
    let owned = format!("{hash}.mov");
    fs::write(root.join(&owned), b"abcdef").unwrap();
    for (key, location) in [
        ("media:owned", json!({"file":owned})),
        ("media:linked", json!({"sourcePath":source})),
    ] {
        let mut raw = json!({"key":key,"revision":"r1","size":6,"type":"video/quicktime","sha256":hash,"mtime":0,"future":{"kept":true}});
        raw.as_object_mut()
            .unwrap()
            .extend(location.as_object().unwrap().clone());
        fs::write(
            root.join("Index")
                .join(format!("{:x}.json", Sha256::digest(key))),
            raw.to_string(),
        )
        .unwrap();
    }
    let adapter = FileMedia::open(&root).unwrap();
    for key in ["media:owned", "media:linked"] {
        assert_eq!(adapter.read_range(key, 2, 3).unwrap(), b"cde");
        assert_eq!(adapter.record(key).unwrap().raw["future"]["kept"], true);
    }
    assert!(adapter.read_range("media:linked", u64::MAX, 1).is_err());
    assert!(
        adapter
            .read_range("media:linked", 0, 8 * 1024 * 1024 + 1)
            .is_err()
    );
    let old_mtime = fs::metadata(&source).unwrap().modified().unwrap();
    fs::write(&source, b"change").unwrap();
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old_mtime))
        .unwrap();
    assert!(adapter.read_range("media:linked", 0, 1).is_err());
    fs::remove_file(&source).unwrap();
    assert_eq!(
        adapter.status("media:linked").unwrap(),
        pm_native_migrate::library_media::SourceStatus::Missing
    );
    std::os::unix::fs::symlink(root.join(&owned), &source).unwrap();
    assert!(adapter.read_range("media:linked", 0, 1).is_err());
    assert_eq!(adapter.read_range("media:owned", 0, 6).unwrap(), b"abcdef");
}

fn private_tempdir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
