//! What Aero keeps beside the core's journal (pm-aero-core's session
//! folder): each prompt and steer before the core takes it (the open sets
//! inputs_durable), and the project context the next turn's delta is
//! measured against.
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub(super) struct Journal {
    dir: PathBuf,
}

impl Journal {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    pub fn has_input(&self, id: &str) -> bool {
        self.dir.join(format!("{id}.input")).is_file()
    }

    pub fn store_input(&self, id: &str, input: &str) -> Result<(), String> {
        if !crate::valid_id(id, 200) {
            return Err("Invalid Aero message id.".into());
        }
        store(&self.dir, &format!("{id}.input"), &input)
    }

    /// The project header at session start, then only what changed.
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
