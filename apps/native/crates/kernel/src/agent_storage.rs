//! Agent history lives beside the settings store under the same profile lock.
use crate::{library::parse_key, persistence::atomic_write, profile::safe_path};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const APPLICATION_STORE: &str = "ext.builtin-application-agent";
const MAX_VALUE: usize = 32 * 1024 * 1024;
const COMPACT_AT: u64 = 32 * 1024 * 1024;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Agent storage keys carry a scope as well as the legacy slot name. The
/// application panel's synthetic project can coincide with a real project id.
fn slot(root: &Path, application: bool, key: &str) -> io::Result<(PathBuf, bool)> {
    let Some(crate::library::StoreKey::Dynamic { prefix, id }) = parse_key(key) else {
        return Err(invalid("Invalid Agent storage key"));
    };
    let (dir, ext, attachment) = match prefix.as_str() {
        "agentThreads" => ("Agent Threads", "jsonl", false),
        "agentAttachment" => ("Agent Attachments", "json", true),
        _ => return Err(invalid("Invalid Agent storage key")),
    };
    let scope = if application {
        "Application"
    } else {
        "Projects"
    };
    let relative = Path::new(dir).join(scope).join(format!("{id}.{ext}"));
    let path = safe_path(root, &relative, false).map_err(io::Error::other)?;
    Ok((path, attachment))
}

pub(crate) fn legacy_key(key: &str) -> bool {
    matches!(
        parse_key(key),
        Some(crate::library::StoreKey::Dynamic { prefix, .. })
            if matches!(prefix.as_str(), "agentThreads" | "agentAttachment")
    )
}

#[derive(Default)]
pub(crate) struct AgentStorage {
    journals: BTreeMap<PathBuf, Journal>,
}

impl AgentStorage {
    pub fn get(&mut self, root: &Path, application: bool, key: &str) -> io::Result<Option<Value>> {
        let (path, attachment) = slot(root, application, key)?;
        if attachment {
            return read(&path)?
                .map(|bytes| serde_json::from_slice(&bytes).map_err(Into::into))
                .transpose();
        }
        Ok(self.journal(path)?.value.clone())
    }

    pub fn set(
        &mut self,
        root: &Path,
        application: bool,
        key: &str,
        expected: Option<Option<Value>>,
        value: Value,
    ) -> io::Result<()> {
        let (path, attachment) = slot(root, application, key)?;
        if attachment {
            if let Some(bytes) = read(&path)? {
                if serde_json::from_slice::<Value>(&bytes)? == value {
                    return Ok(());
                }
                return Err(invalid("Attachment id already owns different bytes"));
            }
            if expected.is_some_and(|value| value.is_some()) {
                return Err(invalid(
                    "Agent attachment changed; reload before replacing it",
                ));
            }
            let bytes = serde_json::to_vec(&value)?;
            if bytes.len() > MAX_VALUE {
                return Err(invalid("Agent attachment exceeds 32 MiB"));
            }
            ensure_parent(&path)?;
            return atomic_write(&path, &bytes);
        }
        let journal = self.journal(path)?;
        if expected.is_some_and(|expected| expected != journal.value) {
            return Err(invalid("Agent archive changed; reload before replacing it"));
        }
        let before = references(journal.value.as_ref());
        let after = references(Some(&value));
        journal.append(value)?;
        let mut removed = before.difference(&after).cloned().collect::<BTreeSet<_>>();
        if !removed.is_empty() {
            // The journal commit is already acknowledged. A collection error
            // keeps the bytes and must never turn a saved send into a retry.
            if let Err(error) = self.collect(root, application, &mut removed) {
                eprintln!("Agent attachment collection: {error}");
            }
        }
        Ok(())
    }

