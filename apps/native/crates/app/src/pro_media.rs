//! Pro media control (plan section 1): proxy and optimized media made on a
//! worker into this Mac's media storage, their Background Tasks, and what
//! the viewer and the Project panel learn from them.
//!
//! - Generation is `pm_proxy::generate`: validated, content-addressed
//!   transcodes (proxies at Settings › Media's size and format; optimized
//!   media as full-size ProRes 422 HQ, 4444 for alpha or deep sources).
//! - The index maps a source file (path, length, modification time) to its
//!   renditions, so reopening any project with that file finds them without
//!   hashing it again. Attached proxies are the user's own files: never
//!   deleted, only forgotten.
//! - Always Play Original is per project and asset.
//! - Exports read originals unless the user picks "Use Proxies (Draft)".
//! - Work runs on one worker thread, never the main thread (N8); pausing a
//!   task stops its encoder and it starts again on Resume.
use pm_engine_service::MediaCatalog;
use pm_promedia::storage::{Location, Storage, StorageClass, StorageSettings};
use pm_promedia::tasks::{
    BackgroundTasks, ProgressUnit, Task, TaskFailure, TaskFailureCode, TaskId, TaskKind, TaskSpec,
    TaskState, Ticket,
};
use pm_protocol::priority::{ProxyFormat, ProxySize};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

/// A generated rendition of an original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Rendition {
    Proxy,
    Optimized,
}

/// Settings › Media › Create Proxies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Create {
    /// On import, for video this Mac is unlikely to play in real time:
    /// larger than 1080p, or deeper than 8 bits at 4K and above.
    WhenNeeded,
    /// On import, for every video.
    AllVideo,
    /// Only from the Project panel.
    OnlyWhenAsked,
}

/// Settings › Media, kept beside the index (this Mac's media settings; the
/// v1.2.0 profile keys stay exactly as they were).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub create: Create,
    pub size: ProxySize,
    pub format: ProxyFormat,
    pub location: Option<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            create: Create::OnlyWhenAsked,
            size: ProxySize::Automatic,
            format: ProxyFormat::ProResProxy,
            location: None,
        }
    }
}

/// A Settings › Media row: id, label, detail, value, and its choices.
pub type Row = (
    &'static str,
    &'static str,
    &'static str,
    serde_json::Value,
    Vec<(serde_json::Value, &'static str)>,
);

impl Preferences {
    /// Settings › Media's rows: id, label, detail, and choices (value,
    /// label), with the current value.
    pub fn rows(&self) -> Vec<Row> {
        let v = |x: &dyn erased::Wire| x.wire();
        vec![
            (
                "media.proxyCreate",
                "Create Proxies",
                "Make proxy media as video comes in, or only when you ask in the Project panel.",
                v(&self.create),
                vec![
                    (serde_json::json!("whenNeeded"), "When Needed"),
                    (serde_json::json!("allVideo"), "For All Video"),
                    (serde_json::json!("onlyWhenAsked"), "Only When I Ask"),
                ],
            ),
            (
                "media.proxySize",
                "Proxy Size",
                "Automatic is half the original's height, never below 540p.",
                v(&self.size),
                vec![
                    (serde_json::json!("automatic"), "Automatic"),
                    (serde_json::json!("full"), "Full"),
                    (serde_json::json!("half"), "Half"),
                    (serde_json::json!("quarter"), "Quarter"),
                    (serde_json::json!("p1080"), "1080p"),
                    (serde_json::json!("p720"), "720p"),
                ],
            ),
            (
                "media.proxyFormat",
                "Proxy Format",
                "Video with alpha, or deeper than 10 bits, uses ProRes 4444.",
                v(&self.format),
                vec![
                    (serde_json::json!("proResProxy"), "ProRes 422 Proxy"),
                    (serde_json::json!("proResLt"), "ProRes 422 LT"),
                    (serde_json::json!("hevc10"), "HEVC (Smaller Files)"),
                ],
            ),
        ]
    }

    /// Sets the row `id` to `value`; an unknown row or value is refused.
    pub fn set(&mut self, id: &str, value: &serde_json::Value) -> Result<(), String> {
        let bad = |_| "Invalid setting value".to_owned();
        match id {
            "media.proxyCreate" => {
                self.create = serde_json::from_value(value.clone()).map_err(bad)?
            }
            "media.proxySize" => self.size = serde_json::from_value(value.clone()).map_err(bad)?,
            "media.proxyFormat" => {
                let format: ProxyFormat = serde_json::from_value(value.clone()).map_err(bad)?;
                if format == ProxyFormat::ProResHq {
                    return Err("Invalid setting value".into());
                }
                self.format = format;
            }
            _ => return Err("Settings service is unavailable for this row".into()),
        }
        Ok(())
    }

    /// Whether importing a video with these facts creates its proxy.
    pub fn creates_on_import(&self, width: u32, height: u32, depth: u8) -> bool {
        match self.create {
            Create::OnlyWhenAsked => false,
            Create::AllVideo => true,
            Create::WhenNeeded => height > 1080 || width > 1920 || (depth > 8 && height >= 2160),
        }
    }
}

/// What a source file has.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Entry {
    proxy: Option<PathBuf>,
    /// The proxy is the user's own file (Attach Proxy Media…).
    #[serde(default)]
    attached: bool,
    optimized: Option<PathBuf>,
}

/// A source file at one state.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
struct Source {
    path: PathBuf,
    len: u64,
    modified: u128,
}

