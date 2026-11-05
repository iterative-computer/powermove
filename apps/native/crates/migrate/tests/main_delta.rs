use pm_native_migrate::library_media::{DiskMediaRecord, read_index};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;

fn record(source: Value) -> Value {
    let mut record = json!({"key":"media:synthetic","revision":"r1","type":"video/quicktime",
        "size":3,"mtime":1,"sha256":format!("{:x}", Sha256::digest(b"abc")),"future":{"keep":true}});
    record
        .as_object_mut()
        .unwrap()
        .extend(source.as_object().unwrap().clone());
    record
}
#[test]
fn source_links_decode_without_dereferencing_offline_sources_and_keep_unknown_fields() {
    let t = tempfile::tempdir().unwrap();
    let raw = record(json!({"sourcePath":"/synthetic/offline.mov"}));
    let decoded = DiskMediaRecord::decode(raw.clone(), t.path()).unwrap();
    assert_eq!(decoded.raw, raw);
    assert!(decoded.owned_path.is_none());
    decoded.verify_owned().unwrap(); // no source I/O or cloud hydration
    for change in [
        json!({"file":"../outside.mov"}),
        json!({"sourcePath":"relative.mov"}),
        json!({"sourcePath":"/synthetic/offline.mov","file":"x"}),
        json!({"size":9007199254740992u64}),
    ] {
        assert!(DiskMediaRecord::decode(record(change), t.path()).is_err());
    }
}
#[test]
fn owned_index_requires_key_identity_full_digest_and_regular_payload() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    fs::create_dir(root.join("Index")).unwrap();
    let file = format!("{:x}.mov", Sha256::digest(b"abc"));
    let raw = record(json!({"file":file}));
    let decoded = DiskMediaRecord::decode(raw.clone(), &root).unwrap();
    fs::write(decoded.owned_path.as_ref().unwrap(), b"abc").unwrap();
    let index_name = format!("{:x}.json", Sha256::digest(b"media:synthetic"));
    fs::write(root.join("Index").join(&index_name), raw.to_string()).unwrap();
    assert_eq!(read_index(&root).unwrap()[0].raw, raw);
    fs::write(decoded.owned_path.as_ref().unwrap(), b"bad").unwrap();
    assert!(read_index(&root).is_err());
    fs::remove_file(decoded.owned_path.as_ref().unwrap()).unwrap();
    #[cfg(unix)]
    {
        let outside = root.join("outside");
        fs::write(&outside, b"abc").unwrap();
        std::os::unix::fs::symlink(&outside, decoded.owned_path.as_ref().unwrap()).unwrap();
        assert!(read_index(&root).is_err());
    }
}

#[test]
fn main_disk_revision_and_mime_are_not_restricted_by_the_pmv4_index_contract() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().canonicalize().unwrap();
    let file = format!("{:x}.mov", Sha256::digest(b"abc"));
    fs::write(source.join(&file), b"abc").unwrap();
    let revision = "r".repeat(256);
    let mime = format!("application/{}", "synthetic".repeat(30));
    let mut raw = record(json!({"file":file}));
    raw["revision"] = revision.clone().into();
    raw["type"] = mime.clone().into();
    let record = DiskMediaRecord::decode(raw, &source).unwrap();
    let closure = pm_native_migrate::migration::ClosureInput {
        inventoried: Default::default(),
        references: Vec::new(),
    };
    let result = pm_native_migrate::migration::migrate_many_with_disk(
        &[],
        &[record],
        &closure,
        &source.join("native-media"),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(result.disk_records["media:synthetic"].revision, revision);
    assert_eq!(result.disk_records["media:synthetic"].mime, mime);
    assert!(!result.cutover_ready); // An incomplete owner inventory is still blocked.
}
