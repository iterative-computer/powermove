//! Extension discovery and records (port of `registry.ts` and `discovery.ts`).
//!
//! Built-ins are compiled into the renderer; the registry only lists their
//! manifests. User mods under `<userData>/extensions` are compiled into
//! `<userData>/extensions-build/<id>/bundle.js`. Every method blocks (file
//! system, compiler); callers run them on blocking threads.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::compiler::{Compiled, Compiler};
use super::manifest::{ForkUpdate, Health, Manifest, Record, Scope, builtin_fork_origin, is_extension_id, limits, parse_manifest, truncate_error};

const SOURCE_FILE_BYTES: u64 = 64 * 1024;

/// Writes the enabled map to the `extensions` store key.
pub type Persist = Box<dyn Fn(Value) + Send + Sync>;

struct Candidate {
    id: String,
    scope: Scope,
    dir: PathBuf,
    manifest: Option<Manifest>,
    error: Option<String>,
}

struct CachedBuild {
    signature: String,
    result: Result<Compiled, String>,
}

#[derive(Default)]
struct State {
    records: HashMap<String, Record>,
    builds: HashMap<PathBuf, CachedBuild>,
    enabled: BTreeMap<String, bool>,
}

pub struct Registry {
    pub user_dir: PathBuf,
    pub build_dir: PathBuf,
    pub builtin_dir: PathBuf,
    pub compiler: Compiler,
    state: Mutex<State>,
    /// Refreshes run one at a time (registry.ts `refreshQueue`).
    refreshing: Mutex<()>,
    persist: Persist,
}

#[derive(Deserialize)]
pub struct CreateRequest {
    pub manifest: Value,
    pub files: Map<String, Value>,
}

#[derive(Serialize)]
pub struct SourceFile {
    pub path: String,
    pub text: String,
}

/// `ExtensionHealthReport['health']`: what a window may report.
#[derive(Debug, Clone, PartialEq)]
pub enum Reported {
    Ok,
    ActivationError(String),
    RuntimeError(String),
}

