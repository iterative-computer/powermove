//! Project files: save, open, export destinations and uploads (port of
//! `main/save.ts`, `main/export-sidecar.ts`, and the `web:upload-*` /
//! `web:project-open-path` channels of `server/index.ts`).
//!
//! Every grant (pending save, upload, export destination, reader) belongs to
//! the window that asked for it and is dropped when that window closes.

mod durable;
mod files;
mod format;
mod save;
mod upload;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::host::Config;
use crate::ids::{FILE_CHUNK_BYTES, FILE_SAVE_BYTES, is_project_id};
use crate::router::{Call, Client, Router};
use crate::wire::Payload;
use files::ProjectFiles;
use format::{js_len, safe_uint, validate_save_media};
use save::ProjectSave;
use upload::{FileUpload, MAX_WEB_UPLOAD_BYTES, WebUpload, safe_file_name};

const IDLE: Duration = Duration::from_secs(120);
const MAX_SAVE_NAME_CHARS: usize = 200;
const MAX_SIDECAR_BYTES: usize = 32 * 1024 * 1024;

/// Registers the project channels and returns the handle that opens files
/// passed to the app from outside (Explorer, a second launch).
pub async fn register(router: &mut Router, config: &Config) -> Result<ProjectOpener> {
    let user_data = config.user_data.clone();
    let files = blocking(move || Ok(ProjectFiles::load(user_data.join("project-files.json"), user_data.join("backups")))).await?;
    let state = Projects(Arc::new(Inner {
        files,
        temp_dir: config.temp_dir.clone(),
        remote_media: config.user_data.join("Remote Uploads"),
        remote_projects: config.user_data.join("Remote Projects"),
        windows: Mutex::default(),
        readers: Mutex::default(),
        web_uploads: Mutex::default(),
        external: Mutex::default(),
    }));

    router.json_with(&state, "project:save-begin", |state, call| async move { state.save_begin(call).await });
    router.json_with(&state, "project:save-chunk", |state, call| async move { state.save_chunk(call).await });
    router.json_with(&state, "project:save-finish", |state, call| async move { state.save_finish(call).await });
    router.json_with(&state, "project:save-abort", |state, call| async move {
        if let Some(token) = call.arg(0).as_str() {
            state.discard_save(call.client.id, Some(token.to_owned())).await;
        }
        Ok(Value::Null)
    });
    router.json_with(&state, "export:choose", |state, call| async move { state.export_choose(call).await });
    router.json_with(&state, "export:release", |state, call| async move {
        let mut windows = state.0.windows.lock();
        if windows.destinations.get(&call.client.id).is_some_and(|destination| call.arg(0).as_str() == Some(destination.token.as_str())) {
            windows.destinations.remove(&call.client.id);
        }
        Ok(Value::Null)
    });
    router.json_with(&state, "export:sidecar", |state, call| async move { Ok(state.write_sidecar(&call).await) });
    router.json_with(&state, "file:save-upload", |state, call| async move { state.file_upload_begin(call).await });
    router.json_with(&state, "file:save-chunk", |state, call| async move { state.file_upload_chunk(call).await });
    router.json_with(&state, "file:save-abort", |state, call| async move {
        if let Some(id) = call.arg(0).as_str() {
            state.discard_upload(call.client.id, Some(id.to_owned())).await;
        }
        Ok(Value::Null)
    });
    router.json_with(&state, "file:save", |state, call| async move { state.file_save(call).await });
    router.on_with(&state, "project:read", |state, call| async move { state.project_read(call).await });
    router.json_with(&state, "project:read-close", |state, call| async move {
        let token = call.arg(0).as_str().filter(|token| state.owns_reader(&call.client, token)).context("Unknown project read token")?;
        state.close_reader(token.to_owned(), true).await?;
        Ok(Value::Null)
    });
    router.json_with(&state, "project:open", |state, call| async move {
        let picked = crate::dialog::file().set_title("Open Project").add_filter("Powermove Project", &["pmv", "json"]).pick_file().await;
        let Some(path) = picked.map(|handle| handle.path().to_owned()) else { return Ok(cancelled()) };
        Ok(state.open_for_window(&call.client, path).await)
    });
    router.json_with(&state, "project:open-path", |state, call| async move {
        let path = call.arg(0).as_str().map(PathBuf::from).filter(|path| path.is_absolute() && has_project_extension(path));
        let path = path.context("project:open-path: invalid project path")?;
        Ok(state.open_for_window(&call.client, path).await)
    });
    router.json_with(&state, "web:upload-begin", |state, call| async move { state.web_upload_begin(call).await });
    router.json_with(&state, "web:upload-chunk", |state, call| async move { state.web_upload_chunk(call).await });
    router.json_with(&state, "web:upload-finish", |state, call| async move { state.web_upload_finish(call).await });
    router.json_with(&state, "web:upload-abort", |state, call| async move {
        if let Some(upload) = call.arg(0).as_str().and_then(|id| state.take_web_upload(&call.client, id)) {
            blocking(move || {
                upload.remove();
                Ok(())
            })
            .await?;
        }
        Ok(Value::Null)
    });
    router.json_with(&state, "web:project-open-path", |state, call| async move { state.web_project_open_path(call).await });
    router.json_with(&state, "project:open-ready", |state, call| async move {
        let pending = {
            let mut external = state.0.external.lock();
            external.ready.push(call.client.clone());
            std::mem::take(&mut external.pending)
        };
        for path in pending {
            let result = state.open_for_window(&call.client, path).await;
            call.client.emit("project:open-external", result);
        }
        Ok(Value::Null)
    });
    Ok(ProjectOpener(state))
}

