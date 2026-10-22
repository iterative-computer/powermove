//! Viewer representation binding. Originals remain the document's media; this
//! resolver is local to a viewer render and is never passed to an export.
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::time::{Duration, Instant, SystemTime};

use super::document::MediaMap;
use pm_eval::{Comp, Evaluator, Scope};
use pm_model::Node;
use pm_protocol::common::{TICKS_PER_SECOND, Ticks};
use pm_protocol::frame::{MediaPreference, Representation};
use pm_protocol::priority::{ProxyFormat, ProxySize};
use pm_proxy::{
    AutomaticInput, DecodeMeasurement, MachineProfile, MediaAvailability, PlaybackMoment,
    Representation as PolicyRepresentation, RepresentationIntent, ResolutionIntent,
    choose_representation,
};
use pm_render::AssetResolver;

pub(super) struct ProxyMedia {
    entries: HashMap<String, LookupEntry>,
    next_lookup: u64,
    lookup_tx: Option<SyncSender<LookupRequest>>,
    lookup_rx: Option<Receiver<LookupResult>>,
    lookup_stop: Arc<AtomicBool>,
    lookup_worker: Option<std::thread::JoinHandle<()>>,
    probe_tx: Option<SyncSender<ProbeRequest>>,
    probe_rx: Option<Receiver<ProbeResult>>,
    probe_worker: Option<std::thread::JoinHandle<()>>,
    probe_state: Arc<ProbeState>,
    probe_entries: HashMap<ProbeKey, ProbeEntry>,
    locations_file: Option<PathBuf>,
    playing: bool,
    last_playhead: Option<i64>,
    intent: String,
    choices: HashMap<(String, Option<String>, String), PlaybackChoice>,
    deletions: Option<(Option<(SystemTime, u64)>, serde_json::Value)>,
    /// Originals this machine measured as unable to play in real time (main's
    /// cold decode assessments, `Proxy Assessments.json`), by content hash.
    assessed_slow: Option<(Option<(SystemTime, u64)>, std::collections::HashSet<String>)>,
}

#[derive(Clone)]
struct PlaybackChoice {
    proxy: Option<PathBuf>,
    proxy_stamp: Option<FileStamp>,
    decode_scale: Option<f64>,
    fallback_decode_scale: Option<f64>,
}

impl Default for ProxyMedia {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            next_lookup: 0,
            lookup_tx: None,
            lookup_rx: None,
            lookup_stop: Arc::new(AtomicBool::new(false)),
            lookup_worker: None,
            probe_tx: None,
            probe_rx: None,
            probe_worker: None,
            probe_state: Arc::new(ProbeState::default()),
            probe_entries: HashMap::new(),
            locations_file: None,
            playing: false,
            last_playhead: None,
            intent: String::new(),
            choices: HashMap::new(),
            deletions: None,
            assessed_slow: None,
        }
    }
}

struct LookupEntry {
    checked: Instant,
    nonce: u64,
    /// `None` is in flight; `Some(None)` is a completed miss.
    result: Option<Option<(PathBuf, FileStamp)>>,
    /// A viewer explicitly waiting for this proxy renews this deadline on
    /// each lookup. A completed miss must be retried when a job publishes it.
    watch_until: Option<Instant>,
}

struct LookupRequest {
    key: String,
    nonce: u64,
    hash: String,
    format: ProxyFormat,
    size: ProxySize,
    roots: Vec<PathBuf>,
}

struct LookupResult {
    key: String,
    nonce: u64,
    found: Option<(PathBuf, FileStamp)>,
}