impl Source {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        Some(Self {
            path: path.to_path_buf(),
            len: meta.len(),
            modified,
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    sources: Vec<(Source, Entry)>,
    /// Always Play Original: project id to asset ids.
    play_original: BTreeMap<String, BTreeSet<String>>,
}

/// One queued generation.
struct Job {
    kind: Rendition,
    project: String,
    asset: String,
    source: PathBuf,
    size: ProxySize,
    format: ProxyFormat,
}

/// A rendition's state for one asset (the Project panel's status).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Absent,
    Queued,
    Creating {
        percent: u8,
    },
    Ready,
    Failed,
}

/// What the main thread hears from the worker.
#[derive(Debug)]
pub enum Event {
    /// Tasks moved; the Background Tasks window redraws.
    Changed,
    /// A rendition landed or was removed: the catalog changes.
    Renditions,
}

/// Bytes per storage class, for Settings › Media › Storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub location: PathBuf,
    pub proxies: u64,
    pub optimized: u64,
    pub renders: u64,
}

struct Shared {
    index: Index,
    jobs: BTreeMap<TaskId, Job>,
    /// Why the running encode waits (low disk, playback or an export).
    waiting: Option<(TaskId, pm_proxy::PauseReason)>,
    /// Pause or cancel the running encode.
    stop: Arc<AtomicBool>,
}

pub struct ProMedia {
    profile: PathBuf,
    shared: Arc<Mutex<Shared>>,
    pub tasks: Arc<Mutex<BackgroundTasks>>,
    storage: Mutex<Option<(PathBuf, Arc<Storage>)>>,
    wake: Mutex<Option<mpsc::Sender<()>>>,
    events: async_channel::Sender<Event>,
    pub event_receiver: async_channel::Receiver<Event>,
    /// Interactive work (an export) pauses generation, as pm-proxy expects.
    interactive: Arc<AtomicBool>,
    /// Imported assets the catalog has not resolved yet.
    imported: Mutex<Vec<String>>,
    preferences: Mutex<Preferences>,
    /// Space used, measured off the main thread after each change.
    usage: Arc<Mutex<Option<Usage>>>,
}

/// The window id tasks are filed under (one editor window per profile).
pub const WINDOW: &str = "main";

fn index_path(profile: &Path) -> PathBuf {
    profile.join("Pro Media").join("index.json")
}

fn settings_path(profile: &Path) -> PathBuf {
    profile.join("Pro Media").join("settings.json")
}

fn persist(profile: &Path, prefs: &Preferences) -> Result<(), String> {
    let path = settings_path(profile);
    let fail = |e: std::io::Error| format!("Settings could not be saved: {e}");
    std::fs::create_dir_all(path.parent().expect("settings folder")).map_err(fail)?;
    let tmp = path.with_extension("json.partial");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(prefs).expect("preferences serialize"),
    )
    .map_err(fail)?;
    std::fs::rename(&tmp, &path).map_err(fail)
}

/// Space each class uses in `storage`.
fn measure(storage: &Storage) -> Usage {
    let bytes = |class| storage.usage(class).map(|u| u.active_bytes).unwrap_or(0);
    Usage {
        location: storage.settings().locations[&StorageClass::Proxies]
            .directory
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        proxies: bytes(StorageClass::Proxies),
        optimized: bytes(StorageClass::Optimized),
        renders: bytes(StorageClass::Renders),
    }
}

fn load(profile: &Path) -> Index {
    std::fs::read(index_path(profile))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(profile: &Path, index: &Index) {
    let path = index_path(profile);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.partial");
    if let Ok(bytes) = serde_json::to_vec_pretty(index)
        && std::fs::write(&tmp, bytes).is_ok()
    {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn entry<'a>(index: &'a Index, path: &Path) -> Option<&'a Entry> {
    let now = Source::of(path)?;
    index
        .sources
        .iter()
        .find(|(s, _)| *s == now)
        .map(|(_, e)| e)
}

fn entry_mut<'a>(index: &'a mut Index, source: &Source) -> &'a mut Entry {
    if let Some(at) = index.sources.iter().position(|(s, _)| s == source) {
        return &mut index.sources[at].1;
    }
    // An earlier state of the same path no longer describes this file.
    index.sources.retain(|(s, _)| s.path != source.path);
    index.sources.push((source.clone(), Entry::default()));
    &mut index.sources.last_mut().expect("pushed").1
}

/// The FFmpeg pair pm-proxy encodes with.
fn ffmpeg() -> Option<(PathBuf, PathBuf)> {
    let pair = |dir: &Path| {
        let (f, p) = (dir.join("ffmpeg"), dir.join("ffprobe"));
        (f.is_file() && p.is_file()).then_some((f, p))
    };
    if let Some(f) = std::env::var_os("POWERMOVE_FFMPEG").map(PathBuf::from)
        && let Some(dir) = f.parent()
        && let Some(found) = pair(dir)
    {
        return Some(found);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PM_FFMPEG_DIR")
        .map(|d| PathBuf::from(d).join("bin"))
        .into_iter()
        .collect();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
    }
    dirs.iter().find_map(|d| pair(d))
}

fn failure(code: TaskFailureCode, diagnostic: impl Into<String>) -> TaskFailure {
    TaskFailure {
        code,
        diagnostic: diagnostic.into(),
    }
}

fn classify(error: &pm_proxy::ProxyError) -> TaskFailureCode {
    use pm_proxy::ProxyError as E;
    match error {
        E::Cancelled => TaskFailureCode::Cancelled,
        E::Io(e) if e.kind() == std::io::ErrorKind::NotFound => TaskFailureCode::SourceOffline,
        E::Io(e) if e.raw_os_error() == Some(libc_enospc()) => TaskFailureCode::DiskFull,
        E::InvalidMedia(_) => TaskFailureCode::CodecUnsupported,
        _ => TaskFailureCode::Unknown,
    }
}

const fn libc_enospc() -> i32 {
    28
}

