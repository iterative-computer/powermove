//! Pro media control (plan section 1): proxy and optimized media made on a
//! worker into this Mac's media storage, their Background Tasks, and what
//! the viewer and the Project panel learn from them.
//!
//! - Generation is `pm_proxy::generate`: validated, content-addressed
//!   transcodes (proxies at Settings › Media's size and format; optimized
//!   media as full-size ProRes 422 HQ, 4444 for alpha or deep sources).
//! - Renditions are keyed by content identity (a media-cache object's name,
//!   or the BLAKE3 of a linked file, hashed once per file state), so
//!   byte-identical originals share them and an offline original keeps its
//!   proxy. A stored file is deleted only when no record names it; attached
//!   proxies are the user's own files: never deleted, only forgotten.
//! - Always Play Original is per project and asset.
//! - Exports read originals unless the user picks "Use Proxies (Draft)".
//! - The main thread reads memory only. File work runs on one file thread,
//!   encodes on one worker (N8); pausing a task stops its encoder and it
//!   starts again on Resume.
use pm_engine_service::MediaCatalog;
use pm_promedia::storage::{Location, Storage, StorageClass, StorageSettings};
use pm_promedia::tasks::{
    BackgroundTasks, ProgressUnit, Task, TaskFailure, TaskFailureCode, TaskId, TaskKind, TaskSpec,
    TaskState, Ticket,
};
use pm_protocol::priority::{ProxyFormat, ProxySize};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    /// Settings › Media › Render Files Limit, in GB: rendered frames past it
    /// go oldest first (kept frames never).
    pub render_limit_gb: u32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            create: Create::OnlyWhenAsked,
            size: ProxySize::Automatic,
            format: ProxyFormat::ProResProxy,
            location: None,
            render_limit_gb: 25,
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

/// Settings › Media › Render Files Limit's choices, in GB.
const RENDER_LIMITS: [(u32, &str); 4] =
    [(10, "10 GB"), (25, "25 GB"), (50, "50 GB"), (100, "100 GB")];

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
            (
                "media.renderLimit",
                "Render Files Limit",
                "Rendered frames past this go oldest first; kept frames stay.",
                serde_json::json!(self.render_limit_gb),
                RENDER_LIMITS
                    .iter()
                    .map(|(gb, label)| (serde_json::json!(gb), *label))
                    .collect(),
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
            "media.renderLimit" => {
                let gb: u32 = serde_json::from_value(value.clone()).map_err(bad)?;
                if !RENDER_LIMITS.iter().any(|(choice, _)| *choice == gb) {
                    return Err("Invalid setting value".into());
                }
                self.render_limit_gb = gb;
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

/// A rendition file: generated into the store, or the user's own (attached).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    path: PathBuf,
    /// The user's own file (Attach Proxy Media…): never deleted, only
    /// forgotten.
    #[serde(default)]
    attached: bool,
}

/// What one content has.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Renditions {
    proxy: Option<File>,
    optimized: Option<File>,
}

impl Renditions {
    fn slot(&mut self, kind: Rendition) -> &mut Option<File> {
        match kind {
            Rendition::Proxy => &mut self.proxy,
            Rendition::Optimized => &mut self.optimized,
        }
    }
    fn get(&self, kind: Rendition) -> Option<&File> {
        match kind {
            Rendition::Proxy => self.proxy.as_ref(),
            Rendition::Optimized => self.optimized.as_ref(),
        }
    }
}

/// A linked file at one state.
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

const INDEX_VERSION: u32 = 2;

/// Renditions by content identity: byte-identical originals share them,
/// a moved or touched file keeps them, and an asset whose original is
/// offline still finds its proxy.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Index {
    version: u32,
    /// Linked files already hashed, at the state they were hashed.
    stamps: Vec<(Source, String)>,
    /// Content (`b3:` the BLAKE3 of the bytes, or `obj:` the name of a
    /// media-cache object, which names its bytes) to its renditions.
    contents: BTreeMap<String, Renditions>,
    /// Project → asset → content, as last identified.
    assets: BTreeMap<String, BTreeMap<String, String>>,
    /// Always Play Original: project → assets.
    play_original: BTreeMap<String, BTreeSet<String>>,
}

impl Index {
    /// Records naming `path` as a proxy or optimized file.
    fn references(&self, path: &Path) -> usize {
        self.contents
            .values()
            .flat_map(|r| [r.proxy.as_ref(), r.optimized.as_ref()])
            .flatten()
            .filter(|f| f.path == path)
            .count()
    }

    /// Deletes a file that left the index: only a generated one, and only
    /// when no record still names it. Returns the bytes freed.
    fn retire_file(&self, file: &File) -> u64 {
        if file.attached || self.references(&file.path) > 0 {
            return 0;
        }
        let size = std::fs::metadata(&file.path).map_or(0, |m| m.len());
        if std::fs::remove_file(&file.path).is_err() {
            return 0;
        }
        // The store's record beside its media.
        let _ = std::fs::remove_file(file.path.with_extension("json"));
        size
    }

    /// Forgets `kind` of `content` and retires its file.
    fn take(&mut self, content: &str, kind: Rendition) -> u64 {
        let Some(old) = self
            .contents
            .get_mut(content)
            .and_then(|r| r.slot(kind).take())
        else {
            return 0;
        };
        self.contents
            .retain(|_, r| r.proxy.is_some() || r.optimized.is_some());
        self.retire_file(&old)
    }