#[derive(Default)]
struct ProbeState {
    playing: AtomicBool,
    exporting: AtomicBool,
    stop: AtomicBool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ProbeScale {
    Full,
    Half,
    Quarter,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ProbeKey {
    path: PathBuf,
    scale: ProbeScale,
    dimensions: (u32, u32),
}

struct ProbeEntry {
    stamp: FileStamp,
    /// None is pending; Some(None) means unavailable or insufficient samples.
    cost: Option<Option<f64>>,
}

struct ProbeRequest {
    key: ProbeKey,
    stamp: FileStamp,
    dimensions: (u32, u32),
}

struct ProbeResult {
    key: ProbeKey,
    stamp: FileStamp,
    cost: Option<f64>,
}

fn wait_for_probe_idle(state: &ProbeState) -> pm_media::error::Result<()> {
    while (state.playing.load(Ordering::Acquire) || state.exporting.load(Ordering::Acquire))
        && !state.stop.load(Ordering::Acquire)
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    if state.stop.load(Ordering::Acquire) {
        Err(pm_media::error::MediaError::Cancelled)
    } else {
        Ok(())
    }
}

fn probe_worker(
    requests: Receiver<ProbeRequest>,
    results: mpsc::Sender<ProbeResult>,
    state: Arc<ProbeState>,
) {
    for request in requests {
        if state.stop.load(Ordering::Acquire) {
            break;
        }
        let purpose = match request.key.scale {
            ProbeScale::Full => pm_media::vt::DecodePurpose::Display,
            ProbeScale::Half | ProbeScale::Quarter => {
                let divisor = if request.key.scale == ProbeScale::Half {
                    2
                } else {
                    4
                };
                pm_media::vt::DecodePurpose::DisplayReduced {
                    width: request.dimensions.0.div_ceil(divisor).max(1),
                    height: request.dimensions.1.div_ceil(divisor).max(1),
                }
            }
        };
        let wait = || wait_for_probe_idle(&state);
        let cost = pm_media::decode::measure::measure_source_purpose_with_pause(
            &request.key.path,
            purpose,
            wait,
        )
        .ok()
        .flatten()
        .filter(|measurement| measurement.sufficient_samples)
        .and_then(|measurement| measurement.sequential_p95_ms);
        if results
            .send(ProbeResult {
                key: request.key,
                stamp: request.stamp,
                cost,
            })
            .is_err()
        {
            break;
        }
    }
}

impl Drop for ProxyMedia {
    fn drop(&mut self) {
        self.lookup_stop.store(true, Ordering::Release);
        self.probe_state.stop.store(true, Ordering::Release);
        self.lookup_tx.take();
        self.probe_tx.take();
        if let Some(worker) = self.lookup_worker.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.probe_worker.take() {
            let _ = worker.join();
        }
    }
}

fn lookup_worker(
    requests: Receiver<LookupRequest>,
    results: mpsc::Sender<LookupResult>,
    stop: Arc<AtomicBool>,
) {
    for request in requests {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let found = request.roots.iter().find_map(|root| {
            if stop.load(Ordering::Acquire) {
                return None;
            }
            let path = pm_proxy::ProxyStore::open(root)
                .ok()?
                .find_for_source_cancel(
                    &request.hash,
                    Some(request.format),
                    Some(request.size),
                    || stop.load(Ordering::Acquire),
                )
                .ok()??
                .path;
            file_stamp(&path).map(|stamp| (path, stamp))
        });
        if results
            .send(LookupResult {
                key: request.key,
                nonce: request.nonce,
                found,
            })
            .is_err()
        {
            break;
        }
    }
}

type FileStamp = (u64, u64, u64, i64, i64, i64, i64);
fn file_stamp(path: &std::path::Path) -> Option<FileStamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}

fn text(node: Option<&Node>) -> Option<String> {
    match node?.kind() {
        pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// Distinguishes adjacent instances of the same footage. A representation is
/// frozen for an active clip, not for an asset across the entire playback run.
fn active_clip_signature(
    doc: &pm_model::Document,
    composition: &str,
    asset: &str,
    seconds: f64,
) -> String {
    let Some(root) = doc.root() else {
        return String::new();
    };
    let open = text(root.get("compId")).or_else(|| text(root.get("id")));
    let Ok(project) = Comp::in_root(root, open.as_deref().unwrap_or(composition), doc.revision())
    else {
        return String::new();
    };
    let mut evaluator = Evaluator::new();
    let mut active = Vec::new();
    collect_active_clips(
        root,
        doc.revision(),
        &project,
        composition,
        asset,
        seconds,
        "",
        0,
        &mut evaluator,
        &mut active,
    );
    active.sort();
    active.join("|")
}

/// Follow the same source-time mapping and six-level precomp limit as the
/// render graph. The full ancestry distinguishes two instances of the same
/// child composition, including after a time remap crosses a child clip.
fn collect_active_clips(
    root: &Node,
    revision: u64,
    project: &Comp<'_>,
    composition: &str,
    asset: &str,
    seconds: f64,
    prefix: &str,
    depth: u32,
    evaluator: &mut Evaluator,
    active: &mut Vec<String>,
) {
    if depth > pm_render::plan::PRECOMP_MAX_DEPTH || !seconds.is_finite() {
        return;
    }
    let Ok(comp) = Comp::in_root(root, composition, revision) else {
        return;
    };
    for (index, layer) in comp.layers().iter().enumerate() {
        let kind = text(layer.get("type"));
        if !matches!(kind.as_deref(), Some("video" | "precomp")) {
            continue;
        }
        let is_active = {
            let mut scope = Scope::new(evaluator, &comp, project);
            scope.active(layer, seconds).unwrap_or(false)
        };
        if !is_active {
            continue;
        }
        let name = text(layer.get("id")).unwrap_or_else(|| index.to_string());
        let identity = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if kind.as_deref() == Some("video") {
            if text(layer.get("d").and_then(|d| d.get("asset"))).as_deref() == Some(asset) {
                active.push(identity);
            }
            continue;
        }
        let Some(child_id) = text(layer.get("d").and_then(|d| d.get("comp"))) else {
            continue;
        };
        // A nested precomp of the open composition is not a valid target in
        // the stored project form, matching the renderer's capability check.
        if root.get("layers").is_some()
            && text(root.get("compId"))
                .or_else(|| text(root.get("id")))
                .as_deref()
                == Some(child_id.as_str())
        {
            continue;
        }
        let Some(child_node) = root.get("comps").and_then(|comps| comps.get(&child_id)) else {
            continue;
        };
        if child_node.get("layers").is_none() {
            continue;
        }
        let child_time = {
            let mut scope = Scope::new(evaluator, &comp, project);
            scope.source_time(layer, seconds, None).ok()
        };
        let (Some(child_time), Ok(child)) = (child_time, Comp::in_root(root, &child_id, revision))
        else {
            continue;
        };
        let child_time = pm_render::retime::keep_frame_rate(layer, &child, child_time);
        collect_active_clips(
            root,
            revision,
            project,
            &child_id,
            asset,
            child_time,
            &identity,
            depth + 1,
            evaluator,
            active,
        );
    }
}

impl ProxyMedia {
    pub fn set_exporting(&mut self, active: bool) {
        self.probe_state.exporting.store(active, Ordering::Release);
    }
    pub fn set_locations_file(&mut self, path: PathBuf) {
        if self.locations_file.as_ref() != Some(&path) {
            self.locations_file = Some(path);
            self.entries.clear();
            self.choices.clear();
            self.deletions = None;
        }
    }

    fn locations_file(&self) -> Option<PathBuf> {
        self.locations_file
            .clone()
            .or_else(|| std::env::var_os("POWERMOVE_PROXY_LOCATIONS").map(PathBuf::from))
    }

    pub fn has_pending_lookups(&self) -> bool {
        self.entries.values().any(|entry| entry.result.is_none())
            || !self.playing
                && self
                    .probe_entries
                    .values()
                    .any(|entry| entry.cost.is_none())
    }

    fn start_probe_worker(&mut self) {
        if self.probe_tx.is_some() {
            return;
        }
        let (request_tx, request_rx) = mpsc::sync_channel(8);
        let (result_tx, result_rx) = mpsc::channel();
        let state = self.probe_state.clone();
        if let Ok(worker) = std::thread::Builder::new()
            .name("pm decode cost probe".into())
            .spawn(move || probe_worker(request_rx, result_tx, state))
        {
            self.probe_tx = Some(request_tx);
            self.probe_rx = Some(result_rx);
            self.probe_worker = Some(worker);
        }
    }

    fn probe_cost(
        &mut self,
        path: &std::path::Path,
        scale: ProbeScale,
        dimensions: (u32, u32),
    ) -> Option<f64> {
        let stamp = file_stamp(path)?;
        let key = ProbeKey {
            path: path.to_path_buf(),
            scale,
            dimensions,
        };
        if let Some(entry) = self.probe_entries.get(&key)
            && entry.stamp == stamp
        {
            return entry.cost.flatten();
        }
        self.start_probe_worker();
        let Some(sender) = &self.probe_tx else {
            return None;
        };
        match sender.try_send(ProbeRequest {
            key: key.clone(),
            stamp,
            dimensions,
        }) {
            Ok(()) => {
                self.probe_entries
                    .insert(key, ProbeEntry { stamp, cost: None });
            }
            Err(TrySendError::Disconnected(_)) => {
                self.probe_tx = None;
                self.probe_rx = None;
                if let Some(worker) = self.probe_worker.take() {
                    let _ = worker.join();
                }
            }
            Err(TrySendError::Full(_)) => {}
        }
        None
    }
    fn start_lookup_worker(&mut self) {
        if self.lookup_tx.is_some() {
            return;
        }
        let (request_tx, request_rx) = mpsc::sync_channel(8);
        let (result_tx, result_rx) = mpsc::channel();
        let stop = self.lookup_stop.clone();
        if let Ok(worker) = std::thread::Builder::new()
            .name("pm proxy integrity lookup".into())
            .spawn(move || lookup_worker(request_rx, result_tx, stop))
        {
            self.lookup_tx = Some(request_tx);
            self.lookup_rx = Some(result_rx);
            self.lookup_worker = Some(worker);
        }
    }

    pub fn poll_updates(&mut self) -> bool {
        let mut changed = false;
        if let Some(results) = &self.lookup_rx {
            while let Ok(result) = results.try_recv() {
                if let Some(entry) = self.entries.get_mut(&result.key)
                    && entry.result.is_none()
                    && entry.nonce == result.nonce
                {
                    entry.checked = Instant::now();
                    entry.result = Some(result.found);
                    changed = true;
                }
            }
        }
        if let Some(results) = &self.probe_rx {
            while let Ok(result) = results.try_recv() {
                if let Some(entry) = self.probe_entries.get_mut(&result.key)
                    && entry.stamp == result.stamp
                    && entry.cost.is_none()
                {
                    entry.cost = Some(result.cost);
                    changed = true;
                }
            }
        }
        // The render thread already polls at least every 50 ms. Expiring a
        // watched miss dirties the view, which makes its next render start a
        // fresh integrity lookup. Without this, a proxy completed inside the
        // one-second negative cache can leave Proxy Only frozen on "pending".
        let now = Instant::now();
        self.entries.retain(|_, entry| {
            let retry = matches!(&entry.result, Some(None))
                && entry.checked.elapsed() >= Duration::from_secs(1)
                && entry.watch_until.is_some_and(|until| until > now);
            if retry {
                changed = true;
                false
            } else {
                true
            }
        });
        changed
    }

    fn proxy_path(
        &mut self,
        key: String,
        hash: String,
        format: ProxyFormat,
        size: ProxySize,
        watch_miss: bool,
    ) -> Option<PathBuf> {
        if let Some(entry) = self.entries.get_mut(&key) {
            if watch_miss {
                entry.watch_until = Some(Instant::now() + Duration::from_secs(2));
            }
            match &entry.result {
                None => return None,
                Some(Some((path, stamp))) if file_stamp(path) == Some(*stamp) => {
                    return Some(path.clone());
                }
                Some(None) if entry.checked.elapsed() < Duration::from_secs(1) => return None,
                _ => {}
            }
        }
        self.entries.remove(&key);
        let roots = self.roots();
        self.start_lookup_worker();
        let Some(sender) = &self.lookup_tx else {
            return None;
        };
        self.next_lookup = self.next_lookup.wrapping_add(1);
        let nonce = self.next_lookup;
        let request = LookupRequest {
            key: key.clone(),
            nonce,
            hash,
            format,
            size,
            roots,
        };
        match sender.try_send(request) {
            Ok(()) => {
                self.entries.insert(
                    key,
                    LookupEntry {
                        checked: Instant::now(),
                        nonce,
                        result: None,
                        watch_until: watch_miss.then(|| Instant::now() + Duration::from_secs(2)),
                    },
                );
            }
            Err(TrySendError::Disconnected(_)) => {
                self.lookup_tx = None;
                self.lookup_rx = None;
                if let Some(worker) = self.lookup_worker.take() {
                    let _ = worker.join();
                }
            }
            Err(TrySendError::Full(_)) => {}
        }
        None
    }

    fn refresh_deletions(&mut self) {
        let Some(file) = self.locations_file() else {
            return;
        };
        let Some(parent) = file.parent() else {
            return;
        };
        let file = parent.join("Proxy Deletions.json");
        let stamp = std::fs::metadata(&file)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        if self
            .deletions
            .as_ref()
            .is_some_and(|(old, _)| *old == stamp)
        {
            return;
        }
        let value = if stamp.is_none() {
            serde_json::json!({"version":1,"deleted":{}})
        } else {
            // Damaged machine-local state cannot revive a proxy the user
            // deleted. Main reports the named state-file error to the UI.
            std::fs::read(&file)
                .ok()
                .and_then(|v| serde_json::from_slice(&v).ok())
                .unwrap_or(serde_json::Value::Null)
        };
        self.deletions = Some((stamp, value));
        self.choices.clear();
    }

    /// Main's machine-local cold decode assessments. Entries from another
    /// machine (a restored profile) are ignored, as main ignores them.
    fn refresh_assessments(&mut self) {
        let Some(file) = self.locations_file() else {
            return;
        };
        let Some(parent) = file.parent() else {
            return;
        };
        let file = parent.join("Proxy Assessments.json");
        let stamp = std::fs::metadata(&file)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        if self
            .assessed_slow
            .as_ref()
            .is_some_and(|(old, _)| *old == stamp)
        {
            return;
        }
        let machine = std::env::var("POWERMOVE_PROXY_MACHINE").ok();
        let slow = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .map(|value| assessed_slow_hashes(&value, machine.as_deref()))
            .unwrap_or_default();
        self.assessed_slow = Some((stamp, slow));
        self.choices.clear();
    }

    fn roots(&self) -> Vec<PathBuf> {
        let Some(file) = self.locations_file() else {
            return Vec::new();
        };
        let Ok(bytes) = std::fs::read(file) else {
            return Vec::new();
        };
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v.get("roots")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(PathBuf::from))
            .filter(|p| p.is_absolute())
            .collect()
    }

    pub fn resolver<'a>(
        &mut self,
        doc: &pm_model::Document,
        media: &'a MediaMap,
        inner: &'a dyn AssetResolver,
        preference: MediaPreference,
        playing: bool,
        slow: &[PathBuf],
        profile: Option<MachineProfile>,
        decode: &[(PathBuf, DecodeMeasurement)],
        viewer_size: (u32, u32),
        fps: f64,
        profile_generation: u64,
        composition: &str,
        playhead: Ticks,
    ) -> ProxyResolver<'a> {
        let mut proxies = BTreeMap::new();
        let mut forced = Vec::new();
        let mut reduced = BTreeMap::new();
        self.refresh_deletions();
        self.refresh_assessments();
        self.probe_state.playing.store(playing, Ordering::Release);
        self.poll_updates();
        let tick = playhead.get();
        let seconds = tick as f64 / TICKS_PER_SECOND as f64;
        let discontinuity = self.last_playhead.is_some_and(|previous| {
            tick < previous || tick.saturating_sub(previous) > TICKS_PER_SECOND
        });
        self.last_playhead = playing.then_some(tick);
        let project = text(doc.root().and_then(|r| r.get("id"))).unwrap_or_default();
        let intent = doc
            .root()
            .and_then(|r| r.get("media"))
            .and_then(|r| r.get("proxies"));
        let format = match text(intent.and_then(|r| r.get("format"))).as_deref() {
            Some("proResLt") => ProxyFormat::ProResLt,
            Some("hevc10") => ProxyFormat::Hevc10,
            _ => ProxyFormat::ProResProxy,
        };
        let size = match text(intent.and_then(|r| r.get("size"))).as_deref() {
            Some("half") => ProxySize::Half,
            Some("quarter") => ProxySize::Quarter,
            Some("p1080") => ProxySize::P1080,
            Some("p720") => ProxySize::P720,
            _ => ProxySize::Automatic,
        };
        let intent_key = format!(
            "{project}:{composition}:{preference:?}:{format:?}:{size:?}:{profile_generation}"
        );
        if !playing || !self.playing || self.intent != intent_key || discontinuity {
            self.choices.clear();
        }
        self.playing = playing;
        self.intent = intent_key;
        if let Some(pm_model::NodeKind::Object(assets)) =
            doc.root().and_then(|r| r.get("assets")).map(Node::kind)
        {
            for (id, meta) in assets {
                if !text(meta.get("kind")).is_some_and(|kind| kind == "video")
                    && !matches!(
                        meta.get("imageSequence").map(Node::kind),
                        Some(pm_model::NodeKind::Bool(true) | pm_model::NodeKind::Object(_))
                    )
                {
                    continue;
                }
                let id = id.to_string_lossy();
                if matches!(
                    meta.get("alwaysPlayOriginal").map(Node::kind),
                    Some(pm_model::NodeKind::Bool(true))
                ) {
                    forced.push(id);
                    continue;
                }
                if preference == MediaPreference::Originals {
                    continue;
                }
                let original = media.path(&id);
                let hash = text(
                    meta.get("imageSequence")
                        .and_then(|r| r.get("originalHash")),
                )
                .or_else(|| text(meta.get("contentHash")))
                .or_else(|| text(meta.get("link").and_then(|r| r.get("hash"))));
                if self.deletions.as_ref().is_some_and(|(_, value)| {
                    is_deleted(value, &project, &id, hash.as_deref(), format, size)
                }) {
                    continue;
                }
                // A paused Automatic frame still draws complete original
                // pixels. Validate any ready proxy off-thread now so the next
                // playback start can choose from it without a first-frame
                // checksum wait or a choice frozen around a pending lookup.
                if !playing
                    && preference == MediaPreference::Automatic
                    && let Some(hash) = &hash
                {
                    let key = format!("{hash}:{format:?}:{size:?}");
                    self.proxy_path(key, hash.clone(), format, size, false);
                }
                let clip = active_clip_signature(doc, composition, &id, seconds);
                if playing {
                    self.choices.retain(|(asset, _, previous_clip), _| {
                        asset != &id || previous_clip == &clip
                    });
                }
                let choice_key = (id.clone(), hash.clone(), clip);
                if playing && let Some(choice) = self.choices.get(&choice_key).cloned() {
                    if choice
                        .proxy
                        .as_ref()
                        .is_none_or(|path| file_stamp(path) == choice.proxy_stamp)
                    {
                        if let Some(path) = choice.proxy {
                            proxies.insert(id, path);
                        } else if let Some(scale) = choice.decode_scale {
                            reduced.insert(id, (scale, choice.fallback_decode_scale));
                        }
                        continue;
                    }
                    self.choices.remove(&choice_key);
                }
                let assessed = hash.as_ref().is_some_and(|h| {
                    self.assessed_slow
                        .as_ref()
                        .is_some_and(|(_, slow)| slow.contains(h))
                });
                let known_slow = original.as_ref().is_some_and(|p| slow.contains(p)) || assessed;
                let needed = preference == MediaPreference::Proxies
                    || original.as_ref().is_none_or(|p| !p.exists())
                    || playing && known_slow
                    || profile.is_some_and(|machine| {
                        original.as_ref().is_some_and(|path| {
                            decode
                                .iter()
                                .find(|(p, _)| p == path)
                                .is_some_and(|(_, sample)| {
                                    let margin = if machine.on_battery { 0.75 } else { 0.9 };
                                    sample.original_p95_ms.is_finite()
                                        && (!fps.is_finite()
                                            || fps <= 0.0
                                            || sample.original_p95_ms + machine.gpu_frame_p95_ms
                                                > 1000.0 / fps * margin)
                                })
                        })
                    });
                if !needed {
                    if playing {
                        self.choices.insert(
                            choice_key,
                            PlaybackChoice {
                                proxy: None,
                                proxy_stamp: None,
                                decode_scale: None,
                                fallback_decode_scale: None,
                            },
                        );
                    }
                    continue;
                }
                let Some(hash) = hash else { continue };
                let key = format!("{hash}:{format:?}:{size:?}");
                let ready_proxy = self.proxy_path(
                    key,
                    hash,
                    format,
                    size,
                    preference == MediaPreference::Proxies && !playing,
                );
                let original_online = original.as_ref().is_some_and(|p| p.exists());
                let number = |key: &str| match meta.get(key).map(Node::kind) {
                    Some(pm_model::NodeKind::Number(value))
                        if value.is_finite() && *value > 0.0 =>
                    {
                        *value as u32
                    }
                    _ => 0,
                };
                let source_size = (number("w"), number("h"));
                let measured = original
                    .as_ref()
                    .and_then(|path| decode.iter().find(|(p, _)| p == path).map(|(_, m)| *m))
                    .filter(|sample| sample.original_p95_ms.is_finite());
                let measured = measured.map(|mut value| {
                    if profile.is_some() && source_size.0 > 0 && source_size.1 > 0 {
                        if let Some(path) = &original {
                            value.reduced_half_p95_ms = value
                                .reduced_half_p95_ms
                                .or_else(|| self.probe_cost(path, ProbeScale::Half, source_size));
                            value.reduced_quarter_p95_ms =
                                value.reduced_quarter_p95_ms.or_else(|| {
                                    self.probe_cost(path, ProbeScale::Quarter, source_size)
                                });
                        }
                    }
                    if value.proxy_p95_ms.is_none() {
                        value.proxy_p95_ms = ready_proxy.as_ref().and_then(|path| {
                            decode
                                .iter()
                                .find(|(p, _)| p == path)
                                .and_then(|(_, m)| {
                                    m.original_p95_ms.is_finite().then_some(m.original_p95_ms)
                                })
                                .or_else(|| {
                                    profile.and_then(|_| {
                                        self.probe_cost(path, ProbeScale::Full, source_size)
                                    })
                                })
                        });
                    }
                    value
                });
                let representation =
                    if preference == MediaPreference::Proxies || (playing && !original_online) {
                        PolicyRepresentation::Proxy
                    } else if !playing {
                        PolicyRepresentation::Original
                    } else if let (Some(profile), Some(measured)) = (profile, measured) {
                        let availability = if ready_proxy.is_some() {
                            MediaAvailability::Online
                        } else {
                            MediaAvailability::ProxyPending
                        };
                        let decision = choose_representation(AutomaticInput {
                            intent: RepresentationIntent::Automatic,
                            resolution: ResolutionIntent::Automatic,
                            moment: PlaybackMoment::Start,
                            availability,
                            current: PolicyRepresentation::Original,
                            proxy_ready: ready_proxy.is_some(),
                            reduced_half_available: measured.reduced_half_p95_ms.is_some(),
                            reduced_quarter_available: measured.reduced_quarter_p95_ms.is_some(),
                            decode: measured,
                            profile,
                            fps,
                            viewer_size,
                            source_size,
                        });
                        decision
                            .representation
                            .unwrap_or(PolicyRepresentation::Original)
                    } else if known_slow {
                        PolicyRepresentation::Proxy
                    } else {
                        PolicyRepresentation::Original
                    };
                let choice = PlaybackChoice {
                    proxy: if representation == PolicyRepresentation::Proxy {
                        ready_proxy
                    } else {
                        None
                    },
                    proxy_stamp: None,
                    decode_scale: match representation {
                        PolicyRepresentation::ReducedHalf => Some(0.5),
                        PolicyRepresentation::ReducedQuarter => Some(0.25),
                        _ => None,
                    },
                    fallback_decode_scale: if representation == PolicyRepresentation::ReducedQuarter
                    {
                        measured
                            .and_then(|sample| {
                                profile.and_then(|machine| {
                                    let frame_budget = 1000.0 / fps;
                                    let margin = if machine.on_battery { 0.75 } else { 0.9 };
                                    sample.reduced_half_p95_ms.filter(|cost| {
                                        cost.is_finite()
                                            && *cost >= 0.0
                                            && *cost + machine.gpu_frame_p95_ms
                                                <= frame_budget * margin
                                    })
                                })
                            })
                            .map(|_| 0.5)
                    } else {
                        None
                    },
                };
                let choice = PlaybackChoice {
                    proxy_stamp: choice.proxy.as_deref().and_then(file_stamp),
                    ..choice
                };
                if playing
                    && !(choice.proxy.is_none()
                        && (preference == MediaPreference::Proxies || !original_online))
                {
                    self.choices.insert(choice_key, choice.clone());
                }
                if let Some(path) = choice.proxy {
                    proxies.insert(id, path);
                } else if let Some(scale) = choice.decode_scale {
                    reduced.insert(id, (scale, choice.fallback_decode_scale));
                }
            }
        }
        // Expired sources do not accumulate as projects are edited/relinked.
        if self.entries.len() > 4096 {
            self.entries.clear();
        }
        if self.probe_entries.len() > 4096 {
            self.probe_entries.retain(|_, entry| entry.cost.is_none());
        }
        ProxyResolver {
            original: inner,
            proxies,
            forced,
            reduced,
            preference,
            used: Mutex::new(Representation::Original),
            assets_used: Default::default(),
        }
    }

