//! Build availability before copying media, including states reachable by undo
//! and redo. A partial asset patch is joined to its surrounding document first.
use crate::{checkpoint::Checkpoint, disk};
use anyhow::{Result, ensure};
use pm_kernel::{History, LibraryStore, library::legacy_apply_patches};
use pm_native_migrate::{
    indexeddb::Record,
    migration::{Availability, ClosureInput, OwnerKind, Reference},
    snapshot::Snapshot,
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
    let found = inventory_pass(cp, root, batches, progress);
    found
}
fn inventory_pass(
    cp: &Checkpoint,
    root: &Path,
    batches: &[(Snapshot, Vec<Record>)],
    progress: &mut dyn FnMut(usize, usize) -> Result<()>,
) -> Result<ClosureInput> {
    let profile = root.join("profile");
    let disk_records = pm_native_migrate::library_media::read_link_index(
        &profile.join("Media Store"),
        &cp.source.join("Media Store"),
    )?;
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
                            let base = asset_state(&p.project);
                            let mut undo = Some(base.clone());
                            for entry in entries[..history.applied()].iter().rev() {
                                legacy_apply_patches(
                                    &mut undo,
                                    &asset_patches(entry.get("backward").unwrap()),
                                );
                                if let Some(v) = &undo {
                                    add(&mut out, id, OwnerKind::Undo, &convert(v)?, batches)?;
                                }
                            }
                            let mut redo = Some(base);
                            for entry in &entries[history.applied()..] {
                                legacy_apply_patches(
                                    &mut redo,
                                    &asset_patches(entry.get("forward").unwrap()),
                                );
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
            if disk
                .source_path
                .as_ref()
                .or(disk.owned_path.as_ref())
                .is_some_and(|path| {
                    pm_native_migrate::library_media::is_resident(path).unwrap_or(false)
                        && std::fs::symlink_metadata(path)
                            .is_ok_and(|m| m.is_file() && m.len() == disk.size)
                })
            {
                r.before = if r.is_proxy() {
                    Availability::ProxyOnly
                } else {
                    Availability::Original
                };
            }
        }
    }
    out.references
        .retain(|r| seen.insert(serde_json::to_string(r).unwrap()));
    Ok(out)
}
// Media closure depends on asset state, not layer geometry or animation data.
// Replay the same asset patches on a projected document so a long edit history
// does not repeatedly clone/serialize the entire project at every undo step.
fn asset_state(value: &pm_model::json::Value) -> pm_model::json::Value {
    let mut out = pm_model::json::Value::Object(Default::default());
    if let Some(assets) = value.get("assets") {
        out.as_object_mut()
            .unwrap()
            .insert("assets", assets.clone());
    }
    out
}
fn asset_patches(value: &pm_model::json::Value) -> pm_model::json::Value {
    let patches = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|patch| {
            let path = patch
                .get("path")
                .and_then(pm_model::json::Value::as_array)?;
            if let Some(first) = path.first() {
                return (first.as_str() == Some("assets")).then(|| patch.clone());
            }
            let mut patch = patch.clone();
            if let Some(value) = patch.get("value") {
                let projected = asset_state(value);
                patch.as_object_mut().unwrap().insert("value", projected);
            }
            Some(patch)
        })
        .collect();
    pm_model::json::Value::Array(patches)
}
fn convert(v: &pm_model::json::Value) -> Result<Value> {
    // Copy only fields consumed by Reference. Geometry can be enormous, and
    // serializing/parsing it into a second JSON representation costs no media
    // information. Still walk unknown containers so nested asset owners survive.
    fn record(v: &pm_model::json::Value, id: Option<&str>) -> Result<Value> {
        let mut out = serde_json::Map::new();
        for field in [
            "id",
            "storageKey",
            "fingerprint",
            "sourceHash",
            "blake3",
            "sourcePath",
            "revision",
            "playbackProxy",
            "playbackProxyVersion",
        ] {
            if let Some(value) = v.get(field) {
                out.insert(
                    field.into(),
                    serde_json::from_str(&pm_model::json::to_canonical_string(value))?,
                );
            }
        }
        if !out.contains_key("id")
            && let Some(id) = id
        {
            out.insert("id".into(), Value::String(id.into()));
        }
        Ok(Value::Object(out))
    }
    fn walk(v: &pm_model::json::Value, out: &mut Vec<Value>) -> Result<()> {
        match v {
            pm_model::json::Value::Object(m) => {
                if let Some(map) = v.get("assets").and_then(pm_model::json::Value::as_object) {
                    for (id, asset) in map.iter() {
                        if asset.as_object().is_some() {
                            out.push(record(asset, id.as_str())?);
                        }
                    }
                }
                if v.get("id").is_some()
                    && ["storageKey", "fingerprint", "playbackProxy", "sourcePath"]
                        .iter()
                        .any(|k| v.get(k).is_some())
                {
                    out.push(record(v, None)?);
                }
                for (_, value) in m.iter() {
                    walk(value, out)?;
                }
            }
            pm_model::json::Value::Array(a) => {
                for value in a {
                    walk(value, out)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(v, &mut out)?;
    Ok(Value::Array(out))
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
        if let Some((snapshot, record)) = candidates.first() {
            if snapshot
                .linked_blob_path(record.database, record.payload().number)?
                .is_some()
            {
                r.before = if r.is_proxy() {
                    Availability::ProxyOnly
                } else {
                    Availability::Original
                };
            }
        }
        if let Some(path) = &r.source_path {
            r.source_path = Some(real_media_path(path));
        }
        if r.before != Availability::Original
            && let Some(path) = &r.source_path
            && pm_native_migrate::library_media::is_resident(path)?
        {
            let expected = r
                .fingerprint
                .as_deref()
                .and_then(|f| f.strip_prefix("v2:"))
                .and_then(|f| f.split(':').next())
                .and_then(|s| s.parse::<u64>().ok());
            if expected.is_some_and(|size| {
                std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == size)
            }) {
                r.before = Availability::Original;
            }
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

#[cfg(test)]
mod projection_tests {
    use super::*;
    #[test]
    fn projected_history_matches_full_replay_for_partial_assets_and_root_replacement() {
        let parse = |text| pm_model::json::parse(text).unwrap();
        let base = parse(
            r#"{"assets":{"a":{"id":"a","storageKey":"old","sourcePath":"/old"}},"layers":[{"name":"layer"}]}"#,
        );
        let mut full = Some(base.clone());
        let mut projected = Some(asset_state(&base));
        for text in [
            r#"[{"exists":true,"path":["layers",0,"name"],"value":"edited"},{"exists":true,"path":["assets","a","storageKey"],"value":"new"}]"#,
            r#"[{"exists":true,"path":[],"value":{"assets":{"b":{"id":"b","sourcePath":"/b"}},"layers":[{}]}}]"#,
            r#"[{"exists":false,"path":["assets","b"]}]"#,
        ] {
            let patches = parse(text);
            legacy_apply_patches(&mut full, &patches);
            legacy_apply_patches(&mut projected, &asset_patches(&patches));
            assert_eq!(full.as_ref().map(asset_state), projected);
        }
    }
}
