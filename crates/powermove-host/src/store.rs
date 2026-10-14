//! The persistent key/value store behind `PM.store` (port of `src/main/storage.ts`).
//!
//! Each registered key is one JSON file under `<userData>/store/`. Values are
//! kept as serialized strings; writes are debounced per key and replaced
//! atomically (sibling temp file, fsync, rename). Other windows are told which
//! keys changed so their synchronous caches can refill.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::io::AsyncWriteExt;

use crate::ids::{FILE_SAVE_BYTES, STORE_VALUE_BYTES};
use crate::router::Router;
use crate::wire::Payload;

const WRITE_DELAY: Duration = Duration::from_millis(150);
/// Keys the renderer reads lazily after boot rather than in the boot snapshot.
const LAZY_PREFIXES: [&str; 6] = ["project.", "projectHistory.", "projectState.", "projectJournal.", "agentThreads.", "agentAttachment."];

/// `STATIC_KEYS` from `src/shared/store-keys.ts`.
const STATIC_KEYS: &[&str] = &[
    "projects", "openWindows", "windowTabs", "openTabs", "restoreWindows", "autoDownloadCloudMedia", "previewMemoryMiB",
    "projectTrash", "takes", "workspaces", "workspace", "defaultWorkspace", "afterEffectsExtensions", "workspaceTrash",
    "panelVisibility", "theme", "themeMode", "exportOpts", "renderQueue", "renderPresets", "bootVersion", "autosave",
    "projectsSection", "projectsSort", "projectsView", "agentScope", "agentPresentation", "agentFloatingPosition",
    "agentBubblePosition", "agentProvider", "agentModel", "agentReasoningEffort", "agentAccessMode", "agentApprovalMode",
    "agentAutoApplyPanels", "extensions", "activeTheme", "cloudAccount",
];
/// `DYNAMIC_PREFIXES` from `src/shared/store-keys.ts`.
const DYNAMIC_PREFIXES: &[&str] = &[
    "project", "projectJournal", "projectState", "projectMeta", "projectHistory", "projectWorkspace", "agentThreads",
    "agentAttachment", "agentModel", "ext",
];

/// Whether `key` is registered. Only registered keys ever become file names.
pub fn is_store_key(key: &str) -> bool {
    if STATIC_KEYS.contains(&key) {
        return true;
    }
    match key.split_once('.') {
        Some((prefix, id)) => DYNAMIC_PREFIXES.contains(&prefix) && crate::ids::is_project_id(id),
        None => false,
    }
}

fn byte_limit(key: &str) -> usize {
    if key == "takes" {
        return 24 * STORE_VALUE_BYTES;
    }
    let large = ["project.", "projectState.", "projectHistory."].iter().any(|prefix| key.starts_with(prefix));
    if large { FILE_SAVE_BYTES } else { STORE_VALUE_BYTES }
}

pub struct Store {
    directory: PathBuf,
    values: Mutex<BTreeMap<String, String>>,
    /// Bumped on every change; a delayed write only lands if it is still current.
    generations: Mutex<HashMap<String, u64>>,
    /// Serializes disk writes so a flush never races a debounced write.
    disk: tokio::sync::Mutex<()>,
}

impl Store {
    pub async fn load(directory: PathBuf) -> Result<Arc<Self>> {
        tokio::fs::create_dir_all(&directory).await.with_context(|| format!("create {}", directory.display()))?;
        let mut values = BTreeMap::new();
        let mut entries = tokio::fs::read_dir(&directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains(".tmp-") {
                let _ = tokio::fs::remove_file(entry.path()).await;
                continue;
            }
            let Some(key) = name.strip_suffix(".json") else { continue };
            if !is_store_key(key) {
                continue;
            }
            let text = tokio::fs::read_to_string(entry.path()).await?;
            if serde_json::from_str::<Value>(&text).is_ok() {
                values.insert(key.to_owned(), text);
            } else {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                let quarantine = directory.join(format!("{name}.corrupt-{stamp}"));
                tracing::warn!("store: quarantined invalid {name}");
                let _ = tokio::fs::rename(entry.path(), quarantine).await;
            }
        }
        Ok(Arc::new(Self { directory, values: Mutex::new(values), generations: Mutex::default(), disk: tokio::sync::Mutex::new(()) }))
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.values.lock().get(key).cloned()
    }