impl Reported {
    /// `{ state: 'ok' }` or `{ state: 'activation-error' | 'runtime-error', error }`, nothing more.
    pub fn parse(value: &Value) -> Option<Self> {
        let health = value.as_object()?;
        let error = || {
            health
                .get("error")
                .and_then(Value::as_str)
                .filter(|error| !error.is_empty() && error.encode_utf16().count() <= limits::ERROR_CHARS && health.len() == 2)
                .map(str::to_owned)
        };
        match health.get("state")?.as_str()? {
            "ok" if health.len() == 1 => Some(Self::Ok),
            "activation-error" => error().map(Self::ActivationError),
            "runtime-error" => error().map(Self::RuntimeError),
            _ => None,
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

pub fn validate_id(id: &str) -> Result<&str> {
    if !is_extension_id(id) {
        bail!("Invalid extension id.");
    }
    Ok(id)
}

impl Registry {
    pub fn new(user_dir: PathBuf, build_dir: PathBuf, builtin_dir: PathBuf, compiler: Compiler, enabled: Option<Value>, persist: Persist) -> Self {
        let enabled = match enabled {
            Some(Value::Object(map)) => map.into_iter().filter_map(|(id, value)| value.as_bool().filter(|_| is_extension_id(&id)).map(|value| (id, value))).collect(),
            _ => BTreeMap::new(),
        };
        Self { user_dir, build_dir, builtin_dir, compiler, state: Mutex::new(State { enabled, ..State::default() }), refreshing: Mutex::new(()), persist }
    }

    /// Records sorted builtin, user, project; then by id.
    pub fn list(&self) -> Vec<Record> {
        let state = self.state.lock();
        let mut records: Vec<Record> = state.records.values().cloned().collect();
        let rank = |scope: Scope| match scope {
            Scope::Builtin => 0,
            Scope::User => 1,
            Scope::Project => 2,
        };
        records.sort_by(|left, right| rank(left.scope).cmp(&rank(right.scope)).then_with(|| left.id.cmp(&right.id)));
        records
    }

    pub fn record(&self, id: &str) -> Option<Record> {
        self.state.lock().records.get(id).cloned()
    }

    fn require(&self, id: &str) -> Result<Record> {
        self.record(id).ok_or_else(|| anyhow!("Unknown extension: {id}"))
    }

    fn enabled_for(&self, id: &str) -> bool {
        *self.state.lock().enabled.get(id).unwrap_or(&true)
    }

    fn persist_enabled(&self, state: &State) {
        (self.persist)(serde_json::to_value(&state.enabled).unwrap_or_default());
    }

    /// Rescan and rebuild `ids`, or everything when `None`. Unchanged sources
    /// reuse their last build.
    pub fn refresh(&self, ids: Option<&[String]>) -> Result<()> {
        if let Some(ids) = ids {
            for id in ids {
                validate_id(id)?;
            }
        }
        let _running = self.refreshing.lock();
        let filter: Option<HashSet<&str>> = ids.map(|ids| ids.iter().map(String::as_str).collect());
        std::fs::create_dir_all(&self.user_dir)?;
        std::fs::create_dir_all(&self.build_dir)?;

        let builtins = read_builtins(&self.builtin_dir);
        let shipped: HashMap<String, Manifest> = builtins.iter().filter_map(|builtin| Some((builtin.id.clone(), builtin.manifest.clone()?))).collect();
        // Later scopes deliberately replace earlier ones: builtin < user.
        let mut candidates: BTreeMap<String, Candidate> = BTreeMap::new();
        for candidate in builtins.into_iter().chain(scan_extension_dir(&self.user_dir, Scope::User)) {
            candidates.insert(candidate.id.clone(), candidate);
        }

        {
            let mut state = self.state.lock();
            state.records.retain(|id, _| candidates.contains_key(id) || filter.as_ref().is_some_and(|filter| !filter.contains(id.as_str())));
        }

        for (id, candidate) in &candidates {
            if filter.as_ref().is_some_and(|filter| !filter.contains(id.as_str())) {
                continue;
            }
            let record = self.build_record(candidate, &shipped);
            self.state.lock().records.insert(id.clone(), record);
        }
        Ok(())
    }

    fn build_record(&self, candidate: &Candidate, shipped: &HashMap<String, Manifest>) -> Record {
        let id = &candidate.id;
        let updated_at = now();
        let record = |manifest: Option<Manifest>, update: Option<ForkUpdate>, enabled: bool, health: Health, bundle: Option<&Compiled>| Record {
            trust: if candidate.scope == Scope::Builtin { "builtin" } else { "local" },
            id: id.clone(),
            scope: candidate.scope,
            manifest,
            update,
            dir: candidate.dir.display().to_string(),
            enabled,
            bundle_url: bundle.map(|bundle| format!("app://powermove/ext/{id}/bundle.js?v={}", bundle.hash)),
            bundle_hash: bundle.map(|bundle| bundle.hash.clone()),
            health,
            updated_at,
        };
        let disabled_or = |enabled: bool, health: Health| if enabled { health } else { Health::Disabled };
        let invalid = || Health::ManifestError { error: truncate_error(candidate.error.as_deref().unwrap_or("Invalid manifest")) };

        let Some(manifest) = candidate.manifest.clone() else {
            let enabled = self.enabled_for(id);
            return record(None, None, enabled, disabled_or(enabled, invalid()), None);
        };
        if candidate.scope == Scope::Builtin {
            let enabled = self.enabled_for(id);
            return record(Some(manifest), None, enabled, disabled_or(enabled, Health::Ok), None);
        }

        let update = stale_fork_update(candidate, shipped);
        let build_error = |error: &str, enabled: bool| disabled_or(enabled, Health::BuildError { error: truncate_error(error) });
        let signature = match directory_signature(&candidate.dir) {
            Ok(signature) => signature,
            Err(error) => {
                let enabled = self.enabled_for(id);
                return record(Some(manifest), update, enabled, build_error(&format!("{error:#}"), enabled), None);
            }
        };
        let cached = self.state.lock().builds.get(&candidate.dir).filter(|cached| cached.signature == signature).map(|cached| cached.result.clone());
        let result = cached.unwrap_or_else(|| {
            let result = self.compiler.compile(&candidate.dir, manifest.entry(), &self.build_dir);
            self.state.lock().builds.insert(candidate.dir.clone(), CachedBuild { signature, result: result.clone() });
            result
        });
        // Compilation can yield while a health report disables this id.
        let enabled = self.enabled_for(id);
        match result {
            Err(error) => record(Some(manifest), update, enabled, build_error(&error, enabled), None),
            Ok(compiled) => record(Some(manifest), update, enabled, disabled_or(enabled, Health::Ok), Some(&compiled)),
        }
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<Vec<Record>> {
        validate_id(id)?;
        self.require(id)?;
        {
            let mut state = self.state.lock();
            state.enabled.insert(id.to_owned(), enabled);
            self.persist_enabled(&state);
            if let Some(record) = state.records.get_mut(id).filter(|_| !enabled) {
                record.enabled = false;
                record.health = Health::Disabled;
                record.updated_at = now();
            }
        }
        if enabled {
            // Re-evaluate so enabling a broken extension restores its specific error.
            self.refresh(Some(&[id.to_owned()]))?;
        }
        Ok(self.list())
    }

    pub fn remove(&self, id: &str) -> Result<Vec<Record>> {
        validate_id(id)?;
        let record = self.require(id)?;
        if record.scope != Scope::User {
            bail!("Only user extensions can be removed.");
        }
        // Validate both deletion targets before deleting either.
        let source = contained_existing_directory(&self.user_dir, Path::new(&record.dir))?;
        let build = contained_child_if_present(&self.build_dir, id)?;
        std::fs::remove_dir_all(&source)?;
        if let Some(build) = build {
            std::fs::remove_dir_all(build)?;
        }
        {
            let mut state = self.state.lock();
            state.builds.remove(Path::new(&record.dir));
            state.records.remove(id);
            state.enabled.remove(id);
            self.persist_enabled(&state);
        }
        // Removing an override may expose a builtin.
        self.refresh(Some(&[id.to_owned()]))?;
        Ok(self.list())
    }

    pub fn reload(&self, id: &str) -> Result<Vec<Record>> {
        validate_id(id)?;
        let record = self.require(id)?;
        self.state.lock().builds.remove(Path::new(&record.dir));
        self.refresh(Some(&[id.to_owned()]))?;
        Ok(self.list())
    }

    /// Write a new user extension from `{ manifest, files }`; returns its id.
    pub fn create(&self, request: &CreateRequest) -> Result<String> {
        let manifest = parse_manifest(&request.manifest).map_err(|error| anyhow!(error))?;
        let id = validate_id(&manifest.id)?.to_owned();
        let files = validate_create_files(&request.files, manifest.entry())?;

        std::fs::create_dir_all(&self.user_dir)?;
        let user_root = dunce::canonicalize(&self.user_dir)?;
        let destination = user_root.join(&id);
        if std::fs::symlink_metadata(&destination).is_ok() {
            bail!("Extension already exists.");
        }
        let temporary = user_root.join(format!(".{id}.tmp-{}", hex::encode(rand::random::<[u8; 8]>())));
        std::fs::create_dir(&temporary)?;
        let written = (|| -> Result<()> {
            for (relative, text) in &files {
                let file = relative.split('/').fold(temporary.clone(), |path, part| path.join(part));
                std::fs::create_dir_all(file.parent().expect("file has a folder"))?;
                std::fs::write(file, text)?;
            }
            std::fs::write(temporary.join("manifest.json"), format!("{}\n", serde_json::to_string_pretty(&manifest)?))?;
            std::fs::rename(&temporary, &destination)?;
            Ok(())
        })();
        if let Err(error) = written {
            let _ = std::fs::remove_dir_all(&temporary);
            return Err(error);
        }
        self.refresh(Some(std::slice::from_ref(&id)))?;
        Ok(id)
    }

    /// The validated folder of a record, for Explorer.
    pub fn reveal_path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        let record = self.require(id)?;
        self.record_directory(&record)?;
        Ok(PathBuf::from(record.dir))
    }

    pub fn read_source(&self, id: &str) -> Result<Vec<SourceFile>> {
        validate_id(id)?;
        let record = self.require(id)?;
        let directory = self.record_directory(&record)?;
        collect_text_source(&directory)
    }

    fn record_directory(&self, record: &Record) -> Result<PathBuf> {
        let root = match record.scope {
            Scope::User => &self.user_dir,
            Scope::Builtin => &self.builtin_dir,
            Scope::Project => bail!("Project extension path escapes its root."),
        };
        contained_existing_directory(root, Path::new(&record.dir))
    }

    /// Apply a window's health report. Returns whether the record changed
    /// (and `ext:changed` with reason `health` should go out).
    pub fn report_health(&self, id: &str, health: Reported) -> Result<bool> {
        validate_id(id)?;
        let mut state = self.state.lock();
        let state = &mut *state;
        let record = state.records.get_mut(id).ok_or_else(|| anyhow!("Unknown extension: {id}"))?;
        // Nothing of an extension waiting for trust ever ran; a report is stale.
        if record.health == Health::NeedsTrust {
            return Ok(false);
        }
        if record.enabled {
            record.health = match health {
                Reported::Ok => Health::Ok,
                Reported::ActivationError(error) => Health::ActivationError { error },
                Reported::RuntimeError(error) => {
                    state.enabled.insert(id.to_owned(), false);
                    record.enabled = false;
                    Health::RuntimeError { error }
                }
            };
        } // Ignore late health reports from a runtime already disabled.
        record.updated_at = now();
        if !record.enabled && matches!(record.health, Health::RuntimeError { .. }) && state.enabled.get(id) == Some(&false) {
            (self.persist)(serde_json::to_value(&state.enabled).unwrap_or_default());
        }
        Ok(true)
    }
}

/// `readBuiltins`: every folder of the built-in root, sorted.
fn read_builtins(root: &Path) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.') && is_extension_id(name))
        .collect();
    ids.sort();
    ids.into_iter()
        .map(|id| {
            let dir = root.join(&id);
            let parsed = std::fs::read_to_string(dir.join("manifest.json"))
                .map_err(|error| error.to_string())
                .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|error| error.to_string()))
                .and_then(|raw| parse_manifest(&raw))
                .and_then(|manifest| if manifest.id == id { Ok(manifest) } else { Err("Directory name does not match manifest id.".into()) });
            let (manifest, error) = match parsed {
                Ok(manifest) => (Some(manifest), None),
                Err(error) => (None, Some(error)),
            };
            Candidate { id, scope: Scope::Builtin, dir, manifest, error }
        })
        .collect()
}