/// Opens project files handed to the app from outside the editor.
#[derive(Clone)]
pub struct ProjectOpener(Projects);

impl ProjectOpener {
    /// Open `path` in the most recent window that listens for external opens,
    /// or hold it until one does.
    pub async fn open(&self, path: PathBuf) {
        let window = {
            let mut external = self.0.0.external.lock();
            external.ready.retain(|client| !client.is_closed());
            match external.ready.last() {
                Some(client) => client.clone(),
                None => return external.pending.push(path),
            }
        };
        let result = self.0.open_for_window(&window, path).await;
        window.emit("project:open-external", result);
    }
}

/// Windows ready for `project:open-external`, and files waiting for one.
#[derive(Default)]
struct External {
    ready: Vec<Arc<Client>>,
    pending: Vec<PathBuf>,
}

#[derive(Clone)]
struct Projects(Arc<Inner>);

struct Inner {
    files: Arc<ProjectFiles>,
    temp_dir: PathBuf,
    remote_media: PathBuf,
    remote_projects: PathBuf,
    windows: Mutex<Windows>,
    /// Open project readers by token.
    readers: Mutex<HashMap<String, ReaderGrant>>,
    /// Browser uploads by id.
    web_uploads: Mutex<HashMap<String, Arc<WebUpload>>>,
    external: Mutex<External>,
}

/// Per-window grants, keyed by `Client::id`.
#[derive(Default)]
struct Windows {
    watched: HashSet<u64>,
    pending: HashSet<u64>,
    destinations: HashMap<u64, Destination>,
    uploads: HashMap<u64, UploadSlot>,
    saves: HashMap<u64, SaveSlot>,
}

struct Destination {
    token: String,
    path: String,
    directory: bool,
}

struct UploadSlot {
    id: String,
    upload: Arc<Mutex<FileUpload>>,
    timer: Arc<IdleTimer>,
}

struct SaveSlot {
    id: String,
    save: Arc<Mutex<ProjectSave>>,
    timer: Arc<IdleTimer>,
    finishing: bool,
}

struct ReaderGrant {
    owner: u64,
    timer: Arc<IdleTimer>,
}

impl Projects {
    /// Release everything `client` holds once it disconnects.
    fn watch(&self, client: &Client) {
        if !self.0.windows.lock().watched.insert(client.id) {
            return;
        }
        let (state, owner, runtime) = (self.clone(), client.id, tokio::runtime::Handle::current());
        client.on_close(move || {
            runtime.spawn(async move { state.forget(owner).await });
        });
    }