    fn collect(
        &self,
        root: &Path,
        application: bool,
        removed: &mut BTreeSet<String>,
    ) -> io::Result<()> {
        let (placeholder, _) = slot(root, application, "agentThreads.placeholder")?;
        let directory = placeholder.parent().expect("Agent journal directory");
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            if !entry.file_type()?.is_file() {
                return Err(invalid("Unexpected Agent journal path"));
            }
            let retained = match self.journals.get(&path) {
                Some(journal) => references(journal.value.as_ref()),
                None => references(Journal::load(path)?.value.as_ref()),
            };
            removed.retain(|id| !retained.contains(id));
            if removed.is_empty() {
                return Ok(());
            }
        }
        for id in removed.iter() {
            let (path, _) = slot(root, application, &format!("agentAttachment.{id}"))?;
            match fs::remove_file(&path) {
                Ok(()) => File::open(path.parent().unwrap())?.sync_all()?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// During profile attach, the existing kernel-state record is authoritative.
    /// A sidecar already published by an interrupted upgrade wins on retry.
    pub fn import(
        &mut self,
        root: &Path,
        application: bool,
        key: &str,
        value: &pm_model::Value,
    ) -> io::Result<()> {
        let (path, attachment) = slot(root, application, key)?;
        if path.try_exists()? {
            return Ok(());
        }
        let encoded = pm_model::json::to_canonical_string(value);
        if attachment {
            // Preserve older attachments even if their original writer
            // allowed a larger payload than today's composer.
            ensure_parent(&path)?;
            atomic_write(&path, encoded.as_bytes())
        } else {
            self.set(
                root,
                application,
                key,
                None,
                serde_json::from_str(&encoded)?,
            )
        }
    }

    fn journal(&mut self, path: PathBuf) -> io::Result<&mut Journal> {
        Ok(match self.journals.entry(path) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let journal = Journal::load(entry.key().clone())?;
                entry.insert(journal)
            }
        })
    }
}