    pub fn finish(&mut self, resolver: &ProxyResolver<'_>) {
        let used = resolver
            .assets_used
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.choices.retain(|(asset, _, _), _| used.contains(asset));
    }
}

fn is_deleted(
    value: &serde_json::Value,
    project: &str,
    asset: &str,
    hash: Option<&str>,
    format: ProxyFormat,
    size: ProxySize,
) -> bool {
    if value.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return true;
    }
    let Some(entry) = value
        .get("deleted")
        .and_then(|v| v.get(project))
        .and_then(|v| v.get(asset))
    else {
        return false;
    };
    entry.get("format") == serde_json::to_value(format).ok().as_ref()
        && entry.get("size") == serde_json::to_value(size).ok().as_ref()
        && (entry.get("hash").is_some_and(|v| v.is_null())
            || entry.get("hash").and_then(|v| v.as_str()) == hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_probe_waits_for_export_and_cancels_during_shutdown() {
        let state = Arc::new(ProbeState::default());
        state.exporting.store(true, Ordering::Release);
        let worker = Arc::clone(&state);
        let (tx, rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            tx.send(wait_for_probe_idle(&worker)).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(40)).is_err());
        state.exporting.store(false, Ordering::Release);
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap().is_ok());
        join.join().unwrap();

        state.playing.store(true, Ordering::Release);
        state.stop.store(true, Ordering::Release);
        assert!(matches!(
            wait_for_probe_idle(&state),
            Err(pm_media::error::MediaError::Cancelled)
        ));
    }

    #[test]
    fn adjacent_instances_of_one_asset_have_distinct_playback_choices() {
        let value = pm_model::json::parse(
            r#"{"id":"project","compId":"main","layers":[
                {"id":"first","type":"video","on":true,"from":0,"dur":1,"d":{"asset":"a"}},
                {"id":"second","type":"video","on":true,"from":1,"dur":1,"d":{"asset":"a"}}
            ]}"#,
        )
        .unwrap();
        let doc = pm_model::Document::new(Some(value), 1);
        assert_eq!(active_clip_signature(&doc, "main", "a", 0.5), "first");
        assert_eq!(active_clip_signature(&doc, "main", "a", 1.5), "second");
        assert_eq!(active_clip_signature(&doc, "main", "a", 2.0), "");
    }

    #[test]
    fn nested_instances_follow_reversed_source_time_and_keep_their_ancestry() {
        let value = pm_model::json::parse(
            r#"{"id":"project","compId":"main","layers":[
                {"id":"left","type":"precomp","on":true,"from":0,"dur":2,"d":{"comp":"child","trim":2,"speed":-1}},
                {"id":"right","type":"precomp","on":true,"from":2,"dur":2,"d":{"comp":"child","trim":2,"speed":-1}}
            ],"comps":{"child":{"id":"child","layers":[
                {"id":"early","type":"video","on":true,"from":0,"dur":1,"d":{"asset":"a"}},
                {"id":"late","type":"video","on":true,"from":1,"dur":1,"d":{"asset":"a"}}
            ]}}}"#,
        ).unwrap();
        let doc = pm_model::Document::new(Some(value), 1);
        assert_eq!(active_clip_signature(&doc, "main", "a", 0.25), "left/late");
        assert_eq!(active_clip_signature(&doc, "main", "a", 1.25), "left/early");
        assert_eq!(active_clip_signature(&doc, "main", "a", 2.25), "right/late");
    }

    #[test]
    fn object_image_sequence_is_eligible_for_proxy_only() {
        let value = pm_model::json::parse(
            r#"{"kind":"image","imageSequence":{"fps":24,"frames":3,"originalHash":"aaa"}}"#,
        )
        .unwrap();
        let doc = pm_model::Document::new(Some(value), 1);
        let resolver = ProxyResolver {
            original: &MediaMap::default(),
            proxies: BTreeMap::new(),
            forced: Vec::new(),
            reduced: BTreeMap::new(),
            preference: MediaPreference::Proxies,
            used: Mutex::new(Representation::Original),
            assets_used: Mutex::new(Vec::new()),
        };
        assert_eq!(AssetResolver::path(&resolver, "sequence", doc.root()), None);
    }

    #[test]
    fn integrity_result_is_installed_only_after_a_nonblocking_poll() {
        let mut media = ProxyMedia::default();
        let key = "sample:proResProxy:half".to_string();
        media.entries.insert(
            key.clone(),
            LookupEntry {
                checked: Instant::now(),
                nonce: 7,
                result: None,
                watch_until: None,
            },
        );
        let (tx, rx) = mpsc::channel();
        media.lookup_rx = Some(rx);
        assert!(media.has_pending_lookups());
        assert!(!media.poll_updates());
        tx.send(LookupResult {
            key: key.clone(),
            nonce: 6,
            found: None,
        })
        .unwrap();
        assert!(!media.poll_updates());
        assert!(media.has_pending_lookups());
        tx.send(LookupResult {
            key: key.clone(),
            nonce: 7,
            found: None,
        })
        .unwrap();
        assert!(media.poll_updates());
        assert!(!media.has_pending_lookups());
        assert!(matches!(
            media.entries.get(&key).unwrap().result,
            Some(None)
        ));
    }
    #[test]
    fn a_watched_negative_lookup_retries_after_its_short_cache_expires() {
        let mut media = ProxyMedia::default();
        let key = "source:ProResProxy:Half".to_string();
        media.entries.insert(
            key.clone(),
            LookupEntry {
                checked: Instant::now() - Duration::from_secs(2),
                nonce: 1,
                result: Some(None),
                watch_until: Some(Instant::now() + Duration::from_secs(1)),
            },
        );
        assert!(media.poll_updates());
        assert!(!media.entries.contains_key(&key));

        media.entries.insert(
            key.clone(),
            LookupEntry {
                checked: Instant::now() - Duration::from_secs(2),
                nonce: 2,
                result: Some(None),
                watch_until: None,
            },
        );
        assert!(!media.poll_updates());
        assert!(media.entries.contains_key(&key));
    }
    #[test]
    fn deletion_is_scoped_to_project_source_and_recipe() {
        let value = serde_json::json!({"version":1,"deleted":{"project":{"asset":{"hash":"source","format":"proResProxy","size":"half","at":0}}}});
        assert!(is_deleted(
            &value,
            "project",
            "asset",
            Some("source"),
            ProxyFormat::ProResProxy,
            ProxySize::Half
        ));
        assert!(!is_deleted(
            &value,
            "other",
            "asset",
            Some("source"),
            ProxyFormat::ProResProxy,
            ProxySize::Half
        ));
        assert!(!is_deleted(
            &value,
            "project",
            "asset",
            Some("relinked"),
            ProxyFormat::ProResProxy,
            ProxySize::Half
        ));
        assert!(!is_deleted(
            &value,
            "project",
            "asset",
            Some("source"),
            ProxyFormat::ProResProxy,
            ProxySize::Quarter
        ));
        assert!(is_deleted(
            &serde_json::Value::Null,
            "project",
            "asset",
            Some("source"),
            ProxyFormat::ProResProxy,
            ProxySize::Half
        ));
    }
}

