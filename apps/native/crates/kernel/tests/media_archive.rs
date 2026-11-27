mod common;
use common::TempDir;
use pm_kernel::{
    media_archive::Archive,
    project_io::{MediaEntry, MediaSource},
};
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn selected_original_is_durable_distinct_and_source_is_never_changed() {
    let dir = TempDir::new("original-archive");
    let profile = dir.path("native");
    std::fs::create_dir(&profile).unwrap();
    let source = dir.path("source.dat");
    let bytes = vec![0xa5; 2_100_013];
    std::fs::write(&source, &bytes).unwrap();
    let archive = Archive::open(&profile).unwrap();
    let original = archive
        .retain_path(&source, "application/octet-stream", &AtomicBool::new(false))
        .unwrap();
    assert_eq!(original.length(), bytes.len() as u64);
    assert_ne!(original.path(), source);
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    std::fs::remove_file(&source).unwrap();
    assert_eq!(std::fs::read(original.path()).unwrap(), bytes);
    let restored = archive
        .recover(original.hash(), original.length(), original.mime())
        .unwrap();
    assert_eq!(std::fs::read(restored.path()).unwrap(), bytes);
}

#[test]
fn physical_reuse_keeps_distinct_logical_versions_and_mime_interpretations() {
    let dir = TempDir::new("original-dedup");
    let archive = Archive::open(&dir.0).unwrap();
    let source = dir.path("chosen.dat");
    std::fs::write(&source, b"same original").unwrap();
    let first = archive
        .retain_path(&source, "image/png", &AtomicBool::new(false))
        .unwrap();
    let second = archive
        .retain_path(&source, "image/png", &AtomicBool::new(false))
        .unwrap();
    let third = archive
        .retain_path(&source, "application/octet-stream", &AtomicBool::new(false))
        .unwrap();
    assert_eq!(first.path(), second.path());
    assert_eq!(first.path(), third.path());
    assert_ne!(first.version(), second.version());
    assert_ne!(second.version(), third.version());
    assert_eq!(first.clone().version(), first.version());
    assert_eq!(
        std::fs::read_dir(dir.path("media/retained"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn existing_original_capability_is_verified_before_a_receipt_is_returned() {
    let dir = TempDir::new("original-capability");
    let archive = Archive::open(&dir.0).unwrap();
    let entry = MediaEntry {
        id: "asset".into(),
        media_type: "application/octet-stream".into(),
        revision: "r".into(),
        source: MediaSource::Memory(Arc::from(b"old bytes".as_slice())),
    };
    let old = archive.retain_entry(&entry).unwrap();
    assert_eq!(std::fs::read(old.path()).unwrap(), b"old bytes");
    let source = dir.path("changed.dat");
    std::fs::write(&source, b"different").unwrap();
    let invalid = MediaEntry {
        id: "asset".into(),
        media_type: "application/octet-stream".into(),
        revision: "r".into(),
        source: MediaSource::File {
            path: source.clone(),
            length: 9,
            sha256: Some(old.hash().into()),
        },
    };
    assert!(archive.retain_entry(&invalid).is_err());
    assert_eq!(std::fs::read(source).unwrap(), b"different");
    assert_eq!(std::fs::read(old.path()).unwrap(), b"old bytes");
}

#[test]
fn cancellation_and_corrupt_existing_object_preserve_the_source() {
    let dir = TempDir::new("original-refusal");
    let archive = Archive::open(&dir.0).unwrap();
    let source = dir.path("chosen.dat");
    std::fs::write(&source, b"original").unwrap();
    assert!(
        archive
            .retain_path(&source, "application/octet-stream", &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(std::fs::read(&source).unwrap(), b"original");
    let original = archive
        .retain_path(&source, "application/octet-stream", &AtomicBool::new(false))
        .unwrap();
    std::fs::write(original.path(), b"corrupt!").unwrap();
    assert!(
        archive
            .recover(original.hash(), original.length(), original.mime())
            .is_err()
    );
    assert!(
        archive
            .retain_path(&source, "application/octet-stream", &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(std::fs::read(source).unwrap(), b"original");
    assert_eq!(std::fs::read(original.path()).unwrap(), b"corrupt!");
    // The complete unadmitted retry is retained for explicit recovery.
    assert!(
        std::fs::read_dir(dir.path("media/retained"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| std::fs::read(e.path()).is_ok_and(|bytes| bytes == b"original"))
    );
}

#[cfg(unix)]
#[test]
fn retained_directory_and_recovery_objects_cannot_be_symlink_aliases() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new("original-symlink");
    let outside = dir.path("outside");
    std::fs::create_dir(&outside).unwrap();
    symlink(&outside, dir.path("media")).unwrap();
    assert!(Archive::open(&dir.0).is_err());
    std::fs::remove_file(dir.path("media")).unwrap();
    let archive = Archive::open(&dir.0).unwrap();
    let source = dir.path("chosen.dat");
    std::fs::write(&source, b"original").unwrap();
    let original = archive
        .retain_path(&source, "application/octet-stream", &AtomicBool::new(false))
        .unwrap();
    std::fs::remove_file(original.path()).unwrap();
    symlink(&source, original.path()).unwrap();
    assert!(
        archive
            .recover(original.hash(), original.length(), original.mime())
            .is_err()
    );
    assert!(
        archive
            .recover("../../outside", 8, "application/octet-stream")
            .is_err()
    );
}
