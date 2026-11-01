mod support;
use pm_native_migrate::{
    indexeddb,
    leveldb::Database,
    local_storage,
    snapshot::{BlobCopy, Snapshot},
    structured_clone,
};
use std::{fs, io::Write};
use support::*;
use tempfile::tempdir;

#[test]
fn independent_electron_44_fixture_decodes_every_record_and_file() {
    let t = tempdir().unwrap();
    let s = fixture_snapshot(t.path());
    let d = indexeddb::read(&s).unwrap();
    assert_eq!(d.schema_version, 5);
    assert_eq!(d.database_versions.get(&1), Some(&2));
    assert_eq!(d.records.len(), 5);
    assert_eq!(d.records.iter().map(|r| r.payload().size).sum::<u64>(), 85);
    assert_eq!(d.records.iter().filter(|r| r.poster).count(), 1);
    for r in &d.records {
        assert_eq!(r.value["unknown"], "café ☃ 🚀");
        assert_eq!(r.store_name, "assets");
        assert!(r.payload_path(&s).unwrap().is_some());
    }
    let file = d.records.iter().find(|r| r.key == "media:file").unwrap();
    assert_eq!(file.payload().file_name.as_deref(), Some("synthetic.txt"));
    assert!(file.payload().last_modified_micros.unwrap() > 0);
    assert!(d.records.iter().any(|r| r.key == "old-bare-id"));
    // Proxy flags do not exist in this IDB schema: the join is required.
    assert!(
        d.records
            .iter()
            .all(|r| r.value.get("playbackProxy").is_none())
    );
}
#[test]
fn local_storage_exports_all_keys_origins_and_both_string_encodings() {
    let t = tempdir().unwrap();
    let s = Snapshot::capture(
        &fixture().join("local-storage"),
        None,
        &t.path().join("local"),
        BlobCopy::All,
    )
    .unwrap();
    let entries = local_storage::read(&s).unwrap();
    assert_eq!(entries.len(), 5);
    assert_eq!(
        entries
            .iter()
            .find(|e| e.key == "synthetic.extension.é")
            .unwrap()
            .value,
        "café ☃ 🚀"
    );
    assert!(
        entries
            .iter()
            .any(|e| e.key == "pm.easing.library.v1" && e.json.is_some())
    );
    assert!(
        entries
            .iter()
            .any(|e| e.key == "powermove:native-evaluator" && e.value == "audit")
    );
}
#[test]
fn source_and_snapshot_are_never_opened_for_database_recovery_or_written() {
    let t = tempdir().unwrap();
    let s = fixture_snapshot(t.path());
    let hashes = |p: &std::path::Path| {
        fs::read_dir(p)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (e.file_name(), fs::read(e.path()).unwrap())
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let source = hashes(&fixture().join("indexeddb"));
    let copied = hashes(&s.leveldb());
    indexeddb::read(&s).unwrap();
    indexeddb::read(&s).unwrap();
    assert_eq!(source, hashes(&fixture().join("indexeddb")));
    assert_eq!(copied, hashes(&s.leveldb()));
    assert!(Snapshot::open(&fixture().join("indexeddb")).is_err());
}
#[test]
fn custom_comparator_scan_applies_manifest_live_set_versions_and_tombstones() {
    let t = tempdir().unwrap();
    let source = t.path().join("db");
    let k1 = indexeddb::encode_prefix(511, 1, 1);
    let k2 = indexeddb::encode_prefix(512, 1, 1);
    assert!(k1 > k2); // byte order differs from idb_cmp1 numeric prefix order.
    database(
        &source,
        "idb_cmp1",
        &[
            (k1.clone(), Some(b"wal-new".to_vec())),
            (b"gone".to_vec(), None),
        ],
    );
    let rows = vec![
        (k1.clone(), 0, 1, b"table-old".to_vec()),
        (k2.clone(), 0, 1, b"other".to_vec()),
        (b"gone".to_vec(), 0, 1, b"removed".to_vec()),
    ];
    let size = table(&source, 5, &rows, true);
    add_table(&source, 5, size);
    let stale = table(
        &source,
        6,
        &[(b"stale".to_vec(), 99, 1, b"must not resurrect".to_vec())],
        false,
    );
    add_table(&source, 6, stale);
    append_log(&source.join("MANIFEST-000001"), &[6, 0, 6]);
    let s = capture(&source, &t.path().join("snapshot"));
    let db = Database::read(&s).unwrap();
    assert_eq!(db.entries.get(&k1).unwrap(), b"wal-new");
    assert_eq!(db.entries.get(&k2).unwrap(), b"other");
    assert!(!db.entries.contains_key(b"gone".as_slice()));
    assert!(!db.entries.contains_key(b"stale".as_slice()));
}
#[test]
fn fragmented_wal_and_torn_uncommitted_tail() {
    let t = tempdir().unwrap();
    let source = t.path().join("db");
    let value = vec![7; 100_000];
    database(
        &source,
        "leveldb.BytewiseComparator",
        &[(b"large".to_vec(), Some(value.clone()))],
    );
    fs::OpenOptions::new()
        .append(true)
        .open(source.join("000003.log"))
        .unwrap()
        .write_all(&[42, 43, 44])
        .unwrap();
    let s = capture(&source, &t.path().join("snapshot"));
    let db = Database::read(&s).unwrap();
    assert_eq!(db.entries.get(b"large".as_slice()), Some(&value));
    assert_eq!(db.recovered_tail_bytes, 3);
}
#[test]
fn corrupt_wal_or_sst_is_an_error_instead_of_an_empty_result() {
    for table_corruption in [false, true] {
        let t = tempdir().unwrap();
        let source = t.path().join("db");
        database(
            &source,
            "leveldb.BytewiseComparator",
            &[(b"x".to_vec(), Some(b"ok".to_vec()))],
        );
        let target = if table_corruption {
            let size = table(
                &source,
                5,
                &[(b"z".to_vec(), 0, 1, b"valid".to_vec())],
                true,
            );
            add_table(&source, 5, size);
            source.join("000005.ldb")
        } else {
            source.join("000003.log")
        };
        let mut b = fs::read(&target).unwrap();
        b[9] ^= 128;
        fs::write(&target, b).unwrap();
        let s = capture(&source, &t.path().join("snapshot"));
        assert_error(Database::read(&s), "checksum");
    }
}
#[test]
fn snapshot_rejects_symlinks_overlapping_output_and_insufficient_space() {
    let t = tempdir().unwrap();
    let source = t.path().join("db");
    database(&source, "leveldb.BytewiseComparator", &[]);
    assert!(Snapshot::capture(&source, None, &source.join("inside"), BlobCopy::All).is_err());
    std::os::unix::fs::symlink(source.join("CURRENT"), source.join("unsafe.log")).unwrap();
    assert_error(
        Snapshot::capture(&source, None, &t.path().join("symlink"), BlobCopy::All),
        "symlink",
    );
    fs::remove_file(source.join("unsafe.log")).unwrap();
    let f = fs::File::create(source.join("999999.log")).unwrap();
    f.set_len(1_000_000_000_000).unwrap();
    assert_error(
        Snapshot::capture(&source, None, &t.path().join("space"), BlobCopy::All),
        "free space",
    );
    assert!(!t.path().join("space").exists());
}
#[test]
fn unknown_structured_values_and_versions_fail_closed() {
    for bytes in [
        &[255, 22, 255, 16, b'o'][..],
        &[255, 20, 255, 16, b'\\', b'?'][..],
        &[255, 20, 255, 16, b'o', b'{', 1][..],
        &[255, 20, 255, 16, b'o', b'"', 1, b'x', b'^', 0, b'{', 1][..],
    ] {
        assert!(structured_clone::decode(bytes).is_err());
    }
}
#[test]
fn compressed_and_external_value_wrappers_decode() {
    // Extract an independently written Blink value, then exercise both IDB
    // wrapper forms around those same bytes using synthetic LevelDB journals.
    let t = tempdir().unwrap();
    let s = fixture_snapshot(t.path());
    let raw = Database::read(&s).unwrap();
    let mut entries: Vec<_> = raw.entries.into_iter().map(|(k, v)| (k, Some(v))).collect();
    let at = entries
        .iter()
        .position(|(k, _)| {
            let ((_, _, i), s) = indexeddb::prefix(k).unwrap();
            i == 1 && s.ends_with(&string_be("media:synthetic")[1..])
        })
        .unwrap();
    let value = entries[at].1.as_ref().unwrap().clone();
    let blink = &value[1..];
    let mut compressed = vec![value[0], 255, 17, 2];
    compressed.extend(snap::raw::Encoder::new().compress_vec(blink).unwrap());
    entries[at].1 = Some(compressed);
    let source = t.path().join("compressed");
    database(&source, "idb_cmp1", &entries);
    let s = Snapshot::capture(
        &source,
        Some(&fixture().join("blobs")),
        &t.path().join("compressed-snapshot"),
        BlobCopy::All,
    )
    .unwrap();
    assert_eq!(indexeddb::read(&s).unwrap().records.len(), 5);
    // External wrapper is the last external object, separate from payload Blob.
    let ((db, store, _), suffix) = indexeddb::prefix(&entries[at].0).unwrap();
    let mut extkey = indexeddb::encode_prefix(db, store, 3);
    extkey.extend_from_slice(suffix);
    let e = entries.iter_mut().find(|(k, _)| k == &extkey).unwrap();
    let mut extra = vec![0];
    extra.extend(var(1000));
    extra.extend(string_be("application/x-idb-value"));
    extra.extend(var(blink.len() as u64));
    e.1.as_mut().unwrap().extend(extra);
    let mut wrapped = vec![value[0], 255, 17, 1];
    wrapped.extend(var(blink.len() as u64));
    wrapped.push(1);
    entries[at].1 = Some(wrapped);
    let blobs = t.path().join("wrapper-blobs");
    fs::create_dir_all(blobs.join("1/03")).unwrap();
    fn cp(src: &std::path::Path, dst: &std::path::Path) {
        fs::create_dir_all(dst).unwrap();
        for e in fs::read_dir(src).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                cp(&e.path(), &dst.join(e.file_name()));
            } else {
                fs::copy(e.path(), dst.join(e.file_name())).unwrap();
            }
        }
    }
    cp(&fixture().join("blobs"), &blobs);
    fs::write(blobs.join("1/03/3e8"), blink).unwrap();
    let source = t.path().join("wrapped");
    database(&source, "idb_cmp1", &entries);
    let s = Snapshot::capture(
        &source,
        Some(&blobs),
        &t.path().join("wrapped-snapshot"),
        BlobCopy::All,
    )
    .unwrap();
    assert_eq!(indexeddb::read(&s).unwrap().records.len(), 5);
}

