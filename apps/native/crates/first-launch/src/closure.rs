//! Build availability before copying media, including states reachable by undo
//! and redo. A partial asset patch is joined to its surrounding document first.
use crate::{checkpoint::Checkpoint, disk};
use anyhow::{Result, ensure};
use pm_kernel::{History, LibraryStore, library::legacy_apply_patches};
use pm_native_migrate::{
    indexeddb::Record,
    migration::{self, Availability, ClosureInput, OwnerKind, Reference},
    snapshot::Snapshot,
};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

pub(crate) fn inventory(
    cp: &Checkpoint,
    root: &Path,
    batches: &[(Snapshot, Vec<Record>)],
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
        for id in cp.projects.keys() {
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
            let available = matches!(
                disk.source_status()?,
                SourceStatus::Owned | SourceStatus::Verified
            );
            if available && let Some(path) = disk.owned_path.as_ref().or(disk.source_path.as_ref())
            {
                // Full main SHA-256 is already verified. Proxy provenance is
                // still determined from authored asset metadata, never filename.
                if !r.is_proxy()
                    && (matches_original(path, r)?
                        || (r.original_blake3.is_none() && r.fingerprint.is_none()))
                {
                    r.original_blake3 = Some(disk::hash(path)?.0);
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
        if let Some(path) = &r.source_path
            && pm_native_migrate::library_media::is_resident(path)?
            && matches_original(path, &r)?
        {
            r.original_blake3 = Some(disk::hash(path)?.0);
            r.before = Availability::Original;
        }
        if r.before == Availability::Offline
            && let Some((snapshot, record)) = candidates.first()
        {
            let path = record.payload_path(snapshot)?;
            if record.payload().size == 0 || path.is_some() {
                let original = if !r.is_proxy() {
                    if let Some(path) = &path {
                        matches_original(path, &r)?
                    } else {
                        false
                    }
                } else {
                    false
                };
                r.before = if original {
                    Availability::Original
                } else {
                    Availability::ProxyOnly
                };
            }
        }
        if r.before == Availability::Original
            && r.original_blake3.is_none()
            && let Some((snapshot, record)) = candidates.first()
            && let Some(path) = record.payload_path(snapshot)?
        {
            r.original_blake3 = Some(disk::hash(&path)?.0);
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
        poster.before = Availability::Offline;
        out.references.push(r);
        out.references.push(poster);
    }
    Ok(())
}
fn matches_original(path: &Path, r: &Reference) -> Result<bool> {
    if let Some(hash) = &r.original_blake3 {
        return Ok(disk::hash(path)?.0 == *hash);
    }
    if let Some(fp) = &r.fingerprint {
        return Ok(migration::sampled_fingerprint(path)? == *fp);
    }
    Ok(false)
}
