mod support;
use pm_native_migrate::{
    indexeddb,
    migration::{self, Availability, Checkpoint, ClosureInput, OwnerKind, Reference, Status},
    snapshot::{BlobCopy, Snapshot},
};
use std::{collections::BTreeSet, fs, io::Write, path::Path};
use support::*;
use tempfile::tempdir;
fn all_domains() -> BTreeSet<OwnerKind> {
    [
        OwnerKind::Current,
        OwnerKind::Trash,
        OwnerKind::Take,
        OwnerKind::Undo,
        OwnerKind::Redo,
        OwnerKind::Agent,
    ]
    .into_iter()
    .collect()
}
fn reference(owner: &str, key: &str, before: Availability) -> Reference {
    Reference {
        owner: owner.into(),
        owner_kind: OwnerKind::Current,
        key: key.into(),
        legacy_id: None,
        revision: None,
        fingerprint: None,
        original_blake3: None,
        source_path: None,
        playback_proxy: false,
        playback_proxy_version: 0,
        poster: false,
        before,
    }
}
fn keys(root: &Path) -> usize {
    fs::read_dir(root.join("keys")).unwrap().count()
}
#[test]
fn proxy_join_original_resolution_bare_fallback_history_and_offline_closure() {
    let t = tempdir().unwrap();
    let s = fixture_snapshot(t.path());
    let db = indexeddb::read(&s).unwrap();
    let root = t.path().join("store");
    let original = db
        .records
        .iter()
        .find(|r| r.key == "media:synthetic")
        .unwrap();
    let fp = original.fingerprint().unwrap().to_string();
    let mut refs = Vec::new();
    let mut r = reference("ordinary", "media:synthetic", Availability::Original);
    r.fingerprint = Some(fp.clone());
    refs.push(r);
    let mut r = reference("proxy-only", "media:proxy", Availability::ProxyOnly);
    r.fingerprint = Some(fp.clone());
    r.playback_proxy = true;
    r.playback_proxy_version = 2;
    refs.push(r);
    let mut r = reference("resolved-proxy", "media:proxy", Availability::Original);
    r.fingerprint = Some(fp);
    r.playback_proxy = true;
    r.source_path = original.payload_path(&s).unwrap();
    refs.push(r);
    let mut r = reference("history-only", "new-content-key", Availability::Original);
    r.legacy_id = Some("old-bare-id".into());
    r.owner_kind = OwnerKind::Undo;
    let old = db.records.iter().find(|r| r.key == "old-bare-id").unwrap();
    r.fingerprint =
        Some(migration::sampled_fingerprint(&old.payload_path(&s).unwrap().unwrap()).unwrap());
    refs.push(r);
    let mut r = reference("already-offline", "absent-key", Availability::Offline);
    r.owner_kind = OwnerKind::Redo;
    refs.push(r);
    let mut r = reference(
        "missing-poster",
        "absent-key:poster",
        Availability::Original,
    );
    r.poster = true;
    refs.push(r);
    let input = ClosureInput {
        inventoried: all_domains(),
        references: refs,
    };
    let pass = migration::migrate(&s, &db.records, &input, &root, |_| Ok(())).unwrap();
    assert!(pass.cutover_ready);
    assert_eq!(keys(&root), 5);
    let statuses: Vec<_> = pass.closure.iter().map(|e| e.status.clone()).collect();
    assert_eq!(
        statuses,
        [
            Status::VerifiedOriginal,
            Status::ProxyOnly,
            Status::VerifiedOriginal,
            Status::VerifiedOriginal,
            Status::AlreadyOffline,
            Status::RegeneratePoster
        ]
    );
    assert!(pass.closure[1].export_warning);
    assert!(pass.closure[1].original.is_none());
    assert!(pass.closure[2].original.is_some());
    // Actual host paths, schema, legacy revisions, and source provenance.
    for r in pass.records.values() {
        let k = r.stored.as_ref().unwrap();
        assert_eq!(k.revision, "synthetic-revision");
        assert_eq!(k.source, "indexeddb");
        let object = root.join("objects").join(&k.hash[..2]).join(&k.hash);
        assert_eq!(fs::metadata(object).unwrap().len(), k.size);
    }
    let before = fs::read(root.join("migration.json")).unwrap();
    let time = fs::metadata(root.join("migration.json"))
        .unwrap()
        .modified()
        .unwrap();
    let second = migration::migrate(&s, &db.records, &input, &root, |_| {
        panic!("completed pass should not publish another step")
    })
    .unwrap();
    assert!(second.cutover_ready);
    assert_eq!(before, fs::read(root.join("migration.json")).unwrap());
    assert_eq!(
        time,
        fs::metadata(root.join("migration.json"))
            .unwrap()
            .modified()
            .unwrap()
    );
}
#[test]
fn every_publication_boundary_resumes_without_losing_bytes() {
    for stop in [
        Checkpoint::ObjectPublished,
        Checkpoint::RecordPublished,
        Checkpoint::ClosurePublished,
    ] {
        let t = tempdir().unwrap();
        let s = fixture_snapshot(t.path());
        let db = indexeddb::read(&s).unwrap();
        let root = t.path().join("store");
        let closure = ClosureInput {
            inventoried: all_domains(),
            references: Vec::new(),
        };
        let mut fired = false;
        assert!(
            migration::migrate(&s, &db.records, &closure, &root, |point| {
                if !fired && point == stop {
                    fired = true;
                    anyhow::bail!("synthetic interruption");
                }
                Ok(())
            })
            .is_err()
        );
        assert!(fired);
        let pass = migration::migrate(&s, &db.records, &closure, &root, |_| Ok(())).unwrap();
        assert!(pass.cutover_ready);
        assert_eq!(pass.records.len(), 5);
        assert_eq!(keys(&root), 5);
    }
}
#[test]
fn newly_missing_blocks_only_its_owner_and_complete_inventory_is_required() {
    let t = tempdir().unwrap();
    let s = fixture_snapshot(t.path());
    let db = indexeddb::read(&s).unwrap();
    let root = t.path().join("store");
    let mut refs = vec![
        reference("newly-missing", "lost", Availability::Original),
        reference("previously-offline", "offline", Availability::Offline),
    ];
    let mut proxy = reference("lost-original", "media:proxy", Availability::Original);
    proxy.playback_proxy = true;
    refs.push(proxy);
    let closure = ClosureInput {
        inventoried: all_domains(),
        references: refs,
    };
    let pass = migration::migrate(&s, &db.records, &closure, &root, |_| Ok(())).unwrap();
    assert!(!pass.cutover_ready);
    assert_eq!(
        pass.blocked_owners,
        BTreeSet::from(["newly-missing".into(), "lost-original".into()])
    );
    assert_eq!(pass.closure[1].status, Status::AlreadyOffline);
    let closure = ClosureInput {
        inventoried: BTreeSet::from([OwnerKind::Current]),
        references: Vec::new(),
    };
    assert!(
        !migration::migrate(&s, &db.records, &closure, &root, |_| Ok(()))
            .unwrap()
            .cutover_ready
    );
}
#[test]
fn changed_delta_mapping_keeps_both_revisions_and_existing_native_mapping() {
    let t = tempdir().unwrap();
    let s = fixture_snapshot(t.path());
    let mut records = indexeddb::read(&s).unwrap().records;
    let root = t.path().join("store");
    let input = ClosureInput {
        inventoried: all_domains(),
        references: Vec::new(),
    };
    migration::migrate(&s, &records, &input, &root, |_| Ok(())).unwrap();
    let bytes: Vec<_> = fs::read_dir(root.join("keys"))
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (p.clone(), fs::read(p).unwrap())
        })
        .collect();
    records[0].value["revision"] = "synthetic-new-revision".into();
    let pass = migration::migrate(&s, &records, &input, &root, |_| Ok(())).unwrap();
    assert!(!pass.cutover_ready);
    assert_eq!(pass.mapping_conflicts.len(), 1);
    for (p, b) in bytes {
        assert_eq!(fs::read(p).unwrap(), b);
    }
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("migration.json")).unwrap()).unwrap();
    assert_eq!(state["passes"].as_object().unwrap().len(), 2);
}
#[test]
fn sample_snapshot_cannot_be_mistaken_for_complete_migration() {
    let t = tempdir().unwrap();
    let s = Snapshot::capture(
        &fixture().join("indexeddb"),
        Some(&fixture().join("blobs")),
        &t.path().join("sample"),
        BlobCopy::Sample {
            max_files: 1,
            max_bytes: 32,
        },
    )
    .unwrap();
    let db = indexeddb::read(&s).unwrap();
    let closure = ClosureInput {
        inventoried: all_domains(),
        references: Vec::new(),
    };
    assert_error(
        migration::migrate(&s, &db.records, &closure, &t.path().join("store"), |_| {
            Ok(())
        }),
        "sampled snapshots",
    );
}
#[test]
fn sampled_fingerprint_deduplicates_small_files_and_checks_middle_end() {
    use sha2::{Digest, Sha256};
    let t = tempdir().unwrap();
    let file = t.path().join("data");
    fs::write(&file, b"abc").unwrap();
    assert_eq!(
        migration::sampled_fingerprint(&file).unwrap(),
        format!(
            "v2:3:{}",
            hex::encode(Sha256::digest(b"powermove-media-v2\n3\nabc"))
        )
    );
    fs::write(&file, vec![3; 400_000]).unwrap();
    let old = migration::sampled_fingerprint(&file).unwrap();
    let mut f = fs::OpenOptions::new().write(true).open(&file).unwrap();
    use std::io::{Seek, SeekFrom};
    f.seek(SeekFrom::Start(180_000)).unwrap();
    f.write_all(b"changed").unwrap();
    drop(f);
    assert_ne!(old, migration::sampled_fingerprint(&file).unwrap());
}
#[test]
fn large_blob_streaming_throughput() {
    let t = tempdir().unwrap();
    let source = t.path().join("source");
    let blobs = t.path().join("blobs");
    fs::create_dir_all(blobs.join("1/00")).unwrap();
    let mut f = fs::File::create(blobs.join("1/00/2")).unwrap();
    let chunk = vec![0x5a; 1024 * 1024];
    for _ in 0..64 {
        f.write_all(&chunk).unwrap();
    }
    f.sync_all().unwrap();
    drop(f);
    database(&source, "idb_cmp1", &[]);
    let s = Snapshot::capture(
        &source,
        Some(&blobs),
        &t.path().join("snapshot"),
        BlobCopy::All,
    )
    .unwrap();
    let record = indexeddb::Record {
        database: 1,
        store: 1,
        store_name: "assets".into(),
        key: "large-synthetic".into(),
        record_version: 1,
        value: serde_json::json!({"id":"large-synthetic","revision":"large-revision","at":1}),
        external: vec![indexeddb::ExternalObject {
            number: 2,
            mime: "video/mp4".into(),
            size: 64 * 1024 * 1024,
            file_name: None,
            last_modified_micros: None,
        }],
        payload_index: 0,
        poster: false,
    };
    let mut r = reference("large-project", "large-synthetic", Availability::Original);
    r.fingerprint = Some(migration::sampled_fingerprint(&blobs.join("1/00/2")).unwrap());
    let input = ClosureInput {
        inventoried: all_domains(),
        references: vec![r],
    };
    let start = std::time::Instant::now();
    let pass =
        migration::migrate(&s, &[record], &input, &t.path().join("store"), |_| Ok(())).unwrap();
    let seconds = start.elapsed().as_secs_f64();
    assert!(pass.cutover_ready);
    println!(
        "synthetic_stream_bytes={} elapsed_seconds={seconds:.6} mib_per_second={:.2}",
        64 * 1024 * 1024,
        64.0 / seconds
    );
}
#[test]
fn asset_discovery_walks_packed_history_and_take_values_without_rewriting() {
    let value = serde_json::json!({"records":[{"patch":{"value":{"id":"history-asset","storageKey":"old","playbackProxy":true}}}],"takes":[{"assets":[{"id":"take-asset","fingerprint":"legacy"}]}]});
    let before = value.clone();
    assert_eq!(migration::discover_assets(&value).len(), 2);
    assert_eq!(value, before);
}