/// `scanExtensionDirs` for one root: immediate, non-link children with a
/// regular `manifest.json`. A missing root yields nothing.
fn scan_extension_dir(root: &Path, scope: Scope) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<Candidate> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| is_extension_id(name))
        .filter_map(|name| scan_candidate(std::path::absolute(root.join(&name)).ok()?, name, scope))
        .collect();
    found.sort_by(|left, right| left.id.cmp(&right.id).then_with(|| left.dir.cmp(&right.dir)));
    found
}

fn scan_candidate(dir: PathBuf, name: String, scope: Scope) -> Option<Candidate> {
    let invalid = |error: String| Some(Candidate { id: name.clone(), scope, dir: dir.clone(), manifest: None, error: Some(truncate_error(&error)) });
    let metadata = std::fs::symlink_metadata(&dir).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    let manifest_path = dir.join("manifest.json");
    let manifest_metadata = std::fs::symlink_metadata(&manifest_path).ok()?;
    if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
        return None;
    }
    let raw = match std::fs::read_to_string(&manifest_path).map_err(|error| error.to_string()).and_then(|text| serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())) {
        Ok(raw) => raw,
        Err(error) => return invalid(error),
    };
    let manifest = match parse_manifest(&raw) {
        Ok(manifest) => manifest,
        Err(error) => return invalid(error),
    };
    if manifest.id != name {
        return invalid(format!("manifest id \"{}\" must match directory name \"{name}\"", manifest.id));
    }
    if let Err(error) = enforce_source_file_limit(&dir) {
        return invalid(format!("{error:#}"));
    }
    Some(Candidate { id: manifest.id.clone(), scope, dir, manifest: Some(manifest), error: None })
}