    pub fn get_value(&self, key: &str) -> Option<Value> {
        self.get(key).and_then(|text| serde_json::from_str(&text).ok())
    }

    /// Store an already-serialized JSON value.
    pub fn set_serialized(self: &Arc<Self>, key: &str, serialized: String) -> Result<()> {
        if !is_store_key(key) {
            bail!("unknown store key: {key}");
        }
        if serialized.len() > byte_limit(key) {
            bail!("serialised value is {} bytes (maximum {})", serialized.len(), byte_limit(key));
        }
        let mut value: Value = serde_json::from_str(&serialized).context("invalid JSON")?;
        let mut serialized = serialized;
        // A session update without history must never erase the last undo stack.
        if key.starts_with("projectState.") {
            if let (Value::Object(next), Some(Value::Object(previous))) = (&mut value, self.get_value(key)) {
                if !next.contains_key("history") {
                    if let Some(history) = previous.get("history") {
                        next.insert("history".into(), history.clone());
                        serialized = serde_json::to_string(&value)?;
                    }
                }
            }
        }
        self.values.lock().insert(key.to_owned(), serialized);
        self.schedule(key);
        Ok(())
    }

    pub fn set(self: &Arc<Self>, key: &str, value: &Value) -> Result<()> {
        self.set_serialized(key, serde_json::to_string(value)?)
    }

    pub fn delete(self: &Arc<Self>, key: &str) -> Result<()> {
        if !is_store_key(key) {
            bail!("unknown store key: {key}");
        }
        self.values.lock().remove(key);
        self.schedule(key);
        Ok(())
    }

    fn schedule(self: &Arc<Self>, key: &str) {
        let generation = {
            let mut generations = self.generations.lock();
            let generation = generations.entry(key.to_owned()).or_default();
            *generation += 1;
            *generation
        };
        let store = self.clone();
        let key = key.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(WRITE_DELAY).await;
            if store.generations.lock().get(&key) == Some(&generation) {
                store.persist(&key).await;
            }
        });
    }

    /// Write every key whose latest change is not on disk yet.
    pub async fn flush(&self) {
        let keys: Vec<String> = self.generations.lock().keys().cloned().collect();
        for key in keys {
            self.persist(&key).await;
        }
    }

    async fn persist(&self, key: &str) {
        let _guard = self.disk.lock().await;
        // Claim the pending change; a newer one re-registers itself.
        if self.generations.lock().remove(key).is_none() {
            return;
        }
        let path = self.directory.join(format!("{key}.json"));
        let result = match self.get(key) {
            Some(text) => atomic_write(&path, text.as_bytes()).await,
            None => match tokio::fs::remove_file(&path).await {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            },
        };
        if let Err(error) = result {
            tracing::error!("store: could not write {key}: {error:#}");
        }
    }

    fn snapshot_serialized(&self) -> Map<String, Value> {
        self.values.lock().iter().map(|(key, text)| (key.clone(), Value::String(text.clone()))).collect()
    }
}

/// Replace `path` only after every byte reached disk.
pub async fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<()> {
    let name = path.file_name().context("no file name")?.to_string_lossy();
    let temporary = path.with_file_name(format!("{name}.tmp-{}", uuid::Uuid::new_v4().simple()));
    let write = async {
        let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&temporary).await?;
        file.write_all(data).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temporary, path).await?;
        anyhow::Ok(())
    };
    let result = write.await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

#[derive(Deserialize)]
struct KeyRequest {
    key: String,
}

#[derive(Deserialize)]
struct SetRequest {
    key: String,
    value: Value,
}

