//! Metadata-only links, followed by explicitly requested, verified adoption.
use crate::{
    Result,
    migration::*,
    snapshot::{atomic_json, open_regular},
};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// A file to link: absolute, regular, of the recorded size. iCloud-only
/// (dataless) files count: linking never reads them; opening downloads them.
pub(crate) fn valid_file(path: &Path, size: u64) -> bool {
    path.is_absolute() && fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == size)
}
/// The byte length a v2 fingerprint records (`v2:<size>:<hash>`).
fn fingerprint_size(r: &Reference) -> Option<u64> {
    r.fingerprint
        .as_deref()
        .and_then(|f| f.strip_prefix("v2:"))
        .and_then(|f| f.split(':').next())
        .and_then(|s| s.parse().ok())
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LinkedFolder {
    pub items: u64,
    pub bytes: u64,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LinkedSummary {
    pub total_items: u64,
    pub total_bytes: u64,
    pub missing: u64,
    /// Available linked files under each ancestor; look up the original app's profile.
    pub folders: BTreeMap<PathBuf, LinkedFolder>,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AdoptReport {
    pub adopted_items: u64,
    pub adopted_bytes: u64,
    pub missing: u64,
}
fn prepare_root(root: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in root.components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            ensure!(metadata.is_dir(), "unsafe media destination ancestor");
        }
    }
    for dir in ["keys", "objects", "tmp"] {
        let path = root.join(dir);
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            ensure!(metadata.is_dir(), "unsafe media destination directory");
        }
        fs::create_dir_all(path)?;
    }
    Ok(())
}
fn keys(root: &Path) -> Result<Vec<(PathBuf, KeyRecord)>> {
    let dir = root.join("keys");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|x| x == "json") {
            let key: KeyRecord = serde_json::from_reader(open_regular(&path)?)?;
            ensure!(
                path.file_name().unwrap().to_string_lossy()
                    == format!("{:x}.json", Sha256::digest(key.key.as_bytes())),
                "key filename mismatch"
            );
            out.push((path, key));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}