impl ProMedia {
    pub fn new(profile: PathBuf, interactive: Arc<AtomicBool>) -> Self {
        let index = load(&profile);
        let profile_clone = profile.clone();
        let (events, event_receiver) = async_channel::unbounded();
        Self {
            profile,
            shared: Arc::new(Mutex::new(Shared {
                index,
                jobs: BTreeMap::new(),
                waiting: None,
                stop: Arc::new(AtomicBool::new(false)),
            })),
            tasks: Arc::new(Mutex::new(BackgroundTasks::default())),
            storage: Mutex::new(None),
            wake: Mutex::new(None),
            events,
            event_receiver,
            interactive,
            imported: Mutex::new(Vec::new()),
            preferences: Mutex::new(
                std::fs::read(settings_path(&profile_clone))
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default(),
            ),
            usage: Arc::new(Mutex::new(None)),
        }
    }

    /// Settings › Media, as this Mac keeps it.
    pub fn preferences(&self) -> Preferences {
        self.preferences.lock().unwrap().clone()
    }

    /// Writes Settings › Media's row `id`.
    pub fn set_preference(&self, id: &str, value: &serde_json::Value) -> Result<(), String> {
        let mut prefs = self.preferences.lock().unwrap();
        let mut next = prefs.clone();
        next.set(id, value)?;
        persist(&self.profile, &next)?;
        *prefs = next;
        Ok(())
    }

    /// Moves the media storage to `folder` (refused unless it is empty or
    /// Powermove's). Existing files stay where they are.
    pub fn set_location(&self, folder: PathBuf) -> Result<(), String> {
        let mut next = self.preferences();
        next.location = Some(folder);
        self.storage(&next)?;
        persist(&self.profile, &next)?;
        *self.preferences.lock().unwrap() = next;
        self.refresh_usage();
        Ok(())
    }

    /// The last measured usage (`None` until the first measurement).
    pub fn cached_usage(&self) -> Option<Usage> {
        self.usage.lock().unwrap().clone()
    }

    /// Measures usage again on a worker; `Changed` follows.
    pub fn refresh_usage(&self) {
        let Ok(storage) = self.storage(&self.preferences()) else {
            return;
        };
        let (usage, events) = (self.usage.clone(), self.events.clone());
        let _ = std::thread::Builder::new()
            .name("pm-media-usage".into())
            .spawn(move || {
                *usage.lock().unwrap() = Some(measure(&storage));
                let _ = events.try_send(Event::Changed);
            });
    }

