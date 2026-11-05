//! Current-state, bidirectional store merge and unpublished Restore preparation.
//! The kernel owner must compact ALL open journals and capture current settings,
//! projects/history and secret tombstones under its save/quit/import leases first.
//! This API deliberately cannot certify media/Keychain/whole-app Restore proof.
use crate::disk;
use anyhow::{Result, ensure};
use pm_kernel::{
    library::parse_key,
    persistence::{DiskFormat, LibraryWriter},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::Path,
};

pub type Store = BTreeMap<String, Value>;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Conflict {
    /// Components, not dotted paths: extension keys may themselves contain dots.
    pub path: Vec<String>,
    pub base: Option<Value>,
    pub current: Option<Value>,
    pub incoming: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    pub values: Store,
    pub conflicts: Vec<Conflict>,
}
/// Same algorithm for forward and reverse: `current` is the destination's latest
/// state, `incoming` is the other application's CURRENT state, never its backup.
/// Missing differs from JSON null. Extension objects merge one member at a time;
/// arrays/projects/history/workspaces remain atomic to preserve authored intent.
pub fn merge(base: &Store, current: &Store, incoming: &Store) -> Plan {
    let keys: BTreeSet<_> = base
        .keys()
        .chain(current.keys())
        .chain(incoming.keys())
        .collect();
    let mut plan = Plan {
        values: Store::new(),
        conflicts: Vec::new(),
    };
    for key in keys {
        let value = merge_value(
            base.get(key),
            current.get(key),
            incoming.get(key),
            vec![key.clone()],
            key.starts_with("ext."),
            &mut plan.conflicts,
        );
        if let Some(value) = value {
            plan.values.insert(key.clone(), value);
        }
    }
    plan
}
fn merge_value(
    base: Option<&Value>,
    current: Option<&Value>,
    incoming: Option<&Value>,
    path: Vec<String>,
    members: bool,
    conflicts: &mut Vec<Conflict>,
) -> Option<Value> {
    if current == incoming || incoming == base {
        return current.cloned();
    }
    if current == base {
        return incoming.cloned();
    }
    if members
        && let (Some(Value::Object(b)), Some(Value::Object(c)), Some(Value::Object(i))) =
            (base, current, incoming)
    {
        let mut out = serde_json::Map::new();
        let keys: BTreeSet<_> = b.keys().chain(c.keys()).chain(i.keys()).collect();
        for key in keys {
            let mut child = path.clone();
            child.push(key.clone());
            if let Some(value) =
                merge_value(b.get(key), c.get(key), i.get(key), child, false, conflicts)
            {
                out.insert(key.clone(), value);
            }
        }
        return Some(Value::Object(out));
    }
    conflicts.push(Conflict {
        path,
        base: base.cloned(),
        current: current.cloned(),
        incoming: incoming.cloned(),
    });
    current.cloned()
}

/// Publish a new, Electron-readable store; never overwrite either source profile.
/// Caller passes a new sibling destination outside both inputs and holds their
/// leases. Full input maps and conflicts remain archived even for unknown keys.
/// A failure keeps `.preparing` intact; an explicit retry archives that attempt.
/// No returned path is an authorization to swap profiles/install applications.
pub fn prepare_store(
    destination: &Path,
    base: &Store,
    current: &Store,
    incoming: &Store,
    mut published: impl FnMut(&str) -> Result<()>,
) -> Result<Plan> {
    ensure!(
        destination.is_absolute(),
        "Restore destination must be absolute"
    );
    ensure!(
        std::fs::symlink_metadata(destination)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "Restore destination already exists or is inaccessible"
    );
    let destination = disk::resolved(destination)?;
    let stage = destination.with_extension("preparing");
    if fs::symlink_metadata(&stage).is_ok() {
        disk::archive(&stage)?;
    }
    disk::private_dir(&stage)?;
    let plan = merge(base, current, incoming);
    disk::atomic(
        &stage.join("restore-inputs.json"),
        &(base, current, incoming),
    )?;
    disk::atomic(&stage.join("restore-plan.json"), &plan)?;
    {
        let writer = LibraryWriter::open(&stage, DiskFormat::V1_2, None)
            .map_err(|e| anyhow::anyhow!("Restore writer unavailable: {e:?}"))?;
        for (key, value) in &plan.values {
            if parse_key(key).is_some() {
                writer.set(key, crate::to_kernel(value)?)?;
                writer.flush()?;
                published(key)?;
            }
        }
        writer.flush()?;
    }
    // Unknown keys are retained in restore-inputs/plan; do not teach the kernel
    // unregistered schemas or flatten extension storage to make a test pass.
    disk::atomic(&stage.join("restore-store-complete.json"), &true)?;
    File::open(&stage)?.sync_all()?;
    fs::rename(&stage, &destination)?;
    File::open(destination.parent().unwrap())?.sync_all()?;
    Ok(plan)
}