    async fn forget(self, owner: u64) {
        let (upload, save) = {
            let mut windows = self.0.windows.lock();
            windows.watched.remove(&owner);
            windows.pending.remove(&owner);
            windows.destinations.remove(&owner);
            (windows.uploads.remove(&owner), windows.saves.remove(&owner))
        };
        upload.iter().for_each(|slot| slot.timer.clear());
        save.iter().for_each(|slot| slot.timer.clear());
        let readers: Vec<String> = self.0.readers.lock().iter().filter(|(_, grant)| grant.owner == owner).map(|(token, _)| token.clone()).collect();
        for token in readers {
            let _ = self.close_reader(token, false).await;
        }
        let uploads: Vec<Arc<WebUpload>> = {
            let mut web = self.0.web_uploads.lock();
            let ids: Vec<String> = web.iter().filter(|(_, upload)| upload.owner == owner).map(|(id, _)| id.clone()).collect();
            ids.iter().filter_map(|id| web.remove(id)).collect()
        };
        let _ = blocking(move || {
            drop((upload, save));
            uploads.iter().for_each(|upload| upload.remove());
            Ok(())
        })
        .await;
    }

    /* ── incremental project saves ── */

    async fn save_begin(self, call: Call) -> Result<Value> {
        let request = call.arg(0);
        let project_id = request.get("projectId").and_then(Value::as_str).filter(|id| is_project_id(id)).map(str::to_owned);
        let save_as = optional_bool(request.get("saveAs"));
        let document_bytes = request.get("documentBytes").and_then(safe_uint).filter(|bytes| (2..=0xffff_ffff).contains(bytes));
        let (true, Some(project_id), Some(save_as), Some(document_bytes)) = (request.is_object(), project_id, save_as, document_bytes) else {
            bail!("Invalid project save request");
        };
        let media = validate_save_media(request.get("media").unwrap_or(&Value::Null))?;
        let name = sanitize_save_name(request.get("name")).context("Invalid project save name")?;
        let owner = call.client.id;
        {
            let mut windows = self.0.windows.lock();
            if windows.pending.contains(&owner) || windows.uploads.contains_key(&owner) || windows.saves.contains_key(&owner) {
                bail!("A save is already in progress.");
            }
            windows.pending.insert(owner);
        }
        self.watch(&call.client);
        let started = async {
            let known = self.0.files.destination(&project_id);
            let mut selected = None;
            if known.is_none() || save_as {
                match save_dialog(known.as_deref(), &name).await {
                    Some(path) => selected = Some(path),
                    None => return Ok(None),
                }
            }
            if call.client.is_closed() {
                bail!("Save window is unavailable");
            }
            let files = self.0.files.clone();
            let save = blocking(move || files.begin_incremental(&project_id, media, document_bytes, selected.as_deref())).await?;
            let id = uuid::Uuid::new_v4().to_string();
            let required = save.required.clone();
            let timer = IdleTimer::start(self.clone().discard_save(owner, Some(id.clone())));
            let save = Arc::new(Mutex::new(save));
            self.0.windows.lock().saves.insert(owner, SaveSlot { id: id.clone(), save, timer, finishing: false });
            if call.client.is_closed() {
                self.clone().discard_save(owner, Some(id)).await;
                bail!("Save cancelled");
            }
            Ok(Some(json!({ "ok": true, "token": id, "required": required })))
        };
        match started.await {
            Ok(Some(value)) => Ok(value),
            Ok(None) => {
                self.0.windows.lock().pending.remove(&owner);
                Ok(cancelled())
            }
            Err(error) => {
                self.0.windows.lock().pending.remove(&owner);
                Ok(failed(&error, "Could not start saving."))
            }
        }
    }

    async fn save_chunk(self, call: Call) -> Result<Value> {
        let request = call.arg(0);
        let asset = match request.get("assetId") {
            Some(Value::Null) => Some(None),
            Some(Value::String(id)) => Some(Some(id.clone())),
            _ => None,
        };
        let data = request.get("data").and_then(|data| call.bytes(data).ok()).filter(|data| (1..=FILE_CHUNK_BYTES).contains(&data.len()));
        let slot = {
            let windows = self.0.windows.lock();
            windows
                .saves
                .get(&call.client.id)
                .filter(|slot| !slot.finishing && request.get("token").and_then(Value::as_str) == Some(slot.id.as_str()))
                .map(|slot| (slot.save.clone(), slot.timer.clone()))
        };
        let (Some((save, timer)), Some(asset), Some(data)) = (slot, asset, data) else {
            bail!("Invalid project save chunk");
        };
        timer.refresh();
        blocking(move || save.lock().write(asset.as_deref(), &data)).await?;
        timer.refresh();
        Ok(Value::Null)
    }

