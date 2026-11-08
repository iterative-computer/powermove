use crate::disk::{self, FileEntry};
use anyhow::{Context, Result, ensure};
use pm_kernel::library::{LibraryStore, StoreKey};
use pm_native_migrate::snapshot::{BlobCopy, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
};

// Durable authored state, including interrupted extension/agent work. Runtime
// credentials and unrelated terminal homes are never discovered or copied.
const FAMILIES: &[&str] = &[
    "store",
    // main c7f46a056: flat SHA-256 payloads / source links and Index records.
    // Retain these verbatim alongside the verified file-index adapter.
    "extensions",
    "Agent Workspaces",
    "Agent State",
    "Agent Change History",
    "Agent Tasks",
    "Fork Updates",
    "Attachment Cache",
    "env",
    "cloud",
    "agent-provider.json",
    "extensions-provenance.json",
    "onboarding-v1.json",
    "whats-new.json",
    "transcription-cache",
    "transcription-warm.json",
    "Media Store",
    "storage-upgrade-v1.json",
    "project-files.json",
    "backups",
    "recovery",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    pub source: PathBuf,
    pub files: BTreeMap<PathBuf, FileEntry>,
    /// The complete file map represents absence too: cutover derives tombstones
    /// by comparing this immutable base with each profile, per logical key.
    pub logical_keys: BTreeMap<String, String>,
    pub projects: BTreeMap<String, String>,
    pub indexeddb: Vec<PathBuf>,
    pub chromium_payloads: BTreeMap<PathBuf, FileEntry>,
    pub local_storage: Option<PathBuf>,
}
impl Checkpoint {
    pub fn load(root: &Path) -> Result<Self> {
        let cp: Self = disk::json(&root.join("checkpoint.json"))?;
        ensure!(cp.version == 1, "unsupported checkpoint");
        for (p, e) in &cp.files {
            relative(p)?;
            ensure!(
                disk::hash(&root.join("profile").join(p))? == (e.hash.clone(), e.bytes),
                "checkpoint content changed"
            );
        }
        for (p, e) in &cp.chromium_payloads {
            relative(p)?;
            ensure!(
                disk::hash(&root.join(p))? == (e.hash.clone(), e.bytes),
                "checkpoint media content changed"
            );
        }
        for p in cp.indexeddb.iter().chain(cp.local_storage.iter()) {
            relative(p)?;
            Snapshot::open(&root.join(p))?;
        }
        Ok(cp)
    }
    pub fn capture(source: &Path, root: &Path) -> Result<Self> {
        ensure!(!root.exists(), "checkpoint already exists");
        let stage = root.with_extension("capture");
        // Only our fixed private staging directory, while the coordinator lock
        // is held. An adopted checkpoint is never removed or rewritten.
        if stage.exists() {
            disk::archive(&stage)?;
        }
        disk::private_dir(&stage)?;
        (|| {
            let before = inventory(source)?;
            let chromium_before = (
                disk::walk(&source.join("IndexedDB"))?,
                disk::walk(&source.join("Local Storage"))?,
            );
            let needed = before.keys().try_fold(0u64, |n, p| {
                Ok::<_, anyhow::Error>(n.saturating_add(fs::metadata(source.join(p))?.len()))
            })?;
            disk::space(&stage, needed.saturating_mul(2))?;
            let profile = stage.join("profile");
            disk::private_dir(&profile)?;
            let mut files = BTreeMap::new();
            for p in before.keys() {
                let target = profile.join(p);
                disk::private_dir(target.parent().unwrap())?;
                files.insert(p.clone(), disk::copy(&source.join(p), &target)?);
            }
            let dbroot = stage.join("chromium");
            disk::private_dir(&dbroot)?;
            let mut indexeddb = Vec::new();
            let mut chromium_payloads = BTreeMap::new();
            if source.join("IndexedDB").exists() {
                let mut dirs = fs::read_dir(source.join("IndexedDB"))?
                    .map(|e| Ok(e?.path()))
                    .collect::<Result<Vec<_>>>()?;
                // The shipped app's origin first: its records win over copies
                // a development server's origin holds (closure.rs).
                dirs.sort_by_key(|d| {
                    let name = d
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    (!name.starts_with("app_powermove_"), name)
                });
                for dir in dirs {
                    let name = dir.file_name().unwrap().to_string_lossy();
                    if let Some(stem) = name.strip_suffix(".indexeddb.leveldb") {
                        let blobs = dir.with_file_name(format!("{stem}.indexeddb.blob"));
                        let out = PathBuf::from(format!("chromium/media-{}", indexeddb.len()));
                        let bytes =
                            disk::tree_bytes(&dir)?.saturating_add(disk::tree_bytes(&blobs)?);
                        disk::space(&stage, bytes.saturating_mul(2))?;
                        Snapshot::capture(
                            &dir,
                            blobs.is_dir().then_some(blobs.as_path()),
                            &stage.join(&out),
                            BlobCopy::All,
                        )?;
                        for payload in disk::walk(&stage.join(&out).join("blobs"))?.keys() {
                            let relative = out.join("blobs").join(payload);
                            let (hash, bytes) = disk::hash(&stage.join(&relative))?;
                            chromium_payloads.insert(
                                relative,
                                FileEntry {
                                    hash,
                                    bytes,
                                    mode: 0o600,
                                },
                            );
                        }
                        indexeddb.push(out);
                    }
                }
            }
            let local = source.join("Local Storage/leveldb");
            let local_storage = if local.is_dir() {
                let out = PathBuf::from("chromium/local-storage");
                disk::space(&stage, disk::tree_bytes(&local)?)?;
                Snapshot::capture(&local, None, &stage.join(&out), BlobCopy::All)?;
                Some(out)
            } else {
                None
            };
            ensure!(
                chromium_before
                    == (
                        disk::walk(&source.join("IndexedDB"))?,
                        disk::walk(&source.join("Local Storage"))?
                    ),
                "Chromium source changed across checkpoint"
            );
            ensure!(
                before == inventory(source)?,
                "source changed during checkpoint; retry after Electron quits"
            );
            ensure!(
                !pm_kernel::locks::older_electron_running(source),
                "Electron started during checkpoint"
            );
            // Rehash the mutable file-store/source files, not merely timestamps.
            for (p, e) in &files {
                ensure!(
                    disk::hash(&source.join(p))? == (e.hash.clone(), e.bytes),
                    "source changed during checkpoint"
                );
            }
            let store = LibraryStore::open(&profile);
            let mut logical_keys = BTreeMap::new();
            let mut ids = std::collections::BTreeSet::new();
            if profile.join("store").is_dir() {
                for key in store.keys()? {
                    let key = match key {
                        StoreKey::Static(k) => k,
                        StoreKey::Dynamic { prefix, id } => {
                            if prefix == "project" {
                                ids.insert(id.clone());
                            }
                            format!("{prefix}.{id}")
                        }
                        _ => continue,
                    };
                    if let Some(value) = store.read(&key)? {
                        logical_keys.insert(
                            key,
                            blake3::hash(pm_model::json::to_canonical_string(&value).as_bytes())
                                .to_hex()
                                .to_string(),
                        );
                    }
                }
                for row in store.projects()?.into_iter().chain(store.trash()?) {
                    ids.insert(row.id);
                }
                if let Some(old) = store.read("autosave")? {
                    let old = old.get("proj").unwrap_or(&old);
                    if let Some(id) = old.get("id").and_then(pm_model::json::Value::as_str) {
                        ids.insert(id.to_owned());
                    }
                }
            }
            let projects = ids
                .into_iter()
                .map(|id| {
                    let key = blake3::hash(format!("{}\0{id}", source.display()).as_bytes())
                        .to_hex()
                        .to_string();
                    (id, format!("native-{}", &key[..32]))
                })
                .collect();
            let cp = Self {
                version: 1,
                source: source.into(),
                files,
                logical_keys,
                projects,
                indexeddb,
                chromium_payloads,
                local_storage,
            };
            disk::atomic(&stage.join("checkpoint.json"), &cp)?;
            File::open(&stage)?.sync_all()?;
            fs::rename(&stage, root)?;
            File::open(root.parent().unwrap())?.sync_all()?;
            Ok(cp)
        })()
        // Leave failed captures intact. A retry archives them before starting.
    }
}
/// Metadata-only stamp of everything a checkpoint would copy. An unchanged
/// source costs a directory walk at launch, never a capture.
pub(crate) fn source_stamp(source: &Path) -> Result<String> {
    let text = format!(
        "{:?}\n{:?}\n{:?}",
        inventory(source)?,
        disk::walk(&source.join("IndexedDB"))?,
        disk::walk(&source.join("Local Storage"))?,
    );
    Ok(blake3::hash(text.as_bytes()).to_hex().to_string())
}
fn inventory(source: &Path) -> Result<BTreeMap<PathBuf, disk::Stamp>> {
    let mut out = BTreeMap::new();
    for family in FAMILIES {
        let path = source.join(family);
        if path.is_file() {
            out.extend(
                disk::walk(&path)?
                    .into_values()
                    .map(|v| (PathBuf::from(family), v)),
            );
        } else {
            out.extend(
                disk::walk(&path)?
                    .into_iter()
                    .map(|(p, v)| (PathBuf::from(family).join(p), v)),
            );
        }
    }
    // Interrupted main association publication is a sibling of the registry,
    // not a store key. Retain every receipt, never activate its destination.
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("project-files.json.pending-") && name.ends_with(".json") {
            for stamp in disk::walk(&entry.path())?.into_values() {
                out.insert(PathBuf::from(&name), stamp);
            }
        }
    }
    Ok(out)
}
pub(crate) fn relative(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "unsafe checkpoint relative path"
    );
    Ok(())
}
pub(crate) fn file_target(path: &Path) -> Result<PathBuf> {
    relative(path)?;
    let first = path.components().next().context("empty path")?.as_os_str();
    if first == "Media Store" {
        // Never mix the main reference's incompatible Index/flat-file schema
        // into the active native BLAKE3 media store.
        Ok(Path::new("electron-media-archive").join(path.strip_prefix("Media Store")?))
    } else if first == "extensions" {
        Ok(Path::new("extensions-shared").join(path.strip_prefix("extensions")?))
    } else if first
        .to_string_lossy()
        .starts_with("project-files.json.pending-")
        || [
            "Media Store",
            "storage-upgrade-v1.json",
            "project-files.json",
            "backups",
            "recovery",
        ]
        .iter()
        .any(|name| first == *name)
    {
        // These are legacy recovery inputs, not live native associations or
        // migration receipts. Never activate an old path into a user's file.
        Ok(Path::new("legacy-main").join(path))
    } else {
        Ok(path.into())
    }
}
