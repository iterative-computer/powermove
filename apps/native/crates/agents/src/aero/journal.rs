use pm_aero_core::{Remote, Runtime};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub(super) struct Append {
    pub id: u64,
    pub start: u64,
    pub end: u64,
    pub events: String,
}

pub(super) struct Journal {
    dir: PathBuf,
    _lock: File,
    events: Vec<Value>,
    seq: u64,
}

impl Journal {
    pub fn open(dir: &Path) -> Result<Self, String> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(show)?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(dir.join("writer.lock"))
            .map_err(show)?;
        lock.try_lock()
            .map_err(|e| format!("Aero's session already has a writer or its lock failed: {e}"))?;
        let events = read::<Vec<Value>>(dir, "journal.json")?.unwrap_or_default();
        let seq = events
            .last()
            .and_then(|event| event["seq"].as_u64())
            .unwrap_or_default();
        Ok(Self {
            dir: dir.into(),
            _lock: lock,
            events,
            seq,
        })
    }

    pub fn open_core(&self, runtime: &mut Runtime, id: &str) -> Result<(), String> {
        if self.events.is_empty() {
            runtime.open_session(id).map_err(|e| e.to_string())?;
        } else {
            let events = serde_json::to_string(&self.events).map_err(show)?;
            runtime
                .open_journal(id, &events, true)
                .map_err(|e| format!("Could not resume the saved Aero session: {e}"))?;
        }
        Ok(())
    }

    pub fn append(&mut self, edge: Remote, append: Append) -> Result<(), String> {
        let events: Vec<Value> = serde_json::from_str(&append.events).map_err(show)?;
        let fenced = append.start != self.seq + 1
            || append.end < append.start
            || events.last().and_then(|event| event["seq"].as_u64()) != Some(append.end)
            || events
                .iter()
                .enumerate()
                .any(|(i, event)| event["seq"].as_u64() != Some(append.start + i as u64));
        if fenced {
            let error = "Aero's journal append does not follow the durable head.";
            let _ = edge.journal_fail(append.id, error, true);
            return Err(error.into());
        }
        self.events.extend(events);
        if let Err(error) = store(&self.dir, "journal.json", &self.events) {
            let _ = edge.journal_fail(append.id, &error, false);
            return Err(error);
        }
        self.seq = append.end;
        edge.journal_ack(append.id).map_err(show)?;
        Ok(())
    }

    pub fn store_input(&self, id: &str, input: &str) -> Result<(), String> {
        if !crate::valid_id(id, 200) {
            return Err("Invalid Aero message id.".into());
        }
        store(&self.dir, &format!("{id}.input"), &input)
    }

    pub fn context_prompt(&self, context: &Value, prompt: &str) -> Result<String, String> {
        let previous = read::<Value>(&self.dir, "context.json")?;
        let header = if let Some(previous) = previous {
            let delta = context
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(key, value)| previous.get(*key) != Some(*value))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<serde_json::Map<_, _>>();
            (!delta.is_empty()).then(|| format!("Project updates\n{}", Value::Object(delta)))
        } else {
            Some(format!("Project at session start\n{}", json!(context)))
        };
        store(&self.dir, "context.json", context)?;
        Ok(header.map_or_else(
            || prompt.to_owned(),
            |header| format!("{header}\n\n{prompt}"),
        ))
    }
}

fn read<T: serde::de::DeserializeOwned>(dir: &Path, name: &str) -> Result<Option<T>, String> {
    match fs::read(dir.join(name)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(show),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(show(error)),
    }
}

fn store(dir: &Path, name: &str, value: &impl serde::Serialize) -> Result<(), String> {
    let pending = dir.join("pending");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&pending)
        .map_err(show)?;
    serde_json::to_writer(&mut file, value).map_err(show)?;
    file.flush().map_err(show)?;
    file.sync_all().map_err(show)?;
    fs::rename(pending, dir.join(name)).map_err(show)?;
    File::open(dir).and_then(|dir| dir.sync_all()).map_err(show)
}

fn show(error: impl std::fmt::Display) -> String {
    format!("Aero's session storage failed: {error}")
}