    async fn save_finish(self, call: Call) -> Result<Value> {
        let owner = call.client.id;
        let save = {
            let mut windows = self.0.windows.lock();
            let slot = windows.saves.get_mut(&owner).filter(|slot| !slot.finishing && call.arg(0).as_str() == Some(slot.id.as_str()));
            let slot = slot.context("Unknown project save token")?;
            slot.finishing = true;
            slot.timer.clear();
            (slot.save.clone(), slot.id.clone())
        };
        let (save, id) = save;
        let result = blocking(move || save.lock().finish()).await;
        self.discard_save(owner, Some(id)).await;
        Ok(match result {
            Ok(path) => json!({ "ok": true, "path": path }),
            Err(error) => failed(&error, "Could not save project."),
        })
    }

    async fn discard_save(self, owner: u64, id: Option<String>) {
        let slot = {
            let mut windows = self.0.windows.lock();
            if !windows.saves.get(&owner).is_some_and(|slot| id.as_ref().is_none_or(|id| *id == slot.id)) {
                return;
            }
            windows.pending.remove(&owner);
            windows.saves.remove(&owner).unwrap()
        };
        slot.timer.clear();
        let _ = blocking(move || {
            drop(slot);
            Ok(())
        })
        .await;
    }

    /* ── export destinations and plain file saves ── */

    async fn export_choose(self, call: Call) -> Result<Value> {
        let request = call.arg(0);
        let name = sanitize_save_name(request.get("name"));
        let directory = optional_bool(request.get("directory"));
        let (true, Some(name), Some(directory)) = (request.is_object(), name, directory) else {
            bail!("Invalid export destination");
        };
        let owner = call.client.id;
        {
            let mut windows = self.0.windows.lock();
            if windows.pending.contains(&owner) || windows.destinations.contains_key(&owner) {
                bail!("An export is already in progress.");
            }
            windows.pending.insert(owner);
        }
        self.watch(&call.client);
        let selected = if directory {
            let picked = crate::dialog::file().set_title("Export PNG Sequence").set_can_create_directories(true).pick_folder().await;
            picked.and_then(|handle| handle.path().to_str().map(str::to_owned))
        } else {
            save_dialog(None, &name).await
        };
        let mut windows = self.0.windows.lock();
        windows.pending.remove(&owner);
        let Some(path) = selected.filter(|_| !call.client.is_closed()) else { return Ok(Value::Null) };
        let token = uuid::Uuid::new_v4().to_string();
        windows.destinations.insert(owner, Destination { token: token.clone(), path, directory });
        Ok(json!(token))
    }

    async fn file_upload_begin(self, call: Call) -> Result<Value> {
        let size = safe_uint(call.arg(0)).filter(|size| *size >= 1).context("Invalid save size")?;
        let owner = call.client.id;
        {
            let windows = self.0.windows.lock();
            if windows.uploads.contains_key(&owner) || windows.pending.contains(&owner) {
                bail!("A save is already in progress.");
            }
        }
        let root = self.0.temp_dir.clone();
        let upload = blocking(move || FileUpload::create(size, &root)).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let timer = IdleTimer::start(self.clone().discard_upload(owner, Some(id.clone())));
        let slot = UploadSlot { id: id.clone(), upload: Arc::new(Mutex::new(upload)), timer };
        {
            let mut windows = self.0.windows.lock();
            if windows.uploads.contains_key(&owner) {
                bail!("A save is already in progress.");
            }
            windows.uploads.insert(owner, slot);
        }
        self.watch(&call.client);
        Ok(json!(id))
    }

    async fn file_upload_chunk(self, call: Call) -> Result<Value> {
        let payload = call.arg(0);
        let data = payload.get("data").and_then(|data| call.bytes(data).ok()).filter(|data| (1..=FILE_CHUNK_BYTES).contains(&data.len()));
        let slot = self.upload_slot(call.client.id, payload.get("uploadId"));
        let (Some((upload, timer)), Some(data)) = (slot, data) else { bail!("Invalid save chunk") };
        timer.refresh();
        blocking(move || upload.lock().write(&data)).await?;
        timer.refresh();
        Ok(Value::Null)
    }

