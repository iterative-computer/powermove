//! Build availability before copying media, including states reachable by undo
//! and redo. A partial asset patch is joined to its surrounding document first.
use crate::{checkpoint::Checkpoint, disk};
use anyhow::{Result, ensure};
use pm_kernel::{History, LibraryStore, library::legacy_apply_patches};
use pm_native_migrate::{
    indexeddb::Record,
    migration::{self, Availability, ClosureInput, OwnerKind, Reference},
    snapshot::Snapshot,
    source_access,
};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

/// `progress(done, total)` after each project.
pub(crate) fn inventory(
    cp: &Checkpoint,
    root: &Path,
    batches: &[(Snapshot, Vec<Record>)],
    progress: &mut dyn FnMut(usize, usize) -> Result<()>,
) -> Result<ClosureInput> {
    DIGESTS.with(|d| d.borrow_mut().clear());
    let found = inventory_pass(cp, root, batches, progress);
    DIGESTS.with(|d| d.borrow_mut().clear());
    found
}
fn inventory_pass(
    cp: &Checkpoint,
    root: &Path,
    batches: &[(Snapshot, Vec<Record>)],
    progress: &mut dyn FnMut(usize, usize) -> Result<()>,
) -> Result<ClosureInput> {
    let profile = root.join("profile");
    let disk_records = pm_native_migrate::library_media::read_index(&profile.join("Media Store"))?;
    let store = LibraryStore::open(&profile);
    let mut out = ClosureInput {
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
        references: Vec::new(),
    };
    if profile.join("store").is_dir() {
        let trash: BTreeSet<_> = store.trash()?.into_iter().map(|r| r.id).collect();
        let total = cp.projects.len();
        for (done, id) in cp.projects.keys().enumerate() {
            progress(done, total)?;
            if let Some(p) = store.load(id)? {
                let current = convert(&p.project)?;
                add(
                    &mut out,
                    id,
                    if trash.contains(id) {
                        OwnerKind::Trash
                    } else {
                        OwnerKind::Current
                    },
                    &current,
                    batches,
                )?;
                if let Some(saved) = &p.history {
                    match History::import(
                        saved,
                        p.project
                            .get("compId")
                            .and_then(pm_model::json::Value::as_str),
                    ) {
                        Ok(history) => {
                            let saved = history.snapshot().to_saved();
                            let entries = saved.get("entries").unwrap().as_array().unwrap();
                            // Discover full objects in dictionaries as well as reachable
                            // documents. Both patch directions can retain payloads.
                            add(&mut out, id, OwnerKind::Undo, &convert(&saved)?, batches)?;
                            let base = pm_model::canonical::canonical_project(p.project.clone());
                            let mut undo = Some(base.clone());
                            for entry in entries[..history.applied()].iter().rev() {
                                legacy_apply_patches(&mut undo, entry.get("backward").unwrap());
                                if let Some(v) = &undo {
                                    add(&mut out, id, OwnerKind::Undo, &convert(v)?, batches)?;
                                }
                            }
                            let mut redo = Some(base);
                            for entry in &entries[history.applied()..] {
                                legacy_apply_patches(&mut redo, entry.get("forward").unwrap());
                                if let Some(v) = &redo {
                                    add(&mut out, id, OwnerKind::Redo, &convert(v)?, batches)?;
                                }
                            }
                        }
                        Err(_) => {
                            // Preserve raw history, report its refusal at library import;
                            // never certify an inventory whose patches cannot be replayed.
                            out.inventoried.remove(&OwnerKind::Undo);
                            out.inventoried.remove(&OwnerKind::Redo);
                            add(&mut out, id, OwnerKind::Undo, &convert(saved)?, batches)?;
                        }
                    }
                }
            }
        }
        if let Some(takes) = store.read("takes")? {
            add(
                &mut out,
                "takes",
                OwnerKind::Take,
                &convert(&takes)?,
                batches,
            )?;
        }
        for key in cp
            .logical_keys
            .keys()
            .filter(|k| k.starts_with("agentThreads.") || k.starts_with("agentAttachment."))
        {
            if let Some(v) = store.read(key)? {
                add(&mut out, key, OwnerKind::Agent, &convert(&v)?, batches)?;
            }
        }
    }
    // Agent checkpoints/artifact JSON can contain references absent from threads.
    for path in cp.files.keys().filter(|p| {
        p.to_string_lossy().starts_with("Agent ") && p.extension().is_some_and(|x| x == "json")
    }) {
        if let Ok(value) = disk::json::<Value>(&profile.join(path)) {
            add(
                &mut out,
                &format!("agent:{}", path.display()),
                OwnerKind::Agent,
                &value,
                batches,
            )?;
        }
    }
    let mut seen = BTreeSet::new();
    for r in &mut out.references {
        let disk = disk_records.iter().find(|d| {
            (d.key == r.key || r.legacy_id.as_deref() == Some(&d.key))
                && r.revision
                    .as_ref()
                    .is_none_or(|revision| revision == &d.revision)
        });
        if let Some(disk) = disk {
            // A linked rendition stays a proxy: it never gains source provenance.
            if r.source_path.is_none() && !r.is_proxy() {
                r.source_path = disk.source_path.clone();
            }
            use pm_native_migrate::library_media::SourceStatus;
            let status = disk.source_status()?;
            if status == SourceStatus::Denied && r.before != Availability::Original {
                r.unreached = true;
            }
            let available = matches!(status, SourceStatus::Owned | SourceStatus::Verified);
            if available && let Some(path) = disk.owned_path.as_ref().or(disk.source_path.as_ref())
            {
                // Full main SHA-256 is already verified. Proxy provenance is
                // still determined from authored asset metadata, never filename.
                if !r.is_proxy()
                    && (matches_original(path, r)?
                        || (r.original_blake3.is_none() && r.fingerprint.is_none()))
                {
                    r.original_blake3 = Some(hashed(path)?);
                    r.before = Availability::Original;
                } else {
                    r.before = Availability::ProxyOnly;
                }
            }
        }
    }
    out.references
        .retain(|r| seen.insert(serde_json::to_string(r).unwrap()));
    Ok(out)
}
fn convert(v: &pm_model::json::Value) -> Result<Value> {
    Ok(serde_json::from_str(&pm_model::json::to_canonical_string(
        v,
    ))?)
}
fn assets(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Object(m) => {
            if let Some(map) = m.get("assets").and_then(Value::as_object) {
                for (id, asset) in map {
                    if let Some(mut a) = asset.as_object().cloned() {
                        a.entry("id").or_insert(Value::String(id.clone()));
                        out.push(Value::Object(a));
                    }
                }
            }
            if m.contains_key("id")
                && (m.contains_key("storageKey")
                    || m.contains_key("fingerprint")
                    || m.contains_key("playbackProxy")
                    || m.contains_key("sourcePath"))
            {
                out.push(v.clone());
            }
            for value in m.values() {
                assets(value, out);
            }
        }
        Value::Array(a) => {
            for v in a {
                assets(v, out);
            }
        }
        _ => {}
    }
}
fn add(
    out: &mut ClosureInput,
    owner: &str,
    kind: OwnerKind,
    v: &Value,
    batches: &[(Snapshot, Vec<Record>)],
) -> Result<()> {
    let mut found = Vec::new();
    assets(v, &mut found);
    for asset in found {
        let Some(id) = asset.get("id").and_then(Value::as_str) else {
            continue;
        };
        // Procedural/text assets with no persisted media identity need no blob.
        if !["storageKey", "fingerprint", "sourcePath", "playbackProxy"]
            .iter()
            .any(|k| asset.get(k).is_some())
        {
            continue;
        }
        let fp = asset
            .get("fingerprint")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let key = asset
            .get("storageKey")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| fp.as_ref().map(|f| format!("media:{f}")))
            .unwrap_or_else(|| id.into());
        let mut r = Reference {
            owner: owner.into(),
            owner_kind: kind,
            key,
            legacy_id: Some(id.into()),
            revision: asset
                .get("revision")
                .and_then(Value::as_str)
                .map(str::to_owned),
            fingerprint: fp,
            original_blake3: asset
                .get("sourceHash")
                .or_else(|| asset.get("blake3"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            source_path: asset
                .get("sourcePath")
                .and_then(Value::as_str)
                .map(Into::into),
            playback_proxy: asset
                .get("playbackProxy")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            playback_proxy_version: asset
                .get("playbackProxyVersion")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            poster: false,
            before: Availability::Offline,
            unreached: false,
        };
        // A record's storage revision is NOT the project's document revision.
        // Only accept a textual identity, never synthesize one from a number.
        let mut candidates = Vec::new();
        for (batch, (s, records)) in batches.iter().enumerate() {
            for record in records {
                if (record.key == r.key || record.key == id)
                    && r.revision
                        .as_ref()
                        .is_none_or(|rev| record.revision() == Some(rev))
                {
                    candidates.push((batch, s, record));
                }
            }
        }
        if candidates.iter().any(|(_, _, rec)| rec.key == r.key) {
            candidates.retain(|(_, _, rec)| rec.key == r.key);
        }
        // Another origin (a development server's) can hold a copy of the
        // same record; the shipped app's origin is captured first and wins.
        if let Some(&(first, _, _)) = candidates.first() {
            candidates.retain(|(batch, _, _)| *batch == first);
        }
        let candidates: Vec<_> = candidates.into_iter().map(|(_, s, r)| (s, r)).collect();
        ensure!(
            candidates.len() <= 1,
            "ambiguous media identity in checkpoint"
        );
        // The captured payload is what the original app played. Prefer it, so an
        // original in a folder macOS guards is read only when nothing else has it.
        if let Some((snapshot, record)) = candidates.first() {
            let path = record.payload_path(snapshot)?;
            if record.payload().size == 0 || path.is_some() {
                let original = match &path {
                    Some(path) if !r.is_proxy() => matches_original(path, &r)?,
                    _ => false,
                };
                r.before = if original {
                    Availability::Original
                } else {
                    Availability::ProxyOnly
                };
            }
        }
        // A project may name its media through a link (a moved or aliased
        // folder). Read the file where it really is: reads never follow links.
        if let Some(path) = &r.source_path {
            r.source_path = Some(real_media_path(path));
        }
        if r.before != Availability::Original
            && let Some(path) = r.source_path.clone()
            && pm_native_migrate::library_media::is_resident(&path)?
        {
            match matches_original(&path, &r) {
                Ok(true) => {
                    r.original_blake3 = Some(hashed(&path)?);
                    r.before = Availability::Original;
                }
                Ok(false) => {}
                Err(e) if source_access::is_unreached(&e) => r.unreached = true,
                Err(e) => return Err(e),
            }
        }
        if r.before == Availability::Original
            && r.original_blake3.is_none()
            && let Some((snapshot, record)) = candidates.first()
            && let Some(path) = record.payload_path(snapshot)?
        {
            r.original_blake3 = Some(hashed(&path)?);
        }
        // Posters are disposable renditions; record their absence explicitly
        // instead of turning a missing thumbnail into a missing original.
        let mut poster = r.clone();
        poster.key = format!("{}:poster", r.key);
        poster.legacy_id = Some(format!("{id}:poster"));
        poster.revision = None;
        poster.fingerprint = None;
        poster.original_blake3 = None;
        poster.source_path = None;
        poster.playback_proxy = false;
        poster.playback_proxy_version = 0;
        poster.poster = true;
        poster.unreached = false;
        poster.before = Availability::Offline;
        out.references.push(r);
        out.references.push(poster);
    }
    Ok(())
}
/// One pass reads each file once: history entries repeat the same assets, and a
/// payload can stand behind many references. Keyed by identity, so a file that
/// changes during the pass is read again.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Digest {
    Blake3,
    Fingerprint,
}
type DigestKey = (std::path::PathBuf, Digest, u64, i64, i64);
thread_local! {
    static DIGESTS: std::cell::RefCell<std::collections::HashMap<DigestKey, String>> =
        Default::default();
}
fn digest(path: &Path, kind: Digest, compute: impl FnOnce() -> Result<String>) -> Result<String> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path)?;
    let key = (path.to_owned(), kind, m.len(), m.mtime(), m.mtime_nsec());
    if let Some(hit) = DIGESTS.with(|d| d.borrow().get(&key).cloned()) {
        return Ok(hit);
    }
    let value = compute()?;
    DIGESTS.with(|d| d.borrow_mut().insert(key, value.clone()));
    Ok(value)
}
fn hashed(path: &Path) -> Result<String> {
    digest(path, Digest::Blake3, || Ok(disk::hash(path)?.0))
}
fn fingerprinted(path: &Path) -> Result<String> {
    digest(path, Digest::Fingerprint, || {
        migration::sampled_fingerprint(path)
    })
}
fn matches_original(path: &Path, r: &Reference) -> Result<bool> {
    if let Some(hash) = &r.original_blake3 {
        return Ok(hashed(path)? == *hash);
    }
    if let Some(fp) = &r.fingerprint {
        return Ok(fingerprinted(path)? == *fp);
    }
    Ok(false)
}
fn real_media_path(path: &Path) -> std::path::PathBuf {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => path
            .canonicalize()
            .ok()
            .filter(|real| real.is_file())
            .unwrap_or_else(|| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}