    /// The media storage at `prefs`' location (default: the profile's
    /// `Media Storage`), opened once per location. A folder Powermove does
    /// not own and that has other files in it is refused.
    pub fn storage(&self, prefs: &Preferences) -> Result<Arc<Storage>, String> {
        let root = prefs
            .location
            .clone()
            .unwrap_or_else(|| self.profile.join("Media Storage"));
        let mut held = self.storage.lock().unwrap();
        if let Some((at, storage)) = held.as_ref()
            && *at == root
        {
            return Ok(storage.clone());
        }
        let class = |name: &str| Location {
            directory: root.join(name),
            quota_bytes: u64::MAX,
        };
        let settings = StorageSettings {
            locations: BTreeMap::from([
                (StorageClass::Originals, class("Originals")),
                (StorageClass::Proxies, class("Proxies")),
                (StorageClass::Optimized, class("Optimized")),
                (StorageClass::Renders, class("Renders")),
            ]),
            archive: root.join("Archive"),
        };
        let storage = Arc::new(Storage::open(settings).map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidInput if e.to_string().contains("nonempty") => {
                "This folder has other files in it. Choose an empty folder.".to_owned()
            }
            std::io::ErrorKind::InvalidInput => "Choose a regular folder.".to_owned(),
            _ => format!("The media storage folder can't be used: {e}"),
        })?);
        *held = Some((root, storage.clone()));
        Ok(storage)
    }

    /// Adds `project`'s renditions to `media`: proxies (an embedded proxy
    /// stays), optimized media and Always Play Original.
    pub fn decorate(&self, project: &str, media: &mut MediaCatalog) {
        let shared = self.shared.lock().unwrap();
        for (id, path) in &media.originals {
            let Some(e) = entry(&shared.index, path) else {
                continue;
            };
            if let Some(proxy) = e.proxy.as_ref().filter(|p| p.is_file()) {
                media
                    .proxies
                    .entry(id.clone())
                    .or_insert_with(|| proxy.clone());
            }
            if let Some(optimized) = e.optimized.as_ref().filter(|p| p.is_file()) {
                media.optimized.insert(id.clone(), optimized.clone());
            }
        }
        media.play_original = shared
            .index
            .play_original
            .get(project)
            .cloned()
            .unwrap_or_default();
    }

    /// `asset`'s rendition states for the Project panel.
    pub fn status(&self, project: &str, asset: &str, original: Option<&Path>) -> (Status, Status) {
        let tasks = self.tasks.lock().unwrap();
        let shared = self.shared.lock().unwrap();
        let mut out = [Status::Absent, Status::Absent];
        if let Some(e) = original.and_then(|p| entry(&shared.index, p)) {
            if e.proxy.as_ref().is_some_and(|p| p.is_file()) {
                out[0] = Status::Ready;
            }
            if e.optimized.as_ref().is_some_and(|p| p.is_file()) {
                out[1] = Status::Ready;
            }
        }
        for (id, job) in &shared.jobs {
            if job.project != project || job.asset != asset {
                continue;
            }
            let Some(task) = tasks.get(*id) else { continue };
            let slot = &mut out[job.kind as usize];
            *slot = match task.state {
                TaskState::Queued | TaskState::Paused | TaskState::PauseRequested => Status::Queued,
                TaskState::Running => Status::Creating {
                    percent: (task.completed * 100)
                        .checked_div(task.total)
                        .unwrap_or(0)
                        .min(100) as u8,
                },
                TaskState::Failed if *slot != Status::Ready => Status::Failed,
                _ => slot.clone(),
            };
        }
        (out[0].clone(), out[1].clone())
    }

    pub fn plays_original(&self, project: &str, asset: &str) -> bool {
        self.shared
            .lock()
            .unwrap()
            .index
            .play_original
            .get(project)
            .is_some_and(|s| s.contains(asset))
    }

    /// Always Play Original on or off for `assets`.
    pub fn set_play_original(&self, project: &str, assets: &[String], on: bool) {
        let mut shared = self.shared.lock().unwrap();
        let set = shared
            .index
            .play_original
            .entry(project.to_owned())
            .or_default();
        for a in assets {
            if on {
                set.insert(a.clone());
            } else {
                set.remove(a);
            }
        }
        if set.is_empty() {
            shared.index.play_original.remove(project);
        }
        save(&self.profile, &shared.index);
        drop(shared);
        let _ = self.events.try_send(Event::Renditions);
    }

    /// Queues one task per asset: (asset id, title, original path). An
    /// asset already queued or creating that rendition is skipped.
    pub fn create(
        &self,
        project: &str,
        kind: Rendition,
        prefs: &Preferences,
        assets: Vec<(String, String, PathBuf)>,
    ) -> Result<usize, String> {
        let mut queued = 0;
        {
            let mut tasks = self.tasks.lock().unwrap();
            let mut shared = self.shared.lock().unwrap();
            for (asset, title, source) in assets {
                let busy = shared.jobs.iter().any(|(id, j)| {
                    j.project == project
                        && j.asset == asset
                        && j.kind == kind
                        && tasks.get(*id).is_some_and(|t| !t.state.terminal())
                });
                if busy {
                    continue;
                }
                let frames = pm_media::probe::probe(&source)
                    .ok()
                    .and_then(|p| p.probe.video.map(|v| v.frames as u64))
                    .unwrap_or(0);
                let id = tasks
                    .enqueue(
                        WINDOW.into(),
                        project.into(),
                        TaskSpec {
                            title,
                            kind: match kind {
                                Rendition::Proxy => TaskKind::Proxy,
                                Rendition::Optimized => TaskKind::Optimized,
                            },
                            unit: ProgressUnit::Frames,
                            priority: 0,
                            total: frames,
                        },
                    )
                    .map_err(|e| format!("The task could not be queued: {e:?}"))?;
                let (size, format) = match kind {
                    Rendition::Proxy => (prefs.size, prefs.format),
                    Rendition::Optimized => (ProxySize::Full, ProxyFormat::ProResHq),
                };
                shared.jobs.insert(
                    id,
                    Job {
                        kind,
                        project: project.into(),
                        asset,
                        source,
                        size,
                        format,
                    },
                );
                queued += 1;
            }
        }
        if queued > 0 {
            let storage = self.storage(prefs)?;
            self.kick(storage);
            let _ = self.events.try_send(Event::Changed);
        }
        Ok(queued)
    }

    /// Forgets `kind` for each original and removes generated files (an
    /// attached proxy is the user's file and stays on disk). A proxy is
    /// kept while its original is offline: it is the only playable copy.
    pub fn delete(&self, kind: Rendition, originals: &[PathBuf]) -> usize {
        let mut shared = self.shared.lock().unwrap();
        let mut removed = 0;
        for path in originals {
            let Some(source) = Source::of(path) else {
                continue;
            };
            let e = entry_mut(&mut shared.index, &source);
            let file = match kind {
                Rendition::Proxy => {
                    let attached = std::mem::take(&mut e.attached);
                    e.proxy.take().filter(|_| !attached)
                }
                Rendition::Optimized => e.optimized.take(),
            };
            if let Some(file) = file {
                let _ = std::fs::remove_file(&file);
            }
            removed += 1;
        }
        shared
            .index
            .sources
            .retain(|(_, e)| e.proxy.is_some() || e.optimized.is_some());
        save(&self.profile, &shared.index);
        drop(shared);
        self.refresh_usage();
        let _ = self.events.try_send(Event::Renditions);
        removed
    }

    /// Attaches `file` as `original`'s proxy after checking that it is the
    /// same picture: frame count, display aspect and duration.
    pub fn attach(&self, original: &Path, file: &Path) -> Result<(), String> {
        let a = pm_media::probe::probe(original)
            .map_err(|_| "The original can't be read.".to_owned())?;
        let b = pm_media::probe::probe(file)
            .map_err(|_| "This file isn't a video Powermove can read.".to_owned())?;
        let (Some(va), Some(vb)) = (a.probe.video.as_ref(), b.probe.video.as_ref()) else {
            return Err("This file isn't a video Powermove can read.".into());
        };
        let aspect = |v: &pm_media::probe::VideoProbe| {
            let (n, d) = v.pixel_aspect;
            v.width as f64 * n.max(1) as f64 / (v.height.max(1) as f64 * d.max(1) as f64)
        };
        let close = |x: Option<i64>, y: Option<i64>| match (x, y) {
            (Some(x), Some(y)) => (x - y).abs() <= 50_000,
            _ => true,
        };
        if va.frames != vb.frames
            || (aspect(va) - aspect(vb)).abs() > 0.01 * aspect(va)
            || !close(a.probe.duration_us, b.probe.duration_us)
        {
            return Err(
                "This file doesn't match the original. Check its frame rate and length.".into(),
            );
        }
        let source = Source::of(original).ok_or("The original can't be read.")?;
        let mut shared = self.shared.lock().unwrap();
        let e = entry_mut(&mut shared.index, &source);
        if let Some(old) = e.proxy.take().filter(|_| !e.attached) {
            let _ = std::fs::remove_file(old);
        }
        e.proxy = Some(file.to_path_buf());
        e.attached = true;
        save(&self.profile, &shared.index);
        drop(shared);
        self.refresh_usage();
        let _ = self.events.try_send(Event::Renditions);
        Ok(())
    }

    #[cfg(test)]
    fn attach_unchecked(&self, original: &Path, file: &Path) {
        let source = Source::of(original).expect("original");
        let mut shared = self.shared.lock().unwrap();
        let e = entry_mut(&mut shared.index, &source);
        e.proxy = Some(file.to_path_buf());
        e.attached = true;
    }

    /// Removes generated renditions that `pinned` (the open projects'
    /// originals) does not use, and every render. Attached proxies are
    /// never removed. Returns the bytes freed.
    pub fn clear(&self, prefs: &Preferences, pinned: &BTreeSet<PathBuf>) -> u64 {
        let mut shared = self.shared.lock().unwrap();
        let mut freed = 0;
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        for (source, e) in &mut shared.index.sources {
            if pinned.contains(&source.path) {
                continue;
            }
            if let Some(p) = e.proxy.take_if(|_| !e.attached) {
                freed += size(&p);
                let _ = std::fs::remove_file(p);
            }
            if let Some(p) = e.optimized.take() {
                freed += size(&p);
                let _ = std::fs::remove_file(p);
            }
        }
        shared
            .index
            .sources
            .retain(|(_, e)| e.proxy.is_some() || e.optimized.is_some());
        save(&self.profile, &shared.index);
        drop(shared);
        if let Ok(storage) = self.storage(prefs)
            && let Ok(cache) = storage.render_cache()
        {
            freed += cache.clear_unkept().unwrap_or(0);
        }
        self.refresh_usage();
        let _ = self.events.try_send(Event::Renditions);
        freed
    }

    /// Pause, resume, cancel, or set the priority of a task.
    pub fn act(&self, id: TaskId, action: TaskAction) -> Result<(), String> {
        let mut tasks = self.tasks.lock().unwrap();
        let running = tasks.get(id).is_some_and(|t| t.state == TaskState::Running);
        match action {
            TaskAction::Pause => tasks.pause(id),
            TaskAction::Resume => tasks.resume(id),
            TaskAction::Cancel => tasks.cancel(id),
            TaskAction::Priority(p) => tasks.priority(id, p),
        }
        .map_err(|e| format!("{e:?}"))?;
        if running && matches!(action, TaskAction::Pause | TaskAction::Cancel) {
            self.shared
                .lock()
                .unwrap()
                .stop
                .store(true, Ordering::Release);
        }
        drop(tasks);
        if action == TaskAction::Resume
            && let Some((_, storage)) = self.storage.lock().unwrap().clone()
        {
            self.kick(storage);
        }
        let _ = self.events.try_send(Event::Changed);
        Ok(())
    }

    /// Why a running task's encode waits, in the task's words.
    pub fn waiting(&self, id: TaskId) -> Option<&'static str> {
        match self.shared.lock().unwrap().waiting {
            Some((task, pm_proxy::PauseReason::LowDisk)) if task == id => {
                Some("Waiting for free disk space (Powermove keeps 10% of the disk free)")
            }
            Some((task, pm_proxy::PauseReason::InteractiveWork)) if task == id => {
                Some("Waiting while the viewer plays or an export runs")
            }
            _ => None,
        }
    }

    /// Every task, newest last.
    pub fn snapshot(&self) -> Vec<Task> {
        self.tasks.lock().unwrap().all()
    }

    /// Assets just imported, waiting for the catalog to resolve them.
    pub fn after_import(&self, assets: Vec<String>) {
        self.imported.lock().unwrap().extend(assets);
    }

    /// The imported assets `resolved` now has, taken from the waiting list.
    pub fn take_imported(&self, resolved: impl Fn(&str) -> bool) -> Vec<String> {
        let mut waiting = self.imported.lock().unwrap();
        let (ready, rest): (Vec<_>, Vec<_>) = waiting.drain(..).partition(|id| resolved(id));
        *waiting = rest;
        ready
    }

    /// Wakes the worker (started on first use).
    fn kick(&self, storage: Arc<Storage>) {
        let mut wake = self.wake.lock().unwrap();
        if let Some(tx) = wake.as_ref()
            && tx.send(()).is_ok()
        {
            return;
        }
        let (tx, rx) = mpsc::channel::<()>();
        let _ = tx.send(());
        let worker = Worker {
            profile: self.profile.clone(),
            shared: self.shared.clone(),
            tasks: self.tasks.clone(),
            storage,
            usage: self.usage.clone(),
            events: self.events.clone(),
            interactive: self.interactive.clone(),
        };
        if std::thread::Builder::new()
            .name("pm-pro-media".into())
            .spawn(move || {
                for () in rx {
                    worker.drain();
                }
            })
            .is_ok()
        {
            *wake = Some(tx);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskAction {
    Pause,
    Resume,
    Cancel,
    Priority(i32),
}

struct Worker {
    profile: PathBuf,
    shared: Arc<Mutex<Shared>>,
    tasks: Arc<Mutex<BackgroundTasks>>,
    storage: Arc<Storage>,
    usage: Arc<Mutex<Option<Usage>>>,
    events: async_channel::Sender<Event>,
    interactive: Arc<AtomicBool>,
}

impl Worker {
    fn drain(&self) {
        loop {
            let Some(ticket) = self.tasks.lock().unwrap().start_next() else {
                return;
            };
            let stop = {
                let shared = self.shared.lock().unwrap();
                shared.stop.store(false, Ordering::Release);
                shared.stop.clone()
            };
            let _ = self.events.try_send(Event::Changed);
            let result = self.run(ticket, &stop);
            self.shared.lock().unwrap().waiting = None;
            let mut tasks = self.tasks.lock().unwrap();
            match tasks.worker_state(ticket) {
                Ok(TaskState::PauseRequested | TaskState::CancelRequested) => {
                    let _ = tasks.settle_control(ticket);
                }
                Ok(_) => {
                    let _ = tasks.finish(ticket, result);
                }
                Err(_) => {}
            }
            drop(tasks);
            *self.usage.lock().unwrap() = Some(measure(&self.storage));
            let _ = self.events.try_send(Event::Changed);
        }
    }

    fn run(&self, ticket: Ticket, stop: &AtomicBool) -> Result<(), TaskFailure> {
        let (kind, source, size, format) = {
            let shared = self.shared.lock().unwrap();
            let job = shared
                .jobs
                .get(&ticket.id)
                .ok_or_else(|| failure(TaskFailureCode::Unknown, "task without a job"))?;
            (job.kind, job.source.clone(), job.size, job.format)
        };
        let before = Source::of(&source)
            .ok_or_else(|| failure(TaskFailureCode::SourceOffline, "source offline"))?;
        let (ffmpeg, ffprobe) = ffmpeg()
            .ok_or_else(|| failure(TaskFailureCode::CodecUnsupported, "FFmpeg is not available"))?;
        let hash = hash_file(&source, stop)
            .map_err(|e| failure(TaskFailureCode::SourceOffline, e.to_string()))?;
        let class = match kind {
            Rendition::Proxy => StorageClass::Proxies,
            Rendition::Optimized => StorageClass::Optimized,
        };
        let root = &self.storage.settings().locations[&class].directory;
        let store = pm_proxy::ProxyStore::open(root.join("store"))
            .map_err(|e| failure(TaskFailureCode::Unknown, e.to_string()))?;
        let total = self
            .tasks
            .lock()
            .unwrap()
            .get(ticket.id)
            .map_or(0, |t| t.total);
        let tasks = self.tasks.clone();
        let events = self.events.clone();
        let last = Mutex::new(std::time::Instant::now());
        let on_progress = move |share: f64| {
            let mut at = last.lock().unwrap();
            if at.elapsed().as_millis() < 250 {
                return;
            }
            *at = std::time::Instant::now();
            let mut tasks = tasks.lock().unwrap();
            let done = tasks.get(ticket.id).map_or(0, |t| t.completed);
            let now = ((share * total as f64) as u64).clamp(done, total);
            if tasks.progress(ticket, now, total).is_ok() {
                let _ = events.try_send(Event::Changed);
            }
        };
        let interactive = self.interactive.clone();
        let pause = move || interactive.load(Ordering::Acquire);
        let (shared, events) = (self.shared.clone(), self.events.clone());
        let on_pause = move |reason: Option<pm_proxy::PauseReason>| {
            shared.lock().unwrap().waiting = reason.map(|r| (ticket.id, r));
            let _ = events.try_send(Event::Changed);
        };
        let generated = pm_proxy::generate(pm_proxy::GenerationRequest {
            source: &source,
            source_hash: &hash,
            format,
            size,
            ffmpeg: &ffmpeg,
            ffprobe: &ffprobe,
            store: &store,
            control: pm_proxy::GenerationControl {
                cancel: stop,
                pause: &pause,
                on_pause: &on_pause,
                on_progress: &on_progress,
            },
        })
        .map_err(|e| failure(classify(&e), e.to_string()))?;
        // A file changed while encoding is not this rendition's source.
        if Source::of(&source).as_ref() != Some(&before) {
            return Err(failure(TaskFailureCode::Unknown, "the source changed"));
        }
        let mut shared = self.shared.lock().unwrap();
        let e = entry_mut(&mut shared.index, &before);
        match kind {
            Rendition::Proxy => {
                e.proxy = Some(generated.stored.path);
                e.attached = false;
            }
            Rendition::Optimized => e.optimized = Some(generated.stored.path),
        }
        save(&self.profile, &shared.index);
        drop(shared);
        let _ = self.events.try_send(Event::Renditions);
        Ok(())
    }
}

fn hash_file(path: &Path, stop: &AtomicBool) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if stop.load(Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "stopped",
            ));
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// `{size}` as Finder writes it (decimal units).
pub fn bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.1} GB", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.0} MB", n / 1e6)
    } else if n >= 1e3 {
        format!("{:.0} KB", n / 1e3)
    } else {
        format!("{n} bytes")
    }
}