    fn upload_slot(&self, owner: u64, id: Option<&Value>) -> Option<(Arc<Mutex<FileUpload>>, Arc<IdleTimer>)> {
        let windows = self.0.windows.lock();
        let slot = windows.uploads.get(&owner).filter(|slot| id.and_then(Value::as_str) == Some(slot.id.as_str()))?;
        Some((slot.upload.clone(), slot.timer.clone()))
    }

    async fn discard_upload(self, owner: u64, id: Option<String>) {
        let slot = {
            let mut windows = self.0.windows.lock();
            if !windows.uploads.get(&owner).is_some_and(|slot| id.as_ref().is_none_or(|id| *id == slot.id)) {
                return;
            }
            windows.uploads.remove(&owner).unwrap()
        };
        slot.timer.clear();
        let _ = blocking(move || {
            drop(slot);
            Ok(())
        })
        .await;
    }

    async fn file_save(self, call: Call) -> Result<Value> {
        let payload = call.arg(0);
        if !payload.is_object() {
            bail!("file:save: expected an object");
        }
        let name = sanitize_save_name(payload.get("name")).context("file:save: invalid name")?;
        let project_id = match payload.get("projectId") {
            None => None,
            Some(Value::String(id)) if is_project_id(id) => Some(id.clone()),
            Some(_) => bail!("file:save: invalid project id"),
        };
        let save_as = optional_bool(payload.get("saveAs")).context("file:save: invalid saveAs")?;
        let owner = call.client.id;

        let destination = match payload.get("destinationToken") {
            None => None,
            Some(token) => {
                let windows = self.0.windows.lock();
                let destination = windows.destinations.get(&owner).filter(|destination| token.as_str() == Some(destination.token.as_str()) && project_id.is_none());
                let destination = destination.context("Unknown export destination")?;
                Some((destination.path.clone(), destination.directory))
            }
        };
        if let Some((_, true)) = destination {
            let png = Path::new(&name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
            if payload.get("name").and_then(Value::as_str) != Some(name.as_str()) || !png {
                bail!("Invalid export frame name");
            }
        }

        let upload_id = payload.get("uploadId").map(|id| id.as_str().map(str::to_owned));
        let source = match &upload_id {
            Some(_) => {
                let (upload, timer) = self.upload_slot(owner, payload.get("uploadId")).context("The save upload is incomplete.")?;
                upload.lock().claim()?;
                timer.clear();
                Source::Upload(upload)
            }
            None => {
                let data = payload.get("data").and_then(|data| call.bytes(data).ok()).context("file:save: data must be Uint8Array")?;
                if data.len() > FILE_SAVE_BYTES {
                    return Ok(json!({ "ok": false, "cancelled": false, "error": "too large; use streaming export" }));
                }
                Source::Bytes(data)
            }
        };

        if !self.0.windows.lock().pending.insert(owner) {
            return Ok(json!({ "ok": false, "cancelled": false, "error": "A save is already in progress." }));
        }
        let saved = async {
            if call.client.is_closed() {
                bail!("Save window is unavailable");
            }
            let known = project_id.as_deref().and_then(|id| self.0.files.destination(id));
            let mut selected = destination.as_ref().map(|(path, directory)| if *directory { path_join(path, &name) } else { path.clone() });
            if destination.is_none() && (known.is_none() || save_as) {
                match save_dialog(known.as_deref(), &name).await {
                    Some(path) => selected = Some(path),
                    None => return Ok(cancelled()),
                }
            }
            let files = self.0.files.clone();
            let target = selected.clone();
            let id = project_id.clone();
            let path = blocking(move || {
                let write = move |reader: &mut dyn std::io::Read| match id {
                    Some(id) => files.save(&id, reader, target.as_deref()),
                    None => {
                        let target = target.context("Choose a destination.")?;
                        durable::atomic_write(Path::new(&target), reader)?;
                        Ok(target)
                    }
                };
                match &source {
                    Source::Upload(upload) => write(&mut upload.lock().reader()?),
                    Source::Bytes(data) => write(&mut data.as_ref()),
                }
            })
            .await?;
            if project_id.is_none() && matches!(destination, Some((_, false))) {
                call.host.remember_export(owner, &path);
            }
            Ok(json!({ "ok": true, "path": path }))
        };
        let result = saved.await.unwrap_or_else(|error| failed(&error, "Save failed. Check disk space and folder permissions."));
        self.0.windows.lock().pending.remove(&owner);
        if let Some(Some(id)) = upload_id {
            self.discard_upload(owner, Some(id)).await;
        }
        Ok(result)
    }

    /* ── caption sidecars ── */

    async fn write_sidecar(&self, call: &Call) -> Value {
        let request = call.arg(0);
        let refuse = |error: &str| json!({ "ok": false, "cancelled": false, "error": error });
        let path = request.get("path").and_then(Value::as_str);
        let exported = path.is_some_and(|path| call.host.exported(call.client.id, path));
        let (Some(path), true) = (path, exported) else {
            return refuse("Captions can only be written next to a video exported from this window.");
        };
        let extension = match request.get("extension").and_then(Value::as_str) {
            Some(extension @ ("srt" | "vtt")) => extension,
            _ => return refuse("Unsupported caption format."),
        };
        let Some(text) = request.get("text").and_then(Value::as_str) else { return refuse("Caption text is missing.") };
        // A bad suffix is refused rather than dropped, so two layers' files can
        // never silently land on the same unsuffixed name.
        let suffix = match request.get("suffix") {
            None => None,
            Some(Value::String(suffix)) if is_sidecar_suffix(suffix) => Some(suffix.as_str()),
            Some(_) => return refuse("Invalid caption file name."),
        };
        if text.len() > MAX_SIDECAR_BYTES {
            return refuse("The caption file is too large.");
        }
        let target = sidecar_path(Path::new(path), extension, suffix);
        let bytes = text.as_bytes().to_vec();
        let written = {
            let target = target.clone();
            blocking(move || durable::atomic_write(&target, &mut bytes.as_slice())).await
        };
        match written {
            Ok(()) => json!({ "ok": true, "path": target.to_string_lossy() }),
            Err(error) => failed(&error, "Could not write the caption file."),
        }
    }

    /* ── opening and reading projects ── */

    async fn open_for_window(&self, client: &Arc<Client>, path: PathBuf) -> Value {
        let files = self.0.files.clone();
        let opened = match blocking(move || files.open(&path)).await {
            Ok(opened) => opened,
            Err(error) => return failed(&error, "Could not open project."),
        };
        if client.is_closed() {
            let files = self.0.files.clone();
            let token = opened.token.clone();
            let _ = blocking(move || files.close(&token, false)).await;
            return failed(&anyhow::anyhow!("Project window is unavailable"), "Could not open project.");
        }
        self.watch(client);
        let timer = IdleTimer::start({
            let (state, token) = (self.clone(), opened.token.clone());
            async move {
                let _ = state.close_reader(token, false).await;
            }
        });
        self.0.readers.lock().insert(opened.token.clone(), ReaderGrant { owner: client.id, timer });
        let mut value = serde_json::to_value(opened).expect("project metadata serializes");
        value["ok"] = json!(true);
        value
    }

    fn owns_reader(&self, client: &Client, token: &str) -> bool {
        self.0.readers.lock().get(token).is_some_and(|grant| grant.owner == client.id)
    }

    async fn project_read(self, call: Call) -> Result<Payload> {
        let request = call.arg(0);
        let token = request.get("token").and_then(Value::as_str).filter(|token| self.owns_reader(&call.client, token)).context("Unknown project read token")?.to_owned();
        let timer = self.0.readers.lock().get(&token).map(|grant| grant.timer.clone()).context("Unknown project read token")?;
        let range = request.get("offset").and_then(safe_uint).zip(request.get("length").and_then(safe_uint));
        let (offset, length) = range.context("Invalid project read range")?;
        timer.refresh();
        let files = self.0.files.clone();
        let data = blocking(move || files.read(&token, offset, length)).await?;
        timer.refresh();
        Ok(Payload::bytes(Bytes::from(data)))
    }

    async fn close_reader(&self, token: String, verify: bool) -> Result<()> {
        let Some(grant) = self.0.readers.lock().remove(&token) else { return Ok(()) };
        grant.timer.clear();
        let files = self.0.files.clone();
        blocking(move || files.close(&token, verify)).await
    }

    /* ── browser uploads (web-bridge.ts) ── */

    async fn web_upload_begin(self, call: Call) -> Result<Value> {
        let payload = call.arg(0);
        if !payload.is_object() {
            bail!("upload: expected {{ name, size, kind }}");
        }
        let name = payload.get("name").and_then(Value::as_str).and_then(safe_file_name);
        let size = payload.get("size").and_then(safe_uint).filter(|size| *size <= MAX_WEB_UPLOAD_BYTES);
        let (Some(name), Some(size)) = (name, size) else { bail!("upload: invalid name or size") };
        let root = match payload.get("kind").and_then(Value::as_str) {
            Some("project") => self.0.remote_projects.clone(),
            _ => self.0.remote_media.clone(),
        };
        let (owner, id) = (call.client.id, uuid::Uuid::new_v4().to_string());
        let upload = {
            let id = id.clone();
            blocking(move || WebUpload::create(owner, &root, &id, &name, size)).await?
        };
        tracing::info!("client {owner}: uploading {} ({:.1} MB)", upload.path.display(), size as f64 / 1_048_576.0);
        self.0.web_uploads.lock().insert(id.clone(), Arc::new(upload));
        self.watch(&call.client);
        Ok(json!(id))
    }

    fn web_upload(&self, client: &Client, id: Option<&str>) -> Option<Arc<WebUpload>> {
        self.0.web_uploads.lock().get(id?).filter(|upload| upload.owner == client.id).cloned()
    }

    fn take_web_upload(&self, client: &Client, id: &str) -> Option<Arc<WebUpload>> {
        let mut uploads = self.0.web_uploads.lock();
        uploads.get(id).filter(|upload| upload.owner == client.id)?;
        uploads.remove(id)
    }

    async fn web_upload_chunk(self, call: Call) -> Result<Value> {
        let payload = call.arg(0);
        let upload = self.web_upload(&call.client, payload.get("id").and_then(Value::as_str)).context("upload: unknown upload")?;
        // Chunks arrive a few at a time and out of order; each names its offset.
        let data = payload.get("data").and_then(|data| call.bytes(data).ok());
        let offset = payload.get("offset").and_then(safe_uint);
        let (Some(data), Some(offset)) = (data, offset) else { bail!("upload: invalid chunk") };
        blocking(move || upload.write(offset, &data)).await?;
        Ok(Value::Null)
    }

    async fn web_upload_finish(self, call: Call) -> Result<Value> {
        let id = call.arg(0).as_str().unwrap_or_default();
        self.web_upload(&call.client, Some(id)).context("upload: unknown upload")?;
        let upload = self.take_web_upload(&call.client, id).context("upload: unknown upload")?;
        if !upload.complete() {
            blocking(move || {
                upload.remove();
                Ok(())
            })
            .await?;
            bail!("upload: incomplete");
        }
        let path = upload.path.to_str().context("upload: invalid path")?.to_owned();
        tracing::info!("client {}: upload complete {path}", call.client.id);
        Ok(json!(path))
    }

    async fn web_project_open_path(self, call: Call) -> Result<Value> {
        let path = call.arg(0).as_str().context("open: expected a path")?.to_owned();
        let root = self.0.remote_projects.clone();
        let resolved = blocking(move || {
            let resolved = std::fs::canonicalize(&path)?;
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            match resolved.strip_prefix(&root) {
                Ok(relative) if relative.components().next().is_some() => Ok(plain_path(&resolved)),
                _ => bail!("open: path is not an uploaded project"),
            }
        })
        .await?;
        let result = self.open_for_window(&call.client, resolved).await;
        tracing::info!("client {}: open project → {}", call.client.id, if result["ok"] == true { "ok" } else { "failed" });
        Ok(result)
    }
}

enum Source {
    Upload(Arc<Mutex<FileUpload>>),
    Bytes(Bytes),
}

/// The TS idle `setTimeout` with `refresh()`/`clearTimeout()`.
struct IdleTimer {
    deadline: Mutex<Option<Instant>>,
}

impl IdleTimer {
    fn start(expire: impl Future<Output = ()> + Send + 'static) -> Arc<Self> {
        let timer = Arc::new(Self { deadline: Mutex::new(Some(Instant::now() + IDLE)) });
        let weak: Weak<Self> = Arc::downgrade(&timer);
        tokio::spawn(async move {
            loop {
                let Some(deadline) = weak.upgrade().and_then(|timer| *timer.deadline.lock()) else { return };
                if Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep_until(deadline).await;
            }
            expire.await;
        });
        timer
    }