pub fn linked_summary(media_root: &Path) -> Result<LinkedSummary> {
    let mut summary = LinkedSummary::default();
    for (_, key) in keys(media_root)? {
        let Some(path) = &key.path else {
            continue;
        };
        summary.total_items += 1;
        summary.total_bytes += key.size;
        if !valid_file(path, key.size) {
            summary.missing += 1;
            continue;
        }
        for folder in path.ancestors().skip(1) {
            let count = summary.folders.entry(folder.into()).or_default();
            count.items += 1;
            count.bytes += key.size;
        }
    }
    Ok(summary)
}
pub fn adopt_linked(
    media_root: &Path,
    mut progress: impl FnMut(u64, u64) -> Result<()>,
) -> Result<AdoptReport> {
    use fs2::FileExt;
    use std::os::unix::fs::OpenOptionsExt;
    prepare_root(media_root)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(media_root.join("migration.lock"))?;
    lock.try_lock_exclusive()
        .context("migration already running")?;
    let entries = keys(media_root)?;
    let total = entries
        .iter()
        .filter(|(_, k)| k.path.is_some())
        .map(|(_, k)| k.size)
        .sum();
    let mut report = AdoptReport::default();
    let mut done = 0;
    progress(done, total)?;
    for (target, key) in entries {
        let Some(source) = &key.path else {
            continue;
        };
        ensure!(
            !source.starts_with(media_root.join("tmp")),
            "linked source overlaps adoption staging"
        );
        if !valid_file(source, key.size) {
            report.missing += 1;
            continue;
        }
        let mut adopted = crate::migration::import_file(
            media_root,
            source,
            key.size,
            crate::migration::ImportInfo {
                key: &key.key,
                mime: &key.mime,
                revision: Some(&key.revision),
                at: Some(key.at),
                expected: key.expected_blake3.as_deref(),
            },
        )?;
        if let Some(expected) = &key.sha256 {
            use std::io::Read;
            let mut file = open_regular(
                &media_root
                    .join("objects")
                    .join(&adopted.hash[..2])
                    .join(&adopted.hash),
            )?;
            let mut hash = Sha256::new();
            let mut buffer = vec![0; 1024 * 1024];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
            }
            ensure!(
                format!("{:x}", hash.finalize()) == *expected,
                "linked source checksum changed"
            );
        }
        adopted.source = key.source;
        atomic_json(&target, &adopted)?;
        report.adopted_items += 1;
        report.adopted_bytes += key.size;
        done += key.size;
        progress(done, total)?;
    }
    Ok(report)
}
fn owned_valid(root: &Path, key: &KeyRecord) -> bool {
    key.path.is_none()
        && key.hash.len() == 64
        && key.hash.bytes().all(|b| b.is_ascii_hexdigit())
        && valid_file(
            &root.join("objects").join(&key.hash[..2]).join(&key.hash),
            key.size,
        )
}
fn make_key(
    key: &str,
    path: PathBuf,
    size: u64,
    mime: &str,
    revision: Option<&str>,
    sha256: Option<String>,
) -> KeyRecord {
    // This is a metadata identity, never a claim that a payload was hashed.
    use std::os::unix::fs::MetadataExt;
    let identity = fs::symlink_metadata(&path)
        .ok()
        .map(|m| (m.dev(), m.ino(), m.mtime(), m.mtime_nsec(), m.len()));
    let hash = blake3::hash(format!("{path:?}:{identity:?}:{sha256:?}").as_bytes())
        .to_hex()
        .to_string();
    KeyRecord {
        key: key.into(),
        path: Some(path),
        sha256,
        expected_blake3: None,
        size,
        mime: mime.into(),
        revision: revision.filter(|r| !r.is_empty()).unwrap_or(&hash).into(),
        hash,
        at: 0.,
        source: "linked".into(),
    }
}
/// First-launch mode: publish paths with metadata checks only. No payload reads.
pub fn link_many_with_disk(
    batches: &[(&crate::snapshot::Snapshot, &[crate::indexeddb::Record])],
    disk: &[crate::library_media::DiskMediaRecord],
    closure: &ClosureInput,
    root: &Path,
    mut hook: impl FnMut(Checkpoint) -> Result<()>,
) -> Result<Pass> {
    use fs2::FileExt;
    use std::os::unix::fs::OpenOptionsExt;
    ensure!(
        batches
            .iter()
            .all(|(_, records)| records.iter().all(|r| r.payload_index < r.external.len())),
        "invalid media payload index"
    );
    for (snapshot, _) in batches {
        snapshot.ensure_destination(root)?;
    }
    prepare_root(root)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join("migration.lock"))?;
    lock.try_lock_exclusive()?;
    let mut available = BTreeMap::new();
    let mut pass = Pass {
        phase: Phase::Complete,
        snapshot: blake3::hash(&serde_json::to_vec(
            &batches
                .iter()
                .map(|(s, _)| s.identity())
                .collect::<Result<Vec<_>>>()?,
        )?)
        .to_hex()
        .to_string(),
        base_hash: blake3::hash(&serde_json::to_vec(closure)?)
            .to_hex()
            .to_string(),
        records: BTreeMap::new(),
        disk_records: BTreeMap::new(),
        disk_sources: BTreeMap::new(),
        closure: Vec::new(),
        blocked_owners: BTreeSet::new(),
        mapping_conflicts: BTreeSet::new(),
        cutover_ready: true,
    };
    let reference_path = |key: &str, size| {
        closure
            .references
            .iter()
            .filter(|r| !r.unreached && (r.key == key || r.legacy_id.as_deref() == Some(key)))
            .filter_map(|r| r.source_path.as_ref())
            .find(|p| valid_file(p, size))
            .cloned()
    };
    for record in disk {
        let path = reference_path(&record.key, record.size)
            .or_else(|| {
                record
                    .source_path
                    .clone()
                    .filter(|p| valid_file(p, record.size))
            })
            .or_else(|| {
                record
                    .owned_path
                    .clone()
                    .filter(|p| valid_file(p, record.size))
            });
        if let Some(path) = path {
            let mut key = make_key(
                &record.key,
                path,
                record.size,
                &record.mime,
                Some(&record.revision),
                (!record.sha256.is_empty()).then(|| record.sha256.clone()),
            );
            if record.sha256.is_empty() {
                key.expected_blake3 = record
                    .raw
                    .get("hash")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
            }
            available.insert(record.key.clone(), key);
        }
    }
    for (snapshot, records) in batches {
        for record in *records {
            let payload = record.payload();
            if !available.contains_key(&record.key) {
                let path = reference_path(&record.key, payload.size)
                    .or_else(|| {
                        record
                            .value
                            .get("sourcePath")
                            .and_then(serde_json::Value::as_str)
                            .map(PathBuf::from)
                            .filter(|p| valid_file(p, payload.size))
                    })
                    .or(snapshot.linked_blob_path(record.database, payload.number)?);
                if let Some(path) = path.filter(|p| valid_file(p, payload.size)) {
                    available.insert(
                        record.key.clone(),
                        make_key(
                            &record.key,
                            path,
                            payload.size,
                            &payload.mime,
                            record.revision(),
                            None,
                        ),
                    );
                }
            }
            pass.records.entry(record.key.clone()).or_insert(Imported {
                source_record: record.clone(),
                stored: available.get(&record.key).cloned(),
            });
        }
    }
    for r in &closure.references {
        if !r.unreached
            && !available.contains_key(&r.key)
            && let Some(path) = &r.source_path
        {
            // With no index/blob, v2 fingerprints carry the recorded byte length.
            let size = fingerprint_size(r).or_else(|| {
                // Older originals carry a full hash but no recorded size, and
                // agent attachments carry neither: the original app opens
                // them by path. Link what's there without reading its bytes;
                // adoption verifies a recorded hash before publication.
                (r.original_blake3.is_some() || r.fingerprint.is_none())
                    .then(|| {
                        fs::symlink_metadata(path)
                            .ok()
                            .filter(|m| m.is_file())
                            .map(|m| m.len())
                    })
                    .flatten()
            });
            if let Some(size) = size
                && valid_file(path, size)
            {
                let mut key = make_key(
                    &r.key,
                    path.clone(),
                    size,
                    "application/octet-stream",
                    r.revision.as_deref(),
                    None,
                );
                key.expected_blake3 = r.original_blake3.clone();
                available.insert(r.key.clone(), key);
            }
        }
    }
    // Adoption remains available after the old profile's payload disappears.
    let mut expected = BTreeMap::new();
    for r in disk {
        expected.insert(r.key.clone(), Some(r.revision.clone()));
    }
    for (_, records) in batches {
        for r in *records {
            expected
                .entry(r.key.clone())
                .or_insert_with(|| r.revision().map(str::to_owned));
        }
    }
    for r in &closure.references {
        expected.entry(r.key.clone()).or_insert(r.revision.clone());
        if let Some(id) = &r.legacy_id {
            expected.entry(id.clone()).or_insert(r.revision.clone());
        }
    }
    for (_, key) in keys(root)? {
        if !available.contains_key(&key.key)
            && owned_valid(root, &key)
            && expected
                .get(&key.key)
                .is_some_and(|rev| rev.as_ref().is_none_or(|rev| rev == &key.revision))
        {
            available.insert(key.key.clone(), key);
        }
    }
    let mut writes = Vec::new();
    for (key, record) in &mut available {
        let target = root
            .join("keys")
            .join(format!("{:x}.json", Sha256::digest(key.as_bytes())));
        let existing: Option<KeyRecord> = if target.exists() {
            Some(serde_json::from_reader(open_regular(&target)?)?)
        } else {
            None
        };
        if let Some(old) = &existing {
            ensure!(old.key == *key, "key filename mismatch");
            if old.revision != record.revision || old.size != record.size || old.mime != record.mime
            {
                pass.mapping_conflicts.insert(key.clone());
                continue;
            }
        }
        if existing.as_ref() == Some(&*record) {
            continue;
        }
        if let Some(old) = existing.filter(|k| {
            owned_valid(root, k)
                && k.revision == record.revision
                && k.size == record.size
                && k.mime == record.mime
        }) {
            *record = old;
        } else {
            writes.push((target, record.clone()));
        }
    }
    for (target, record) in writes {
        crate::snapshot::atomic_json_staged(&target, &record)?;
    }
    for record in pass.records.values_mut() {
        record.stored = available.get(&record.source_record.key).cloned();
    }
    pass.disk_records = available.clone();
    for r in &closure.references {
        let stored = available
            .get(&r.key)
            .or_else(|| r.legacy_id.as_ref().and_then(|id| available.get(id)))
            .filter(|key| {
                r.revision
                    .as_ref()
                    .is_none_or(|revision| revision == &key.revision)
            })
            .cloned();
        // A proxy reference's key holds the proxy; its original is the file
        // at its source path, available when it's there at its recorded size.
        let original_on_disk = r.is_proxy()
            && !r.unreached
            && r.source_path
                .as_ref()
                .zip(fingerprint_size(r))
                .is_some_and(|(p, size)| valid_file(p, size));
        let original_available = original_on_disk
            || stored.as_ref().is_some_and(|k| {
                !r.is_proxy()
                    || r.source_path
                        .as_ref()
                        .is_some_and(|p| k.path.as_ref() == Some(p))
            });
        let status = if stored.is_some() {
            if !original_available {
                Status::ProxyOnly
            } else {
                Status::VerifiedOriginal
            }
        } else if r.poster {
            Status::RegeneratePoster
        } else if r.unreached {
            Status::NotReached
        } else if r.before != Availability::Offline {
            Status::NewlyMissing
        } else {
            Status::AlreadyOffline
        };
        if status == Status::NewlyMissing
            || (r.before == Availability::Original && !original_available && !r.poster)
        {
            pass.blocked_owners.insert(r.owner.clone());
        }
        pass.closure.push(ClosureEntry {
            reference: r.clone(),
            status,
            original: stored.clone().filter(|_| original_available && !r.poster),
            stored,
            export_warning: r.is_proxy() && !original_available,
        });
    }
    pass.cutover_ready =
        closure.complete() && pass.blocked_owners.is_empty() && pass.mapping_conflicts.is_empty();
    let state_path = root.join("migration.json");
    let mut state = if state_path.exists() {
        serde_json::from_reader(open_regular(&state_path)?)?
    } else {
        serde_json::json!({"version":1,"passes":{}})
    };
    state["passes"][format!("{}:{}", pass.snapshot, pass.base_hash)] = serde_json::to_value(&pass)?;
    atomic_json(&state_path, &state)?;
    for _ in 0..batches
        .iter()
        .map(|(_, records)| records.len())
        .sum::<usize>()
        + disk.len()
    {
        hook(Checkpoint::RecordPublished)?;
    }
    hook(Checkpoint::ClosurePublished)?;
    Ok(pass)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_publish_and_resumable_adoption() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let source = base.join("original.bin");
        fs::write(&source, b"synthetic payload").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
        let root = base.join("media");
        fs::create_dir_all(root.join("keys")).unwrap();
        let key = make_key("item", source.clone(), 17, "video/mp4", Some("v1"), None);
        // A sparse multi-GB input proves publication itself does not hash bytes.
        let sparse = base.join("sparse");
        fs::File::create(&sparse)
            .unwrap()
            .set_len(50 * 1024 * 1024 * 1024)
            .unwrap();
        let activity = std::sync::Arc::new(crate::source_access::Activity::default());
        let _scope =
            crate::source_access::install(crate::source_access::Choice::Allow, activity.clone());
        let started = std::time::Instant::now();
        let large_root = base.join("large-media");
        let record = crate::library_media::DiskMediaRecord {
            key: "large".into(),
            revision: "v1".into(),
            mime: "video/mp4".into(),
            size: 50 * 1024 * 1024 * 1024,
            sha256: "a".repeat(64),
            owned_path: Some(sparse),
            source_path: None,
            raw: serde_json::json!({}),
        };
        link_many_with_disk(
            &[],
            &[record],
            &ClosureInput {
                inventoried: BTreeSet::new(),
                references: vec![],
            },
            &large_root,
            |_| Ok(()),
        )
        .unwrap();
        assert!(started.elapsed().as_secs() < 5);
        assert_eq!(
            activity.bytes().0,
            0,
            "publishing links reads no payload bytes"
        );
        assert_eq!(
            linked_summary(&large_root).unwrap().total_bytes,
            50 * 1024 * 1024 * 1024
        );
        assert_eq!(fs::read_dir(large_root.join("objects")).unwrap().count(), 0);
        atomic_json(
            &root
                .join("keys")
                .join(format!("{:x}.json", Sha256::digest(b"item"))),
            &key,
        )
        .unwrap();
        assert_eq!(linked_summary(&root).unwrap().total_bytes, 17);
        assert!(adopt_linked(&root, |_, _| anyhow::bail!("pause")).is_err());
        assert_eq!(adopt_linked(&root, |_, _| Ok(())).unwrap().adopted_items, 1);
        assert_eq!(adopt_linked(&root, |_, _| Ok(())).unwrap().adopted_items, 0);
        assert_eq!(linked_summary(&root).unwrap().total_items, 0);
        let (_, stored) = keys(&root).unwrap().pop().unwrap();
        assert!(stored.path.is_none());
        assert_eq!(
            fs::read(
                root.join("objects")
                    .join(&stored.hash[..2])
                    .join(&stored.hash)
            )
            .unwrap(),
            fs::read(&source).unwrap()
        );
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::remove_file(&source).unwrap();
        let record = crate::library_media::DiskMediaRecord {
            key: "item".into(),
            revision: "v1".into(),
            mime: "video/mp4".into(),
            size: 17,
            sha256: String::new(),
            owned_path: Some(source),
            source_path: None,
            raw: serde_json::json!({}),
        };
        let pass = link_many_with_disk(
            &[],
            &[record],
            &ClosureInput {
                inventoried: BTreeSet::new(),
                references: vec![],
            },
            &root,
            |_| Ok(()),
        )
        .unwrap();
        assert!(pass.disk_records["item"].path.is_none());
        assert_eq!(linked_summary(&root).unwrap().total_items, 0);
    }
}