#[test]
fn unfinished_chromium_scopes_are_not_mistaken_for_committed_transactions() {
    let t = tempdir().unwrap();
    let source = t.path().join("db");
    database(
        &source,
        "idb_cmp1",
        &[(vec![0, 0, 0, 0, 50, 1, 0], Some(vec![10, 0]))],
    );
    let s = capture(&source, &t.path().join("snapshot"));
    assert_error(indexeddb::read(&s), "unfinished IndexedDB scope");
}

#[test]
fn sample_skips_unreadable_blobs_but_full_capture_fails() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempdir().unwrap();
    let source = t.path().join("db");
    database(&source, "idb_cmp1", &[]);
    let blobs = t.path().join("blobs");
    fs::create_dir_all(blobs.join("1/00")).unwrap();
    fs::write(blobs.join("1/00/2"), b"private synthetic denied file").unwrap();
    fs::write(blobs.join("1/00/3"), b"readable synthetic sample").unwrap();
    fs::set_permissions(blobs.join("1/00/2"), fs::Permissions::from_mode(0o000)).unwrap();
    let sample = Snapshot::capture(
        &source,
        Some(&blobs),
        &t.path().join("sample"),
        BlobCopy::Sample {
            max_files: 1,
            max_bytes: 1024,
        },
    )
    .unwrap();
    assert!(!sample.complete());
    assert_eq!(sample.blob_counts().2, 1);
    assert!(sample.blob_path(1, 2).unwrap().is_none());
    assert!(sample.blob_path(1, 3).unwrap().is_some());
    assert!(
        Snapshot::capture(&source, Some(&blobs), &t.path().join("full"), BlobCopy::All).is_err()
    );
    fs::set_permissions(blobs.join("1/00/2"), fs::Permissions::from_mode(0o600)).unwrap();
}