    fn refresh(&self) {
        let mut deadline = self.deadline.lock();
        if deadline.is_some() {
            *deadline = Some(Instant::now() + IDLE);
        }
    }

    fn clear(&self) {
        *self.deadline.lock() = None;
    }
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await?
}

fn cancelled() -> Value {
    json!({ "ok": false, "cancelled": true })
}

fn failed(error: &anyhow::Error, fallback: &str) -> Value {
    let message = error.to_string();
    json!({ "ok": false, "cancelled": false, "error": if message.is_empty() { fallback.to_owned() } else { message } })
}

/// `undefined` → false, a boolean → itself, anything else → invalid.
fn optional_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        None => Some(false),
        Some(Value::Bool(flag)) => Some(*flag),
        Some(_) => None,
    }
}

/// Convert an untrusted suggestion into one plain filename.
fn sanitize_save_name(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.replace('\0', "");
    let name = text.split(['/', '\\']).rfind(|part| !part.is_empty()).unwrap_or_default();
    (!matches!(name, "" | "." | "..") && js_len(name) <= MAX_SAVE_NAME_CHARS).then(|| name.to_owned())
}

fn save_filter(name: &str) -> Option<(String, String)> {
    let extension = Path::new(name).extension()?.to_str()?.to_lowercase();
    if !(1..=20).contains(&extension.len()) || !extension.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()) {
        return None;
    }
    let label = match extension.as_str() {
        "gif" => "GIF Image".into(),
        "jpeg" | "jpg" => "JPEG Image".into(),
        "json" => "JSON".into(),
        "mp3" => "MP3 Audio".into(),
        "mp4" => "MPEG-4 Video".into(),
        "pmv" => "Powermove Project".into(),
        "png" => "PNG Image".into(),
        "svg" => "SVG Image".into(),
        "wav" => "WAV Audio".into(),
        "webm" => "WebM Video".into(),
        other => format!("{} File", other.to_uppercase()),
    };
    Some((label, extension))
}