fn enforce_source_file_limit(root: &Path) -> Result<()> {
    let mut files = 0;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
                continue;
            }
            files += 1;
            if files > limits::SOURCE_FILES {
                bail!("extension has more than {} source files", limits::SOURCE_FILES);
            }
        }
    }
    Ok(())
}

fn stale_fork_update(candidate: &Candidate, shipped: &HashMap<String, Manifest>) -> Option<ForkUpdate> {
    let manifest = candidate.manifest.as_ref().filter(|_| candidate.scope == Scope::User)?;
    let (builtin, base) = builtin_fork_origin(manifest.forked_from.as_deref()?)?;
    let current = shipped.get(builtin)?;
    if !manifest.replaces.as_ref().is_some_and(|replaces| replaces.iter().any(|id| id == builtin)) || current.version == base {
        return None;
    }
    Some(ForkUpdate { forked_from: builtin.to_owned(), base: base.to_owned(), current: current.version.clone() })
}

fn sorted_entries(directory: &Path) -> Result<Vec<std::fs::DirEntry>> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn modified_ms(metadata: &std::fs::Metadata) -> f64 {
    metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |time| time.as_secs_f64() * 1000.0)
}

/// Sizes and modification times of every non-hidden entry: a change means rebuild.
pub(crate) fn directory_signature(root: &Path) -> Result<String> {
    fn walk(directory: &Path, prefix: &str, parts: &mut Vec<String>, files: &mut usize) -> Result<()> {
        for entry in sorted_entries(directory)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                parts.push(format!("l:{relative}:{}", modified_ms(&metadata)));
            } else if metadata.is_dir() {
                parts.push(format!("d:{relative}:{}", modified_ms(&metadata)));
                walk(&entry.path(), &relative, parts, files)?;
            } else if metadata.is_file() {
                *files += 1;
                if *files > limits::SOURCE_FILES {
                    bail!("Extension has too many source files.");
                }
                parts.push(format!("f:{relative}:{}:{}", metadata.len(), modified_ms(&metadata)));
            }
        }
        Ok(())
    }
    let mut parts = Vec::new();
    walk(root, "", &mut parts, &mut 0)?;
    Ok(parts.join("\n"))
}

