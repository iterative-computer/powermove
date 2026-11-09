//! Downloaded speech models on disk, in main's layout (`models.ts`), so a
//! migrated profile's models are found as they are:
//!
//!   <root>/settings.json             active model + language
//!   <root>/<id>/…                    a complete model; model.json is written last
//!   <root>/.partial/<id>/<file>.part a file still arriving (resumed with Range)
//!   <root>/.partial/<id>/<file>      a file that arrived and matched its SHA-256
//!
//! A model becomes visible only by renaming its finished partial folder into
//! place. Downloads run the system curl (no configuration, no credentials) on
//! a worker; `status` never waits on them.
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct File {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub name: String,
    pub description: String,
    pub family: String,
    pub languages: String,
    pub language_codes: Vec<String>,
    pub detects_language: bool,
    pub timing: String,
    pub speed: f64,
    pub accuracy: f64,
    pub recommended: bool,
    pub featured: bool,
    pub word_timestamps: bool,
    #[serde(default)]
    pub lag: Option<[f64; 3]>,
    pub license: String,
    pub files: Vec<File>,
}
impl Model {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}
/// Main's catalog (`catalog.ts`), extracted by running it: pinned Hugging Face
/// revisions and their SHA-256.
pub fn catalog() -> Vec<Model> {
    #[derive(Deserialize)]
    struct Catalog {
        models: Vec<Model>,
    }
    serde_json::from_str::<Catalog>(include_str!("../catalog-main.json"))
        .expect("pinned catalog")
        .models
}
/// Headroom kept free on the disk after a model lands.
const DISK_HEADROOM: u64 = 256 * 1024 * 1024;
const CANCELLED: &str = "The download was cancelled.";
pub fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{} MB", (b / 1e6).round())
    } else if b >= 1e3 {
        format!("{} KB", (b / 1e3).round())
    } else {
        format!("{bytes} bytes")
    }
}
fn valid_language(tag: &str) -> bool {
    if tag == "auto" {
        return true;
    }
    let mut parts = tag.split('-');
    let first = parts.next().unwrap_or_default();
    (2..=3).contains(&first.len())
        && first.bytes().all(|c| c.is_ascii_lowercase())
        && parts.enumerate().all(|(i, p)| {
            i < 2 && (2..=8).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_alphanumeric())
        })
}
struct Download {
    received: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct State {
    active: Option<String>,
    language: String,
    ready: BTreeSet<String>,
    downloads: BTreeMap<String, Download>,
    errors: BTreeMap<String, String>,
    partial: BTreeMap<String, u64>,
    /// Why the last automatic choice of active model wasn't saved.
    settings_error: Option<String>,
}
/// The models folder. Clone freely; all clones share one state.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    models: Arc<Vec<Model>>,
    state: Arc<Mutex<State>>,
    /// Signalled under `state` whenever a download worker finishes.
    finished: Arc<Condvar>,
    changed: Arc<dyn Fn() + Send + Sync>,
}
fn size_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}
impl Store {
    /// Scan `root` (main's `ModelStore.load`). `changed` runs on a worker
    /// whenever status changes (progress at most every 200 ms).
    /// A settings file that is missing or not JSON gives main's defaults.
    pub fn open(
        root: &Path,
        models: Vec<Model>,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        let store = Self {
            root: root.to_owned(),
            models: Arc::new(models),
            state: Arc::new(Mutex::new(State {
                language: "auto".into(),
                ..Default::default()
            })),
            finished: Arc::new(Condvar::new()),
            changed,
        };
        let unopened =
            |e: std::io::Error| format!("The transcription models folder couldn’t be opened: {e}");
        std::fs::create_dir_all(root).map_err(unopened)?;
        let settings: Value = match std::fs::read(root.join("settings.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
            Err(e) => return Err(unopened(e)),
        };
        let mut state = store.state.lock().unwrap();
        state.active = settings["activeModelId"].as_str().map(str::to_owned);
        if let Some(language) = settings["language"].as_str().filter(|l| valid_language(l)) {
            state.language = language.into();
        }
        for model in store.models.iter() {
            if store.complete(model) {
                state.ready.insert(model.id.clone());
            } else {
                store.remove_stale(model);
                let partial = store.partial_size(model);
                if partial > 0 {
                    state.partial.insert(model.id.clone(), partial);
                }
            }
        }
        if state
            .active
            .as_ref()
            .is_some_and(|id| !state.ready.contains(id))
        {
            state.active = None;
        }
        if state.active.is_none() {
            state.active = store
                .models
                .iter()
                .find(|m| state.ready.contains(&m.id))
                .map(|m| m.id.clone());
        }
        drop(state);
        Ok(store)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn models(&self) -> &[Model] {
        &self.models
    }
    fn model(&self, id: &str) -> Result<&Model, String> {
        self.models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| "That transcription model is not available.".into())
    }
    pub fn model_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }
    fn partial_dir(&self, id: &str) -> PathBuf {
        self.root.join(".partial").join(id)
    }
    /// Its manifest names exactly the catalog's files, each at its size.
    fn complete(&self, model: &Model) -> bool {
        let dir = self.model_dir(&model.id);
        let Some(manifest) = std::fs::read(dir.join("model.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            return false;
        };
        let listed: BTreeMap<&str, &str> = manifest["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| Some((f["name"].as_str()?, f["sha256"].as_str()?)))
            .collect();
        model.files.iter().all(|f| {
            listed.get(f.name.as_str()) == Some(&f.sha256.as_str())
                && size_of(&dir.join(&f.name)) == Some(f.size)
        })
    }
    /// A finished folder naming none of today's files is an older build: give
    /// its space back (main's `removeStale`).
    fn remove_stale(&self, model: &Model) {
        let dir = self.model_dir(&model.id);
        let Some(manifest) = std::fs::read(dir.join("model.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            return;
        };
        let current: BTreeSet<String> = model
            .files
            .iter()
            .map(|f| format!("{}:{}", f.name, f.sha256))
            .collect();
        let shared = manifest["files"].as_array().into_iter().flatten().any(|f| {
            current.contains(&format!(
                "{}:{}",
                f["name"].as_str().unwrap_or_default(),
                f["sha256"].as_str().unwrap_or_default()
            ))
        });
        if !shared {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    fn partial_size(&self, model: &Model) -> u64 {
        let dir = self.partial_dir(&model.id);
        model
            .files
            .iter()
            .map(|f| {
                size_of(&dir.join(&f.name))
                    .or_else(|| size_of(&dir.join(format!("{}.part", f.name))))
                    .unwrap_or(0)
            })
            .sum()
    }
    /// Main's `TranscriptionStatus`: every catalog model and its state.
    pub fn status(&self) -> Value {
        let state = self.state.lock().unwrap();
        let mut storage = 0;
        let models: Vec<Value> = self
            .models
            .iter()
            .map(|m| {
                let size = m.size();
                let mut info = json!({"id":m.id,"name":m.name,"description":m.description,"size":size,
                    "languages":m.languages,"languageCodes":m.language_codes,"speed":m.speed,
                    "accuracy":m.accuracy,"recommended":m.recommended,"wordTimestamps":m.word_timestamps,
                    "timing":m.timing,"detectsLanguage":m.detects_language,"featured":m.featured,"state":"absent"});
                if state.ready.contains(&m.id) {
                    storage += size;
                    info["state"] = json!("ready");
                } else if let Some(d) = state.downloads.get(&m.id) {
                    let received = d.received.load(Ordering::Relaxed);
                    storage += received;
                    info["state"] = json!("downloading");
                    info["progress"] = json!((received as f64 / size.max(1) as f64).min(1.));
                    info["downloadedBytes"] = json!(received);
                } else {
                    if let Some(error) = state.errors.get(&m.id) {
                        info["state"] = json!("error");
                        info["error"] = json!(error);
                    }
                    if let Some(&partial) = state.partial.get(&m.id).filter(|p| **p > 0) {
                        storage += partial;
                        info["downloadedBytes"] = json!(partial);
                        info["progress"] = json!((partial as f64 / size.max(1) as f64).min(1.));
                    }
                }
                info
            })
            .collect();
        let active = state.active.clone().filter(|id| state.ready.contains(id));
        let mut status = json!({"models":models,"activeModelId":active,"available":true,"language":state.language,"storageBytes":storage});
        if let Some(error) = &state.settings_error {
            status["settingsError"] = json!(error);
        }
        status
    }
    /// The active model and its folder, or `None` when none is ready.
    pub fn active(&self) -> Option<(Model, PathBuf)> {
        let state = self.state.lock().unwrap();
        let id = state.active.clone().filter(|id| state.ready.contains(id))?;
        drop(state);
        let model = self.model(&id).ok()?.clone();
        Some((model, self.model_dir(&id)))
    }
    /// The model a request runs on (main's `pickModel`): the active one, unless
    /// word timing is needed and it lacks it; then a ready word-timed model.
    pub fn model_for(&self, words: bool) -> Option<(Model, PathBuf)> {
        let active = self.active();
        if active
            .as_ref()
            .is_some_and(|(m, _)| !words || m.word_timestamps)
        {
            return active;
        }
        let state = self.state.lock().unwrap();
        let model = self
            .models
            .iter()
            .filter(|m| state.ready.contains(&m.id))
            .find(|m| m.word_timestamps && (!words || m.timing == "word"))?
            .clone();
        let dir = self.model_dir(&model.id);
        Some((model, dir))
    }
    pub fn language(&self) -> String {
        self.state.lock().unwrap().language.clone()
    }
    /// Writes the settings file; the caller publishes them only once it is saved.
    fn persist(&self, active: Option<&str>, language: &str) -> Result<(), String> {
        let file = self.root.join("settings.json");
        let staged = self
            .root
            .join(format!("settings.json.{}.tmp", std::process::id()));
        let text = format!("{}\n", json!({"activeModelId":active,"language":language}));
        std::fs::write(&staged, text)
            .and_then(|()| std::fs::rename(&staged, &file))
            .map_err(|e| {
                let _ = std::fs::remove_file(&staged);
                format!("Transcription settings couldn’t be saved: {e}")
            })
    }
    /// Keeps the active model a ready one: main's choice is the first ready
    /// model in catalog order. Readiness stands even if saving the choice fails.
    fn reselect(&self, state: &mut State) {
        if state
            .active
            .as_ref()
            .is_some_and(|id| state.ready.contains(id))
        {
            return;
        }
        state.active = self
            .models
            .iter()
            .find(|m| state.ready.contains(&m.id))
            .map(|m| m.id.clone());
        state.settings_error = self.persist(state.active.as_deref(), &state.language).err();
    }
    pub fn set_active(&self, id: &str) -> Result<(), String> {
        self.model(id)?;
        let mut state = self.state.lock().unwrap();
        if !state.ready.contains(id) {
            return Err("Download this model before using it.".into());
        }
        self.persist(Some(id), &state.language)?;
        state.active = Some(id.into());
        state.settings_error = None;
        drop(state);
        (self.changed)();
        Ok(())
    }
    pub fn set_language(&self, language: &str) -> Result<(), String> {
        if !valid_language(language) {
            return Err("Choose a language.".into());
        }
        let mut state = self.state.lock().unwrap();
        self.persist(state.active.as_deref(), language)?;
        state.language = language.into();
        state.settings_error = None;
        drop(state);
        (self.changed)();
        Ok(())
    }
    /// Start (or join) a download on a worker. What arrived stays on disk, so
    /// a cancelled or failed download resumes where it stopped.
    pub fn download(&self, id: &str) -> Result<(), String> {
        let model = self.model(id)?.clone();
        let mut state = self.state.lock().unwrap();
        if state.ready.contains(id) || state.downloads.contains_key(id) {
            return Ok(());
        }
        let received = Arc::new(AtomicU64::new(state.partial.get(id).copied().unwrap_or(0)));
        let cancel = Arc::new(AtomicBool::new(false));
        state.errors.remove(id);
        state.downloads.insert(
            id.into(),
            Download {
                received: received.clone(),
                cancel: cancel.clone(),
            },
        );
        drop(state);
        (self.changed)();
        let store = self.clone();
        let spawned = std::thread::Builder::new()
            .name("pm-transcription-download".into())
            .spawn(move || {
                let outcome = store.fetch(&model, &received, &cancel);
                let mut state = store.state.lock().unwrap();
                state.downloads.remove(&model.id);
                match outcome {
                    Ok(()) => {
                        state.ready.insert(model.id.clone());
                        state.partial.remove(&model.id);
                        store.reselect(&mut state);
                    }
                    Err(error) => {
                        state
                            .partial
                            .insert(model.id.clone(), store.partial_size(&model));
                        if !cancel.load(Ordering::Relaxed) {
                            state.errors.insert(model.id.clone(), error);
                        }
                    }
                }
                store.finished.notify_all();
                drop(state);
                (store.changed)();
            });
        if let Err(e) = spawned {
            self.state.lock().unwrap().downloads.remove(id);
            self.finished.notify_all();
            (self.changed)();
            return Err(format!("The download couldn’t start: {e}"));
        }
        Ok(())
    }
    /// Stop a download; what arrived stays for the next attempt.
    pub fn cancel(&self, id: &str) {
        if let Some(d) = self.state.lock().unwrap().downloads.get(id) {
            d.cancel.store(true, Ordering::Relaxed);
        }
    }
    /// Cancels its download and waits for the worker to stop, so nothing it
    /// publishes outlives the removal. Status then reports what is left on disk.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let model = self.model(id)?;
        let state = self.state.lock().unwrap();
        if let Some(d) = state.downloads.get(id) {
            d.cancel.store(true, Ordering::Relaxed);
        }
        let mut state = self
            .finished
            .wait_while(state, |s| s.downloads.contains_key(id))
            .unwrap();
        let removed = [self.model_dir(id), self.partial_dir(id)]
            .into_iter()
            .try_for_each(|dir| match std::fs::remove_dir_all(dir) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            });
        if !self.complete(model) {
            state.ready.remove(id);
        }
        state.errors.remove(id);
        state.partial.insert(id.into(), self.partial_size(model));
        self.reselect(&mut state);
        drop(state);
        (self.changed)();
        removed.map_err(|e| format!("The model couldn’t be removed: {e}"))
    }
    fn fetch(
        &self,
        model: &Model,
        received: &AtomicU64,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let partial = self.partial_dir(&model.id);
        std::fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
        let have = self.partial_size(model);
        received.store(have, Ordering::Relaxed);
        let needed = model.size().saturating_sub(have);
        if free_bytes(&self.root).is_some_and(|free| free < needed + DISK_HEADROOM) {
            return Err(format!(
                "Not enough disk space. {} needs {} free.",
                model.name,
                format_bytes(needed + DISK_HEADROOM)
            ));
        }
        let mut before = 0;
        for file in &model.files {
            let done = partial.join(&file.name);
            if size_of(&done) != Some(file.size) {
                let part = partial.join(format!("{}.part", file.name));
                self.fetch_file(file, &part, received, before, cancel)?;
                std::fs::rename(&part, &done).map_err(|e| e.to_string())?;
            }
            before += file.size;
            received.store(before, Ordering::Relaxed);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let manifest = json!({"id":model.id,"files":model.files.iter().map(|f| json!({"name":f.name,"size":f.size,"sha256":f.sha256})).collect::<Vec<_>>(),
            "completedAt":completed_at()});
        std::fs::write(
            partial.join("model.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&manifest).unwrap_or_default()
            ),
        )
        .map_err(|e| e.to_string())?;
        let target = self.model_dir(&model.id);
        let _ = std::fs::remove_dir_all(&target);
        std::fs::rename(&partial, &target).map_err(|e| e.to_string())
    }
    fn fetch_file(
        &self,
        file: &File,
        part: &Path,
        received: &AtomicU64,
        before: u64,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        for attempt in 0..2 {
            let offset = size_of(part).unwrap_or(0);
            if offset > file.size {
                std::fs::write(part, b"").map_err(|e| e.to_string())?;
            }
            if size_of(part).unwrap_or(0) < file.size {
                // No config, no credentials; a quiet minute is a stall.
                let mut child: Child = Command::new("/usr/bin/curl")
                    .args([
                        "-q",
                        "--silent",
                        "--show-error",
                        "--fail",
                        "--location",
                        "--proto",
                        "=https,file",
                    ])
                    .args([
                        "--speed-limit",
                        "1",
                        "--speed-time",
                        "60",
                        "--continue-at",
                        "-",
                        "--output",
                    ])
                    .arg(part)
                    .arg(&file.url)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("The download couldn’t start: {e}"))?;
                let status = loop {
                    if cancel.load(Ordering::Relaxed) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(CANCELLED.into());
                    }
                    received.store(before + size_of(part).unwrap_or(0), Ordering::Relaxed);
                    (self.changed)();
                    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                        break status;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                };
                if !status.success() {
                    let mut detail = String::new();
                    if let Some(mut err) = child.stderr.take() {
                        let _ = err.read_to_string(&mut detail);
                    }
                    // What is on disk is not a prefix the server recognises: start over once.
                    if status.code() == Some(33) && attempt == 0 {
                        std::fs::write(part, b"").map_err(|e| e.to_string())?;
                        continue;
                    }
                    if status.code() == Some(28) {
                        return Err(
                            "The download stalled. Check your connection and try again.".into()
                        );
                    }
                    let http = detail.rsplit("error: ").next().unwrap_or("").trim();
                    return Err(if http.is_empty() {
                        "The download failed. Try again.".into()
                    } else {
                        format!("The download failed ({http}). Try again.")
                    });
                }
            }
            return verify(file, part, cancel);
        }
        verify(file, part, cancel)
    }
}
fn verify(file: &File, part: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let mut hash = Sha256::new();
    let mut input = std::fs::File::open(part).map_err(|e| e.to_string())?;
    let mut buf = vec![0; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let n = input.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    if size_of(part) != Some(file.size) || format!("{:x}", hash.finalize()) != file.sha256 {
        let _ = std::fs::remove_file(part);
        return Err("The downloaded model did not match its checksum. Try again.".into());
    }
    Ok(())
}
fn free_bytes(dir: &Path) -> Option<u64> {
    let output = Command::new("/bin/df").arg("-k").arg(dir).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().nth(1)?;
    line.split_whitespace()
        .nth(3)?
        .parse::<u64>()
        .ok()
        .map(|k| k * 1024)
}
fn completed_at() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pm-transcribe-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn fixture(dir: &Path, bytes: &[u8]) -> Model {
        let source = dir.join("source.gguf");
        std::fs::write(&source, bytes).unwrap();
        let mut model = catalog()[0].clone();
        model.files = vec![File {
            name: "model.gguf".into(),
            url: format!("file://{}", source.display()),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }];
        model
    }
    fn wait(store: &Store, id: &str) {
        for _ in 0..500 {
            if store.status()["models"][0]["state"] != "downloading" {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("download {id} never finished");
    }
    #[test]
    fn the_pinned_catalog_is_mains() {
        let models = catalog();
        assert_eq!(models.len(), 7);
        let first = &models[0];
        assert_eq!(first.id, "parakeet-unified-en-0.6b");
        assert!(first.recommended && first.featured && first.word_timestamps);
        assert_eq!(first.lag, Some([0.32, 0.2, 0.]));
        assert!(
            first.files[0]
                .url
                .starts_with("https://huggingface.co/handy-computer/")
        );
        assert_eq!(first.files[0].sha256.len(), 64);
    }
    #[test]
    fn a_download_lands_whole_in_mains_layout_and_resumes() {
        let root = scratch("download");
        let model = fixture(&root, &[7u8; 300_000]);
        let id = model.id.clone();
        // Part of the file already arrived in an earlier session.
        let partial = root.join("models/.partial").join(&id);
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join("model.gguf.part"), [7u8; 1000]).unwrap();
        let store =
            Store::open(&root.join("models"), vec![model.clone()], Arc::new(|| {})).unwrap();
        assert_eq!(store.status()["models"][0]["downloadedBytes"], 1000);
        store.download(&id).unwrap();
        wait(&store, &id);
        let status = store.status();
        assert_eq!(status["models"][0]["state"], "ready", "{status}");
        assert_eq!(status["activeModelId"], json!(id));
        let dir = root.join("models").join(&id);
        assert_eq!(
            std::fs::read(dir.join("model.gguf")).unwrap(),
            vec![7u8; 300_000]
        );
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("model.json")).unwrap()).unwrap();
        assert_eq!(manifest["files"][0]["sha256"], json!(model.files[0].sha256));
        assert!(!root.join("models/.partial").join(&id).exists());
        // Reopening finds it; settings name it active.
        let again =
            Store::open(&root.join("models"), vec![model.clone()], Arc::new(|| {})).unwrap();
        assert_eq!(again.active().unwrap().0.id, id);
        again.remove(&id).unwrap();
        assert!(!dir.exists());
        assert_eq!(again.status()["activeModelId"], Value::Null);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn a_wrong_checksum_never_becomes_a_model() {
        let root = scratch("checksum");
        let mut model = fixture(&root, b"not the model");
        model.files[0].sha256 = "0".repeat(64);
        let store =
            Store::open(&root.join("models"), vec![model.clone()], Arc::new(|| {})).unwrap();
        store.download(&model.id).unwrap();
        wait(&store, &model.id);
        let status = store.status();
        assert_eq!(status["models"][0]["state"], "error");
        assert_eq!(
            status["models"][0]["error"],
            "The downloaded model did not match its checksum. Try again."
        );
        assert!(!root.join("models").join(&model.id).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn languages_and_activity_follow_mains_rules() {
        let root = scratch("settings");
        let store = Store::open(&root, catalog(), Arc::new(|| {})).unwrap();
        assert_eq!(store.language(), "auto");
        store.set_language("pt-BR").unwrap();
        assert_eq!(
            store.set_language("Portuguese").unwrap_err(),
            "Choose a language."
        );
        assert_eq!(
            store.set_active("parakeet-unified-en-0.6b").unwrap_err(),
            "Download this model before using it."
        );
        assert_eq!(
            Store::open(&root, catalog(), Arc::new(|| {}))
                .unwrap()
                .language(),
            "pt-BR"
        );
        assert_eq!(format_bytes(731_357_568), "731 MB");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn a_setting_that_isnt_saved_is_neither_reported_nor_kept() {
        let root = scratch("unsaved");
        let store = Store::open(&root, catalog(), Arc::new(|| {})).unwrap();
        store.set_language("de").unwrap();
        // A directory where the file goes makes the rename fail.
        std::fs::remove_file(root.join("settings.json")).unwrap();
        std::fs::create_dir_all(root.join("settings.json/blocked")).unwrap();
        assert!(
            store
                .set_language("fr")
                .unwrap_err()
                .starts_with("Transcription settings couldn’t be saved")
        );
        assert_eq!(store.language(), "de");
        assert!(
            Store::open(&root, catalog(), Arc::new(|| {}))
                .err()
                .unwrap()
                .starts_with("The transcription models folder couldn’t be opened")
        );
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("settings.json"), "{not json").unwrap();
        assert_eq!(
            Store::open(&root, catalog(), Arc::new(|| {}))
                .unwrap()
                .language(),
            "auto"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn a_downloaded_model_stays_ready_when_its_selection_cant_be_saved() {
        let root = scratch("unsaved-active");
        let model = fixture(&root, &[3u8; 4096]);
        let store =
            Store::open(&root.join("models"), vec![model.clone()], Arc::new(|| {})).unwrap();
        std::fs::create_dir_all(root.join("models/settings.json/blocked")).unwrap();
        store.download(&model.id).unwrap();
        wait(&store, &model.id);
        let status = store.status();
        assert_eq!(status["models"][0]["state"], "ready", "{status}");
        assert_eq!(status["activeModelId"], json!(model.id));
        assert!(
            status["settingsError"]
                .as_str()
                .unwrap()
                .starts_with("Transcription settings couldn’t be saved")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn removing_a_model_mid_download_leaves_nothing_and_it_downloads_again() {
        let root = scratch("remove-mid-download");
        let model = fixture(&root, &[5u8; 64 << 20]);
        let store =
            Store::open(&root.join("models"), vec![model.clone()], Arc::new(|| {})).unwrap();
        for _ in 0..3 {
            store.download(&model.id).unwrap();
            store.remove(&model.id).unwrap();
            let status = store.status();
            assert_eq!(status["models"][0]["state"], "absent", "{status}");
            assert_eq!(status["models"][0]["downloadedBytes"], Value::Null);
            assert_eq!(status["activeModelId"], Value::Null);
            assert!(!root.join("models").join(&model.id).exists());
            assert!(!root.join("models/.partial").join(&model.id).exists());
        }
        store.download(&model.id).unwrap();
        wait(&store, &model.id);
        assert_eq!(store.status()["models"][0]["state"], "ready");
        std::fs::remove_dir_all(root).unwrap();
    }
}