fn references(archive: Option<&Value>) -> BTreeSet<String> {
    let Some(archive) = archive else {
        return BTreeSet::new();
    };
    let threads = ["threads", "archivedThreads"]
        .into_iter()
        .flat_map(|kind| archive[kind].as_array().into_iter().flatten());
    let draft_undo = threads
        .clone()
        .flat_map(|thread| {
            thread["nativeDraftAttachmentIds"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str);
    threads
        .flat_map(|thread| {
            std::iter::once(thread).chain(thread["conversation"].as_array().into_iter().flatten())
        })
        .flat_map(|owner| owner["attachments"].as_array().into_iter().flatten())
        .filter_map(|file| file["id"].as_str())
        .chain(draft_undo)
        .filter(|id| parse_key(&format!("agentAttachment.{id}")).is_some())
        .map(str::to_owned)
        .collect()
}

fn ensure_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().expect("Agent file has parent");
    if !parent.is_dir() {
        fs::create_dir_all(parent)?;
        // The first file's atomic write syncs its parent, also sync both
        // directory entries created beneath the already-open profile.
        File::open(parent.parent().unwrap())?.sync_all()?;
        File::open(parent.parent().unwrap().parent().unwrap())?.sync_all()?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
enum Record {
    Snapshot(Value),
    Change(Edit),
}

#[derive(Serialize, Deserialize)]
enum Edit {
    Replace(Value),
    Object {
        remove: Vec<String>,
        update: BTreeMap<String, Edit>,
    },
    Array {
        len: usize,
        update: BTreeMap<usize, Edit>,
        append: Vec<Value>,
    },
}

impl Edit {
    fn between(old: &Value, new: &Value) -> Option<Self> {
        if old == new {
            return None;
        }
        Some(match (old, new) {
            (Value::Object(old), Value::Object(new)) => Self::Object {
                remove: old
                    .keys()
                    .filter(|key| !new.contains_key(*key))
                    .cloned()
                    .collect(),
                update: new
                    .iter()
                    .filter_map(|(key, value)| {
                        let edit = match old.get(key) {
                            Some(old) => Self::between(old, value)?,
                            None => Self::Replace(value.clone()),
                        };
                        Some((key.clone(), edit))
                    })
                    .collect(),
            },
            (Value::Array(old), Value::Array(new)) => {
                let shared = old.len().min(new.len());
                Self::Array {
                    len: shared,
                    update: old
                        .iter()
                        .zip(new)
                        .enumerate()
                        .filter_map(|(i, (old, new))| Self::between(old, new).map(|edit| (i, edit)))
                        .collect(),
                    append: new[shared..].to_vec(),
                }
            }
            _ => Self::Replace(new.clone()),
        })
    }

    fn apply(self, value: &mut Value) -> io::Result<()> {
        match self {
            Self::Replace(replacement) => *value = replacement,
            Self::Object { remove, update } => {
                let object = value
                    .as_object_mut()
                    .ok_or_else(|| invalid("Invalid Agent object edit"))?;
                for key in remove {
                    object.remove(&key);
                }
                for (key, edit) in update {
                    edit.apply(object.entry(key).or_insert(Value::Null))?;
                }
            }
            Self::Array {
                len,
                update,
                append,
            } => {
                let array = value
                    .as_array_mut()
                    .ok_or_else(|| invalid("Invalid Agent array edit"))?;
                if len > array.len() {
                    return Err(invalid("Invalid Agent array length"));
                }
                array.truncate(len);
                for (i, edit) in update {
                    edit.apply(
                        array
                            .get_mut(i)
                            .ok_or_else(|| invalid("Invalid Agent array index"))?,
                    )?;
                }
                array.extend(append);
            }
        }
        Ok(())
    }
}

struct Journal {
    path: PathBuf,
    value: Option<Value>,
    committed: u64,
    snapshot_size: u64,
}

impl Journal {
    fn load(path: PathBuf) -> io::Result<Self> {
        let mut journal = Self {
            path,
            value: None,
            committed: 0,
            snapshot_size: 0,
        };
        let Some(bytes) = read(&journal.path)? else {
            return Ok(journal);
        };
        // A crash can leave an incomplete last append. A complete malformed
        // record is preserved and reported instead of losing later messages.
        for record in bytes.split_inclusive(|&byte| byte == b'\n') {
            if !record.ends_with(b"\n") {
                break;
            }
            match serde_json::from_slice(record)? {
                Record::Snapshot(value) if journal.value.is_none() => {
                    journal.snapshot_size = record.len() as u64;
                    journal.value = Some(value);
                }
                Record::Change(edit) => {
                    edit.apply(
                        journal
                            .value
                            .as_mut()
                            .ok_or_else(|| invalid("Missing Agent snapshot"))?,
                    )?;
                }
                _ => return Err(invalid("Unexpected Agent snapshot")),
            }
            journal.committed += record.len() as u64;
        }
        Ok(journal)
    }

    fn append(&mut self, next: Value) -> io::Result<()> {
        let size_limit = COMPACT_AT.max(self.snapshot_size.saturating_mul(2));
        if let Some(old) = &self.value
            && self.committed < size_limit
        {
            let Some(edit) = Edit::between(old, &next) else {
                return Ok(());
            };
            let mut bytes = serde_json::to_vec(&Record::Change(edit))?;
            bytes.push(b'\n');
            let mut file = OpenOptions::new().write(true).open(&self.path)?;
            file.set_len(self.committed)?;
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(self.committed))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            self.committed += bytes.len() as u64;
        } else {
            let mut bytes = serde_json::to_vec(&Record::Snapshot(next.clone()))?;
            bytes.push(b'\n');
            ensure_parent(&self.path)?;
            atomic_write(&self.path, &bytes)?;
            self.committed = bytes.len() as u64;
            self.snapshot_size = self.committed;
        }
        self.value = Some(next);
        Ok(())
    }
}

impl crate::Kernel {
    fn agent_profile(&self) -> Option<PathBuf> {
        self.library
            .as_ref()?
            .store()
            .dir()
            .parent()
            .map(Path::to_path_buf)
    }

    pub(crate) fn agent_get(
        &mut self,
        application: bool,
        key: &str,
    ) -> Result<Option<Value>, crate::KernelFailure> {
        if let Some(profile) = self.agent_profile() {
            return self
                .agent_storage
                .get(&profile, application, key)
                .map_err(storage_error);
        }
        // A detached kernel has no durable history. Detached panels and the
        // headless harnesses use the existing in-memory state contract.
        let value = if application {
            match self.state.settings.get(APPLICATION_STORE) {
                None => None,
                Some(value) => value
                    .as_object()
                    .ok_or_else(|| storage_error(invalid("Invalid application Agent storage")))?
                    .get(key),
            }
        } else {
            self.state.settings.get(key)
        };
        value
            .map(|value| {
                serde_json::from_str(&pm_model::json::to_canonical_string(value))
                    .map_err(storage_error)
            })
            .transpose()
    }

    pub(crate) fn agent_set(
        &mut self,
        application: bool,
        key: &str,
        expected: Option<Option<Value>>,
        value: Value,
    ) -> Result<(), crate::KernelFailure> {
        if let Some(profile) = self.agent_profile() {
            return self
                .agent_storage
                .set(&profile, application, key, expected, value)
                .map_err(storage_error);
        }
        let current = self.agent_get(application, key)?;
        if expected.is_some_and(|expected| expected != current) {
            return Err(storage_error(invalid(
                "Agent archive changed; reload before replacing it",
            )));
        }
        let value = pm_model::json::parse(&serde_json::to_string(&value).map_err(storage_error)?)
            .map_err(storage_error)?;
        let (key, value) = if application {
            let mut fields = self
                .state
                .settings
                .get(APPLICATION_STORE)
                .and_then(pm_model::Value::as_object)
                .cloned()
                .unwrap_or_default();
            fields.insert(key, value);
            (APPLICATION_STORE, pm_model::Value::Object(fields))
        } else {
            (key, value)
        };
        self.state_transaction(crate::state::StateTransaction {
            expected_revision: None,
            ops: vec![crate::state::StateOp::Setting {
                key: key.into(),
                value: Some(value),
            }],
        })?;
        Ok(())
    }

    pub(crate) fn restore_legacy_agent_settings(&mut self, value: &pm_model::Value) {
        use pm_model::Value;
        let Some(legacy) = value.get("settings").and_then(Value::as_object) else {
            return;
        };
        let current = std::sync::Arc::make_mut(&mut self.state.settings);
        for (key, value) in legacy {
            let key = key.to_string_lossy();
            if legacy_key(&key) {
                current.entry(key).or_insert_with(|| value.clone());
            }
        }
        let Some(legacy_app) = legacy.get(APPLICATION_STORE).and_then(Value::as_object) else {
            return;
        };
        let mut app = current.get(APPLICATION_STORE)
            .and_then(Value::as_object).cloned().unwrap_or_default();
        for (key, value) in legacy_app {
            if legacy_key(&key.to_string_lossy()) && app.get_key(key).is_none() {
                app.insert(key.clone(), value.clone());
            }
        }
        if !app.is_empty() {
            current.insert(APPLICATION_STORE.to_owned(), Value::Object(app));
        }
    }

    /// Publish every sidecar before removing the legacy keys. The original
    /// settings record remains available for rollback as a one-time backup.
    pub(crate) fn migrate_agent_storage(
        &mut self,
        profile: &Path,
        record_exists: bool,
    ) -> io::Result<()> {
        use pm_model::{Value as Native, json::to_canonical_string};
        let mut settings = self.state.settings.clone();
        let next = std::sync::Arc::make_mut(&mut settings);
        let project_keys = next
            .keys()
            .filter(|key| legacy_key(key))
            .cloned()
            .collect::<Vec<_>>();
        let application_keys = next
            .get(APPLICATION_STORE)
            .and_then(Native::as_object)
            .map(|fields| {
                fields
                    .keys()
                    .map(|key| key.to_string_lossy())
                    .filter(|key| legacy_key(key))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if project_keys.is_empty() && application_keys.is_empty() {
            return Ok(());
        }
        for key in project_keys {
            let value = next.remove(&key).expect("selected legacy key");
            self.agent_storage.import(profile, false, &key, &value)?;
        }
        if !application_keys.is_empty() {
            let mut fields = next
                .get(APPLICATION_STORE)
                .and_then(Native::as_object)
                .cloned()
                .expect("selected application Agent fields");
            for key in application_keys {
                let value = fields.get(&key).expect("selected application Agent key");
                self.agent_storage.import(profile, true, &key, value)?;
                fields.remove(&key);
            }
            if fields.is_empty() {
                next.remove(APPLICATION_STORE);
            } else {
                next.insert(APPLICATION_STORE.into(), Native::Object(fields));
            }
        }

        let backup = safe_path(
            profile,
            Path::new("kernel-state.pre-agent-storage.json"),
            false,
        )
        .map_err(io::Error::other)?;
        let record =
            safe_path(profile, Path::new("kernel-state.json"), false).map_err(io::Error::other)?;
        if !backup.try_exists()? {
            if record_exists {
                fs::hard_link(&record, &backup)?;
                File::open(profile)?.sync_all()?;
            } else {
                atomic_write(
                    &backup,
                    to_canonical_string(&self.state.durable_value()).as_bytes(),
                )?;
            }
        }
        // Keep the original record intact until the library has committed
        // the changed settings. An interrupted attach can retry the migration.
        self.state.settings = settings;
        Ok(())
    }
}

fn storage_error(error: impl std::fmt::Display) -> crate::KernelFailure {
    crate::KernelFailure::Storage(error.to_string())
}