impl crate::session::Controller {
    /// Settings › Media, as this Mac keeps it.
    pub(crate) fn pro_media_preferences(&self) -> Preferences {
        self.pro_media.preferences()
    }

    /// Settings › Media's rows and storage, for the Settings panel.
    pub(crate) fn media_settings_fact(&self) -> serde_json::Value {
        let prefs = self.pro_media.preferences();
        let rows: Vec<serde_json::Value> = prefs
            .rows()
            .into_iter()
            .map(|(id, label, detail, value, options)| {
                serde_json::json!({"id":id,"label":label,"detail":detail,"value":value,
                    "options":options.into_iter().map(|(v, label)| serde_json::json!({"v":v,"label":label})).collect::<Vec<_>>()})
            })
            .collect();
        let usage = self.pro_media.cached_usage();
        let location = usage
            .as_ref()
            .map(|u| u.location.clone())
            .unwrap_or_else(|| {
                prefs
                    .location
                    .clone()
                    .unwrap_or_else(|| self.profile.join("Media Storage"))
            });
        serde_json::json!({
            "rows": rows,
            "location": location.to_string_lossy(),
            "used": usage.map(|u| serde_json::json!({
                "proxies": bytes(u.proxies),
                "optimized": bytes(u.optimized),
                "renders": bytes(u.renders),
            })),
        })
    }