pub(super) struct ProxyResolver<'a> {
    /// The host's resolver for originals (and Blender map PNGs).
    original: &'a dyn AssetResolver,
    proxies: BTreeMap<String, PathBuf>,
    forced: Vec<String>,
    reduced: BTreeMap<String, (f64, Option<f64>)>,
    preference: MediaPreference,
    used: Mutex<Representation>,
    assets_used: Mutex<Vec<String>>,
}
impl ProxyResolver<'_> {
    pub fn representation(&self) -> Representation {
        *self.used.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn signature(&self) -> String {
        let sources: Vec<_> = self
            .proxies
            .iter()
            .map(|(id, path)| (id, path, file_stamp(path)))
            .collect();
        format!(
            "{:?}:{:?}:{sources:?}:{:?}",
            self.preference, self.forced, self.reduced
        )
    }
}
impl AssetResolver for ProxyResolver<'_> {
    fn is_proxy(&self, asset: &str) -> bool {
        self.proxies.contains_key(asset)
    }
    fn unavailable_reason(&self, asset: &str, record: Option<&Node>) -> Option<String> {
        (self.preference == MediaPreference::Proxies
            && !self.forced.iter().any(|id| id == asset)
            && record.is_some_and(|node| {
                text(node.get("kind")).as_deref() == Some("video")
                    || matches!(
                        node.get("imageSequence").map(Node::kind),
                        Some(pm_model::NodeKind::Bool(true) | pm_model::NodeKind::Object(_))
                    )
            }))
        .then(|| "Proxy pending. Create a proxy or choose Originals.".to_string())
    }

    fn force_full_original(&self, asset: &str) -> bool {
        self.forced.iter().any(|id| id == asset)
    }

    fn preview_decode_scale(&self, asset: &str) -> Option<f64> {
        self.reduced.get(asset).map(|(scale, _)| *scale)
    }

    fn preview_decode_fallback_scale(&self, asset: &str) -> Option<f64> {
        self.reduced.get(asset).and_then(|(_, fallback)| *fallback)
    }

    fn path(&self, asset: &str, record: Option<&Node>) -> Option<PathBuf> {
        self.assets_used
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(asset.to_owned());
        if let Some(path) = self.proxies.get(asset) {
            *self.used.lock().unwrap_or_else(|e| e.into_inner()) = Representation::Proxy;
            return Some(path.clone());
        }
        if self.preference == MediaPreference::Proxies
            && !self.forced.iter().any(|id| id == asset)
            && record.is_some_and(|r| {
                text(r.get("kind")).is_some_and(|kind| kind == "video" || kind == "imageSequence")
                    || matches!(
                        r.get("imageSequence").map(Node::kind),
                        Some(pm_model::NodeKind::Bool(true) | pm_model::NodeKind::Object(_))
                    )
            })
        {
            // Proxy Only is a named missing-media layer, never an original.
            return None;
        }
        self.original.path(asset, record)
    }

    fn blender_png(
        &self,
        asset: &str,
        source: &std::path::Path,
    ) -> Result<Option<std::sync::Arc<Vec<u8>>>, String> {
        self.original.blender_png(asset, source)
    }
}

