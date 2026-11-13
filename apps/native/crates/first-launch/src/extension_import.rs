//! Publish captured extensions into the active native root. Receipts bind the
//! entire source tree (including its Store marker); native edits win as a unit.
use crate::{
    State,
    checkpoint::Checkpoint,
    delta::{Conflict, ConflictKind},
    disk, publish,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Receipt {
    hash: String,
    #[serde(default)]
    pending: Option<String>,
}
fn tree(root: &Path) -> Result<Option<String>> {
    if !root.exists() {
        return Ok(None);
    }
    let mut files = BTreeMap::new();
    for p in disk::walk(root)?.keys() {
        files.insert(p.clone(), disk::hash(&root.join(p))?.0);
    }
    Ok(Some(
        blake3::hash(&serde_json::to_vec(&files)?)
            .to_hex()
            .to_string(),
    ))
}
fn conflict(state: &mut State, id: &str) {
    state.conflicts.insert(Conflict {
        kind: ConflictKind::File,
        id: format!("extensions/{id}"),
    });
}
pub(crate) fn import(
    cp: &Checkpoint,
    capture: &Path,
    native: &Path,
    state: &mut State,
) -> Result<()> {
    let active = native.join("extensions");
    disk::private_dir(&active)?;
    let mut groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in cp.files.keys().filter(|p| p.starts_with("extensions")) {
        let Some(id) = path.iter().nth(1).and_then(|s| s.to_str()) else {
            continue;
        };
        if id.starts_with('.') {
            continue;
        }
        groups.entry(id.into()).or_default().push(path.clone());
    }
    let mut blocked = std::collections::BTreeSet::new();
    for (id, files) in groups {
        let source = capture.join("profile/extensions").join(&id);
        if !source.join("manifest.json").is_file() {
            continue;
        }
        let next = tree(&source)?.unwrap();
        let target = active.join(&id);
        let current = tree(&target)?;
        let receipt = state
            .extension_receipts
            .get(&id)
            .cloned()
            .unwrap_or_default();
        if current.is_none() && receipt.hash.is_empty() {
            let shared = tree(&native.join("extensions-shared").join(&id))?;
            if shared.as_ref().is_some_and(|hash| hash != &next) {
                conflict(state, &id);
                blocked.insert(id);
                continue;
            }
        }
        if current.as_ref() != Some(&next) {
            let allowed = current.as_ref() == Some(&receipt.hash)
                || (current.is_none()
                    && (receipt.hash.is_empty() || receipt.pending.as_ref() == Some(&next)));
            if !allowed {
                conflict(state, &id);
                blocked.insert(id);
                continue;
            }
            let stage = active.join(format!(".{id}.migration"));
            if stage.exists() {
                disk::archive(&stage)?;
            }
            disk::private_dir(&stage)?;
            for relative in files {
                let suffix = relative.strip_prefix(Path::new("extensions").join(&id))?;
                let out = stage.join(suffix);
                disk::private_dir(out.parent().unwrap())?;
                let copied = disk::copy(&capture.join("profile").join(&relative), &out)?;
                ensure!(
                    copied.hash == cp.files[&relative].hash,
                    "extension copy differs"
                );
            }
            state
                .extension_receipts
                .entry(id.clone())
                .or_default()
                .pending = Some(next.clone());
            publish(native, state)?;
            if target.exists() {
                // Hidden archives cannot become discovered extension providers.
                let old = active.join(format!(".{id}.previous"));
                if old.exists() {
                    disk::archive(&old)?;
                }
                fs::rename(&target, &old)?;
                File::open(&active)?.sync_all()?;
            }
            fs::rename(&stage, &target)?;
            File::open(&active)?.sync_all()?;
        }
        state.extension_receipts.insert(
            id,
            Receipt {
                hash: next,
                pending: None,
            },
        );
        publish(native, state)?;
    }
    // Merge provenance per extension: unrelated native Store installs and local
    // authorship must survive. Never infer trust from a missing marker.
    let source = capture.join("profile/extensions-provenance.json");
    let target = native.join("extensions-provenance.json");
    let incoming: BTreeMap<String, Value> = if source.exists() {
        disk::json(&source)?
    } else {
        BTreeMap::new()
    };
    let mut current: BTreeMap<String, Value> = if target.exists() {
        disk::json(&target)?
    } else {
        BTreeMap::new()
    };
    for (id, value) in incoming {
        if blocked.contains(&id) {
            continue;
        }
        if current.get(&id) == Some(&value) {
            state.extension_provenance.insert(id, value);
            continue;
        }
        if current.get(&id) == state.extension_provenance.get(&id) {
            current.insert(id.clone(), value.clone());
            disk::atomic(&target, &current)?;
            state.extension_provenance.insert(id, value);
        } else {
            conflict(state, &format!("{id}/provenance"));
        }
    }
    publish(native, state)
}