    /// A Settings › Media write or action (`media.settings`).
    pub(crate) fn media_settings(
        &mut self,
        input: &serde_json::Value,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(id) = input["write"].as_str() {
            if let Err(error) = self.pro_media.set_preference(id, &input["value"]) {
                self.message = error;
            }
            self.update_builtins(cx);
            return;
        }
        let window = self.host.borrow().window.clone();
        match input["action"].as_str() {
            Some("media.location") => {
                let rx = crate::platform::choose_paths(&window, true, false, &[], "Use Folder");
                let pro_media = self.pro_media.clone();
                cx.spawn(async move |this, cx| {
                    let Ok(folders) = rx.recv().await else { return };
                    let Some(folder) = folders.into_iter().next() else {
                        return;
                    };
                    let result = pro_media.set_location(folder);
                    let _ = this.update(cx, |v, cx| {
                        if let Err(error) = result {
                            v.message = error;
                        }
                        v.update_builtins(cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            Some("media.reveal") => {
                let prefs = self.pro_media.preferences();
                if let Ok(storage) = self.pro_media.storage(&prefs)
                    && let Some(root) = storage.settings().locations[&StorageClass::Proxies]
                        .directory
                        .parent()
                {
                    crate::platform::reveal(root);
                }
            }
            Some("media.clear") => {
                let rx = crate::platform::choice(
                    &window,
                    "Clear generated media?",
                    "Proxies, optimized media and renders the open project doesn’t use are removed. Originals, attached proxies and this project’s media stay. Powermove can make them again.",
                    &["Clear", "Cancel"],
                );
                let pro_media = self.pro_media.clone();
                let prefs = self.pro_media.preferences();
                let pinned: BTreeSet<PathBuf> =
                    self.media_base.originals.values().cloned().collect();
                cx.spawn(async move |this, cx| {
                    if rx.recv().await != Ok(0) {
                        return;
                    }
                    let freed = cx
                        .background_executor()
                        .spawn(async move { pro_media.clear(&prefs, &pinned) })
                        .await;
                    let _ = this.update(cx, |v, cx| {
                        v.message = format!("Freed {}", bytes(freed));
                        v.update_builtins(cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            _ => {}
        }
    }

    /// The asset's name, for its task's title.
    fn asset_title(&self, id: &str) -> String {
        self.published
            .as_ref()
            .and_then(|p| p.snapshot.root())
            .and_then(|r| r.get("assets"))
            .and_then(|a| a.get(id))
            .and_then(|a| a.get("name"))
            .and_then(crate::node_string)
            .unwrap_or(id)
            .to_owned()
    }

    /// A Project panel rendition command for `assets` (selected videos).
    pub(crate) fn renditions(
        &mut self,
        action: pm_panel_media::RenditionAction,
        assets: Vec<String>,
        cx: &mut gpui::Context<Self>,
    ) {
        use pm_panel_media::RenditionAction as A;
        let Some(project) = self.session.as_ref().map(|s| s.project.clone()) else {
            return;
        };
        let prefs = self.pro_media_preferences();
        let online: Vec<(String, String, PathBuf)> = assets
            .iter()
            .filter_map(|id| {
                let path = self.media_base.originals.get(id)?;
                Some((id.clone(), self.asset_title(id), path.clone()))
            })
            .collect();
        let paths: Vec<PathBuf> = online.iter().map(|(_, _, p)| p.clone()).collect();
        let result = match action {
            A::CreateProxy => self
                .pro_media
                .create(&project, Rendition::Proxy, &prefs, online),
            A::RebuildProxy => {
                self.pro_media.delete(Rendition::Proxy, &paths);
                self.pro_media
                    .create(&project, Rendition::Proxy, &prefs, online)
            }
            A::DeleteProxy => Ok(self.pro_media.delete(Rendition::Proxy, &paths)),
            A::CreateOptimized => {
                self.pro_media
                    .create(&project, Rendition::Optimized, &prefs, online)
            }
            A::DeleteOptimized => Ok(self.pro_media.delete(Rendition::Optimized, &paths)),
            A::PlayOriginal(on) => {
                self.pro_media.set_play_original(&project, &assets, on);
                Ok(assets.len())
            }
            A::AttachProxy => {
                let Some(original) = paths.first().cloned() else {
                    return;
                };
                let rx = crate::platform::choose_paths(
                    &self.host.borrow().window,
                    false,
                    false,
                    &["mov", "mp4", "m4v", "mxf"],
                    "Attach",
                );
                let pro_media = self.pro_media.clone();
                cx.spawn(async move |this, cx| {
                    let Ok(files) = rx.recv().await else { return };
                    let Some(file) = files.into_iter().next() else {
                        return;
                    };
                    let result = cx
                        .background_executor()
                        .spawn(async move { pro_media.attach(&original, &file) })
                        .await;
                    if let Err(error) = result {
                        let _ = this.update(cx, |v, cx| {
                            v.message = error;
                            cx.notify();
                        });
                    }
                })
                .detach();
                return;
            }
        };
        if let Err(error) = result {
            self.message = error;
        }
        cx.notify();
    }

    /// Create Proxies on import (Settings › Media): the new videos whose
    /// facts call for one.
    pub(crate) fn proxies_on_import(&mut self, assets: &[String]) {
        let prefs = self.pro_media_preferences();
        if prefs.create == Create::OnlyWhenAsked {
            return;
        }
        let Some(project) = self.session.as_ref().map(|s| s.project.clone()) else {
            return;
        };
        let wanted: Vec<(String, String, PathBuf)> = assets
            .iter()
            .filter_map(|id| {
                let path = self.media_base.originals.get(id)?;
                let video = pm_media::probe::probe(path).ok()?.probe.video?;
                prefs
                    .creates_on_import(video.width, video.height, video.codec.bit_depth)
                    .then(|| (id.clone(), self.asset_title(id), path.clone()))
            })
            .collect();
        if !wanted.is_empty()
            && let Err(error) = self
                .pro_media
                .create(&project, Rendition::Proxy, &prefs, wanted)
        {
            self.message = error;
        }
    }
}

mod erased {
    /// A value as its settings wire form.
    pub trait Wire {
        fn wire(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Wire for T {
        fn wire(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> (tempfile::TempDir, ProMedia) {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let media = ProMedia::new(profile, Arc::new(AtomicBool::new(false)));
        (dir, media)
    }

    /// The index gives each original its renditions, Always Play Original
    /// is per project, an edited source loses them, and Delete removes
    /// generated files but never an attached one.
    #[test]
    fn renditions_follow_the_source_file_and_delete_keeps_attached_files() {
        let (dir, media) = service();
        let original = dir.path().join("clip.mov");
        let (proxy, optimized, mine) = (
            dir.path().join("proxy.mov"),
            dir.path().join("optimized.mov"),
            dir.path().join("camera-proxy.mov"),
        );
        for f in [&original, &proxy, &optimized, &mine] {
            std::fs::write(f, b"bytes").unwrap();
        }
        {
            let mut shared = media.shared.lock().unwrap();
            let e = entry_mut(&mut shared.index, &Source::of(&original).unwrap());
            e.proxy = Some(proxy.clone());
            e.optimized = Some(optimized.clone());
        }
        let mut catalog = MediaCatalog::default();
        catalog.originals.insert("a".into(), original.clone());
        media.decorate("p", &mut catalog);
        assert_eq!(catalog.proxies.get("a"), Some(&proxy));
        assert_eq!(catalog.optimized.get("a"), Some(&optimized));
        assert!(catalog.play_original.is_empty());
        media.set_play_original("p", &["a".into()], true);
        assert!(media.plays_original("p", "a") && !media.plays_original("q", "a"));
        let mut again = MediaCatalog::default();
        again.originals.insert("a".into(), original.clone());
        media.decorate("p", &mut again);
        assert!(again.play_original.contains("a"));
        // Reopened from disk, the index and the choice are still there.
        let reopened = ProMedia::new(media.profile.clone(), Arc::new(AtomicBool::new(false)));
        assert!(reopened.plays_original("p", "a"));

        assert_eq!(
            media.delete(Rendition::Proxy, std::slice::from_ref(&original)),
            1
        );
        assert!(!proxy.exists() && optimized.exists());
        media.attach_unchecked(&original, &mine);
        media.delete(Rendition::Proxy, std::slice::from_ref(&original));
        assert!(mine.exists(), "an attached proxy is the user's file");

        // An edited original is another source: its old renditions don't apply.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&original, b"edited bytes").unwrap();
        let mut edited = MediaCatalog::default();
        edited.originals.insert("a".into(), original);
        media.decorate("p", &mut edited);
        assert!(edited.optimized.is_empty());
    }

    /// A Create Proxy Media task encodes on the worker, reports frames,
    /// completes, and the proxy reaches the catalog and the usage.
    #[test]
    fn a_created_proxy_completes_its_task_and_reaches_the_catalog() {
        let Some((ffmpeg, _)) = ffmpeg() else {
            eprintln!("NOT RUN: no FFmpeg pair (PM_FFMPEG_DIR/bin)");
            return;
        };
        // This Mac's free space is the production guard's business, not this
        // test's: reserve 1 GiB here.
        pm_proxy::set_test_disk_reserve(Some(1 << 30));
        let (dir, media) = service();
        let source = dir.path().join("clip.mov");
        let made = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg("testsrc2=size=320x180:rate=24:duration=1")
            .args([
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "hq",
                "-pix_fmt",
                "p216le",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let queued = media
            .create(
                "p",
                Rendition::Proxy,
                &Preferences::default(),
                vec![("a".into(), "Clip".into(), source.clone())],
            )
            .unwrap();
        assert_eq!(queued, 1);
        let started = std::time::Instant::now();
        let task = loop {
            let tasks = media.snapshot();
            if let Some(t) = tasks.iter().find(|t| t.state.terminal()) {
                break t.clone();
            }
            assert!(started.elapsed().as_secs() < 120, "the task finishes");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert_eq!(
            task.state,
            TaskState::Completed,
            "{:?}",
            task.diagnostic_for_log()
        );
        assert_eq!(
            (task.total, task.completed),
            (24, 24),
            "progress counts frames"
        );
        let mut catalog = MediaCatalog::default();
        catalog.originals.insert("a".into(), source.clone());
        media.decorate("p", &mut catalog);
        let proxy = catalog
            .proxies
            .get("a")
            .expect("the proxy is in the catalog");
        assert!(proxy.is_file());
        assert_eq!(media.status("p", "a", Some(&source)).0, Status::Ready);
        let started = std::time::Instant::now();
        while media.cached_usage().is_none_or(|u| u.proxies == 0) {
            assert!(started.elapsed().as_secs() < 10, "usage is measured");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn preferences_accept_only_their_choices_and_persist() {
        let (_dir, media) = service();
        assert_eq!(media.preferences(), Preferences::default());
        media
            .set_preference("media.proxySize", &serde_json::json!("p720"))
            .unwrap();
        media
            .set_preference("media.proxyCreate", &serde_json::json!("whenNeeded"))
            .unwrap();
        assert!(
            media
                .set_preference("media.proxyFormat", &serde_json::json!("proResHq"))
                .is_err()
        );
        assert!(
            media
                .set_preference("media.proxySize", &serde_json::json!("huge"))
                .is_err()
        );
        let reopened = ProMedia::new(media.profile.clone(), Arc::new(AtomicBool::new(false)));
        let prefs = reopened.preferences();
        assert_eq!(
            (prefs.size, prefs.create),
            (ProxySize::P720, Create::WhenNeeded)
        );
        // Every row offers its current value among its choices.
        for (_, _, _, value, options) in prefs.rows() {
            assert!(options.iter().any(|(v, _)| *v == value));
        }
        assert!(prefs.creates_on_import(3840, 2160, 8));
        assert!(!prefs.creates_on_import(1920, 1080, 10));
    }
}