fn validate_create_files(files: &Map<String, Value>, entry: &str) -> Result<Vec<(String, String)>> {
    // manifest.json is generated and counts toward discovery's total.
    if files.len() + 1 > limits::SOURCE_FILES {
        bail!("Extension has too many source files.");
    }
    let mut bytes = 0u64;
    let mut has_entry = false;
    let mut list = Vec::with_capacity(files.len());
    for (relative, value) in files {
        validate_source_path(relative)?;
        if relative == "manifest.json" {
            bail!("manifest.json is generated from the manifest.");
        }
        let Some(text) = value.as_str().filter(|text| !text.contains('\0')) else { bail!("Extension file is not text: {relative}") };
        bytes += text.len() as u64;
        if bytes > limits::SOURCE_BYTES {
            bail!("Extension source is too large.");
        }
        has_entry |= relative == entry;
        list.push((relative.clone(), text.to_owned()));
    }
    if !has_entry {
        bail!("Extension entry file is missing: {entry}");
    }
    Ok(list)
}

fn validate_source_path(relative: &str) -> Result<()> {
    let invalid = relative.is_empty()
        || relative.encode_utf16().count() > 200
        || relative.contains('\\')
        || relative.contains('\0')
        || relative.starts_with('/')
        || relative.as_bytes().get(1) == Some(&b':')
        || relative.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..")
        // Characters Windows cannot store in a file name.
        || relative.chars().any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*') || character.is_control());
    if invalid {
        bail!("Invalid extension file path: {relative}");
    }
    Ok(())
}

/// `validateContainedExistingDirectory(root, candidate, true)`.
pub(crate) fn contained_existing_directory(root: &Path, candidate: &Path) -> Result<PathBuf> {
    let root = dunce::canonicalize(root)?;
    let metadata = std::fs::symlink_metadata(candidate)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("Extension path is not a regular directory.");
    }
    let real = dunce::canonicalize(candidate)?;
    if real == root || !real.starts_with(&root) {
        bail!("Extension path escapes its root.");
    }
    if real.parent() != Some(root.as_path()) {
        bail!("Extension path is not an immediate child of its root.");
    }
    Ok(real)
}

fn contained_child_if_present(root: &Path, id: &str) -> Result<Option<PathBuf>> {
    std::fs::create_dir_all(root)?;
    let root = dunce::canonicalize(root)?;
    let candidate = root.join(validate_id(id)?);
    let metadata = match std::fs::symlink_metadata(&candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        bail!("Extension build path is a symbolic link.");
    }
    let real = dunce::canonicalize(&candidate)?;
    if real.parent() != Some(root.as_path()) {
        bail!("Extension build path escapes its root.");
    }
    Ok(Some(real))
}

/// Text files of an extension (≤ 64 KB, no NUL), for the source viewer.
fn collect_text_source(root: &Path) -> Result<Vec<SourceFile>> {
    fn walk(directory: &Path, prefix: &str, files: &mut Vec<SourceFile>, visited: &mut usize) -> Result<()> {
        if *visited >= limits::SOURCE_FILES {
            return Ok(());
        }
        for entry in sorted_entries(directory)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if *visited >= limits::SOURCE_FILES {
                return Ok(());
            }
            let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                walk(&entry.path(), &relative, files, visited)?;
            } else if metadata.is_file() {
                *visited += 1;
                if metadata.len() > SOURCE_FILE_BYTES {
                    continue;
                }
                let contents = std::fs::read(entry.path()).with_context(|| format!("read {relative}"))?;
                if contents.contains(&0) {
                    continue;
                }
                files.push(SourceFile { path: relative, text: String::from_utf8_lossy(&contents).into_owned() });
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, "", &mut files, &mut 0)?;
    Ok(files)
}