    /// Sets `kind` of `content` to `file`, retiring what it replaces.
    fn put(&mut self, content: &str, kind: Rendition, file: File) {
        let old = self
            .contents
            .entry(content.to_owned())
            .or_default()
            .slot(kind)
            .replace(file.clone());
        if let Some(old) = old
            && old.path != file.path
        {
            self.retire_file(&old);
        }
    }
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

/// What the main thread hears from the workers.
#[derive(Debug)]
pub enum Event {
    /// Tasks moved; the Background Tasks window redraws.
    Changed,
    /// Renditions or identities changed: the catalog changes.
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

struct State {
    index: Index,
    /// Original path → content, as the last identification found it. The
    /// main thread reads this, never the disk.
    known: HashMap<PathBuf, String>,
    /// Rendition files found on disk at the last identification.
    present: HashSet<PathBuf>,
    jobs: BTreeMap<TaskId, Job>,
    /// Why the running encode waits (low disk, playback or an export).
    waiting: Option<(TaskId, pm_proxy::PauseReason)>,
}

type IoJob = Box<dyn FnOnce(&Arc<Inner>) + Send>;
/// An online original: asset id, title (for its task) and path.
pub(crate) type Original = (String, String, PathBuf);

struct Inner {
    profile: PathBuf,
    /// The media cache's content-addressed objects (named by their bytes).
    objects: PathBuf,
    state: Mutex<State>,
    tasks: Mutex<BackgroundTasks>,
    /// Pause or cancel the running encode.
    stop: AtomicBool,
    storage: Mutex<Option<(PathBuf, Arc<Storage>)>>,
    preferences: Mutex<Preferences>,
    /// Space used, measured off the main thread after each change.
    usage: Mutex<Option<Usage>>,
    events: async_channel::Sender<Event>,
    /// Interactive work (playback, an export) pauses generation.
    interactive: Arc<AtomicBool>,
    /// Imported assets the catalog has not resolved yet.
    imported: Mutex<Vec<String>>,
    /// File work, one job at a time, off the main thread.
    io: Mutex<Option<mpsc::Sender<IoJob>>>,
    /// The generation worker.
    wake: Mutex<Option<mpsc::Sender<()>>>,
}

pub struct ProMedia {
    inner: Arc<Inner>,
    pub event_receiver: async_channel::Receiver<Event>,
}

/// The window id media tasks are filed under (one editor window).
pub const WINDOW: &str = "main";

fn index_path(profile: &Path) -> PathBuf {
    profile.join("Pro Media").join("index.json")
}

fn settings_path(profile: &Path) -> PathBuf {
    profile.join("Pro Media").join("settings.json")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a folder"))?;
    let tmp = path.with_extension("json.partial");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
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
        .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
        .filter(|i| i.version == INDEX_VERSION)
        .unwrap_or(Index {
            version: INDEX_VERSION,
            ..Default::default()
        })
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
        E::Io(e) if e.raw_os_error() == Some(28) => TaskFailureCode::DiskFull,
        E::InvalidMedia(_) => TaskFailureCode::CodecUnsupported,
        _ => TaskFailureCode::Unknown,
    }
}

/// Whether `file` can stand in for `original` as its proxy: the same
/// frames at the same rate, start and timecode, display aspect, alpha,
/// audio layout and colour interpretation (a proxy plays its own audio
/// and shows its own colour).
fn attach_check(original: &Path, file: &Path) -> Result<(), String> {
    let a = pm_media::probe::probe(original).map_err(|_| "The original can't be read.")?;
    let b =
        pm_media::probe::probe(file).map_err(|_| "This file isn't a video Powermove can read.")?;
    let (Some(va), Some(vb)) = (a.probe.video.as_ref(), b.probe.video.as_ref()) else {
        return Err("This file isn't a video Powermove can read.".into());
    };
    let refuse = |why: &str| Err(format!("This file doesn’t match the original. {why}"));
    let aspect = |v: &pm_media::probe::VideoProbe| {
        let (n, d) = v.pixel_aspect;
        v.width as f64 * n.max(1) as f64 / (v.height.max(1) as f64 * d.max(1) as f64)
    };
    if va.frames != vb.frames {
        return refuse("Check its length.");
    }
    if let (Some(ra), Some(rb)) = (&va.nominal_rate, &vb.nominal_rate)
        && ra.rate != rb.rate
    {
        return refuse("Check its frame rate.");
    }
    // Half a frame, from the original's frames over its duration.
    let half_frame_us = a
        .probe
        .duration_us
        .map_or(20_000, |d| d / (2 * va.frames.max(1) as i64));
    let near = |x: Option<i64>, y: Option<i64>| match (x, y) {
        (Some(x), Some(y)) => (x - y).abs() <= half_frame_us,
        (None, None) => true,
        _ => false,
    };
    if !near(a.probe.duration_us, b.probe.duration_us) {
        return refuse("Check its length.");
    }
    if !near(
        Some(a.probe.start_us.unwrap_or(0)),
        Some(b.probe.start_us.unwrap_or(0)),
    ) {
        return refuse("Check its start.");
    }
    if let (Some(ta), Some(tb)) = (&a.probe.timecode, &b.probe.timecode)
        && ta.text != tb.text
    {
        return refuse("Its timecode is different.");
    }
    if (aspect(va) - aspect(vb)).abs() > 0.01 * aspect(va) {
        return refuse("Check its shape.");
    }
    if va.alpha.is_some() && vb.alpha.is_none() {
        return refuse("The original has transparency; this file doesn’t.");
    }
    let audio = |p: &pm_media::probe::Probed| {
        p.probe
            .audio_stream
            .and_then(|i| p.probe.streams.iter().find(|s| s.index == i))
            .and_then(|s| s.audio.as_ref())
            .map(|au| (au.sample_rate, au.channels))
    };
    if audio(&a) != audio(&b) {
        return refuse("Its audio is different.");
    }
    let (ca, cb) = (&va.color.interpreted, &vb.color.interpreted);
    if ca.primaries != cb.primaries || ca.transfer != cb.transfer {
        return refuse("Its colour is different.");
    }
    Ok(())
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

impl Inner {
    /// Runs `job` on the file thread (started on first use), in order.
    fn io(self: &Arc<Self>, job: impl FnOnce(&Arc<Inner>) + Send + 'static) {
        let mut io = self.io.lock().unwrap();
        let mut job: IoJob = Box::new(job);
        if let Some(tx) = io.as_ref() {
            match tx.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(back)) => job = back,
            }
        }
        let (tx, rx) = mpsc::channel::<IoJob>();
        let _ = tx.send(job);
        let inner = Arc::downgrade(self);
        if std::thread::Builder::new()
            .name("pm-pro-media-io".into())
            .spawn(move || {
                for job in rx {
                    let Some(inner) = inner.upgrade() else { break };
                    job(&inner);
                }
            })
            .is_ok()
        {
            *io = Some(tx);
        }
    }

    fn preferences(&self) -> Preferences {
        self.preferences.lock().unwrap().clone()
    }

