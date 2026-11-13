//! v1.2 store keys and atomic protocol, with an explicit v1.3 codec opt-in.
use crate::{
    library::{LibraryError, LibraryStore, parse_key, value_limit},
    locks::{CopyReason, FileLock, OpenDecision, open_decision},
};
use pm_model::json::{Map, Value, to_canonical_string};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiskFormat {
    #[default]
    V1_2,
    V1_3,
}
pub fn encode_disk_value(value: &Value, format: DiskFormat) -> io::Result<Vec<u8>> {
    let plain = to_canonical_string(value).into_bytes();
    if format == DiskFormat::V1_2 || plain.len() < 32 * 1024 {
        return Ok(plain);
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&plain)?;
    let data = crate::project_io::base64::encode(&gzip.finish()?);
    let mut m = Map::new();
    m.insert("__powermoveDiskValue", Value::Number(1.0));
    m.insert("encoding", Value::from("gzip"));
    m.insert("bytes", Value::Number(plain.len() as f64));
    m.insert("data", Value::from(data));
    let envelope = to_canonical_string(&Value::Object(m)).into_bytes();
    Ok(if (envelope.len() as f64) < plain.len() as f64 * 0.9 {
        envelope
    } else {
        plain
    })
}
/// Same sibling temp / exclusive create / write / fsync / rename as storage.ts;
/// also sync the directory so a successful barrier survives a power failure.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with(path, |file| file.write_all(bytes))
}
fn atomic_write_with(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let temp = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name().unwrap().to_string_lossy(),
        crate::new_id("w")
    ));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        write(&mut f)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&temp, path)?;
        File::open(path.parent().unwrap())?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn persist(dir: &Path, key: &str, value: Option<&Value>, format: DiskFormat) -> io::Result<()> {
    let path = dir.join(format!("{key}.json"));
    match value {
        Some(v) => atomic_write(&path, &encode_disk_value(v, format)?),
        None => {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            };
            File::open(dir)?.sync_all()
        }
    }
}
type FlushReply = mpsc::Sender<Result<(), String>>;
enum WriteMsg {
    Set(String, Option<Value>),
    Batch(Vec<(String, Option<Value>)>, FlushReply),
    Flush(FlushReply),
    Stop,
}
/// Owned by the kernel. A worker debounces each key for 150 ms. Failed writes
/// stay pending, and every explicit save/close uses the error-reporting barrier.
pub struct LibraryWriter {
    store: LibraryStore,
    tx: mpsc::Sender<WriteMsg>,
    worker: Option<thread::JoinHandle<()>>,
    _lock: FileLock,
}
impl LibraryWriter {
    /// `profile` is an explicit native/test profile, never inferred from HOME.
    pub fn open(
        profile: &Path,
        format: DiskFormat,
        electron_profile: Option<&Path>,
    ) -> Result<Self, OpenDecision> {
        let decision = open_decision(profile, electron_profile);
        if decision != OpenDecision::Writable {
            return Err(decision);
        }
        fs::create_dir_all(profile.join("store"))
            .map_err(|_| OpenDecision::AsCopy(CopyReason::ReadOnly))?;
        let store = LibraryStore::open(&profile.join("store"));
        let lock = FileLock::acquire(&store.dir().join("library.lock")).map_err(|e| {
            OpenDecision::AsCopy(if e.kind() == io::ErrorKind::WouldBlock {
                CopyReason::Locked
            } else {
                CopyReason::UncertainLock
            })
        })?;
        replay_bundle(store.dir(), format)
            .map_err(|_| OpenDecision::AsCopy(CopyReason::UncertainLock))?;
        let (tx, rx) = mpsc::channel();
        let dir = store.dir().to_path_buf();
        let worker = thread::Builder::new()
            .name("pm-library-writer".into())
            .spawn(move || {
                let mut pending: BTreeMap<String, (Option<Value>, Instant)> = BTreeMap::new();
                loop {
                    let wait = pending
                        .values()
                        .map(|(_, deadline)| deadline.saturating_duration_since(Instant::now()))
                        .min()
                        .unwrap_or(Duration::from_secs(60));
                    match rx.recv_timeout(wait) {
                        Ok(WriteMsg::Set(k, v)) => {
                            pending.insert(k, (v, Instant::now() + Duration::from_millis(150)));
                        }
                        Ok(WriteMsg::Batch(values, reply)) => {
                            let r =
                                flush_pending(&dir, &mut pending, format, true).and_then(|()| {
                                    commit_bundle(&dir, &values, format).map_err(|e| e.to_string())
                                });
                            let _ = reply.send(r);
                        }
                        Ok(WriteMsg::Flush(reply)) => {
                            let r = flush_pending(&dir, &mut pending, format, true);
                            let _ = reply.send(r);
                        }
                        Ok(WriteMsg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            let _ = flush_pending(&dir, &mut pending, format, true);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let _ = flush_pending(&dir, &mut pending, format, false);
                        }
                    }
                }
            })
            .map_err(|_| OpenDecision::AsCopy(CopyReason::UncertainLock))?;
        Ok(Self {
            store,
            tx,
            worker: Some(worker),
            _lock: lock,
        })
    }
    pub fn store(&self) -> &LibraryStore {
        &self.store
    }
    pub fn set(&self, key: &str, mut value: Value) -> Result<(), LibraryError> {
        if parse_key(key).is_none() {
            return Err(LibraryError::InvalidKey(key.into()));
        }
        // The split-history writer must not erase a legacy combined history.
        if key.starts_with("projectState.") {
            self.flush()?;
        }
        if key.starts_with("projectState.")
            && let Value::Object(m) = &mut value
            && !m.contains_key("history")
            && let Some(previous) = self.store.read(key)?
            && let Some(h) = previous.get("history")
        {
            m.insert("history", h.clone());
        }
        if to_canonical_string(&value).len() > value_limit(key) {
            return Err(io::Error::other("store value exceeds its size limit").into());
        }
        self.tx
            .send(WriteMsg::Set(key.into(), Some(value)))
            .map_err(|_| io::Error::other("library writer stopped"))?;
        Ok(())
    }
    pub fn delete(&self, key: &str) -> Result<(), LibraryError> {
        if parse_key(key).is_none() {
            return Err(LibraryError::InvalidKey(key.into()));
        }
        self.tx
            .send(WriteMsg::Set(key.into(), None))
            .map_err(|_| io::Error::other("library writer stopped"))?;
        Ok(())
    }
    pub fn flush(&self) -> Result<(), LibraryError> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(WriteMsg::Flush(tx))
            .map_err(|_| io::Error::other("library writer stopped"))?;
        rx.recv()
            .map_err(|_| io::Error::other("library writer stopped"))?
            .map_err(|e| io::Error::other(e).into())
    }
    /// Explicit compaction barrier. Publish data before index, remove old TS
    /// patches only after the snapshot is durable. Native journals are retained.
    pub fn save_project(
        &self,
        id: &str,
        project: Value,
        history: Value,
        workspace: Value,
        meta: Value,
    ) -> Result<(), LibraryError> {
        self.save_project_inner(id, project, history, workspace, meta, false)
    }
    /// `save_project` that leaves a trashed project in the trash.
    pub fn compact_project(
        &self,
        id: &str,
        project: Value,
        history: Value,
        workspace: Value,
        meta: Value,
    ) -> Result<(), LibraryError> {
        self.save_project_inner(id, project, history, workspace, meta, true)
    }
    fn save_project_inner(
        &self,
        id: &str,
        project: Value,
        history: Value,
        workspace: Value,
        meta: Value,
        preserve_trash: bool,
    ) -> Result<(), LibraryError> {
        let (rows, trash) = self.entry_rows(id, &project, preserve_trash)?;
        let mut envelope = Map::new();
        envelope.insert("v", Value::from("1.2.0"));
        envelope.insert("proj", project);
        self.batch(vec![
            (format!("projectHistory.{id}"), Some(history)),
            (format!("projectWorkspace.{id}"), Some(workspace)),
            (format!("projectMeta.{id}"), Some(meta)),
            (format!("project.{id}"), Some(Value::Object(envelope))),
            ("projects".into(), Some(Value::Array(rows))),
            ("projectTrash".into(), Some(Value::Array(trash))),
            (format!("projectJournal.{id}"), None),
        ])
    }
    /// The entry of a project whose body is its `.pmv` file: the row and the
    /// metadata that names the file, never a second copy of the project.
    /// `summary` holds the row's `name`, `w` and `h`.
    pub fn save_file_entry(
        &self,
        id: &str,
        summary: &Value,
        meta: Value,
        preserve_trash: bool,
    ) -> Result<(), LibraryError> {
        let (rows, trash) = self.entry_rows(id, summary, preserve_trash)?;
        self.batch(vec![
            (format!("projectMeta.{id}"), Some(meta)),
            ("projects".into(), Some(Value::Array(rows))),
            ("projectTrash".into(), Some(Value::Array(trash))),
            // An older build kept a copy here; the file holds all of it.
            (format!("project.{id}"), None),
            (format!("projectHistory.{id}"), None),
            (format!("projectWorkspace.{id}"), None),
            (format!("projectJournal.{id}"), None),
        ])
    }
    /// A library project opened unchanged: first in recents, with its view
    /// state, and its body left as it is.
    pub fn touch_entry(&self, id: &str, summary: &Value, meta: Value) -> Result<(), LibraryError> {
        let (rows, trash) = self.entry_rows(id, summary, true)?;
        self.batch(vec![
            (format!("projectMeta.{id}"), Some(meta)),
            ("projects".into(), Some(Value::Array(rows))),
            ("projectTrash".into(), Some(Value::Array(trash))),
        ])
    }
    /// `projects` with `id`'s row first (unless it stays trashed), and
    /// `projectTrash` without it.
    fn entry_rows(
        &self,
        id: &str,
        project: &Value,
        preserve_trash: bool,
    ) -> Result<(Vec<Value>, Vec<Value>), LibraryError> {
        let mut rows = self.rows("projects")?;
        let mut row = rows
            .iter()
            .find(|r| r.get("id").and_then(Value::as_str) == Some(id))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        row.insert("id", Value::from(id));
        row.insert(
            "name",
            project
                .get("name")
                .cloned()
                .unwrap_or(Value::from("Untitled")),
        );
        row.insert("at", Value::Number(crate::locks::now_ms() as f64));
        for k in ["w", "h"] {
            if let Some(v) = project.get(k) {
                row.insert(k, v.clone());
            }
        }
        rows.retain(|r| r.get("id").and_then(Value::as_str) != Some(id));
        let mut trash = self.rows("projectTrash")?;
        let keep_trashed = preserve_trash
            && trash
                .iter()
                .any(|r| r.get("id").and_then(Value::as_str) == Some(id));
        if !keep_trashed {
            rows.insert(0, Value::Object(row));
            trash.retain(|r| r.get("id").and_then(Value::as_str) != Some(id));
        }
        Ok((rows, trash))
    }
    fn batch(&self, values: Vec<(String, Option<Value>)>) -> Result<(), LibraryError> {
        for (key, value) in &values {
            if parse_key(key).is_none() {
                return Err(LibraryError::InvalidKey(key.clone()));
            }
            if value
                .as_ref()
                .is_some_and(|v| to_canonical_string(v).len() > value_limit(key))
            {
                return Err(io::Error::other("store value exceeds its size limit").into());
            }
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(WriteMsg::Batch(values, tx))
            .map_err(|_| io::Error::other("library writer stopped"))?;
        rx.recv()
            .map_err(|_| io::Error::other("library writer stopped"))?
            .map_err(|e| io::Error::other(e).into())
    }

    pub fn rows(&self, key: &str) -> Result<Vec<Value>, LibraryError> {
        self.flush()?;
        Ok(self
            .store
            .read(key)?
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default())
    }
    /// Recoverable delete: keep document, undo, workspace and all opaque slots.
    pub fn trash(&self, id: &str) -> Result<bool, LibraryError> {
        let mut rows = self.rows("projects")?;
        let Some(index) = rows
            .iter()
            .position(|r| r.get("id").and_then(Value::as_str) == Some(id))
        else {
            return Ok(false);
        };
        let mut row = rows.remove(index);
        if let Value::Object(m) = &mut row {
            m.insert("deletedAt", Value::Number(crate::locks::now_ms() as f64));
        }
        let mut trash = self.rows("projectTrash")?;
        trash.retain(|r| r.get("id").and_then(Value::as_str) != Some(id));
        trash.insert(0, row);
        self.batch(vec![
            ("projectTrash".into(), Some(Value::Array(trash))),
            ("projects".into(), Some(Value::Array(rows))),
        ])?;
        Ok(true)
    }
    pub fn restore(&self, id: &str) -> Result<bool, LibraryError> {
        let mut trash = self.rows("projectTrash")?;
        let Some(index) = trash
            .iter()
            .position(|r| r.get("id").and_then(Value::as_str) == Some(id))
        else {
            return Ok(false);
        };
        let mut row = trash.remove(index);
        if let Value::Object(m) = &mut row {
            m.remove("deletedAt");
        }
        let mut rows = self.rows("projects")?;
        if !rows
            .iter()
            .any(|r| r.get("id").and_then(Value::as_str) == Some(id))
        {
            rows.insert(0, row);
        }
        self.batch(vec![
            ("projects".into(), Some(Value::Array(rows))),
            ("projectTrash".into(), Some(Value::Array(trash))),
        ])?;
        Ok(true)
    }
}
fn flush_pending(
    dir: &Path,
    pending: &mut BTreeMap<String, (Option<Value>, Instant)>,
    format: DiskFormat,
    all: bool,
) -> Result<(), String> {
    let keys: Vec<_> = pending
        .iter()
        .filter(|(_, (_, deadline))| all || *deadline <= Instant::now())
        .map(|(k, _)| k.clone())
        .collect();
    let mut error = None;
    for key in keys {
        let (value, deadline) = pending.get_mut(&key).unwrap();
        match persist(dir, &key, value.as_ref(), format) {
            Ok(()) => {
                pending.remove(&key);
            }
            Err(e) => {
                *deadline = Instant::now() + Duration::from_millis(150);
                error = Some(e.to_string());
            }
        }
    }
    error.map_or(Ok(()), Err)
}
impl Drop for LibraryWriter {
    fn drop(&mut self) {
        let _ = self.tx.send(WriteMsg::Stop);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

// A roll-forward intent makes a multi-slot checkpoint restartable; plain legacy
// keys remain the published format. No native-only fields enter those slots.
fn commit_bundle(
    dir: &Path,
    values: &[(String, Option<Value>)],
    format: DiskFormat,
) -> io::Result<()> {
    // Finish an interrupted previous checkpoint before replacing its intent.
    replay_bundle(dir, format)?;
    let records = Value::Array(
        values
            .iter()
            .map(|(k, v)| {
                let mut m = Map::new();
                m.insert("key", Value::from(k.clone()));
                m.insert("deleted", Value::Bool(v.is_none()));
                if let Some(v) = v {
                    m.insert("value", v.clone());
                }
                Value::Object(m)
            })
            .collect(),
    );
    atomic_write(
        &dir.join("library.pending"),
        to_canonical_string(&records).as_bytes(),
    )?;
    replay_bundle(dir, format)
}
fn replay_bundle(dir: &Path, format: DiskFormat) -> io::Result<()> {
    let path = dir.join("library.pending");
    let text = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let value = pm_model::json::parse(&text).map_err(io::Error::other)?;
    let rows = value
        .as_array()
        .ok_or_else(|| io::Error::other("Invalid library checkpoint intent"))?;
    // Validate the whole intent before writing any part of it.
    for row in rows {
        let key = row
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| io::Error::other("Missing checkpoint key"))?;
        if parse_key(key).is_none()
            || row.get("deleted").and_then(Value::as_bool).is_none()
            || (row.get("deleted").and_then(Value::as_bool) == Some(false)
                && row.get("value").is_none())
        {
            return Err(io::Error::other("Invalid checkpoint operation"));
        }
        if row
            .get("value")
            .is_some_and(|v| to_canonical_string(v).len() > value_limit(key))
        {
            return Err(io::Error::other("Checkpoint exceeds slot limit"));
        }
    }
    for row in rows {
        persist(
            dir,
            row.get("key").and_then(Value::as_str).unwrap(),
            if row.get("deleted").and_then(Value::as_bool) == Some(true) {
                None
            } else {
                row.get("value")
            },
            format,
        )?;
    }
    fs::remove_file(path)?;
    File::open(dir)?.sync_all()
}

#[cfg(test)]
mod atomic_lock_tests {
    use super::*;

    #[test]
    fn enospc_during_lock_record_write_preserves_the_previous_record() {
        let root = std::env::temp_dir().join(crate::new_id("atomic-lock"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("library.lock.owner");
        let previous = br#"{"pid":0,"heartbeat":1}"#;
        atomic_write(&path, previous).unwrap();
        let error = atomic_write_with(&path, |file| {
            file.write_all(br#"{"pid":"#)?;
            Err(io::Error::from_raw_os_error(libc::ENOSPC))
        }).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1, "failed temp is removed");
        atomic_write(&path, br#"{"pid":0,"heartbeat":2}"#).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