/// Content hashes main assessed as unable to play in real time on this
/// machine (`{"version":1,"machine":…,"results":{hash:{"needsProxy":…}}}`).
fn assessed_slow_hashes(
    value: &serde_json::Value,
    machine: Option<&str>,
) -> std::collections::HashSet<String> {
    if value["version"] != 1 || value["machine"].as_str() != machine {
        return Default::default();
    }
    value["results"]
        .as_object()
        .map(|results| {
            results
                .iter()
                .filter(|(hash, row)| {
                    hash.len() == 64
                        && hash.bytes().all(|c| c.is_ascii_hexdigit())
                        && row["needsProxy"].as_bool() == Some(true)
                })
                .map(|(hash, _)| hash.clone())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod assessment_tests {
    use super::assessed_slow_hashes;

    #[test]
    fn only_this_machines_slow_assessments_select_playback_proxies() {
        let slow = "a".repeat(64);
        let fast = "b".repeat(64);
        let value = serde_json::json!({"version":1,"machine":"darwin:arm64:M","results":{
            slow.clone(): {"needsProxy": true, "at": 1}, fast.clone(): {"needsProxy": false, "at": 1}, "bad": {"needsProxy": true}
        }});
        let hashes = assessed_slow_hashes(&value, Some("darwin:arm64:M"));
        assert!(hashes.contains(&slow));
        assert!(!hashes.contains(&fast));
        assert_eq!(hashes.len(), 1);
        assert!(assessed_slow_hashes(&value, Some("darwin:x64:Intel")).is_empty());
        assert!(assessed_slow_hashes(&serde_json::json!({"version":2,"machine":"darwin:arm64:M","results":{}}), Some("darwin:arm64:M")).is_empty());
    }
}