    /// The media storage at `prefs`' location (default: the profile's
    /// `Media Storage`), opened once per location. A folder Powermove does
    /// not own and that has other files in it is refused.
    fn storage(&self, prefs: &Preferences) -> Result<Arc<Storage>, String> {
        let root = storage_root(&self.profile, prefs);
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

    fn measure_usage(&self) {
        if let Ok(storage) = self.storage(&self.preferences()) {
            *self.usage.lock().unwrap() = Some(measure(&storage));
            let _ = self.events.try_send(Event::Changed);
        }
    }

    /// Writes the index (call on a worker).
    fn save(&self) {
        let bytes = serde_json::to_vec_pretty(&self.state.lock().unwrap().index);
        if let Ok(bytes) = bytes {
            let _ = write_atomic(&index_path(&self.profile), &bytes);
        }
    }

    /// The content of the original at `path`: a media-cache object's name,
    /// or the BLAKE3 of a linked file (hashed once per file state). Call on
    /// a worker: it may read the whole file.
    fn content_of(&self, path: &Path, stop: &AtomicBool) -> std::io::Result<String> {
        if path.parent() == Some(self.objects.as_path())
            && let Some(name) = path.file_name()
        {
            std::fs::metadata(path)?;
            return Ok(format!("obj:{}", name.to_string_lossy()));
        }
        let source = Source::of(path)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "offline"))?;
        if let Some((_, content)) = self
            .state
            .lock()
            .unwrap()
            .index
            .stamps
            .iter()
            .find(|(s, _)| *s == source)
        {
            return Ok(content.clone());
        }
        let content = format!("b3:{}", hash_file(path, stop)?);
        let mut state = self.state.lock().unwrap();
        state.index.stamps.retain(|(s, _)| s.path != source.path);
        state.index.stamps.push((source, content.clone()));
        Ok(content)
    }

    /// Notes `asset`'s content for `project` and its original's path.
    fn note(&self, project: &str, asset: &str, path: &Path, content: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        let fresh =
            state.known.insert(path.to_path_buf(), content.to_owned()) != Some(content.to_owned());
        let assets = state.index.assets.entry(project.to_owned()).or_default();
        fresh | (assets.insert(asset.to_owned(), content.to_owned()) != Some(content.to_owned()))
    }

    /// Looks for every rendition file on disk (call on a worker).
    fn refresh_present(&self) -> bool {
        let files: Vec<PathBuf> = {
            let state = self.state.lock().unwrap();
            state
                .index
                .contents
                .values()
                .flat_map(|r| [r.proxy.as_ref(), r.optimized.as_ref()])
                .flatten()
                .map(|f| f.path.clone())
                .collect()
        };
        let present: HashSet<PathBuf> = files.into_iter().filter(|p| p.is_file()).collect();
        let mut state = self.state.lock().unwrap();
        let changed = state.present != present;
        state.present = present;
        changed
    }

    /// The content `asset` of `project` has: its original's, or (original
    /// offline) the last one identified.
    fn content_for(
        state: &State,
        project: &str,
        asset: &str,
        original: Option<&Path>,
    ) -> Option<String> {
        match original {
            Some(path) => state.known.get(path).cloned(),
            None => state
                .index
                .assets
                .get(project)
                .and_then(|a| a.get(asset))
                .cloned(),
        }
    }

    /// Queues one task per asset: (asset id, title, original path). An
    /// asset already queued or creating that rendition is skipped. No file
    /// work: the worker reads the source.
    fn create(
        self: &Arc<Self>,
        project: &str,
        kind: Rendition,
        prefs: &Preferences,
        assets: Vec<Original>,
    ) -> Result<usize, String> {
        let mut queued = 0;
        {
            let mut tasks = self.tasks.lock().unwrap();
            let mut state = self.state.lock().unwrap();
            for (asset, title, source) in assets {
                let busy = state.jobs.iter().any(|(id, j)| {
                    j.project == project
                        && j.asset == asset
                        && j.kind == kind
                        && tasks.get(*id).is_some_and(|t| !t.state.terminal())
                });
                if busy {
                    continue;
                }
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
                            total: 0,
                        },
                    )
                    .map_err(|e| format!("The task could not be queued: {e:?}"))?;
                let (size, format) = match kind {
                    Rendition::Proxy => (prefs.size, prefs.format),
                    Rendition::Optimized => (ProxySize::Full, ProxyFormat::ProResHq),
                };
                state.jobs.insert(
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
            self.kick();
            let _ = self.events.try_send(Event::Changed);
        }
        Ok(queued)
    }

    /// Forgets `kind` for `assets` (id, original path or `None` offline)
    /// and retires generated files nothing else names. A proxy of an
    /// offline original stays: it is the only playable copy.
    fn delete(
        &self,
        project: &str,
        kind: Rendition,
        assets: &[(String, Option<PathBuf>)],
    ) -> usize {
        let mut removed = 0;
        let mut state = self.state.lock().unwrap();
        for (asset, original) in assets {
            if original.is_none() && kind == Rendition::Proxy {
                continue;
            }
            let Some(content) = Self::content_for(&state, project, asset, original.as_deref())
            else {
                continue;
            };
            state.index.take(&content, kind);
            removed += 1;
        }
        drop(state);
        removed
    }

    /// Attaches `file` as the proxy of `asset`'s original (checked first).
    fn attach(
        &self,
        project: &str,
        asset: &str,
        original: &Path,
        file: &Path,
    ) -> Result<(), String> {
        attach_check(original, file)?;
        let content = self
            .content_of(original, &AtomicBool::new(false))
            .map_err(|_| "The original can't be read.".to_owned())?;
        self.note(project, asset, original, &content);
        let mut state = self.state.lock().unwrap();
        state.index.put(
            &content,
            Rendition::Proxy,
            File {
                path: file.to_path_buf(),
                attached: true,
            },
        );
        state.present.insert(file.to_path_buf());
        Ok(())
    }

    /// Removes generated renditions the open project (`pinned` contents)
    /// does not use, and unkept renders. Never removes an attached file, a
    /// file another record names, media a task is making, or a proxy whose
    /// original is nowhere on disk (the only playable copy). Returns the
    /// bytes freed.
    fn clear(&self, pinned: &BTreeSet<String>) -> u64 {
        let mut freed = 0;
        let mut state = self.state.lock().unwrap();
        let busy: BTreeSet<PathBuf> = state.jobs.values().map(|j| j.source.clone()).collect();
        let busy_contents: BTreeSet<String> = state
            .known
            .iter()
            .filter(|(p, _)| busy.contains(*p))
            .map(|(_, c)| c.clone())
            .collect();
        // Where each content's bytes can still be read.
        let mut online: BTreeSet<String> = BTreeSet::new();
        for (source, content) in &state.index.stamps {
            if source.path.is_file() {
                online.insert(content.clone());
            }
        }
        for content in state.index.contents.keys() {
            if let Some(name) = content.strip_prefix("obj:")
                && self.objects.join(name).is_file()
            {
                online.insert(content.clone());
            }
        }
        let contents: Vec<String> = state.index.contents.keys().cloned().collect();
        for content in contents {
            if pinned.contains(&content) || busy_contents.contains(&content) {
                continue;
            }
            for kind in [Rendition::Proxy, Rendition::Optimized] {
                // An attached record is the person's: never forgotten here.
                let generated = state.index.contents[&content]
                    .get(kind)
                    .is_some_and(|f| !f.attached);
                // A proxy of bytes nowhere on disk is the only playable copy.
                if generated && (kind == Rendition::Optimized || online.contains(&content)) {
                    freed += state.index.take(&content, kind);
                }
                if !state.index.contents.contains_key(&content) {
                    break;
                }
            }
        }
        drop(state);
        if let Ok(storage) = self.storage(&self.preferences())
            && let Ok(cache) = storage.render_cache()
        {
            freed += cache.clear_unkept().unwrap_or(0);
        }
        freed
    }

    /// Wakes the generation worker (started on first use).
    fn kick(self: &Arc<Self>) {
        let mut wake = self.wake.lock().unwrap();
        if let Some(tx) = wake.as_ref()
            && tx.send(()).is_ok()
        {
            return;
        }
        let (tx, rx) = mpsc::channel::<()>();
        let _ = tx.send(());
        let inner = Arc::downgrade(self);
        if std::thread::Builder::new()
            .name("pm-pro-media".into())
            .spawn(move || {
                for () in rx {
                    let Some(inner) = inner.upgrade() else { break };
                    inner.drain();
                }
            })
            .is_ok()
        {
            *wake = Some(tx);
        }
    }

    fn drain(&self) {
        loop {
            let Some(ticket) = self.tasks.lock().unwrap().start_next() else {
                return;
            };
            self.stop.store(false, Ordering::Release);
            let _ = self.events.try_send(Event::Changed);
            let result = self.run(ticket);
            self.state.lock().unwrap().waiting = None;
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
            self.measure_usage();
        }
    }

    fn run(&self, ticket: Ticket) -> Result<(), TaskFailure> {
        let (kind, project, asset, source, size, format) = {
            let state = self.state.lock().unwrap();
            let job = state
                .jobs
                .get(&ticket.id)
                .ok_or_else(|| failure(TaskFailureCode::Unknown, "task without a job"))?;
            (
                job.kind,
                job.project.clone(),
                job.asset.clone(),
                job.source.clone(),
                job.size,
                job.format,
            )
        };
        let before_source = Source::of(&source)
            .ok_or_else(|| failure(TaskFailureCode::SourceOffline, "source offline"))?;
        if let Ok(probed) = pm_media::probe::probe(&source)
            && let Some(frames) = probed.probe.video.map(|v| v.frames as u64)
        {
            let _ = self.tasks.lock().unwrap().progress(ticket, 0, frames);
        }
        let (ffmpeg, ffprobe) = ffmpeg()
            .ok_or_else(|| failure(TaskFailureCode::CodecUnsupported, "FFmpeg is not available"))?;
        let content = self
            .content_of(&source, &self.stop)
            .map_err(|e| failure(TaskFailureCode::SourceOffline, e.to_string()))?;
        let hash = match content.strip_prefix("b3:") {
            Some(hash) => hash.to_owned(),
            None => hash_file(&source, &self.stop)
                .map_err(|e| failure(TaskFailureCode::SourceOffline, e.to_string()))?,
        };
        if self.note(&project, &asset, &source, &content) {
            let _ = self.events.try_send(Event::Renditions);
        }
        // What the content had when this began: anything else at the end
        // (an attachment, a delete) was the person's, and wins.
        let before = self
            .state
            .lock()
            .unwrap()
            .index
            .contents
            .get(&content)
            .and_then(|r| r.get(kind))
            .cloned();
        // The location as it is now, not when the worker started.
        let storage = self
            .storage(&self.preferences())
            .map_err(|e| failure(TaskFailureCode::Unknown, e))?;
        let class = match kind {
            Rendition::Proxy => StorageClass::Proxies,
            Rendition::Optimized => StorageClass::Optimized,
        };
        let root = storage.settings().locations[&class].directory.join("store");
        let store = pm_proxy::ProxyStore::open(root)
            .map_err(|e| failure(TaskFailureCode::Unknown, e.to_string()))?;
        let last = Mutex::new(std::time::Instant::now());
        let on_progress = |share: f64| {
            let mut at = last.lock().unwrap();
            if at.elapsed().as_millis() < 250 {
                return;
            }
            *at = std::time::Instant::now();
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get(ticket.id) else {
                return;
            };
            let (done, total) = (task.completed, task.total);
            let now = ((share * total as f64) as u64).clamp(done, total);
            if total > 0 && tasks.progress(ticket, now, total).is_ok() {
                let _ = self.events.try_send(Event::Changed);
            }
        };
        let pause = || self.interactive.load(Ordering::Acquire);
        let on_pause = |reason: Option<pm_proxy::PauseReason>| {
            self.state.lock().unwrap().waiting = reason.map(|r| (ticket.id, r));
            let _ = self.events.try_send(Event::Changed);
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
                cancel: &self.stop,
                pause: &pause,
                on_pause: &on_pause,
                on_progress: &on_progress,
            },
        })
        .map_err(|e| failure(classify(&e), e.to_string()))?;
        // A file changed while encoding is not this rendition's source.
        if Source::of(&source).as_ref() != Some(&before_source) {
            return Err(failure(TaskFailureCode::Unknown, "the source changed"));
        }
        let made = File {
            path: generated.stored.path,
            attached: false,
        };
        self.publish(&content, kind, before.as_ref(), made);
        self.save();
        let _ = self.events.try_send(Event::Renditions);
        Ok(())
    }
}