/// The native Save dialog, starting at `known` (the project's file) or `name`.
async fn save_dialog(known: Option<&str>, name: &str) -> Option<String> {
    let mut dialog = crate::dialog::file();
    match known.map(Path::new) {
        Some(known) => {
            if let Some(parent) = known.parent() {
                dialog = dialog.set_directory(parent);
            }
            dialog = dialog.set_file_name(known.file_name().map(|file| file.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_owned()));
        }
        None => dialog = dialog.set_file_name(name),
    }
    if let Some((label, extension)) = save_filter(name) {
        dialog = dialog.add_filter(label, &[extension]);
    }
    let picked = dialog.save_file().await?;
    picked.path().to_str().map(str::to_owned)
}

fn path_join(directory: &str, name: &str) -> String {
    Path::new(directory).join(name).to_string_lossy().into_owned()
}

fn has_project_extension(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("pmv") || extension.eq_ignore_ascii_case("json"))
}

/// `SIDECAR_SUFFIX_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$/`
fn is_sidecar_suffix(suffix: &str) -> bool {
    let bytes = suffix.as_bytes();
    (1..=32).contains(&bytes.len()) && bytes[0].is_ascii_alphanumeric() && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
}

/// "Film.mp4" → "Film.srt" or "Film.en.vtt", in the video's folder.
fn sidecar_path(video: &Path, extension: &str, suffix: Option<&str>) -> PathBuf {
    let base = video.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let tag = suffix.map(|suffix| format!(".{suffix}")).unwrap_or_default();
    video.with_file_name(format!("{base}{tag}.{extension}"))
}

/// Drop the `\\?\` prefix `canonicalize` adds on Windows when it is not needed.
fn plain_path(path: &Path) -> PathBuf {
    let mut components = path.components();
    match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(letter) => PathBuf::from(format!("{}:\\", letter as char)).join(components.as_path()),
            Prefix::VerbatimUNC(server, share) => {
                PathBuf::from(format!("\\\\{}\\{}\\", server.to_string_lossy(), share.to_string_lossy())).join(components.as_path())
            }
            _ => path.to_owned(),
        },
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests;
