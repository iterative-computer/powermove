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

#[test]
fn source_link_import_verifies_full_identity_and_retries_availability_as_new_pass() {
    use pm_native_migrate::{library_media::SourceStatus, migration::*};
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let source = root.join("linked.mov");
    let raw = record(json!({"sourcePath":source}));
    let record = DiskMediaRecord::decode(raw.clone(), &root).unwrap();
    let closure = ClosureInput {
        inventoried: [
            OwnerKind::Current,
            OwnerKind::Trash,
            OwnerKind::Take,
            OwnerKind::Undo,
            OwnerKind::Redo,
            OwnerKind::Agent,
        ]
        .into_iter()
        .collect(),
        references: vec![Reference {
            owner: "synthetic".into(),
            owner_kind: OwnerKind::Current,
            key: record.key.clone(),
            legacy_id: None,
            revision: Some("r1".into()),
            fingerprint: None,
            original_blake3: None,
            source_path: Some(source.clone()),
            playback_proxy: false,
            playback_proxy_version: 0,
            poster: false,
            before: Availability::Original,
        }],
    };
    let dest = root.join("native");
    let run = || {
        migrate_many_with_disk(&[], std::slice::from_ref(&record), &closure, &dest, |_| {
            Ok(())
        })
        .unwrap()
    };
    let missing = run();
    assert_eq!(
        missing.disk_sources[&record.key],
        (raw, SourceStatus::Missing)
    );
    assert_eq!(missing.closure[0].status, Status::NewlyMissing);
    assert!(!missing.cutover_ready);
    fs::write(&source, b"bad").unwrap();
    let changed = run();
    assert_eq!(changed.disk_sources[&record.key].1, SourceStatus::Changed);
    assert!(changed.disk_records.is_empty());
    fs::write(&source, b"abc").unwrap();
    let verified = run();
    assert!(verified.cutover_ready);
    assert_eq!(verified.closure[0].status, Status::VerifiedOriginal);
    assert_ne!(verified.base_hash, missing.base_hash);
    let before = fs::read(dest.join("migration.json")).unwrap();
    assert!(run().cutover_ready);
    assert_eq!(fs::read(dest.join("migration.json")).unwrap(), before);
    fs::remove_file(&source).unwrap();
    assert!(
        dest.join("objects")
            .join(&verified.disk_records[&record.key].hash[..2])
            .join(&verified.disk_records[&record.key].hash)
            .exists()
    );
    assert_eq!(run().closure[0].status, Status::VerifiedOriginal);
}

#[test]
fn linked_proxy_stays_proxy_and_interrupted_copy_resumes_without_deleting_sources() {
    use pm_native_migrate::migration::*;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let source = root.join("proxy.mov");
    fs::write(&source, b"abc").unwrap();
    let record = DiskMediaRecord::decode(record(json!({"sourcePath":source})), &root).unwrap();
    let closure = ClosureInput {
        inventoried: [
            OwnerKind::Current,
            OwnerKind::Trash,
            OwnerKind::Take,
            OwnerKind::Undo,
            OwnerKind::Redo,
            OwnerKind::Agent,
        ]
        .into_iter()
        .collect(),
        references: vec![Reference {
            owner: "synthetic".into(),
            owner_kind: OwnerKind::Undo,
            key: record.key.clone(),
            legacy_id: None,
            revision: Some("r1".into()),
            fingerprint: None,
            original_blake3: None,
            source_path: None,
            playback_proxy: true,
            playback_proxy_version: 0,
            poster: false,
            before: Availability::ProxyOnly,
        }],
    };
    let dest = root.join("native");
    assert!(
        migrate_many_with_disk(
            &[],
            std::slice::from_ref(&record),
            &closure,
            &dest,
            |_| anyhow::bail!("synthetic interruption")
        )
        .is_err()
    );
    let pass = migrate_many_with_disk(&[], &[record], &closure, &dest, |_| Ok(())).unwrap();
    assert_eq!(pass.closure[0].status, Status::ProxyOnly);
    assert!(pass.closure[0].export_warning);
    assert!(pass.closure[0].original.is_none());
    assert_eq!(fs::read(source).unwrap(), b"abc");
}

#[test]
fn interrupted_link_copy_can_resume_offline_before_public_key_was_written() {
    use pm_native_migrate::{library_media::SourceStatus, migration::*};
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let source = root.join("source.mov");
    fs::write(&source, b"abc").unwrap();
    let input = DiskMediaRecord::decode(record(json!({"sourcePath":source})), &root).unwrap();
    let closure = ClosureInput {
        inventoried: [
            OwnerKind::Current,
            OwnerKind::Trash,
            OwnerKind::Take,
            OwnerKind::Undo,
            OwnerKind::Redo,
            OwnerKind::Agent,
        ]
        .into_iter()
        .collect(),
        references: vec![],
    };
    let dest = root.join("native");
    assert!(
        migrate_many_with_disk(&[], std::slice::from_ref(&input), &closure, &dest, |step| {
            assert_eq!(step, Checkpoint::ObjectPublished);
            anyhow::bail!("synthetic process interruption")
        })
        .is_err()
    );
    assert_eq!(fs::read_dir(dest.join("keys")).unwrap().count(), 0);
    fs::remove_file(source).unwrap();
    let resumed =
        migrate_many_with_disk(&[], std::slice::from_ref(&input), &closure, &dest, |_| {
            Ok(())
        })
        .unwrap();
    assert_eq!(resumed.disk_sources[&input.key].1, SourceStatus::Missing);
    let owned = &resumed.disk_records[&input.key];
    assert_eq!(
        fs::read(
            dest.join("objects")
                .join(&owned.hash[..2])
                .join(&owned.hash)
        )
        .unwrap(),
        b"abc"
    );
    assert_eq!(fs::read_dir(dest.join("keys")).unwrap().count(), 1);
}