#[derive(Deserialize)]
struct SetSerializedRequest {
    key: String,
    serialized: String,
}

/// Register the `store:*` channels.
pub fn register(router: &mut Router, store: Arc<Store>) {
    for channel in ["store:snapshot", "store:snapshot-sync"] {
        router.json_with(&store, channel, |store, _| async move {
            let mut snapshot: Map<String, Value> =
                store.values.lock().iter().filter_map(|(key, text)| Some((key.clone(), serde_json::from_str(text).ok()?))).collect();
            snapshot.insert("__powermoveAsyncStore".into(), Value::Bool(true));
            Ok(Value::Object(snapshot))
        });
    }
    router.json_with(&store, "store:snapshot-serialized-sync", |store, _| async move {
        let mut snapshot = store.snapshot_serialized();
        snapshot.insert("__powermoveAsyncStore".into(), json!("true"));
        Ok(Value::Object(snapshot))
    });
    router.json_with(&store, "store:bootstrap-sync", |store, _| async move {
        let mut snapshot = Map::new();
        let mut lazy = Vec::new();
        for (key, text) in store.values.lock().iter() {
            if LAZY_PREFIXES.iter().any(|prefix| key.starts_with(prefix)) {
                lazy.push(key.clone());
            } else {
                snapshot.insert(key.clone(), Value::String(text.clone()));
            }
        }
        snapshot.insert("__powermoveLazyKeys".into(), Value::String(serde_json::to_string(&lazy)?));
        snapshot.insert("__powermoveAsyncStore".into(), json!("true"));
        Ok(Value::Object(snapshot))
    });
    for channel in ["store:get-sync", "store:get-encoded-sync"] {
        router.json_with(&store, channel, |store, call| async move {
            let request: KeyRequest = call.parse()?;
            Ok(store.get(&request.key).map_or(Value::Null, Value::String))
        });
    }
    router.on_with(&store, "store:set", |store, call| async move {
        let request: SetRequest = call.parse()?;
        report(&call, &request.key, store.set(&request.key, &request.value));
        Ok(Payload::default())
    });
    router.on_with(&store, "store:set-serialized", |store, call| async move {
        let request: SetSerializedRequest = call.parse()?;
        store.set_serialized(&request.key, request.serialized)?;
        call.host.broadcast("store:changed", json!({ "keys": [request.key] }), Some(call.client.id));
        Ok(Payload::default())
    });
    router.on_with(&store, "store:delete", |store, call| async move {
        let request: KeyRequest = call.parse()?;
        report(&call, &request.key, store.delete(&request.key));
        Ok(Payload::default())
    });
    router.on_with(&store, "store:flush", |store, _| async move {
        store.flush().await;
        Ok(Payload::default())
    });
}

/// One-way writes report failures as `store:error` to the writer and
/// announce successes to every other window.
fn report(call: &crate::router::Call, key: &str, result: Result<()>) {
    match result {
        Ok(()) => call.host.broadcast("store:changed", json!({ "keys": [key] }), Some(call.client.id)),
        Err(error) => call.client.emit("store:error", json!({ "key": key, "error": format!("{error:#}") })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn persists_and_reloads_values() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::load(directory.path().to_path_buf()).await.unwrap();
        store.set("theme", &json!("dark")).unwrap();
        assert!(store.set("../escape", &json!(1)).is_err());
        store.flush().await;
        let reloaded = Store::load(directory.path().to_path_buf()).await.unwrap();
        assert_eq!(reloaded.get_value("theme"), Some(json!("dark")));
    }

    #[tokio::test]
    async fn keeps_history_when_a_session_update_omits_it() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::load(directory.path().to_path_buf()).await.unwrap();
        store.set("projectState.p1", &json!({ "history": [1], "playhead": 0 })).unwrap();
        store.set("projectState.p1", &json!({ "playhead": 5 })).unwrap();
        assert_eq!(store.get_value("projectState.p1"), Some(json!({ "history": [1], "playhead": 5 })));
    }
}