impl Inner {
    /// Puts a finished generation in the index, unless the content's
    /// rendition changed while it encoded (an attachment, a delete): the
    /// person's choice wins and the made file is retired. Returns whether
    /// it was published.
    fn publish(&self, content: &str, kind: Rendition, before: Option<&File>, made: File) -> bool {
        let mut state = self.state.lock().unwrap();
        let now = state
            .index
            .contents
            .get(content)
            .and_then(|r| r.get(kind))
            .cloned();
        if now.as_ref() == before {
            state.index.put(content, kind, made.clone());
            state.present.insert(made.path);
            true
        } else {
            state.index.retire_file(&made);
            false
        }
    }
}

/// Where the media storage lives for `prefs` (no file work).
fn storage_root(profile: &Path, prefs: &Preferences) -> PathBuf {
    prefs
        .location
        .clone()
        .unwrap_or_else(|| profile.join("Media Storage"))
}

impl ProMedia {
    pub fn new(profile: PathBuf, objects: PathBuf, interactive: Arc<AtomicBool>) -> Self {
        let (events, event_receiver) = async_channel::unbounded();
        let preferences = std::fs::read(settings_path(&profile))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    index: load(&profile),
                    known: HashMap::new(),
                    present: HashSet::new(),
                    jobs: BTreeMap::new(),
                    waiting: None,
                }),
                profile,
                objects,
                tasks: Mutex::new(BackgroundTasks::default()),
                stop: AtomicBool::new(false),
                storage: Mutex::new(None),
                preferences: Mutex::new(preferences),
                usage: Mutex::new(None),
                events,
                interactive,
                imported: Mutex::new(Vec::new()),
                io: Mutex::new(None),
                wake: Mutex::new(None),
            }),
            event_receiver,
        }
    }

    /// Runs `job` on the file thread and hands its answer back.
    fn io_answer<T: Send + 'static>(
        &self,
        job: impl FnOnce(&Arc<Inner>) -> T + Send + 'static,
    ) -> async_channel::Receiver<T> {
        let (tx, rx) = async_channel::bounded(1);
        self.inner.io(move |inner| {
            let _ = tx.try_send(job(inner));
        });
        rx
    }

    /// Settings › Media, as this Mac keeps it.
    pub fn preferences(&self) -> Preferences {
        self.inner.preferences()
    }

    /// Where the media storage lives now (no file work).
    pub fn location(&self) -> PathBuf {
        storage_root(&self.inner.profile, &self.preferences())
    }

    /// Storage › Renders' folder for the disk render cache, opened (and
    /// checked) at the current location. Disk work: call off the main thread.
    pub fn render_frames(&self) -> Option<(PathBuf, u64)> {
        let prefs = self.preferences();
        self.inner.storage(&prefs).ok()?;
        let limit = u64::from(prefs.render_limit_gb) * 1_000_000_000;
        Some((self.location().join("Renders").join("frames"), limit))
    }

    /// Writes Settings › Media's row `id`: checked and applied now, saved on
    /// the file thread.
    pub fn set_preference(&self, id: &str, value: &serde_json::Value) -> Result<(), String> {
        let next = {
            let mut prefs = self.inner.preferences.lock().unwrap();
            let mut next = prefs.clone();
            next.set(id, value)?;
            *prefs = next.clone();
            next
        };
        self.inner.io(move |inner| {
            if let Ok(bytes) = serde_json::to_vec_pretty(&next) {
                let _ = write_atomic(&settings_path(&inner.profile), &bytes);
            }
        });
        Ok(())
    }

    /// Moves the media storage to `folder` (refused unless it is empty or
    /// Powermove's), on the file thread. Existing files stay where they
    /// are; the next task writes to the new place.
    pub fn set_location(&self, folder: PathBuf) -> async_channel::Receiver<Result<(), String>> {
        self.io_answer(move |inner| {
            let mut next = inner.preferences();
            next.location = Some(folder);
            inner.storage(&next)?;
            let bytes = serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?;
            write_atomic(&settings_path(&inner.profile), &bytes)
                .map_err(|e| format!("Settings could not be saved: {e}"))?;
            *inner.preferences.lock().unwrap() = next;
            inner.measure_usage();
            Ok(())
        })
    }

    /// The last measured usage (`None` until the first measurement).
    pub fn cached_usage(&self) -> Option<Usage> {
        self.inner.usage.lock().unwrap().clone()
    }

    /// Measures usage again on the file thread; `Changed` follows.
    pub fn refresh_usage(&self) {
        self.inner.io(|inner| inner.measure_usage());
    }

    /// Identifies `project`'s originals (asset, path) on the file thread:
    /// their content (hashing a linked file once per state), and which
    /// rendition files exist. `Renditions` follows when anything changed.
    pub fn identify(&self, project: String, originals: Vec<(String, PathBuf)>) {
        self.inner.io(move |inner| {
            let mut changed = false;
            let stop = AtomicBool::new(false);
            for (asset, path) in &originals {
                if let Ok(content) = inner.content_of(path, &stop) {
                    changed |= inner.note(&project, asset, path, &content);
                }
            }
            changed |= inner.refresh_present();
            if changed {
                inner.save();
                let _ = inner.events.try_send(Event::Renditions);
            }
        });
    }

    /// Adds `project`'s renditions to `media` from memory: proxies (an
    /// embedded proxy stays), optimized media, Always Play Original, and
    /// for `assets` whose original is offline, the renditions their last
    /// identified content has.
    pub fn decorate(&self, project: &str, media: &mut MediaCatalog, assets: &[String]) {
        let state = self.inner.state.lock().unwrap();
        let present = |f: Option<&File>| {
            f.filter(|f| state.present.contains(&f.path))
                .map(|f| f.path.clone())
        };
        for asset in assets {
            let original = media.originals.get(asset).cloned();
            let Some(content) = Inner::content_for(&state, project, asset, original.as_deref())
            else {
                continue;
            };
            let Some(r) = state.index.contents.get(&content) else {
                continue;
            };
            if let Some(proxy) = present(r.proxy.as_ref()) {
                media.proxies.entry(asset.clone()).or_insert(proxy);
            }
            if let Some(optimized) = present(r.optimized.as_ref()) {
                media.optimized.insert(asset.clone(), optimized);
            }
        }
        media.play_original = state
            .index
            .play_original
            .get(project)
            .cloned()
            .unwrap_or_default();
    }

    /// `asset`'s rendition states for the Project panel (memory only).
    pub fn status(&self, project: &str, asset: &str, original: Option<&Path>) -> (Status, Status) {
        let tasks = self.inner.tasks.lock().unwrap();
        let state = self.inner.state.lock().unwrap();
        let mut out = [Status::Absent, Status::Absent];
        if let Some(r) = Inner::content_for(&state, project, asset, original)
            .and_then(|c| state.index.contents.get(&c))
        {
            for (i, kind) in [Rendition::Proxy, Rendition::Optimized]
                .into_iter()
                .enumerate()
            {
                if r.get(kind).is_some_and(|f| state.present.contains(&f.path)) {
                    out[i] = Status::Ready;
                }
            }
        }
        for (id, job) in &state.jobs {
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
        self.inner
            .state
            .lock()
            .unwrap()
            .index
            .play_original
            .get(project)
            .is_some_and(|s| s.contains(asset))
    }

    /// Always Play Original on or off for `assets` (saved on the file thread).
    pub fn set_play_original(&self, project: &str, assets: &[String], on: bool) {
        {
            let mut state = self.inner.state.lock().unwrap();
            let set = state
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
                state.index.play_original.remove(project);
            }
        }
        self.inner.io(|inner| inner.save());
        let _ = self.inner.events.try_send(Event::Renditions);
    }

    /// Queues one task per asset (asset id, title, original path).
    pub fn create(
        &self,
        project: &str,
        kind: Rendition,
        prefs: &Preferences,
        assets: Vec<Original>,
    ) -> Result<usize, String> {
        self.inner.create(project, kind, prefs, assets)
    }

    /// Forgets `kind` for `assets` (id, original path or `None` offline)
    /// on the file thread; then, for a rebuild, queues new ones.
    pub fn delete(
        &self,
        project: String,
        kind: Rendition,
        assets: Vec<(String, Option<PathBuf>)>,
        rebuild: Option<(Preferences, Vec<Original>)>,
    ) {
        self.inner.io(move |inner| {
            inner.delete(&project, kind, &assets);
            inner.save();
            let _ = inner.refresh_present();
            let _ = inner.events.try_send(Event::Renditions);
            if let Some((prefs, again)) = rebuild {
                let _ = inner.create(&project, kind, &prefs, again);
            }
            inner.measure_usage();
        });
    }

    /// Attaches `file` as `asset`'s proxy on the file thread, after checking
    /// it matches the original.
    pub fn attach(
        &self,
        project: String,
        asset: String,
        original: PathBuf,
        file: PathBuf,
    ) -> async_channel::Receiver<Result<(), String>> {
        self.io_answer(move |inner| {
            inner.attach(&project, &asset, &original, &file)?;
            inner.save();
            let _ = inner.events.try_send(Event::Renditions);
            Ok(())
        })
    }

    /// Clear… on the file thread: see `Inner::clear`. `pinned` are the open
    /// project's assets; their contents are kept.
    pub fn clear(&self, project: String, pinned: Vec<String>) -> async_channel::Receiver<u64> {
        self.io_answer(move |inner| {
            let contents: BTreeSet<String> = {
                let state = inner.state.lock().unwrap();
                let assets = state.index.assets.get(&project);
                pinned
                    .iter()
                    .filter_map(|a| assets.and_then(|m| m.get(a)).cloned())
                    .collect()
            };
            let freed = inner.clear(&contents);
            inner.save();
            let _ = inner.refresh_present();
            inner.measure_usage();
            let _ = inner.events.try_send(Event::Renditions);
            freed
        })
    }

    /// Create Proxies on import: probes `assets` (id, title, path) on the
    /// file thread and queues the ones `prefs` asks for.
    pub fn create_on_import(&self, project: String, prefs: Preferences, assets: Vec<Original>) {
        self.inner.io(move |inner| {
            let wanted: Vec<_> = assets
                .into_iter()
                .filter(|(_, _, path)| {
                    pm_media::probe::probe(path)
                        .ok()
                        .and_then(|p| p.probe.video)
                        .is_some_and(|v| {
                            prefs.creates_on_import(v.width, v.height, v.codec.bit_depth)
                        })
                })
                .collect();
            if !wanted.is_empty() {
                let _ = inner.create(&project, Rendition::Proxy, &prefs, wanted);
            }
        });
    }

    /// Pause, resume, cancel, or set the priority of a media task.
    pub fn act(&self, id: TaskId, action: TaskAction) -> Result<(), String> {
        let mut tasks = self.inner.tasks.lock().unwrap();
        let running = tasks.get(id).is_some_and(|t| t.state == TaskState::Running);
        match action {
            TaskAction::Pause => tasks.pause(id),
            TaskAction::Resume => tasks.resume(id),
            TaskAction::Cancel => tasks.cancel(id),
            TaskAction::Priority(p) => tasks.priority(id, p),
        }
        .map_err(|e| format!("{e:?}"))?;
        if running && matches!(action, TaskAction::Pause | TaskAction::Cancel) {
            self.inner.stop.store(true, Ordering::Release);
        }
        drop(tasks);
        if action == TaskAction::Resume {
            self.inner.kick();
        }
        let _ = self.inner.events.try_send(Event::Changed);
        Ok(())
    }

    /// Why a running task's encode waits, in the task's words.
    pub fn waiting(&self, id: TaskId) -> Option<&'static str> {
        match self.inner.state.lock().unwrap().waiting {
            Some((task, pm_proxy::PauseReason::LowDisk)) if task == id => {
                Some("Waiting for free disk space (Powermove keeps 10% of the disk free)")
            }
            Some((task, pm_proxy::PauseReason::InteractiveWork)) if task == id => {
                Some("Waiting while the viewer plays or an export runs")
            }
            _ => None,
        }
    }

    /// Every media task, in queue order.
    pub fn snapshot(&self) -> Vec<Task> {
        self.inner.tasks.lock().unwrap().all()
    }

    /// Assets just imported, waiting for the catalog to resolve them.
    pub fn after_import(&self, assets: Vec<String>) {
        self.inner.imported.lock().unwrap().extend(assets);
    }

    /// The imported assets `resolved` now has, taken from the waiting list.
    pub fn take_imported(&self, resolved: impl Fn(&str) -> bool) -> Vec<String> {
        let mut waiting = self.inner.imported.lock().unwrap();
        let (ready, rest): (Vec<_>, Vec<_>) = waiting.drain(..).partition(|id| resolved(id));
        *waiting = rest;
        ready
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskAction {
    Pause,
    Resume,
    Cancel,
    Priority(i32),
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

    /// The open project's asset ids.
    /// Project files that can't be played from here (the Project panel's
    /// Missing), for the status line.
    pub(crate) fn offline_count(&self) -> usize {
        let Some(pm_model::NodeKind::Object(assets)) = self
            .published
            .as_ref()
            .and_then(|p| p.snapshot.root())
            .and_then(|r| r.get("assets"))
            .map(|a| a.kind())
        else {
            return 0;
        };
        self.builtins
            .media_runtime
            .missing(assets.keys().filter_map(|k| k.as_str()))
    }
    /// The status line's offline count, pressed: the Project panel lists the
    /// missing files.
    pub(crate) fn show_offline_media(&mut self, cx: &mut gpui::Context<Self>) {
        self.host.borrow().send_command("panel:show:assets", vec![]);
        self.builtins.show_missing = true;
        self.update_builtins(cx);
    }
    pub(crate) fn project_assets(&self) -> Vec<String> {
        self.published
            .as_ref()
            .and_then(|p| p.snapshot.root())
            .and_then(|r| r.get("assets"))
            .and_then(|a| match a.kind() {
                pm_model::NodeKind::Object(map) => {
                    Some(map.keys().map(|k| k.to_string()).collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Asks the pro media service to identify the open project's originals
    /// (off the main thread); the catalog follows when it answers.
    pub(crate) fn identify_media(&self) {
        if let Some(project) = self.session.as_ref().map(|s| s.project.clone()) {
            let originals = self
                .media_base
                .originals
                .iter()
                .map(|(id, path)| (id.clone(), path.clone()))
                .collect();
            self.pro_media.identify(project, originals);
        }
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
        serde_json::json!({
            "rows": rows,
            "location": self.pro_media.location().to_string_lossy(),
            "folders": self.waiting_folders.iter().map(|f| serde_json::json!({
                "name": f.name,
                "files": f.files,
                "refused": f.outcome == pm_first_launch::source_access::Outcome::Denied,
            })).collect::<Vec<_>>(),
            "used": usage.map(|u| serde_json::json!({
                "proxies": bytes(u.proxies),
                "optimized": bytes(u.optimized),
                "renders": bytes(u.renders),
            })),
        })
    }

    fn first_launch_config(&self) -> Option<pm_first_launch::Config> {
        Some(pm_first_launch::Config {
            electron_profile: self.electron.clone()?,
            native_profile: self.profile.clone(),
        })
    }
    /// Reads which guarded folders kept originals out (metadata only, off the
    /// main thread), for Settings › Media.
    pub(crate) fn refresh_waiting_folders(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(config) = self.first_launch_config() else {
            return;
        };
        let read = cx
            .background_executor()
            .spawn(async move { pm_first_launch::waiting_folders(&config) });
        cx.spawn(async move |this, cx| {
            let folders = read.await.unwrap_or_default();
            let _ = this.update(cx, |v, cx| {
                if v.waiting_folders != folders {
                    v.waiting_folders = folders;
                    v.update_builtins(cx);
                }
            });
        })
        .detach();
    }
    /// Settings › Media › Allow Access… / Check Again…: explains macOS's prompt
    /// (or, for folders macOS refused, where to allow them) first, then opens
    /// the folders and brings their originals in. A folder allowed in System
    /// Settings since opens now; one still refused stays refused, unasked.
    fn allow_waiting_folders(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(config) = self.first_launch_config() else {
            return;
        };
        let folders = self.waiting_folders.clone();
        if folders.is_empty() {
            return;
        }
        let window = self.host.borrow().window.clone();
        let rx = crate::platform::choice(
            &window,
            if crate::panels::askable(&folders) {
                "Allow Access to Your Media"
            } else {
                "Check Folder Access Again"
            },
            &crate::panels::folder_access_prompt(&folders),
            &["Continue", "Not Now"],
        );
        cx.spawn(async move |this, cx| {
            if rx.recv().await != Ok(0) {
                return;
            }
            let paths: Vec<_> = folders.iter().map(|f| f.path.clone()).collect();
            let result = cx
                .background_executor()
                .spawn(async move { pm_first_launch::allow_folders(&config, &paths, |_| Ok(())) })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.message = match result {
                    Ok(pm_first_launch::Outcome::Complete { not_reached: 0, .. }) => {
                        "Your media from those folders came across.".into()
                    }
                    Ok(pm_first_launch::Outcome::Complete { not_reached, .. }) => format!(
                        "{} still couldn’t be opened.",
                        crate::panels::file_count(not_reached)
                    ),
                    Ok(_) => "Quit the original app, then try again.".into(),
                    Err(_) => "Your media couldn’t be brought in. Try again.".into(),
                };
                v.media_refresh.reset();
                v.refresh_media(cx);
                v.refresh_waiting_folders(cx);
                cx.notify();
            });
        })
        .detach();
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
                    let result = pro_media
                        .set_location(folder)
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err("The media storage could not be changed".into()));
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
            Some("media.reveal") => crate::platform::reveal(&self.pro_media.location()),
            Some("media.folders") => self.allow_waiting_folders(cx),
            Some("media.clear") => {
                let rx = crate::platform::choice(
                    &window,
                    "Clear generated media?",
                    "Proxies, optimized media and renders the open project doesn’t use are removed. Originals, attached proxies and this project’s media stay. Powermove can make them again.",
                    &["Clear", "Cancel"],
                );
                let pro_media = self.pro_media.clone();
                let project = self
                    .session
                    .as_ref()
                    .map(|s| s.project.clone())
                    .unwrap_or_default();
                let pinned = self.project_assets();
                cx.spawn(async move |this, cx| {
                    if rx.recv().await != Ok(0) {
                        return;
                    }
                    let freed = pro_media.clear(project, pinned).recv().await.unwrap_or(0);
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
    /// Nothing here touches the disk: the pro media threads do.
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
        let online: Vec<Original> = assets
            .iter()
            .filter_map(|id| {
                let path = self.media_base.originals.get(id)?;
                Some((id.clone(), self.asset_title(id), path.clone()))
            })
            .collect();
        let all: Vec<(String, Option<PathBuf>)> = assets
            .iter()
            .map(|id| (id.clone(), self.media_base.originals.get(id).cloned()))
            .collect();
        let result = match action {
            A::CreateProxy => self
                .pro_media
                .create(&project, Rendition::Proxy, &prefs, online)
                .map(|_| ()),
            A::RebuildProxy => {
                self.pro_media
                    .delete(project, Rendition::Proxy, all, Some((prefs, online)));
                Ok(())
            }
            A::DeleteProxy => {
                self.pro_media.delete(project, Rendition::Proxy, all, None);
                Ok(())
            }
            A::CreateOptimized => self
                .pro_media
                .create(&project, Rendition::Optimized, &prefs, online)
                .map(|_| ()),
            A::DeleteOptimized => {
                self.pro_media
                    .delete(project, Rendition::Optimized, all, None);
                Ok(())
            }
            A::PlayOriginal(on) => {
                self.pro_media.set_play_original(&project, &assets, on);
                Ok(())
            }
            A::AttachProxy => {
                let Some((asset, _, original)) = online.into_iter().next() else {
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
                    let result = pro_media
                        .attach(project, asset, original, file)
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err("The proxy could not be attached".into()));
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
    /// facts call for one, probed off the main thread.
    pub(crate) fn proxies_on_import(&mut self, assets: &[String]) {
        let prefs = self.pro_media_preferences();
        if prefs.create == Create::OnlyWhenAsked {
            return;
        }
        let Some(project) = self.session.as_ref().map(|s| s.project.clone()) else {
            return;
        };
        let candidates: Vec<Original> = assets
            .iter()
            .filter_map(|id| {
                let path = self.media_base.originals.get(id)?;
                Some((id.clone(), self.asset_title(id), path.clone()))
            })
            .collect();
        if !candidates.is_empty() {
            self.pro_media.create_on_import(project, prefs, candidates);
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
        let objects = crate::media::objects_dir(&profile);
        std::fs::create_dir_all(&objects).unwrap();
        let media = ProMedia::new(profile, objects, Arc::new(AtomicBool::new(false)));
        (dir, media)
    }

    /// Waits for the file thread to finish what was queued before.
    fn settle(media: &ProMedia) {
        let rx = media.io_answer(|_| ());
        rx.recv_blocking().unwrap();
    }

    fn identify(media: &ProMedia, project: &str, originals: &[(&str, &Path)]) {
        media.identify(
            project.into(),
            originals
                .iter()
                .map(|(a, p)| (a.to_string(), p.to_path_buf()))
                .collect(),
        );
        settle(media);
    }

    fn generated(content: &str, path: &Path) -> (String, File) {
        (
            content.to_owned(),
            File {
                path: path.to_path_buf(),
                attached: false,
            },
        )
    }

    /// The review's blocker: byte-identical originals share one stored
    /// proxy; clearing with only the second one's project open keeps it,
    /// and a record still naming a file (attached elsewhere) keeps it too.
    #[test]
    fn clear_never_deletes_media_another_original_or_record_uses() {
        let (dir, media) = service();
        let (a, b) = (dir.path().join("a.mov"), dir.path().join("b.mov"));
        std::fs::write(&a, b"same bytes").unwrap();
        std::fs::write(&b, b"same bytes").unwrap();
        identify(&media, "pa", &[("x", &a)]);
        identify(&media, "pb", &[("y", &b)]);
        let content = {
            let state = media.inner.state.lock().unwrap();
            assert_eq!(state.known[&a], state.known[&b], "one content");
            state.known[&a].clone()
        };
        let proxy = dir.path().join("shared-proxy.mov");
        std::fs::write(&proxy, b"proxy").unwrap();
        let (c, f) = generated(&content, &proxy);
        media
            .inner
            .state
            .lock()
            .unwrap()
            .index
            .put(&c, Rendition::Proxy, f);
        let freed = media
            .clear("pb".into(), vec!["y".into()])
            .recv_blocking()
            .unwrap();
        assert_eq!(freed, 0);
        assert!(
            proxy.exists(),
            "the open project's identical original uses it"
        );

        // Another content's attached record names the same file: an
        // unpinned generated record of it can go, the file can't.
        let elsewhere = dir.path().join("other.mov");
        std::fs::write(&elsewhere, b"other bytes").unwrap();
        identify(&media, "pc", &[("z", &elsewhere)]);
        let other = media.inner.state.lock().unwrap().known[&elsewhere].clone();
        media.inner.state.lock().unwrap().index.put(
            &other,
            Rendition::Proxy,
            File {
                path: proxy.clone(),
                attached: true,
            },
        );
        media.clear("none".into(), vec![]).recv_blocking().unwrap();
        assert!(proxy.exists(), "an attached record still names it");
        let state = media.inner.state.lock().unwrap();
        assert!(
            state
                .index
                .contents
                .get(&other)
                .is_some_and(|r| r.proxy.is_some())
        );
    }

    /// Renditions follow content: a media-cache object whose modification
    /// time changes on reuse keeps them, an offline original keeps its
    /// proxy through the last identification, and Always Play Original
    /// persists per project.
    #[test]
    fn renditions_survive_touched_objects_and_offline_originals() {
        let (dir, media) = service();
        let object = media.inner.objects.join("0123abcd.mov");
        std::fs::write(&object, b"embedded bytes").unwrap();
        let proxy = dir.path().join("proxy.mov");
        std::fs::write(&proxy, b"proxy").unwrap();
        identify(&media, "p", &[("a", &object)]);
        let content = media.inner.state.lock().unwrap().known[&object].clone();
        assert_eq!(content, "obj:0123abcd.mov");
        let (c, f) = generated(&content, &proxy);
        media
            .inner
            .state
            .lock()
            .unwrap()
            .index
            .put(&c, Rendition::Proxy, f);
        identify(&media, "p", &[("a", &object)]);
        // The resolver touches reused objects.
        std::fs::File::options()
            .append(true)
            .open(&object)
            .unwrap()
            .set_modified(std::time::SystemTime::now())
            .unwrap();
        let reopened = ProMedia::new(
            media.inner.profile.clone(),
            media.inner.objects.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        identify(&reopened, "p", &[("a", &object)]);
        let mut catalog = MediaCatalog::default();
        catalog.originals.insert("a".into(), object.clone());
        reopened.decorate("p", &mut catalog, &["a".into()]);
        assert_eq!(catalog.proxies.get("a"), Some(&proxy));
        // Offline: no original in the catalog, the proxy still plays.
        let mut offline = MediaCatalog::default();
        reopened.decorate("p", &mut offline, &["a".into()]);
        assert_eq!(offline.proxies.get("a"), Some(&proxy));
        assert_eq!(reopened.status("p", "a", None).0, Status::Ready);
        // Deleting the only playable copy of an offline original is refused.
        reopened.delete("p".into(), Rendition::Proxy, vec![("a".into(), None)], None);
        settle(&reopened);
        assert!(proxy.exists());
        reopened.set_play_original("p", &["a".into()], true);
        settle(&reopened);
        let again = ProMedia::new(
            media.inner.profile.clone(),
            media.inner.objects.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        assert!(again.plays_original("p", "a") && !again.plays_original("q", "a"));
    }

    /// A generation finishing after the person attached a proxy (or deleted
    /// the rendition) leaves their choice and retires its own file;
    /// replacing a generated file retires the old one.
    #[test]
    fn a_late_generation_yields_and_replaced_files_are_retired() {
        let (dir, media) = service();
        let files: Vec<PathBuf> = (0..3)
            .map(|i| {
                let p = dir.path().join(format!("{i}.mov"));
                std::fs::write(&p, b"x").unwrap();
                p
            })
            .collect();
        let mine = File {
            path: files[0].clone(),
            attached: true,
        };
        let inner = &media.inner;
        inner
            .state
            .lock()
            .unwrap()
            .index
            .put("b3:c", Rendition::Proxy, mine.clone());
        let made = generated("b3:c", &files[1]).1;
        assert!(!inner.publish("b3:c", Rendition::Proxy, None, made));
        assert!(!files[1].exists(), "the late file is retired");
        assert!(files[0].exists());
        let r = inner.state.lock().unwrap().index.contents["b3:c"].clone();
        assert_eq!(r.proxy, Some(mine.clone()));
        // Unchanged since it began: published; the replaced attached file
        // stays on disk, a replaced generated one goes.
        let made = generated("b3:c", &files[1]).1;
        std::fs::write(&files[1], b"x").unwrap();
        assert!(inner.publish("b3:c", Rendition::Proxy, Some(&mine), made.clone()));
        assert!(files[0].exists());
        let newer = generated("b3:c", &files[2]).1;
        assert!(inner.publish("b3:c", Rendition::Proxy, Some(&made), newer));
        assert!(!files[1].exists(), "the replaced generated file is retired");
    }

    fn clip(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
        let (ffmpeg, _) = ffmpeg().expect("FFmpeg");
        let out = dir.join(name);
        let status = std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .arg(&out)
            .status()
            .unwrap();
        assert!(status.success(), "{name}");
        out
    }

    /// Attach refuses a file that is not the same picture and sound.
    #[test]
    fn attach_checks_alpha_audio_start_and_colour() {
        if ffmpeg().is_none() {
            eprintln!("NOT RUN: no FFmpeg pair (PM_FFMPEG_DIR/bin)");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let original = clip(
            d,
            "original.mov",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24:duration=1,format=yuva444p",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=1",
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "ayuv64le",
                "-c:a",
                "pcm_s24le",
                "-shortest",
            ],
        );
        let opaque_silent = clip(
            d,
            "opaque.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=24:duration=1",
                "-c:v",
                "h264_videotoolbox",
                "-pix_fmt",
                "nv12",
            ],
        );
        assert!(attach_check(&original, &opaque_silent).is_err());
        let matching = clip(
            d,
            "proxy.mov",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=24:duration=1,format=yuva444p",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=1",
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "ayuv64le",
                "-c:a",
                "pcm_s24le",
                "-shortest",
            ],
        );
        attach_check(&original, &matching).unwrap();
        let other_audio = clip(
            d,
            "mono.mov",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=24:duration=1,format=yuva444p",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100:duration=1",
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "ayuv64le",
                "-c:a",
                "pcm_s24le",
                "-shortest",
            ],
        );
        assert!(attach_check(&original, &other_audio).is_err());
        let wrong_colour = clip(
            d,
            "pq.mov",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=24:duration=1,format=yuva444p,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=1",
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "ayuv64le",
                "-c:a",
                "pcm_s24le",
                "-shortest",
            ],
        );
        assert!(attach_check(&original, &wrong_colour).is_err());
        let later = clip(
            d,
            "later.mov",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=24:duration=1,format=yuva444p",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=1",
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "ayuv64le",
                "-c:a",
                "pcm_s24le",
                "-shortest",
                "-output_ts_offset",
                "1",
            ],
        );
        assert!(attach_check(&original, &later).is_err());
    }

    /// Create Proxy Media encodes on the worker, reports frames, completes,
    /// and reaches the catalog; after a location change the next task
    /// writes into the new location.
    #[test]
    fn created_proxies_reach_the_catalog_and_follow_the_location() {
        if ffmpeg().is_none() {
            eprintln!("NOT RUN: no FFmpeg pair (PM_FFMPEG_DIR/bin)");
            return;
        }
        // This Mac's free space is the production guard's business, not this
        // test's: reserve 1 GiB here.
        pm_proxy::set_test_disk_reserve(Some(1 << 30));
        let (dir, media) = service();
        let make = |name: &str, size: &str| {
            clip(
                dir.path(),
                name,
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("testsrc2=size={size}:rate=24:duration=1"),
                    "-c:v",
                    "prores_videotoolbox",
                    "-profile:v",
                    "hq",
                    "-pix_fmt",
                    "p216le",
                ],
            )
        };
        let (one, two) = (make("one.mov", "320x180"), make("two.mov", "352x198"));
        let finished = |n: usize| {
            let started = std::time::Instant::now();
            loop {
                let tasks = media.snapshot();
                if tasks.iter().filter(|t| t.state.terminal()).count() >= n {
                    return tasks;
                }
                assert!(started.elapsed().as_secs() < 120, "the task finishes");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        };
        let prefs = Preferences::default();
        media
            .create(
                "p",
                Rendition::Proxy,
                &prefs,
                vec![("a".into(), "One".into(), one.clone())],
            )
            .unwrap();
        let task = finished(1)[0].clone();
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
        identify(&media, "p", &[("a", &one), ("b", &two)]);
        let mut catalog = MediaCatalog::default();
        catalog.originals.insert("a".into(), one.clone());
        media.decorate("p", &mut catalog, &["a".into()]);
        let first = catalog
            .proxies
            .get("a")
            .expect("the proxy is in the catalog")
            .clone();
        assert!(first.starts_with(dir.path().join("profile/Media Storage")));

        let elsewhere = dir.path().join("Elsewhere");
        media
            .set_location(elsewhere.clone())
            .recv_blocking()
            .unwrap()
            .unwrap();
        media
            .create(
                "p",
                Rendition::Proxy,
                &prefs,
                vec![("b".into(), "Two".into(), two.clone())],
            )
            .unwrap();
        finished(2);
        identify(&media, "p", &[("a", &one), ("b", &two)]);
        let mut catalog = MediaCatalog::default();
        catalog.originals.insert("b".into(), two.clone());
        media.decorate("p", &mut catalog, &["b".into()]);
        let second = catalog.proxies.get("b").expect("the second proxy").clone();
        assert!(second.starts_with(&elsewhere), "{second:?}");
        assert!(first.is_file(), "existing files stay where they are");
        assert_eq!(media.location(), elsewhere);
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
        settle(&media);
        let reopened = ProMedia::new(
            media.inner.profile.clone(),
            media.inner.objects.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        let prefs = reopened.preferences();
        assert_eq!(
            (prefs.size, prefs.create),
            (ProxySize::P720, Create::WhenNeeded)
        );
        for (_, _, _, value, options) in prefs.rows() {
            assert!(options.iter().any(|(v, _)| *v == value));
        }
        assert!(prefs.creates_on_import(3840, 2160, 8));
        assert!(!prefs.creates_on_import(1920, 1080, 10));
    }
}

/// Counts finished media passes; editors read the protected folders again.
#[derive(Default)]
pub(crate) struct MediaPassDone(u64);
impl gpui::Global for MediaPassDone {}
impl MediaPassDone {
    pub(crate) fn bump(cx: &mut gpui::App) {
        let n = cx.try_global::<Self>().map_or(0, |d| d.0) + 1;
        cx.set_global(Self(n));
    }
}