#[test]
fn migration_refuses_source_destinations_before_any_write_and_detects_lost_keys() {
    let t = tempdir().unwrap();
    let s = fixture_snapshot(t.path());
    let records = indexeddb::read(&s).unwrap().records;
    let closure = ClosureInput {
        inventoried: all_domains(),
        references: Vec::new(),
    };
    assert_error(
        migration::migrate(&s, &records, &closure, &fixture().join("indexeddb"), |_| {
            Ok(())
        }),
        "overlaps",
    );
    let root = t.path().join("store");
    migration::migrate(&s, &records, &closure, &root, |_| Ok(())).unwrap();
    let key = fs::read_dir(root.join("keys"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_file(key).unwrap();
    assert!(migration::migrate(&s, &records, &closure, &root, |_| Ok(())).is_err());
}

#[test]
fn transformed_payload_is_retained_without_becoming_an_original() {
    let t = tempdir().unwrap();
    let s = fixture_snapshot(t.path());
    let records = indexeddb::read(&s).unwrap().records;
    let fp = records
        .iter()
        .find(|r| r.key == "media:synthetic")
        .unwrap()
        .fingerprint()
        .unwrap();
    let mut r = reference("transformed-still", "old-bare-id", Availability::ProxyOnly);
    r.fingerprint = Some(fp.into());
    let closure = ClosureInput {
        inventoried: all_domains(),
        references: vec![r],
    };
    let pass =
        migration::migrate(&s, &records, &closure, &t.path().join("store"), |_| Ok(())).unwrap();
    assert!(pass.cutover_ready);
    assert_eq!(pass.closure[0].status, Status::ProxyOnly);
    assert!(pass.closure[0].export_warning);
    assert!(pass.closure[0].original.is_none());
}
