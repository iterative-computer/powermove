//! Disk render cache (plan section 1, pro media): a composition's frames as
//! the paused viewer draws them, rendered in the background, kept in
//! Storage › Renders, and shown by playback and scrubbing instead of
//! rendering them again.
//!
//! - **Identity.** A frame is keyed by a global fingerprint (the contributed
//!   definitions, each media file's content (sampled) and stamp, the
//!   installed fonts, the viewer's output tone, the running renderer binary,
//!   and (with raster overrides) the override wrappers' code) and by the
//!   digest of the document parts that frame draws from
//!   (`crate::frame_inputs`): an edit misses only the frames it touched.
//!   Frames of an earlier state stay until the limit (least recently used
//!   first) or Clear… removes them.
//! - **Rendering** runs in an `--engine-job` child (`jobs::fill_cache`) with
//!   the export's admission gate and its exact override answers, writing to
//!   the folder captured when it started. View › Render Selection renders a
//!   range; **background rendering** renders, once the viewer has been idle
//!   for the delay in Settings, the chunks of the open composition that
//!   can't play in real time (measured: by the viewer as it plays, and by
//!   the child's probes), playhead first, then the work area, then the
//!   rest. Its child gets no GPU time and no override answers until the
//!   viewer has been idle for the delay again, so any interaction pauses it
//!   at once; an edit that changes its frames, an export, or Settings
//!   stops it. One fill runs at a time; Render Selection replaces a
//!   background one.
//! - The viewer's engine thread never renders, reads or writes cache frames
//!   itself: reads come from a reader thread ahead of the playhead.
//! - **Generations** fence fingerprints and indexes: they land only if no
//!   observation, storage change or deletion came after they started. A
//!   fill's frames land by key, while the storage they were written to is
//!   current.
//! - The shared state is never held across disk IO; the render bar reads a
//!   snapshot published on every change.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use pm_promedia::cache::{BarState, RenderCache, RenderKey};
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::definitions::DefinitionSnapshot;
use pm_time::Rate;
use serde::{Deserialize, Serialize};

use crate::render::Scene;

/// Frames on disk: RGBA16F rows, top-down, the export's premultiplied
/// working values rounded to the half floats the viewer's surface holds,
/// coded losslessly (`crate::frame_codec`).
pub const FORMAT: &str = "rgba16f-export-lzfse-v4";
/// Media stamps are looked at again at most this often.
const MEDIA_RECHECK: Duration = Duration::from_secs(2);
/// The same storage is indexed again at most this often.
const REINDEX_EVERY: Duration = Duration::from_secs(2);
/// Frames read ahead of the playhead, and kept read.
const AHEAD: i64 = 6;
const READY_FRAMES: usize = 8;
/// Reader threads.
const READERS: usize = 3;
/// Frames kept read at a paused playhead, so a play starts from them.
const WARM: i64 = 3;
/// How long a playing frame waits for its frame on disk to be read. A
/// frame is on disk because drawing it takes longer than its interval, so
/// a short wait for the reader always beats drawing it again.
pub(crate) const PATIENCE: Duration = Duration::from_millis(40);
/// How long a paused frame waits for its frame on disk: longer than a
/// playing one, and still far less than drawing and refining it.
pub(crate) const PAUSED_PATIENCE: Duration = Duration::from_millis(250);
/// Playing frames in a row that give way to the reader before one is drawn
/// after all (a disk too slow to play from must not freeze the viewer).
pub(crate) const LATE_AT_MOST: u32 = 6;
/// Background rendering judges and renders the composition in chunks of
/// about this long.
const CHUNK_SECONDS: f64 = 0.5;
/// A frame that costs this share of its interval or more can't be counted
/// on to play in real time (the viewer also presents; calibrated on Paper
/// against the frames live playback dropped).
const BUDGET_SHARE: f64 = 0.9;
/// The playhead's neighbourhood background rendering starts with.
const NEAR_BEHIND: f64 = 1.;
const NEAR_AHEAD: f64 = 5.;
/// A background fill waits paused for the person at most this long; then
/// its child stops (and gives back its memory) until they are idle again.
const PAUSED_AT_MOST: Duration = Duration::from_secs(20);
/// The volume's free space is looked at again at most this often.
const FLOOR_EVERY: Duration = Duration::from_secs(3);
/// A background fill that failed is not tried again on the same state
/// sooner than this.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// The cost a frame playback dropped stands for: more than any budget.
const DROPPED_US: f64 = 1e9;
/// Measured costs and refusals kept, by frame key, at most.
const KNOWN_AT_MOST: usize = 200_000;

/// What a fill child renders: frames by time and key, into `dir`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheFill {
    pub dir: PathBuf,
    pub quota: u64,
    pub width: u32,
    pub height: u32,
    pub frames: Vec<CacheFrame>,
    /// Background rendering: GPU microseconds a frame may cost and still
    /// play in real time; chunks whose probes cost less are not stored.
    #[serde(default)]
    pub budget_us: Option<f64>,
    /// One frame's length in ticks: a probe is warmed by the frame before it.
    #[serde(default)]
    pub frame_ticks: i64,
    /// Probes draw at this share of the composition's size: the viewer's.
    #[serde(default = "whole_size")]
    pub probe_scale: f64,
}
fn whole_size() -> f64 {
    1.
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheFrame {
    pub ticks: i64,
    pub key: RenderKey,
    /// The chunk it belongs to (a chunk's frames are contiguous in the
    /// fill, its probes first).
    #[serde(default)]
    pub chunk: u32,
    #[serde(default)]
    pub probe: bool,
}
/// What a fill child did with its frame `at` (an index into
/// [`CacheFill::frames`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameReport {
    pub at: u32,
    /// Its GPU time, when it was drawn.
    pub cost_us: Option<f64>,
    pub stored: bool,
    /// It can't be drawn exactly (a layer failed): not stored, not asked
    /// for again in this state.
    pub refused: bool,
}

/// A fill child's failure when the volume is nearly full.
pub const DISK_FULL: &str =
    "Rendering stopped because the disk is almost full. Free some space and render again.";
/// Background rendering leaves at least this much free on the volume that
/// holds rendered frames: below it no fill starts and a running one stops.
/// (A render the person asks for keeps the export's own reserve.)
pub const BACKGROUND_FLOOR: u64 = 20 << 30;
/// Why background rendering is not rendering while the volume is that full.
pub const BACKGROUND_LOW_DISK: &str = "Background rendering is paused: less than 20 GB is free on the disk that holds rendered frames.";
/// A fill child's failure when kept frames fill Settings › Media › Render Files Limit.
pub const LIMIT_FULL: &str =
    "Kept rendered frames fill the Render Files Limit. Raise the limit or delete rendered frames.";

/// One media file's stamp: path, length, modification time, inode.
type Stamp = (PathBuf, u64, u128, u64);
/// A frame's key digest: the global fingerprint, its inputs' digest and its
/// time.
type FrameId = [u8; 32];

/// The composition state a fingerprint is taken of.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    doc: usize,
    revision: u64,
    content: u64,
    comp: String,
    tone: Option<String>,
    overrides: Option<u64>,
    /// The engine's font registry generation: an installed or removed font
    /// is observed again (the fingerprint reads the font files).
    fonts: Option<u64>,
    media: Vec<Stamp>,
}
impl Identity {
    fn scene_matches(&self, scene: &Scene) -> bool {
        self.doc == Arc::as_ptr(&scene.doc) as usize
            && self.revision == scene.doc.revision()
            && self.content == scene.content
            && self.comp == scene.comp
    }
}

/// What a fill renders from: the viewer's own scene.
#[derive(Clone)]
struct Source {
    scene: Scene,
    definitions: Vec<DefinitionSnapshot>,
    doc_id: u64,
}

/// The open composition's frames.
struct Frames {
    identity: Identity,
    source: Source,
    /// `None` while it is being taken.
    fingerprint: Option<[u8; 32]>,
    /// Each frame's key digest (empty while the fingerprint is taken).
    ids: Arc<Vec<FrameId>>,
    size: Size,
    rate: Rate,
    count: i64,
    present: HashSet<i64>,
    kept: HashSet<i64>,
}
impl Frames {
    fn ticks(&self, n: i64) -> Option<Ticks> {
        Ticks::new(self.rate.frame_start(n).ok()?).ok()
    }
    /// The frame starting exactly at `time`.
    fn index(&self, time: Ticks) -> Option<i64> {
        let n = self.rate.frame_of(time.get());
        (self.ticks(n)? == time && (0..self.count).contains(&n)).then_some(n)
    }
    /// The frame showing at `time`.
    fn at(&self, time: Ticks) -> i64 {
        self.rate
            .frame_of(time.get())
            .clamp(0, (self.count - 1).max(0))
    }
    fn id(&self, n: i64) -> Option<FrameId> {
        self.fingerprint?;
        self.ids.get(usize::try_from(n).ok()?).copied()
    }
    fn key(&self, n: i64) -> Option<RenderKey> {
        Some(RenderKey {
            graph: self.id(n)?,
            media: self.fingerprint?,
            ticks: self.ticks(n)?.get(),
            width: self.size.width,
            height: self.size.height,
            format: FORMAT.into(),
            renderer_version: renderer_identity().into(),
        })
    }
    fn seconds(&self, n: i64) -> f64 {
        self.ticks(n)
            .map_or(0., |t| t.get() as f64 / TICKS_PER_SECOND as f64)
    }
    /// GPU microseconds a frame may cost and still play in real time.
    fn budget_us(&self) -> f64 {
        (self.seconds(1) - self.seconds(0)) * 1e6 * BUDGET_SHARE
    }
    fn chunk_len(&self) -> i64 {
        let fps = 1. / (self.seconds(1) - self.seconds(0)).max(1e-6);
        ((fps * CHUNK_SECONDS).round() as i64).max(1)
    }
}

/// Whether a chunk can play in real time, as far as is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    Unknown,
    Light,
    Heavy,
    /// A frame in it can't be drawn exactly: left alone.
    Refused,
}

/// What is known about frames' cost, beyond the frames themselves.
#[derive(Default)]
struct Known {
    /// GPU (or viewer frame) microseconds, by frame key: the most measured.
    costs: HashMap<FrameId, f64>,
    refused: HashSet<FrameId>,
    /// The last cost measured at each frame of the open composition,
    /// whatever its key: an edit inside a heavy range still needs render.
    hints: HashMap<i64, f64>,
    /// The composition the hints are of: id, rate, frame count.
    hints_of: Option<(String, Rate, i64)>,
}
impl Known {
    fn cost(&mut self, frames: &Frames, n: Option<i64>, id: FrameId, cost: f64) {
        if self.costs.len() >= KNOWN_AT_MOST {
            self.costs.clear();
        }
        // The same key is the same inputs: the worst measure stands (a
        // viewer frame measured late under load still needs render).
        let worst = self.costs.entry(id).or_insert(cost);
        *worst = worst.max(cost);
        if let Some(n) = n
            && frames.id(n) == Some(id)
        {
            self.hints.insert(n, cost);
        }
    }
    fn refuse(&mut self, id: FrameId) {
        if self.refused.len() >= KNOWN_AT_MOST {
            self.refused.clear();
        }
        self.refused.insert(id);
    }
    /// Chunk `[a, b)`'s need: frames playback dropped, or all of at least
    /// two measured frames (one in a one-frame chunk) at the budget or
    /// over, make it heavy (one slow sample among fast ones is noise, not a
    /// range that can't play); enough measured otherwise make it light;
    /// unmeasured under its keys, what its frames measured before an edit
    /// decides the same way.
    fn need(&self, frames: &Frames, a: i64, b: i64) -> Need {
        let budget = frames.budget_us();
        let ids: Vec<FrameId> = (a..b).filter_map(|n| frames.id(n)).collect();
        if ids.iter().any(|id| self.refused.contains(id)) {
            return Need::Refused;
        }
        let enough = (b - a).min(2) as usize;
        let heavy = |costs: &[f64]| {
            costs.iter().any(|c| *c >= DROPPED_US)
                || (costs.len() >= enough && costs.iter().all(|c| *c >= budget))
        };
        let known: Vec<f64> = ids
            .iter()
            .filter_map(|id| self.costs.get(id).copied())
            .collect();
        let before: Vec<f64> = (a..b).filter_map(|n| self.hints.get(&n).copied()).collect();
        if heavy(&known) {
            Need::Heavy
        } else if known.len() >= enough {
            Need::Light
        } else if heavy(&before) {
            Need::Heavy
        } else {
            Need::Unknown
        }
    }
}

/// Settings › General › Playback: Background Rendering and its delay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Background {
    pub enabled: bool,
    pub delay: Duration,
    /// Free space it leaves on the volume ([`BACKGROUND_FLOOR`]).
    pub floor: u64,
}
impl Default for Background {
    fn default() -> Self {
        Self {
            enabled: true,
            delay: Duration::from_secs(5),
            floor: BACKGROUND_FLOOR,
        }
    }
}
/// Answers a fill's raster overrides for a document (see [`BackgroundHost`]).
pub type Pin = Arc<dyn Fn(u64, &mut crate::jobs::JobSession) + Send + Sync>;
/// What background rendering needs from the app.
#[derive(Clone)]
pub struct BackgroundHost {
    /// The executable whose `--engine-job` renders.
    pub executable: PathBuf,
    /// Answers a fill's raster overrides for document `doc_id`, as Render
    /// Selection does.
    pub pin: Pin,
    /// Something else is rendering for the person (an export): background
    /// rendering stops and waits.
    pub busy: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// What the timeline's render bar and the Render task show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderBars {
    /// Rises whenever anything below changes.
    pub revision: u64,
    pub composition: Option<String>,
    /// Seconds, half-open, ordered; stretches neither rendered nor known to
    /// need rendering are left out.
    pub bars: Vec<(f64, f64, BarState)>,
    /// The current fill: frames queued and rendered, whether it runs, and why
    /// it stopped (`disk_full`: the volume or the limit was full).
    pub queued: u64,
    pub done: u64,
    pub running: bool,
    /// The current fill is background rendering, not Render Selection.
    pub background: bool,
    pub error: Option<String>,
    pub disk_full: bool,
}

#[derive(Default)]
struct Fill {
    serial: u64,
    /// The observation it rendered from.
    generation: u64,
    background: bool,
    /// Frames shown as rendering (Render Selection's).
    queued: BTreeSet<i64>,
    total: u64,
    done: u64,
    running: bool,
    error: Option<String>,
    disk_full: bool,
    ended: Option<Instant>,
}

type Notify = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    notify: Option<Notify>,
    wake: Option<std::sync::mpsc::SyncSender<()>>,
    root: Option<(PathBuf, u64)>,
    /// The volume's free space, as a test says it is.
    free: Option<Arc<dyn Fn() -> u64 + Send + Sync>>,
    /// The folder whose files are named by their bytes
    /// ([`DiskFrames::set_content_named`]).
    content_named: Option<PathBuf>,
    cache: Option<Arc<RenderCache>>,
    /// Rises with every new observation, storage or deletion: fences
    /// fingerprints and indexes that started before it.
    generation: u64,
    /// Rises with every storage change or deletion: fences fills' frames.
    storage: u64,
    frames: Option<Frames>,
    known: Known,
    media_checked: Option<Instant>,
    indexed: Option<Instant>,
    fill: Fill,
    revision: u64,
    /// The frame read last, and whether reads move backward (skimming or
    /// scrubbing back): read-ahead follows the motion.
    last_read: Option<i64>,
    backward: bool,
    /// The time the viewer drew last, and the size it rendered at.
    playhead: Option<Ticks>,
    viewer: Option<Size>,
    activity: Option<crate::jobs::Activity>,
    background: Background,
    host: Option<BackgroundHost>,
}

struct Running {
    /// The fill's child, if it started.
    job: Option<crate::jobs::JobControl>,
    thread: std::thread::JoinHandle<()>,
    background: bool,
}

struct Inner {
    state: Mutex<State>,
    published: Mutex<Arc<RenderBars>>,
    running: Mutex<Option<Running>>,
    reader: Reader,
    /// The background delay in milliseconds, read live by fill children.
    delay_ms: Arc<AtomicU64>,
    scheduling: std::sync::Once,
}

/// What Keep Rendered (or its undo) did to a range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    /// Rendered frames that changed state.
    pub changed: u64,
    /// Rendered frames that already were in the asked state.
    pub already: u64,
}

/// The engine's render cache, shared by the engine thread (observation,
/// reads) and the app (commands, the render bar).
#[derive(Clone)]
pub struct DiskFrames {
    inner: Arc<Inner>,
}
impl Default for DiskFrames {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::default(),
                published: Mutex::default(),
                running: Mutex::default(),
                reader: Reader::start(),
                delay_ms: Arc::new(AtomicU64::new(
                    Background::default().delay.as_millis() as u64
                )),
                scheduling: std::sync::Once::new(),
            }),
        }
    }
}

/// Something the render bar shows changed: publish a snapshot and tell
/// whoever listens (never blocking).
fn bump(state: &mut State, published: &Mutex<Arc<RenderBars>>) {
    state.revision += 1;
    *published.lock().unwrap() = Arc::new(snapshot(state));
    if let Some(notify) = &state.notify {
        notify();
    }
    if let Some(wake) = &state.wake {
        let _ = wake.try_send(());
    }
}

fn snapshot(state: &State) -> RenderBars {
    let fill = &state.fill;
    let mut out = RenderBars {
        revision: state.revision,
        queued: fill.total,
        done: fill.done,
        running: fill.running,
        background: fill.background,
        error: fill.error.clone(),
        disk_full: fill.disk_full,
        ..RenderBars::default()
    };
    let Some(frames) = state.frames.as_ref() else {
        return out;
    };
    out.composition = Some(frames.identity.comp.clone());
    let mut marks: Vec<(i64, BarState)> = frames
        .present
        .iter()
        .map(|n| {
            let kept = frames.kept.contains(n);
            (
                *n,
                if kept {
                    BarState::Kept
                } else {
                    BarState::Rendered
                },
            )
        })
        .chain(
            fill.queued
                .iter()
                .filter(|n| !frames.present.contains(n))
                .map(|n| (*n, BarState::Rendering)),
        )
        .collect();
    // Chunks known not to play in real time: needs render.
    if frames.fingerprint.is_some() {
        let step = frames.chunk_len();
        for a in (0..frames.count).step_by(step as usize) {
            let b = (a + step).min(frames.count);
            if state.known.need(frames, a, b) == Need::Heavy {
                marks.extend(
                    (a..b)
                        .filter(|n| !frames.present.contains(n) && !fill.queued.contains(n))
                        .map(|n| (n, BarState::Missing)),
                );
            }
        }
    }
    marks.sort_by_key(|m| m.0);
    for (n, bar) in marks {
        let (from, to) = (frames.seconds(n), frames.seconds(n + 1));
        match out.bars.last_mut() {
            Some(last) if last.2 == bar && (last.1 - from).abs() < 1e-9 => last.1 = to,
            _ => out.bars.push((from, to, bar)),
        }
    }
    out
}

impl DiskFrames {
    /// Drop CPU read-ahead while stopped, except the few frames at the
    /// paused playhead a play starts from ([`WARM`]); queued and in-flight
    /// reads beyond them are fenced by the wanted range. Authored/kept
    /// files and scene identity stay.
    pub(crate) fn release_idle_reads(&self) {
        let (generation, at) = self.with(|s| ((s.generation, s.last_read), false));
        match at {
            Some(n) => self.inner.reader.keep(generation, n, n + WARM - 1),
            None => self.inner.reader.clear(generation),
        }
    }

    pub(crate) fn ready_bytes(&self) -> u64 {
        self.inner
            .reader
            .ready
            .lock()
            .unwrap()
            .frames
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .sum()
    }

    /// Called (on any thread, never blocking) whenever the render bar changes.
    pub fn set_notify(&self, notify: Option<Notify>) {
        self.inner.state.lock().unwrap().notify = notify;
    }
    /// Wakes the engine thread when frames land (playback reads them).
    pub(crate) fn set_wake(&self, wake: std::sync::mpsc::SyncSender<()>) {
        self.inner.state.lock().unwrap().wake = Some(wake);
    }
    /// The viewer's activity: background rendering waits for it to be idle.
    pub(crate) fn set_activity(&self, activity: crate::jobs::Activity) {
        self.inner.state.lock().unwrap().activity = Some(activity);
    }
    /// Settings › General › Playback. Turning it off stops a background fill
    /// (on the scheduler's thread, never the caller's).
    pub fn set_background(&self, background: Background) {
        self.inner
            .delay_ms
            .store(background.delay.as_millis() as u64, Ordering::Release);
        self.inner.state.lock().unwrap().background = background;
    }
    /// What background rendering renders with; set once the app can answer
    /// overrides. Starts the scheduler.
    pub fn set_background_host(&self, host: Option<BackgroundHost>) {
        self.inner.state.lock().unwrap().host = host;
        let weak = Arc::downgrade(&self.inner);
        self.inner.scheduling.call_once(|| {
            std::thread::Builder::new()
                .name("pm-render-background".into())
                .spawn(move || schedule(weak))
                .ok();
        });
    }
    fn with<R>(&self, f: impl FnOnce(&mut State) -> (R, bool)) -> R {
        let mut state = self.inner.state.lock().unwrap();
        let (r, changed) = f(&mut state);
        if changed {
            bump(&mut state, &self.inner.published);
        }
        r
    }

    /// Drop a closed project's scene and read-ahead. The generation fences
    /// any fingerprint or disk read that was already running.
    pub(crate) fn forget_document(&self, doc_id: u64) {
        let generation = self.with(|s| {
            if s.frames.as_ref().is_none_or(|f| f.source.doc_id != doc_id) {
                return (None, false);
            }
            s.generation += 1;
            s.frames = None;
            s.media_checked = None;
            s.indexed = None;
            s.last_read = None;
            (Some(s.generation), true)
        });
        if let Some(generation) = generation {
            self.inner.reader.clear(generation);
            self.cancel_fill();
        }
    }

    /// The folder whose files are named by their bytes (the app's media
    /// cache objects). Such a file's name is its content; its modification
    /// time is the media cache's recency mark, set again at every open, and
    /// its inode changes when it is extracted again. Neither is part of a
    /// frame's identity: with them, every launch orphaned the frames the
    /// one before had rendered.
    pub fn set_content_named(&self, dir: Option<PathBuf>) {
        self.with(|s| {
            s.content_named = dir;
            ((), false)
        });
    }

    /// Storage › Renders: the folder and the Render Files Limit (`None`: no
    /// render cache). The same folder again finds its frames again (Clear…
    /// may have removed some). Disk work: call off the main thread.
    pub fn set_storage(&self, root: Option<(PathBuf, u64)>) -> std::io::Result<()> {
        let same = self.with(|s| (s.root == root, false));
        if same {
            return self.reindex(false);
        }
        // A fill writing into the old folder stops first.
        self.cancel_fill();
        let cache = match &root {
            Some((dir, quota)) => Some(Arc::new(RenderCache::open(dir, *quota)?)),
            None => None,
        };
        let generation = self.with(|s| {
            s.root = root;
            s.cache = cache;
            s.generation += 1;
            s.storage += 1;
            // Observed again (and fingerprinted against the new folder).
            s.frames = None;
            s.fill = Fill::default();
            (s.generation, true)
        });
        self.inner.reader.clear(generation);
        Ok(())
    }

    /// Finds the current state's frames on disk again (`force`: whenever
    /// asked, else at most every [`REINDEX_EVERY`]). Disk work: off the
    /// main thread.
    fn reindex(&self, force: bool) -> std::io::Result<()> {
        let plan = self.with(|s| {
            if !force && s.indexed.is_some_and(|at| at.elapsed() < REINDEX_EVERY) {
                return (None, false);
            }
            s.indexed = Some(Instant::now());
            let (Some(cache), Some(frames)) = (s.cache.clone(), s.frames.as_ref()) else {
                return (None, false);
            };
            let keys: Vec<(i64, RenderKey)> = (0..frames.count)
                .filter_map(|n| Some((n, frames.key(n)?)))
                .collect();
            (Some((cache, keys, s.generation)), false)
        });
        let Some((cache, keys, generation)) = plan else {
            return Ok(());
        };
        let (present, kept) = index(&cache, &keys);
        self.with(|s| match s.frames.as_mut() {
            Some(frames) if s.generation == generation && frames.fingerprint.is_some() => {
                frames.present = present;
                frames.kept = kept;
                ((), true)
            }
            _ => ((), false),
        });
        Ok(())
    }

    /// The engine thread's current scene, drawn at `time`. A new
    /// composition state (scene, output tone, override wrappers, or, looked
    /// at every 2 s, a media file's stamp) takes a new fingerprint off this
    /// thread and finds its frames on disk. Returns whether it did.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe(
        &self,
        scene: &Scene,
        definitions: &[DefinitionSnapshot],
        overrides: Option<u64>,
        tone: Option<String>,
        fonts: Option<u64>,
        doc_id: u64,
        time: Ticks,
    ) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        state.playhead = Some(time);
        if state.cache.is_none() {
            return false;
        }
        let unchanged = state.frames.as_ref().is_some_and(|f| {
            f.identity.scene_matches(scene)
                && f.identity.tone == tone
                && f.identity.overrides == overrides
                && f.identity.fonts == fonts
        });
        let recheck = state
            .media_checked
            .is_none_or(|at| at.elapsed() >= MEDIA_RECHECK);
        if unchanged && !recheck {
            return false;
        }
        drop(state);
        // A few stats, every 2 s at most, outside the lock.
        let content_named = self.with(|s| (s.content_named.clone(), false));
        let media = stamps(&scene.media, content_named.as_deref());
        let mut state = self.inner.state.lock().unwrap();
        state.media_checked = Some(Instant::now());
        if unchanged
            && state
                .frames
                .as_ref()
                .is_some_and(|f| f.identity.media == media)
        {
            return false;
        }
        let Some((size, rate, count)) = composition(scene) else {
            state.frames = None;
            state.generation += 1;
            self.inner.reader.clear(state.generation);
            bump(&mut state, &self.inner.published);
            return true;
        };
        let identity = Identity {
            doc: Arc::as_ptr(&scene.doc) as usize,
            revision: scene.doc.revision(),
            content: scene.content,
            comp: scene.comp.clone(),
            tone,
            overrides,
            fonts,
            media,
        };
        let source = Source {
            scene: scene.clone(),
            definitions: definitions.to_vec(),
            doc_id,
        };
        state.generation += 1;
        let generation = state.generation;
        // Until its keys are known, the bar keeps showing what it showed.
        let (present, kept) = state
            .frames
            .take()
            .filter(|f| f.identity.comp == identity.comp && f.count == count)
            .map(|f| (f.present, f.kept))
            .unwrap_or_default();
        self.inner.reader.clear(generation);
        state.frames = Some(Frames {
            identity: identity.clone(),
            source: source.clone(),
            fingerprint: None,
            ids: Arc::default(),
            size,
            rate,
            count,
            present,
            kept,
        });
        let of = (identity.comp.clone(), rate, count);
        if state.known.hints_of.as_ref() != Some(&of) {
            state.known.hints.clear();
            state.known.hints_of = Some(of);
        }
        bump(&mut state, &self.inner.published);
        let cache = state.cache.clone().expect("checked above");
        drop(state);
        let weak = Arc::downgrade(&self.inner);
        std::thread::Builder::new()
            .name("pm-render-cache".into())
            .spawn(move || {
                let fingerprint = fingerprint(&source, &identity);
                let inputs = crate::frame_inputs::frame_digests(
                    &source.scene.doc,
                    &identity.comp,
                    rate,
                    count,
                    // Override code can read anything in the document.
                    identity.overrides.is_some(),
                )
                .unwrap_or_default();
                // A frame is its inputs at its time.
                let ids: Vec<FrameId> = inputs
                    .iter()
                    .enumerate()
                    .map(|(n, d)| {
                        let mut id = blake3::Hasher::new();
                        id.update(&fingerprint);
                        id.update(d);
                        id.update(&rate.frame_start(n as i64).unwrap_or(-1).to_le_bytes());
                        *id.finalize().as_bytes()
                    })
                    .collect();
                let template = Frames {
                    identity,
                    source,
                    fingerprint: Some(fingerprint),
                    ids: Arc::new(ids),
                    size,
                    rate,
                    count,
                    present: HashSet::new(),
                    kept: HashSet::new(),
                };
                let keys: Vec<(i64, RenderKey)> = (0..count)
                    .filter_map(|n| Some((n, template.key(n)?)))
                    .collect();
                let (present, kept) = index(&cache, &keys);
                let Some(inner) = weak.upgrade() else { return };
                let mut state = inner.state.lock().unwrap();
                // Only the observation it was taken for: anything since
                // (another observation, storage, a deletion) wins.
                if state.generation == generation {
                    state.frames = Some(Frames {
                        present,
                        kept,
                        ..template
                    });
                    bump(&mut state, &inner.published);
                }
            })
            .ok();
        true
    }

    /// The key digest of the frame `scene` shows at `time`, when known: a
    /// viewer frame's measured cost lands under it ([`Self::measured`]).
    pub(crate) fn frame_id(&self, scene: &Scene, time: Ticks) -> Option<(i64, FrameId)> {
        let state = self.inner.state.lock().unwrap();
        let frames = state.frames.as_ref()?;
        if !frames.identity.scene_matches(scene) {
            return None;
        }
        let n = frames.index(time)?;
        Some((n, frames.id(n)?))
    }
    /// The viewer drew at this size: background probes measure frames as
    /// it draws them.
    pub(crate) fn viewing(&self, size: Size) {
        self.inner.state.lock().unwrap().viewer = Some(size);
    }
    /// Playback skipped the frames between `before` and `n` (`id` is
    /// frame `n`'s key, so the skip is of the current state): they didn't
    /// play in real time and need render.
    pub(crate) fn dropped(&self, before: i64, (n, id): (i64, FrameId)) {
        let mut state = self.inner.state.lock().unwrap();
        let state = &mut *state;
        let Some(frames) = state.frames.as_ref().filter(|f| f.id(n) == Some(id)) else {
            return;
        };
        let mut changed = false;
        for k in before + 1..n {
            if let Some(kid) = frames.id(k) {
                let step = frames.chunk_len();
                let a = k - k % step;
                let b = (a + step).min(frames.count);
                let was = state.known.need(frames, a, b);
                state.known.cost(frames, Some(k), kid, DROPPED_US);
                changed |= state.known.need(frames, a, b) != was;
            }
        }
        if changed {
            bump(state, &self.inner.published);
        }
    }
    /// A frame known to cost `cost_us` (tests).
    #[cfg(test)]
    fn measured(&self, (n, id): (i64, FrameId), cost_us: f64) {
        let mut state = self.inner.state.lock().unwrap();
        let state = &mut *state;
        let Some(frames) = state.frames.as_ref() else {
            return;
        };
        let step = frames.chunk_len();
        let a = n - n % step;
        let b = (a + step).min(frames.count);
        let before = state.known.need(frames, a, b);
        state.known.cost(frames, Some(n), id, cost_us);
        if state.known.need(frames, a, b) != before {
            bump(state, &self.inner.published);
        }
    }

    /// Render Selection: the composition's frames in `[from, to)` seconds that
    /// are not on disk render in an `--engine-job` child (`executable`),
    /// admitted around viewer `activity` as exports are, its raster overrides
    /// answered as `overrides` sets up. A background fill stops for it.
    /// Returns the frames queued.
    pub fn start_fill(
        &self,
        from: f64,
        to: f64,
        executable: &Path,
        activity: crate::jobs::Activity,
        overrides: impl FnOnce(&mut crate::jobs::JobSession),
    ) -> Result<u64, String> {
        let frames = {
            let state = self.inner.state.lock().unwrap();
            let frames = ready(&state)?;
            frame_range(frames, from, to)
                .filter(|n| !frames.present.contains(n))
                .filter_map(|n| Some((n, cache_frame(frames, n, 0, false)?)))
                .collect::<Vec<_>>()
        };
        self.launch(frames, None, executable, activity, overrides)
    }

    /// Starts a fill of `frames` (index, frame); `budget_us` makes it a
    /// background fill. Returns the frames queued.
    fn launch(
        &self,
        frames: Vec<(i64, CacheFrame)>,
        budget_us: Option<f64>,
        executable: &Path,
        activity: crate::jobs::Activity,
        overrides: impl FnOnce(&mut crate::jobs::JobSession),
    ) -> Result<u64, String> {
        let background = budget_us.is_some();
        let mut running = loop {
            let mut running = self.inner.running.lock().unwrap();
            match running.as_ref() {
                Some(r) if !r.thread.is_finished() => {
                    if background || !r.background {
                        return Err("A render is already running.".into());
                    }
                    // Render Selection replaces background rendering.
                    let r = running.take().expect("checked");
                    drop(running);
                    stop(r);
                }
                _ => break running,
            }
        };
        let mut state = self.inner.state.lock().unwrap();
        let frames_now = ready(&state)?;
        let (dir, quota) = state
            .root
            .clone()
            .ok_or("No storage for rendered frames.")?;
        // Keys from an observation since are another state's: plan again.
        let frames: Vec<(i64, CacheFrame)> = frames
            .into_iter()
            .filter(|(n, f)| frames_now.key(*n).as_ref() == Some(&f.key))
            .collect();
        if frames.is_empty() {
            return Ok(0);
        }
        let order: Vec<(i64, FrameId)> = frames.iter().map(|(n, f)| (*n, f.key.graph)).collect();
        let (source, size, rate) = (frames_now.source.clone(), frames_now.size, frames_now.rate);
        let serial = state.fill.serial + 1;
        state.fill = Fill {
            serial,
            generation: state.generation,
            background,
            queued: if background {
                BTreeSet::new()
            } else {
                frames.iter().map(|m| m.0).collect()
            },
            total: frames.len() as u64,
            running: true,
            ..Fill::default()
        };
        let storage = state.storage;
        // Background rendering stops at its floor; a render asked for keeps
        // the export's reserve.
        let reserve = if background {
            state.background.floor
        } else {
            0
        };
        // Probes measure frames at the size the viewer draws them.
        let probe_scale = state.viewer.map_or(1., |v| {
            (f64::from(v.width) / f64::from(size.width.max(1))).clamp(0.05, 1.)
        });
        bump(&mut state, &self.inner.published);
        drop(state);
        let fill = CacheFill {
            dir,
            quota,
            width: size.width,
            height: size.height,
            frames: frames.into_iter().map(|m| m.1).collect(),
            budget_us,
            frame_ticks: rate.frame_start(1).unwrap_or(0),
            probe_scale,
        };
        let delay = self.inner.delay_ms.clone();
        let started = fill_request(&source, rate, fill, reserve)
            .and_then(|request| {
                let mut session = crate::jobs::JobSession::new(request);
                if background {
                    session.in_background(delay);
                }
                overrides(&mut session);
                crate::jobs::ExportHandle::start_scheduled(executable, session, activity)
            })
            .map_err(|e| e.to_string());
        let job = started
            .as_ref()
            .ok()
            .map(crate::jobs::ExportHandle::control);
        let weak = Arc::downgrade(&self.inner);
        let thread = std::thread::Builder::new()
            .name("pm-render-fill".into())
            .spawn(move || run_fill(weak, serial, storage, order, started))
            .map_err(|e| e.to_string())?;
        *running = Some(Running {
            job,
            thread,
            background,
        });
        Ok(self.inner.published.lock().unwrap().queued)
    }

    /// Stops the running fill and waits for its child to exit (frames it
    /// finished stay). Blocks: call off the main thread.
    pub fn cancel_fill(&self) {
        let running = self.inner.running.lock().unwrap().take();
        if let Some(running) = running {
            stop(running);
        }
    }

    /// Free space on the volume that holds rendered frames.
    fn free_space(&self) -> Option<u64> {
        let (dir, free) = {
            let state = self.inner.state.lock().unwrap();
            (
                state.root.as_ref().map(|r| r.0.clone())?,
                state.free.clone(),
            )
        };
        match free {
            Some(free) => Some(free()),
            None => crate::jobs::free_bytes(&dir).ok(),
        }
    }

    /// One look of the background scheduler (see the module).
    fn tick(&self, watch: &mut Watch) {
        let (host, config, activity, generation) = {
            let state = self.inner.state.lock().unwrap();
            (
                state.host.clone(),
                state.background,
                state.activity.clone(),
                state.generation,
            )
        };
        let Some(activity) = activity else { return };
        let epoch = activity.epoch();
        if activity.busy() || epoch != watch.seen {
            watch.seen = epoch;
            watch.active_at = Instant::now();
            watch.active_since.get_or_insert(watch.active_at);
        }
        let idle = watch.active_at.elapsed() >= config.delay && !activity.busy();
        if idle {
            watch.active_since = None;
        }
        let allowed = config.enabled && host.as_ref().is_some_and(|h| !(h.busy)());
        let running = self
            .inner
            .running
            .lock()
            .unwrap()
            .as_ref()
            .filter(|r| !r.thread.is_finished())
            .map(|r| r.background);
        let fill = {
            let state = self.inner.state.lock().unwrap();
            (
                state.fill.generation,
                // Stopped at the floor is not a failure to wait out: it
                // goes on as soon as there is room.
                state
                    .fill
                    .error
                    .as_deref()
                    .is_some_and(|e| e != BACKGROUND_LOW_DISK),
                state.fill.ended,
            )
        };
        match running {
            Some(true) => {
                // Paused long enough: its child gives its memory back.
                let paused_long = watch
                    .active_since
                    .is_some_and(|at| at.elapsed() >= PAUSED_AT_MOST);
                if !allowed || fill.0 != generation || paused_long {
                    self.cancel_fill();
                }
                return;
            }
            Some(false) => return,
            None => {}
        }
        let (Some(host), true, true) = (host, allowed, idle) else {
            return;
        };
        // Never below the floor: nothing starts, and the bars say why
        // (looked at every few seconds; rendering goes on once there is room).
        let low = match watch.floor {
            Some((at, low)) if at.elapsed() < FLOOR_EVERY => low,
            _ => {
                let low = self.free_space().is_some_and(|free| free < config.floor);
                watch.floor = Some((Instant::now(), low));
                low
            }
        };
        if low {
            self.with(|s| {
                let said = s.fill.background
                    && s.fill.generation == generation
                    && s.fill.error.as_deref() == Some(BACKGROUND_LOW_DISK);
                if !said {
                    s.fill = Fill {
                        serial: s.fill.serial + 1,
                        generation,
                        background: true,
                        error: Some(BACKGROUND_LOW_DISK.to_owned()),
                        disk_full: true,
                        ended: Some(Instant::now()),
                        ..Fill::default()
                    };
                }
                ((), !said)
            });
            return;
        }
        if fill.1 && fill.0 == generation && fill.2.is_some_and(|at| at.elapsed() < RETRY_AFTER) {
            return;
        }
        let planned = {
            let state = self.inner.state.lock().unwrap();
            ready(&state)
                .ok()
                .filter(|f| f.identity.tone.is_none())
                .map(|frames| {
                    (
                        plan(frames, &state.known, state.playhead),
                        frames.budget_us(),
                        frames.source.doc_id,
                    )
                })
        };
        let Some((frames, budget, doc_id)) = planned.filter(|p| !p.0.is_empty()) else {
            return;
        };
        let pin = host.pin.clone();
        let _ = self.launch(
            frames,
            Some(budget),
            &host.executable,
            activity,
            |session| pin(doc_id, session),
        );
    }

    /// Keep Rendered: the rendered frames in `[from, to)` are pinned against
    /// the limit and Clear… (`keep`), or unpinned. Returns the frames changed
    /// and the rendered frames that already were. Disk work: off the main
    /// thread.
    pub fn keep_range(&self, from: f64, to: f64, keep: bool) -> std::io::Result<Kept> {
        let plan = self.with(|s| {
            let (Some(cache), Some(frames)) = (s.cache.clone(), s.frames.as_ref()) else {
                return (None, false);
            };
            let rendered: Vec<i64> = frame_range(frames, from, to)
                .filter(|n| frames.present.contains(n))
                .collect();
            let already = rendered
                .iter()
                .filter(|n| frames.kept.contains(n) == keep)
                .count() as u64;
            let keys: Vec<(i64, RenderKey)> = rendered
                .into_iter()
                .filter(|n| frames.kept.contains(n) != keep)
                .filter_map(|n| Some((n, frames.key(n)?)))
                .collect();
            (Some((cache, keys, already, s.generation)), false)
        });
        let Some((cache, keys, already, generation)) = plan else {
            return Ok(Kept::default());
        };
        let mut changed = Vec::new();
        for (n, key) in keys {
            cache.keep_rendered(&key, keep)?;
            changed.push(n);
        }
        let count = changed.len() as u64;
        self.with(|s| match s.frames.as_mut() {
            Some(frames) if s.generation == generation => {
                for n in changed {
                    if keep {
                        frames.kept.insert(n);
                    } else {
                        frames.kept.remove(&n);
                    }
                }
                ((), true)
            }
            _ => ((), false),
        });
        Ok(Kept {
            changed: count,
            already,
        })
    }

    /// Delete Rendered Frames: every frame of the open composition's current
    /// state, kept ones too. A running fill stops first, so no write lands
    /// after the deletion. Returns frames deleted. Disk work: off the main
    /// thread.
    pub fn delete_all(&self) -> std::io::Result<u64> {
        self.cancel_fill();
        let plan = self.with(|s| {
            s.generation += 1;
            s.storage += 1;
            s.fill = Fill::default();
            let (Some(cache), Some(frames)) = (s.cache.clone(), s.frames.as_mut()) else {
                return (None, true);
            };
            let keys: Vec<RenderKey> = (0..frames.count).filter_map(|n| frames.key(n)).collect();
            frames.present.clear();
            frames.kept.clear();
            (Some((cache, keys)), true)
        });
        let Some((cache, keys)) = plan else {
            return Ok(0);
        };
        let mut deleted = 0;
        for key in keys {
            if cache.presence(&key)?.is_some() {
                cache.remove(&key)?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }

    /// The render bar's latest snapshot: cheap, for the main thread.
    pub fn bars(&self) -> Arc<RenderBars> {
        self.inner.published.lock().unwrap().clone()
    }

    /// A frame on disk for `scene` at `time` that the reader has loaded, and
    /// the next frames asked of it. Never waits on disk.
    #[cfg(test)]
    pub(crate) fn read(&self, scene: &Scene, time: Ticks) -> Option<(Size, Arc<Vec<u8>>)> {
        match self.read_within(scene, time, Duration::ZERO) {
            Read::Frame(size, bytes) => Some((size, bytes)),
            Read::Late(_) | Read::Absent => None,
        }
    }

    /// [`Self::read`], waiting up to `patience` for the frame when it is on
    /// disk and still being read (a playing frame: [`PATIENCE`]).
    pub(crate) fn read_within(&self, scene: &Scene, time: Ticks, patience: Duration) -> Read {
        let Some((size, generation, n, found)) = self.ask_ahead(scene, time, AHEAD, false) else {
            return Read::Absent;
        };
        match found {
            Some(bytes) => Read::Frame(size, bytes),
            None => match self.inner.reader.wait(generation, n, patience) {
                Ok(Some(bytes)) => Read::Frame(size, bytes),
                Ok(None) => Read::Absent,
                Err(Late) => Read::Late(size),
            },
        }
    }

    /// The media files the open composition's rendered frames are drawn
    /// from, by asset.
    pub fn rendered_from(&self) -> Option<BTreeMap<String, PathBuf>> {
        let state = self.inner.state.lock().unwrap();
        Some(state.frames.as_ref()?.source.scene.media.paths.clone())
    }

    /// Whether `scene`'s frame at `time` is rendered on disk: playback
    /// reads it, so nothing ahead of the playhead needs preparing for it.
    pub(crate) fn rendered(&self, scene: &Scene, time: Ticks) -> bool {
        let state = self.inner.state.lock().unwrap();
        state.frames.as_ref().is_some_and(|frames| {
            frames.identity.scene_matches(scene)
                && frames.fingerprint.is_some()
                && frames
                    .index(time)
                    .is_some_and(|n| frames.present.contains(&n))
        })
    }

    /// The paused playhead is at `time`: its frame and the next few are
    /// read ([`WARM`]), so a play from here starts on frames already read.
    pub(crate) fn warm(&self, scene: &Scene, time: Ticks) {
        self.ask_ahead(scene, time, WARM, true);
    }

    /// Asks the reader for the frame at `time` and the `ahead - 1` after it
    /// in the direction of motion (forward when `still`: a play starts
    /// forward): the stored size, the generation and index asked at, and
    /// the frame if it is read already.
    #[allow(clippy::type_complexity)]
    fn ask_ahead(
        &self,
        scene: &Scene,
        time: Ticks,
        ahead: i64,
        still: bool,
    ) -> Option<(Size, u64, i64, Option<Arc<Vec<u8>>>)> {
        let mut state = self.inner.state.lock().unwrap();
        let state = &mut *state;
        let frames = state.frames.as_ref()?;
        if !frames.identity.scene_matches(scene) || frames.fingerprint.is_none() {
            return None;
        }
        let n = frames.index(time)?;
        if let Some(last) = state.last_read.replace(n)
            && last != n
        {
            state.backward = n < last;
        }
        state.backward &= !still;
        let size = frames.size;
        // The frame, then the next ones in the direction of motion.
        let window = if state.backward {
            (n - ahead + 1).max(0)..=n
        } else {
            n..=(n + ahead - 1).min(frames.count - 1)
        };
        let order: Vec<i64> = if state.backward {
            window.clone().rev().collect()
        } else {
            window.clone().collect()
        };
        let asks: Vec<(i64, RenderKey)> = order
            .into_iter()
            .filter(|k| frames.present.contains(k))
            .filter_map(|k| Some((k, frames.key(k)?)))
            .collect();
        let (generation, cache) = (state.generation, state.cache.clone()?);
        let found = self.inner.reader.take(generation, n);
        self.inner
            .reader
            .want(generation, *window.start(), *window.end());
        for (k, key) in asks {
            self.inner.reader.ask(generation, k, key, &cache, size);
        }
        Some((size, generation, n, found))
    }
}

/// The open composition's frames, if a fill can start from them.
fn ready(state: &State) -> Result<&Frames, String> {
    let frames = state
        .frames
        .as_ref()
        .ok_or("Open a composition to render.")?;
    if frames.fingerprint.is_none() || frames.ids.is_empty() {
        return Err("Still reading the composition. Try again in a moment.".into());
    }
    if frames.identity.tone.is_some() {
        return Err(
            "Rendering to disk isn't available while the viewer tone-maps HDR for this display."
                .into(),
        );
    }
    Ok(frames)
}

fn cache_frame(frames: &Frames, n: i64, chunk: u32, probe: bool) -> Option<CacheFrame> {
    Some(CacheFrame {
        ticks: frames.ticks(n)?.get(),
        key: frames.key(n)?,
        chunk,
        probe,
    })
}

/// Background rendering's next fill: chunks not on disk that are known not
/// to play in real time (all their missing frames), and chunks not judged
/// yet (their first and last missing frames as probes, then the rest), in
/// order: the playhead's neighbourhood, the work area, then the rest,
/// nearest first, ahead before behind.
fn plan(frames: &Frames, known: &Known, playhead: Option<Ticks>) -> Vec<(i64, CacheFrame)> {
    let step = frames.chunk_len();
    let p = playhead.map_or(0, |t| frames.at(t));
    let interval = (frames.seconds(1) - frames.seconds(0)).max(1e-9);
    let seconds = |s: f64| (s / interval).round() as i64;
    let near = (p - seconds(NEAR_BEHIND), p + seconds(NEAR_AHEAD));
    let work = work_area(frames).unwrap_or((0, frames.count));
    let mut chunks: Vec<(u8, i64, i64)> = (0..frames.count)
        .step_by(step as usize)
        .map(|a| {
            let b = (a + step).min(frames.count);
            let tier = if a < near.1 && b > near.0 {
                0
            } else if a < work.1 && b > work.0 {
                1
            } else {
                2
            };
            let distance = if b > p {
                (a - p).max(0)
            } else {
                2 * (p - b + 1)
            };
            (tier, distance, a)
        })
        .collect();
    chunks.sort();
    let mut out = Vec::new();
    for (_, _, a) in chunks {
        let b = (a + step).min(frames.count);
        let missing: Vec<i64> = (a..b).filter(|n| !frames.present.contains(n)).collect();
        let (Some(&first), Some(&last)) = (missing.first(), missing.last()) else {
            continue;
        };
        let chunk = (a / step) as u32;
        let probes: Vec<i64> = match known.need(frames, a, b) {
            Need::Light | Need::Refused => continue,
            Need::Heavy => vec![],
            Need::Unknown if first == last => vec![first],
            Need::Unknown => vec![first, last],
        };
        let rest = missing.iter().filter(|n| !probes.contains(n));
        for (n, probe) in probes
            .iter()
            .map(|n| (*n, true))
            .chain(rest.map(|n| (*n, false)))
        {
            if let Some(frame) = cache_frame(frames, n, chunk, probe) {
                out.push((n, frame));
            }
        }
    }
    out
}

/// The composition's work area, in frames.
fn work_area(frames: &Frames) -> Option<(i64, i64)> {
    let comp = pm_eval::Comp::resolve(&frames.source.scene.doc, &frames.identity.comp).ok()?;
    let work = comp.member("work")?;
    let number = |i: usize| pm_eval::node::to_number(work.at(i));
    let (a, b) = (number(0), number(1));
    if !(a.is_finite() && b.is_finite() && b > a) {
        return None;
    }
    Some((
        frame_range(frames, a, b).start,
        frame_range(frames, a, b).end,
    ))
}

/// The background scheduler's view of the person's activity.
struct Watch {
    seen: u64,
    active_at: Instant,
    /// When the current stretch of activity began (`None` while idle).
    active_since: Option<Instant>,
    /// When the volume's free space was last looked at, and whether it
    /// was under [`BACKGROUND_FLOOR`].
    floor: Option<(Instant, bool)>,
}

/// The background scheduler: looks ten times a second while the cache lives.
fn schedule(inner: Weak<Inner>) {
    let mut watch = Watch {
        seen: 0,
        active_at: Instant::now(),
        active_since: None,
        floor: None,
    };
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let Some(inner) = inner.upgrade() else { return };
        DiskFrames { inner }.tick(&mut watch);
    }
}

fn stop(running: Running) {
    if let Some(job) = running.job {
        job.cancel();
    }
    let _ = running.thread.join();
}

/// The project's export default for motion blur (`exportDefaults.mblur`,
/// on unless turned off, as the Export sheet reads it).
fn export_motion_blur(doc: &pm_model::Document) -> bool {
    doc.root()
        .and_then(|r| r.get("exportDefaults"))
        .and_then(|d| d.get("mblur"))
        .is_none_or(|m| !matches!(m.kind(), pm_model::NodeKind::Bool(false)))
}

/// The fill child's request: the viewer's scene as an export job (png
/// settings stand in for delivery; the child renders `fill`'s frames only).
fn fill_request(
    source: &Source,
    rate: Rate,
    fill: CacheFill,
    reserve: u64,
) -> anyhow::Result<crate::jobs::ExportRequest> {
    use pm_protocol::{
        color::{OutputColor, Precision},
        common::{FrameRate, SessionId, TickRange},
        priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, QualityTier},
    };
    let first = fill.frames.iter().map(|f| f.ticks).min().unwrap_or(0);
    let last = fill.frames.iter().map(|f| f.ticks).max().unwrap_or(0);
    let step = rate.frame_start(1).unwrap_or(1).max(1);
    let fps = (TICKS_PER_SECOND as f64 / step as f64 * 1000.).round() as u32;
    let settings = ExportSettings {
        format: ExportFormat::PngSequence { sixteen_bit: true },
        // Cached frames are the export's: opaque (the viewer shows the
        // background), with the project's export motion blur.
        alpha: AlphaOutput::None,
        quality: QualityTier::High,
        size: Size::new(fill.width, fill.height),
        frame_rate: FrameRate::new(fps, 1000),
        range: TickRange {
            start: Ticks::new(first)?,
            end: Ticks::new(last + step)?,
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: export_motion_blur(&source.scene.doc),
        color: OutputColor::SrgbImage,
        precision: Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    let document = source
        .scene
        .doc
        .to_value()
        .ok_or_else(|| anyhow::anyhow!("missing document"))?;
    let dir = fill.dir.clone();
    Ok(crate::jobs::ExportRequest {
        job: pm_kernel_free_id(),
        spec: JobSpec::Export {
            session: SessionId::new(source.doc_id)?,
            composition: source.scene.comp.as_str().into(),
            settings,
        },
        revision: source.scene.doc.revision(),
        document: pm_model::json::to_canonical_string(&document),
        originals: source.scene.media.paths.clone(),
        proxies: Default::default(),
        destination: dir,
        saving_app_version: None,
        resources: crate::resources::ResourceSnapshot {
            definitions: source.definitions.clone(),
            ..Default::default()
        },
        disk_reserve: reserve.max(crate::jobs::DISK_RESERVE),
        // Cached frames are exact: a missing effect is never left out.
        bypass_missing_effects: false,
        without_offline: true,
        preview: false,
        cache: Some(fill),
    })
}

fn pm_kernel_free_id() -> pm_protocol::common::JobId {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!(
        "render-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
    .as_str()
    .into()
}

/// The fill thread: follows the child and lands what it reports: costs and
/// refusals by key, stored frames while their storage is current and their
/// key is the frame's, progress while the fill is the current one.
fn run_fill(
    inner: Weak<Inner>,
    serial: u64,
    storage: u64,
    order: Vec<(i64, FrameId)>,
    started: Result<crate::jobs::ExportHandle, String>,
) {
    use pm_protocol::priority::JobState;
    let land = |reports: Vec<FrameReport>, done: u64, end: Option<(Option<String>, bool)>| {
        let Some(inner) = inner.upgrade() else { return };
        let mut state = inner.state.lock().unwrap();
        let state = &mut *state;
        let current = state.fill.serial == serial;
        for report in reports {
            let Some(&(n, id)) = order.get(report.at as usize) else {
                continue;
            };
            if let Some(frames) = state.frames.as_mut() {
                if let Some(cost) = report.cost_us {
                    state.known.cost(frames, Some(n), id, cost);
                }
                if report.stored && state.storage == storage && frames.id(n) == Some(id) {
                    frames.present.insert(n);
                }
            }
            if report.refused {
                state.known.refuse(id);
            }
            if current {
                state.fill.queued.remove(&n);
            }
        }
        if !current {
            return;
        }
        state.fill.done = state.fill.done.max(done);
        if let Some((error, disk_full)) = end {
            state.fill.running = false;
            state.fill.queued.clear();
            // Background rendering stopped at its floor says so in its
            // own words (the child's are an export's).
            let error =
                if state.fill.background && disk_full && error.as_deref() != Some(LIMIT_FULL) {
                    Some(BACKGROUND_LOW_DISK.to_owned())
                } else {
                    error
                };
            state.fill.error = error;
            state.fill.disk_full = disk_full;
            state.fill.ended = Some(Instant::now());
        }
        bump(state, &inner.published);
    };
    let handle = match started {
        Ok(handle) => handle,
        Err(error) => return land(vec![], 0, Some((Some(error), false))),
    };
    let total = order.len() as f64;
    let mut status = handle.status();
    while !status.state.is_terminal() {
        land(
            handle.take_frames(),
            (status.progress * total).round() as u64,
            None,
        );
        status = handle.next_status(&status);
    }
    let landed = (status.progress * total).floor() as u64;
    // Every frame report comes before the terminal status.
    let reports = handle.take_frames();
    let end = match handle.wait().map(|status| status.state) {
        Err(error) => (landed, Some(error.to_string()), false),
        Ok(JobState::Done) => (order.len() as u64, None, false),
        Ok(JobState::Cancelled) => (landed, None, false),
        Ok(_) => {
            let message = status.error.map(|e| e.message).unwrap_or_default();
            let full =
                message == DISK_FULL || message == LIMIT_FULL || message == crate::jobs::LOW_DISK;
            (landed, Some(message), full)
        }
    };
    land(reports, end.0, Some((end.1, end.2)));
    // The limit may have evicted older frames as these landed.
    if let Some(inner) = inner.upgrade() {
        let _ = DiskFrames { inner }.reindex(true);
    }
}

/// What [`DiskFrames::read_within`] found.
pub(crate) enum Read {
    /// The frame, at its stored size.
    Frame(Size, Arc<Vec<u8>>),
    /// On disk at this size, and still being read when the patience ran
    /// out.
    Late(Size),
    /// Not on disk for this state.
    Absent,
}
/// A frame still being read when the wait for it ended.
struct Late;
/// Reads frames on threads of its own, ahead of the playhead.
struct Reader {
    asks: std::sync::mpsc::SyncSender<Ask>,
    ready: Arc<Mutex<Ready>>,
    /// Signalled whenever a read ends ([`Reader::wait`]).
    ended: Arc<std::sync::Condvar>,
}
struct Ask {
    id: (u64, i64),
    key: RenderKey,
    cache: Arc<RenderCache>,
    size: Size,
}
/// A read frame: the generation it was asked at, and its index.
type ReadId = (u64, i64);
#[derive(Default)]
struct Ready {
    frames: VecDeque<(ReadId, Arc<Vec<u8>>)>,
    asked: HashSet<(u64, i64)>,
    /// The frames the latest read wants (generation, first, last): asks
    /// queued before a reversal or a jump are dropped, not read.
    wanted: Option<(u64, i64, i64)>,
}
impl Reader {
    /// The thread holds only the ready list: dropping the cache's last
    /// owner closes the channel and ends it.
    fn start() -> Self {
        let (asks, rx) = std::sync::mpsc::sync_channel::<Ask>(2 * AHEAD as usize);
        let ready = Arc::new(Mutex::new(Ready::default()));
        let shared = ready.clone();
        let ended = Arc::new(std::sync::Condvar::new());
        let end = ended.clone();
        // Frames are read side by side: a frame takes several times longer
        // to read on a busy machine, and one reader then falls behind the
        // playhead for good while three still deliver a frame an interval.
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..READERS {
            let (rx, shared, end) = (rx.clone(), shared.clone(), end.clone());
            std::thread::Builder::new()
                .name("pm-render-cache-read".into())
                .spawn(move || Self::read_asks(&rx, &shared, &end))
                .expect("render cache reader thread");
        }
        Self { asks, ready, ended }
    }
    /// One reader: takes asks until the cache's last owner is gone.
    fn read_asks(
        rx: &Mutex<std::sync::mpsc::Receiver<Ask>>,
        shared: &Mutex<Ready>,
        end: &std::sync::Condvar,
    ) {
        // A frame on screen waits for these reads.
        crate::qos::set_interactive_qos();
        loop {
            let ask = rx.lock().unwrap().recv();
            let Ok(ask) = ask else { return };
            {
                {
                    let stale = {
                        let mut ready = shared.lock().unwrap();
                        let (generation, n) = ask.id;
                        let stale = ready.wanted.is_some_and(|(g, first, last)| {
                            g != generation || n < first || n > last
                        });
                        if stale {
                            ready.asked.remove(&ask.id);
                        }
                        stale
                    };
                    if stale {
                        end.notify_all();
                        continue;
                    }
                    // Stored as the viewer's surface holds it.
                    let data = ask.cache.get(&ask.key).ok().flatten().and_then(|d| {
                        crate::frame_codec::decode(&d, ask.size.width, ask.size.height)
                    });
                    if data.is_some() {
                        // Played frames are the last the limit removes.
                        let _ = ask.cache.touch(&ask.key);
                    }
                    let mut ready = shared.lock().unwrap();
                    ready.asked.remove(&ask.id);
                    end.notify_all();
                    if ready.wanted.is_some_and(|(g, first, last)| {
                        ask.id.0 != g || ask.id.1 < first || ask.id.1 > last
                    }) {
                        continue;
                    }
                    if let Some(data) = data {
                        ready.frames.push_back((ask.id, Arc::new(data)));
                        while ready.frames.len() > READY_FRAMES {
                            ready.frames.pop_front();
                        }
                    }
                }
            }
        }
    }
    /// Frame `n` asked at `generation`, waiting up to `patience` while it
    /// is being read: [`Late`] when it is still being read after that,
    /// `None` at once when nobody is reading it (it is not on disk, or its
    /// read was dropped).
    fn wait(
        &self,
        generation: u64,
        n: i64,
        patience: Duration,
    ) -> Result<Option<Arc<Vec<u8>>>, Late> {
        let id = (generation, n);
        let until = Instant::now() + patience;
        let mut ready = self.ready.lock().unwrap();
        loop {
            if let Some((_, bytes)) = ready.frames.iter().find(|(i, _)| *i == id) {
                return Ok(Some(bytes.clone()));
            }
            if !ready.asked.contains(&id) {
                return Ok(None);
            }
            let left = until
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
                .ok_or(Late)?;
            ready = self.ended.wait_timeout(ready, left).unwrap().0;
        }
    }
    /// Keeps only the read frames `first..=last` of `generation`, and
    /// fences reads beyond them.
    fn keep(&self, generation: u64, first: i64, last: i64) {
        let mut ready = self.ready.lock().unwrap();
        ready
            .frames
            .retain(|((g, n), _)| *g == generation && (first..=last).contains(n));
        ready.wanted = Some((generation, first, last));
    }
    fn take(&self, generation: u64, n: i64) -> Option<Arc<Vec<u8>>> {
        let ready = self.ready.lock().unwrap();
        ready
            .frames
            .iter()
            .find(|(id, _)| *id == (generation, n))
            .map(|(_, bytes)| bytes.clone())
    }
    fn clear(&self, generation: u64) {
        let mut ready = self.ready.lock().unwrap();
        *ready = Ready {
            wanted: Some((generation, 1, 0)),
            ..Default::default()
        };
    }
    fn want(&self, generation: u64, first: i64, last: i64) {
        self.ready.lock().unwrap().wanted = Some((generation, first, last));
    }
    fn ask(&self, generation: u64, n: i64, key: RenderKey, cache: &Arc<RenderCache>, size: Size) {
        let id = (generation, n);
        {
            let mut ready = self.ready.lock().unwrap();
            if ready.asked.contains(&id) || ready.frames.iter().any(|(i, _)| *i == id) {
                return;
            }
            ready.asked.insert(id);
        }
        let ask = Ask {
            id,
            key,
            cache: cache.clone(),
            size,
        };
        if self.asks.try_send(ask).is_err() {
            self.ready.lock().unwrap().asked.remove(&id);
        }
    }
}

/// Which of `keys`' frames are on disk, and which of those are kept.
fn index(cache: &RenderCache, keys: &[(i64, RenderKey)]) -> (HashSet<i64>, HashSet<i64>) {
    let (mut present, mut kept) = (HashSet::new(), HashSet::new());
    for (n, key) in keys {
        if let Ok(Some(keep)) = cache.presence(key) {
            present.insert(*n);
            if keep {
                kept.insert(*n);
            }
        }
    }
    (present, kept)
}

/// Frame indices of `[from, to)` seconds, clamped to the composition.
fn frame_range(frames: &Frames, from: f64, to: f64) -> std::ops::Range<i64> {
    let tick = |s: f64| (s * TICKS_PER_SECOND as f64).round() as i64;
    let first = frames.rate.frame_of(tick(from.max(0.)));
    let end = frames.rate.frame_of(tick(to.max(0.)));
    let end = if frames.ticks(end).is_some_and(|t| t.get() < tick(to)) {
        end + 1
    } else {
        end
    };
    first.clamp(0, frames.count)..end.clamp(0, frames.count)
}

/// The composition's size, frame rate and frame count.
fn composition(scene: &Scene) -> Option<(Size, Rate, i64)> {
    let comp = pm_eval::Comp::resolve(&scene.doc, &scene.comp).ok()?;
    let number = |key: &str| {
        let v = pm_eval::node::to_number(comp.member(key));
        (v.is_finite() && v > 0.).then_some(v)
    };
    let (w, h) = (number("w")?, number("h")?);
    let fps = number("fps").unwrap_or(30.);
    let duration = number("dur")?;
    let rate = Rate::new((fps * 1000.).round() as i64, 1000).ok()?;
    let count = (duration * fps).round().max(1.) as i64;
    Some((Size::new(w.round() as u32, h.round() as u32), rate, count))
}

fn stamp(path: &Path, content_named: Option<&Path>) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok();
    // Named by its bytes: its name (in the path) and length say it all.
    if content_named.is_some() && path.parent() == content_named {
        return (path.to_owned(), meta.as_ref().map_or(0, |m| m.len()), 0, 0);
    }
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    (
        path.to_owned(),
        meta.as_ref().map_or(0, |m| m.len()),
        modified,
        meta.as_ref().map_or(0, |m| m.ino()),
    )
}

fn stamps(media: &pm_export::render::MediaPaths, content_named: Option<&Path>) -> Vec<Stamp> {
    media
        .paths
        .values()
        .map(|p| stamp(p, content_named))
        .collect()
}

/// The running renderer: a digest of this executable (any rebuild is
/// another renderer), taken once. A development build takes
/// `PM_RENDER_CACHE_RENDERER` instead when set, so a rebuild that did not
/// change drawing keeps a long fill (measurement runs; never a release).
fn renderer_identity() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        if cfg!(debug_assertions)
            && let Ok(pinned) = std::env::var("PM_RENDER_CACHE_RENDERER")
        {
            return format!("pinned {pinned}");
        }
        let digest = std::env::current_exe()
            .ok()
            .and_then(|exe| std::fs::read(exe).ok())
            .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
            .unwrap_or_else(|| format!("unknown-{}", std::process::id()));
        format!("powermove-native {digest}")
    })
}

/// A media file's content, sampled: its length and up to four 64 KiB
/// pieces (start, thirds, end), with its stamp.
fn media_content(path: &Path, hash: &mut blake3::Hasher) {
    use std::io::{Read, Seek, SeekFrom};
    const PIECE: u64 = 64 << 10;
    let Ok(mut file) = std::fs::File::open(path) else {
        hash.update(b"offline");
        return;
    };
    let len = file.metadata().map_or(0, |m| m.len());
    hash.update(&len.to_le_bytes());
    let mut buf = vec![0u8; PIECE as usize];
    for at in [0, len / 3, len * 2 / 3, len.saturating_sub(PIECE)] {
        if file.seek(SeekFrom::Start(at)).is_ok()
            && let Ok(n) = file.read(&mut buf)
        {
            hash.update(&buf[..n]);
        }
    }
}

/// The installed fonts, by file: what resolves a family by name.
fn fonts_identity(hash: &mut blake3::Hasher) {
    let mut files = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let roots = [
        Some(PathBuf::from("/System/Library/Fonts")),
        Some(PathBuf::from("/Library/Fonts")),
        home.map(|h| h.join("Library/Fonts")),
    ];
    fn walk(dir: &Path, out: &mut Vec<Stamp>, depth: u32) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() && depth < 4 => walk(&path, out, depth + 1),
                Ok(t) if t.is_file() => out.push(stamp(&path, None)),
                _ => {}
            }
        }
    }
    for root in roots.into_iter().flatten() {
        walk(&root, &mut files, 0);
    }
    files.sort();
    for (path, len, modified, _) in files {
        hash.update(path.as_os_str().as_encoded_bytes());
        hash.update(&len.to_le_bytes());
        hash.update(&modified.to_le_bytes());
    }
}

/// The fingerprint of everything that draws the composition's frames
/// besides the document, whose parts each frame draws from are in its key
/// (`crate::frame_inputs`).
fn fingerprint(source: &Source, identity: &Identity) -> [u8; 32] {
    let parts = fingerprint_parts(source, identity);
    let mut hash = blake3::Hasher::new_derive_key("powermove render cache v5");
    for (_, part) in &parts {
        hash.update(part);
    }
    let fingerprint = *hash.finalize().as_bytes();
    // Which part of a state's identity changed (diagnostics only).
    if let Some(trace) = std::env::var_os("PM_ENGINE_TRACE") {
        let mut path = PathBuf::from(trace).into_os_string();
        path.push(".cache.jsonl");
        let short = |digest: &[u8; 32]| digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        let mut row: serde_json::Map<String, serde_json::Value> = parts
            .iter()
            .map(|(name, part)| ((*name).to_owned(), serde_json::Value::String(short(part))))
            .collect();
        row.insert(
            "fingerprint".into(),
            serde_json::Value::String(short(&fingerprint)),
        );
        row.insert("revision".into(), identity.revision.into());
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{}", serde_json::Value::Object(row));
        }
    }
    fingerprint
}

/// What a composition state's frames depend on beyond its document, each
/// part digested on its own.
fn fingerprint_parts(source: &Source, identity: &Identity) -> Vec<(&'static str, [u8; 32])> {
    let part = |fill: &dyn Fn(&mut blake3::Hasher)| {
        let mut hash = blake3::Hasher::new();
        fill(&mut hash);
        *hash.finalize().as_bytes()
    };
    vec![
        (
            "composition",
            part(&|hash| {
                hash.update(identity.comp.as_bytes());
                // Frames are rendered as exported: with or without its motion blur.
                hash.update(&[u8::from(export_motion_blur(&source.scene.doc))]);
            }),
        ),
        (
            "definitions",
            part(&|hash| {
                hash.update(&serde_json::to_vec(&source.definitions).unwrap_or_default());
            }),
        ),
        (
            "media",
            part(&|hash| {
                for (asset, path) in &source.scene.media.paths {
                    hash.update(asset.as_bytes());
                    hash.update(path.as_os_str().as_encoded_bytes());
                    media_content(path, hash);
                }
            }),
        ),
        (
            "media_stamps",
            part(&|hash| {
                for (path, len, modified, inode) in &identity.media {
                    hash.update(path.as_os_str().as_encoded_bytes());
                    hash.update(&len.to_le_bytes());
                    hash.update(&modified.to_le_bytes());
                    hash.update(&inode.to_le_bytes());
                }
            }),
        ),
        ("fonts", part(&|hash| fonts_identity(hash))),
        (
            "tone",
            part(&|hash| {
                hash.update(format!("{:?}", identity.tone).as_bytes());
            }),
        ),
        (
            "renderer",
            part(&|hash| {
                hash.update(renderer_identity().as_bytes());
            }),
        ),
        // `CompatStore::cache_identity`: the wrappers' code and generation,
        // or this process alone when their code is unknown.
        (
            "overrides",
            part(&|hash| {
                if let Some(overrides) = identity.overrides {
                    hash.update(&overrides.to_le_bytes());
                }
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_releases_read_ahead_without_forgetting_the_open_scene() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().to_owned(), 64 << 20)))
            .unwrap();
        let scene = scene();
        assert!(disk.observe(&scene, &[], None, None, None, 7, Ticks::new(0).unwrap()));
        ready(&disk);
        let generation = disk.inner.state.lock().unwrap().generation;
        disk.inner
            .reader
            .ready
            .lock()
            .unwrap()
            .frames
            .push_back(((generation, 0), Arc::new(vec![0; 4096])));
        assert_eq!(disk.ready_bytes(), 4096);
        disk.release_idle_reads();
        assert_eq!(disk.ready_bytes(), 0);
        assert!(disk.inner.state.lock().unwrap().frames.is_some());
        assert_eq!(disk.inner.state.lock().unwrap().generation, generation);
        // The empty range refuses both queued work and a late completion;
        // the next explicit read can establish its normal window again.
        assert_eq!(
            disk.inner.reader.ready.lock().unwrap().wanted,
            Some((generation, 1, 0))
        );
        disk.inner.reader.want(generation, 0, 3);
        assert_eq!(
            disk.inner.reader.ready.lock().unwrap().wanted,
            Some((generation, 0, 3))
        );
    }

    #[test]
    fn closing_document_releases_the_scene_and_ready_disk_frames() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().to_owned(), 64 << 20)))
            .unwrap();
        let scene = scene();
        let doc = Arc::downgrade(&scene.doc);
        assert!(disk.observe(&scene, &[], None, None, None, 7, Ticks::new(0).unwrap()));
        ready(&disk);
        {
            let mut ready = disk.inner.reader.ready.lock().unwrap();
            ready.frames.push_back(((1, 0), Arc::new(vec![0; 1024])));
        }
        drop(scene);
        disk.forget_document(8);
        assert!(doc.upgrade().is_some(), "another document must stay live");
        disk.forget_document(7);
        assert!(disk.inner.state.lock().unwrap().frames.is_none());
        assert!(disk.inner.reader.ready.lock().unwrap().frames.is_empty());
        let deadline = Instant::now() + Duration::from_secs(5);
        while doc.upgrade().is_some() {
            assert!(
                Instant::now() < deadline,
                "fingerprint worker retained closed scene"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let pixels = Arc::new(vec![0; 1024]);
        let released = Arc::downgrade(&pixels);
        disk.inner
            .reader
            .ready
            .lock()
            .unwrap()
            .frames
            .push_back(((1, 0), pixels));
        disk.set_storage(None).unwrap();
        assert!(
            released.upgrade().is_none(),
            "storage changes release old read-ahead too"
        );
    }

    fn scene() -> Scene {
        scene_from(include_str!("../tests/fixtures/shapes.json"))
    }
    fn scene_from(source: &str) -> Scene {
        let project = pm_model::json::parse(source).unwrap();
        let doc = pm_model::Document::new(Some(pm_model::canonical::canonical_project(project)), 1);
        Scene {
            doc: Arc::new(doc),
            comp: "Pshapes-primitives-root".into(),
            media: Arc::default(),
            content: 1,
        }
    }
    fn settle(disk: &DiskFrames, until: impl Fn(&RenderBars) -> bool) -> Arc<RenderBars> {
        let start = Instant::now();
        loop {
            let bars = disk.bars();
            if until(&bars) {
                return bars;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "{bars:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn ready(disk: &DiskFrames) {
        let start = Instant::now();
        while disk
            .inner
            .state
            .lock()
            .unwrap()
            .frames
            .as_ref()
            .is_none_or(|f| f.fingerprint.is_none())
        {
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// Frame 30 of the fixture written as a fill child writes it.
    fn write_frame(disk: &DiskFrames, n: i64) -> Ticks {
        let (key, time, cache) = {
            let state = disk.inner.state.lock().unwrap();
            let frames = state.frames.as_ref().unwrap();
            (
                frames.key(n).unwrap(),
                frames.ticks(n).unwrap(),
                state.cache.clone().unwrap(),
            )
        };
        let bytes: Vec<u8> =
            std::iter::repeat_n(half::f16::from_f32(0.25).to_le_bytes(), 640 * 360 * 4)
                .flatten()
                .collect();
        cache
            .put(&key, &crate::frame_codec::encode(&bytes, 640, 360))
            .unwrap();
        disk.reindex(true).unwrap();
        time
    }
    fn read_eventually(disk: &DiskFrames, scene: &Scene, time: Ticks) -> Option<Arc<Vec<u8>>> {
        let start = Instant::now();
        loop {
            if let Some((_, bytes)) = disk.read(scene, time) {
                return Some(bytes);
            }
            if start.elapsed() > Duration::from_secs(2) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A frame on disk is read back (by the reader, never waiting) for its
    /// composition state only; Keep Rendered pins it; Delete Rendered
    /// Frames removes kept frames too, and a later state misses.
    #[test]
    fn frames_are_read_for_their_state_only_and_deleted_with_keeps() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = scene();
        assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        assert!(!disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        let time = write_frame(&disk, 30);
        let bars = settle(&disk, |b| !b.bars.is_empty());
        assert_eq!(bars.bars, vec![(1., 31. / 30., BarState::Rendered)]);
        let bytes = read_eventually(&disk, &scene, time).expect("read ahead");
        assert_eq!(&bytes[..2], &half::f16::from_f32(0.25).to_le_bytes());
        assert!(
            disk.read(&scene, Ticks::new(time.get() + 1).unwrap())
                .is_none()
        );
        let mut edited = scene.clone();
        edited.content = 2;
        assert!(disk.read(&edited, time).is_none());
        assert_eq!(
            disk.keep_range(0., 3., true).unwrap(),
            Kept {
                changed: 1,
                already: 0
            }
        );
        assert_eq!(disk.bars().bars[0].2, BarState::Kept);
        // Keeping it again says it already is (not "render it first").
        assert_eq!(
            disk.keep_range(0., 3., true).unwrap(),
            Kept {
                changed: 0,
                already: 1
            }
        );
        assert_eq!(disk.delete_all().unwrap(), 1);
        assert!(disk.bars().bars.is_empty());
        assert!(read_eventually(&disk, &scene, time).is_none());
    }

    /// Reads ahead follow the direction of motion: skimming back reads the
    /// frames before the skimmer, and asks left behind by a reversal are
    /// dropped instead of read.
    #[test]
    fn read_ahead_follows_a_reversal() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = scene();
        assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        let times: Vec<Ticks> = (10..=40).map(|n| write_frame(&disk, n)).collect();
        let read_already = |n: i64| {
            let generation = disk.inner.state.lock().unwrap().generation;
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(2) {
                if disk.inner.reader.take(generation, n).is_some() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            false
        };
        // Forward from frame 20: 21 to 25 are read without being asked for.
        read_eventually(&disk, &scene, times[10]).unwrap();
        read_eventually(&disk, &scene, times[11]).unwrap();
        assert!(read_already(25));
        // Back from frame 35: 34 to 30 are read before the skimmer gets there.
        read_eventually(&disk, &scene, times[25]).unwrap();
        read_eventually(&disk, &scene, times[24]).unwrap();
        assert!(read_already(30));
        let wanted = disk.inner.reader.ready.lock().unwrap().wanted;
        let generation = disk.inner.state.lock().unwrap().generation;
        assert_eq!(wanted, Some((generation, 34 - AHEAD + 1, 34)));
    }

    /// The app's media cache names a file by its bytes and marks its
    /// recency in its modification time at every open: the frames rendered
    /// from it stay the state's frames (every launch orphaned the last
    /// one's). Any other file's modification time is still its identity.
    #[test]
    fn a_touched_media_cache_object_keeps_its_frames() {
        let objects = tempfile::tempdir().unwrap();
        let linked = tempfile::tempdir().unwrap();
        let fingerprint_after_touch = |file: &Path, content_named: Option<&Path>| {
            std::fs::write(file, b"media bytes").unwrap();
            let dir = tempfile::tempdir().unwrap();
            let disk = DiskFrames::default();
            disk.set_content_named(content_named.map(Path::to_owned));
            disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
                .unwrap();
            let scene = Scene {
                media: Arc::new(pm_export::render::MediaPaths {
                    paths: [("clip".to_owned(), file.to_owned())].into(),
                    ..Default::default()
                }),
                ..scene()
            };
            let at = Ticks::new(0).unwrap();
            let fingerprint = |disk: &DiskFrames| {
                disk.observe(&scene, &[], None, None, None, 1, at);
                ready(disk);
                let state = disk.inner.state.lock().unwrap();
                state.frames.as_ref().unwrap().fingerprint.unwrap()
            };
            let before = fingerprint(&disk);
            // The next open: the recency mark moves, the bytes don't.
            std::fs::File::options()
                .append(true)
                .open(file)
                .unwrap()
                .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
                .unwrap();
            disk.inner.state.lock().unwrap().media_checked = None;
            (before, fingerprint(&disk))
        };
        let object = objects.path().join("0a1b2c.mp4");
        let (before, after) = fingerprint_after_touch(&object, Some(objects.path()));
        assert_eq!(before, after, "a media cache object");
        let file = linked.path().join("clip.mp4");
        let (before, after) = fingerprint_after_touch(&file, Some(objects.path()));
        assert_ne!(before, after, "a linked file");
    }

    /// A playing frame gets its frame from disk on the first ask, waiting
    /// for the reader (a frame is on disk because it can't be drawn in
    /// time; never waiting, playback drew it again, fell behind, and the
    /// reader then read frames the playhead had already passed). A frame
    /// that is not on disk is not waited for.
    #[test]
    fn a_playing_frame_waits_for_its_frame_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = scene();
        assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        let times: Vec<Ticks> = (10..=40).map(|n| write_frame(&disk, n)).collect();
        // Each frame as playback meets it: asked once, a skip of five
        // frames at a time (what drawing a slow frame costs). The patience
        // here is a slow test build's, not playback's.
        let patience = Duration::from_secs(10);
        for n in [10usize, 15, 20, 25] {
            assert!(
                matches!(
                    disk.read_within(&scene, times[n - 10], patience),
                    Read::Frame(..)
                ),
                "frame {n} on the first ask"
            );
        }
        let absent = Ticks::new(Rate::FPS_30.frame_start(60).unwrap()).unwrap();
        let asked = Instant::now();
        assert!(matches!(
            disk.read_within(&scene, absent, patience),
            Read::Absent
        ));
        assert!(asked.elapsed() < patience / 4, "nothing to wait for");
        // Still being read when the patience ran out: late, not absent, so
        // playback gives way a tick instead of drawing the frame again.
        let late = (0..200).any(|k| {
            let n = 10 + (k * 7) % 31;
            disk.release_idle_reads();
            disk.inner
                .reader
                .clear(disk.inner.state.lock().unwrap().generation);
            matches!(
                disk.read_within(&scene, times[n - 10], Duration::ZERO),
                Read::Late(_)
            )
        });
        assert!(late, "a frame asked and not yet read is late");
    }

    /// A paused playhead keeps the frames a play starts from read, through
    /// the idle release too, so the first playing frames don't wait on disk.
    #[test]
    fn a_paused_playhead_keeps_the_frames_a_play_starts_from() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = scene();
        assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        let times: Vec<Ticks> = (10..=40).map(|n| write_frame(&disk, n)).collect();
        let generation = disk.inner.state.lock().unwrap().generation;
        let read = |n: i64| disk.inner.reader.take(generation, n).is_some();
        // Played to frame 25, then paused at 20.
        read_eventually(&disk, &scene, times[15]).unwrap();
        disk.warm(&scene, times[10]);
        let start = Instant::now();
        while !(20..20 + WARM).all(read) {
            assert!(start.elapsed() < Duration::from_secs(2), "warm frames");
            std::thread::sleep(Duration::from_millis(5));
        }
        disk.release_idle_reads();
        assert!((20..20 + WARM).all(read), "kept through the idle release");
        assert!(!read(25), "the rest is released");
        assert_eq!(disk.ready_bytes(), WARM as u64 * 640 * 360 * 8);
        // The play starts on a read frame, never waiting.
        assert!(disk.read(&scene, times[10]).is_some());
        // What is rendered needs no look-ahead; what is not still does.
        assert!(disk.rendered(&scene, times[10]));
        let beyond = Ticks::new(Rate::FPS_30.frame_start(60).unwrap()).unwrap();
        assert!(!disk.rendered(&scene, beyond));
    }

    /// The viewer's output tone and its override wrappers are part of the
    /// identity: a frame rendered for one is not read for another.
    #[test]
    fn tone_and_override_changes_miss_earlier_frames() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = scene();
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        let time = write_frame(&disk, 30);
        assert!(read_eventually(&disk, &scene, time).is_some());
        assert!(disk.observe(
            &scene,
            &[],
            None,
            Some("Pq".into()),
            None,
            1,
            Ticks::new(0).unwrap()
        ));
        ready(&disk);
        assert!(read_eventually(&disk, &scene, time).is_none());
        assert!(disk.observe(&scene, &[], Some(3), None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        assert!(read_eventually(&disk, &scene, time).is_none());
        // Back to the first state: its frame is found again.
        assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        assert!(read_eventually(&disk, &scene, time).is_some());
    }

    /// Writes one override frame under identity 7 into `PM_RELAUNCH_DIR`:
    /// the first launch of the relaunch test, run as its own process.
    #[test]
    #[ignore]
    fn relaunch_writer() {
        let Some(dir) = std::env::var_os("PM_RELAUNCH_DIR") else {
            return;
        };
        let disk = DiskFrames::default();
        disk.set_storage(Some((PathBuf::from(dir), u64::MAX)))
            .unwrap();
        let scene = scene();
        disk.observe(&scene, &[], Some(7), None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        let time = write_frame(&disk, 30);
        assert!(read_eventually(&disk, &scene, time).is_some());
    }

    /// Override frames another process rendered (a relaunch) are found again
    /// with the same override identity, and never with another.
    #[test]
    fn override_frames_survive_a_relaunch_with_the_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "render_cache::tests::relaunch_writer",
            ])
            .env("PM_RELAUNCH_DIR", &frames)
            .status()
            .unwrap();
        assert!(status.success(), "the first launch wrote its frame");
        let scene = scene();
        let relaunched = DiskFrames::default();
        relaunched.set_storage(Some((frames, u64::MAX))).unwrap();
        relaunched.observe(&scene, &[], Some(7), None, None, 1, Ticks::new(0).unwrap());
        ready(&relaunched);
        let time = relaunched.with(|s| (s.frames.as_ref().unwrap().ticks(30).unwrap(), false));
        assert!(read_eventually(&relaunched, &scene, time).is_some());
        assert!(relaunched.observe(&scene, &[], Some(8), None, None, 1, Ticks::new(0).unwrap()));
        ready(&relaunched);
        assert!(read_eventually(&relaunched, &scene, time).is_none());
    }

    /// Render Selection through the real `--engine-job` child: the frames
    /// land on disk as the paused viewer draws them, the render bar and the
    /// fill's progress follow, and playback reads them back.
    #[test]
    fn a_fill_child_renders_frames_that_playback_reads() {
        let exe = std::env::current_exe().unwrap();
        let exe = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("pm-engine-service");
        assert!(exe.exists(), "cargo test builds the job binary: {exe:?}");
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        let scene = scene();
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        let queued = disk
            .start_fill(1., 1.2, &exe, crate::jobs::Activity::default(), |_| {})
            .unwrap();
        assert_eq!(queued, 6);
        let bars = settle(&disk, |b| !b.running);
        assert_eq!((bars.done, bars.error.as_deref()), (6, None), "{bars:?}");
        assert_eq!(bars.bars, vec![(1., 1.2, BarState::Rendered)]);
        let time = Ticks::new(pm_time::Rate::FPS_30.frame_start(31).unwrap()).unwrap();
        let bytes = read_eventually(&disk, &scene, time).expect("rendered frame");
        assert_eq!(bytes.len(), 640 * 360 * 8);
        assert!(
            bytes.chunks_exact(8).any(|p| p[6..8] != [0, 0]),
            "the frame has drawn pixels"
        );
        // Nothing is left to render.
        let again = disk.start_fill(1., 1.2, &exe, crate::jobs::Activity::default(), |_| {});
        assert_eq!(again, Ok(0));
    }

    /// One frame filled through the real child at 1 s.
    fn filled_frame(scene: &Scene) -> Arc<Vec<u8>> {
        let exe = std::env::current_exe().unwrap();
        let exe = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("pm-engine-service");
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        disk.observe(scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        disk.start_fill(1., 1.03, &exe, crate::jobs::Activity::default(), |_| {})
            .unwrap();
        let bars = settle(&disk, |b| !b.running);
        assert_eq!(bars.error.as_deref(), None, "{bars:?}");
        let time = Ticks::new(pm_time::Rate::FPS_30.frame_start(30).unwrap()).unwrap();
        read_eventually(&disk, scene, time).expect("rendered frame")
    }

    /// A stored frame is a delivery: an effect whose extension isn't
    /// installed refuses the fill and nothing is stored (the viewer draws it
    /// bypassed live; a stored frame without it would play back silently).
    #[test]
    fn a_fill_refuses_an_effect_whose_extension_is_not_installed() {
        let mut project: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
        project["layers"][0]["fx"] = serde_json::json!([
            {"id":"e1","type":"acme.glow","on":true,"p":{}}
        ]);
        let scene = scene_from(&project.to_string());
        let exe = std::env::current_exe().unwrap();
        let exe = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("pm-engine-service");
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        disk.start_fill(1., 1.03, &exe, crate::jobs::Activity::default(), |_| {})
            .unwrap();
        let bars = settle(&disk, |b| !b.running);
        assert!(bars.error.is_some(), "{bars:?}");
        let time = Ticks::new(pm_time::Rate::FPS_30.frame_start(30).unwrap()).unwrap();
        assert!(disk.read(&scene, time).is_none(), "nothing stored");
    }

    /// Frame 30 filled through the real child, as stored.
    fn filled_values(scene: &Scene) -> Vec<f32> {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        disk.observe(scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        disk.start_fill(
            1.,
            1.03,
            &job_binary(),
            crate::jobs::Activity::default(),
            |_| {},
        )
        .unwrap();
        let bars = settle(&disk, |b| !b.running);
        assert_eq!(bars.error.as_deref(), None, "{bars:?}");
        stored_values(&disk, 30)
    }

    /// Cached playback shows what the viewer shows: an offline clip fills as
    /// the Media Offline card, never left out as an export without it would.
    #[test]
    fn a_fill_draws_offline_media_as_the_viewer_does() {
        let mut project: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
        let mut layer = project["layers"][0].clone();
        layer["id"] = "Limage".into();
        layer["type"] = "image".into();
        layer["name"] = "Gone".into();
        layer["d"] = serde_json::json!({"asset": "Aimage", "w": 200, "h": 120});
        project["layers"].as_array_mut().unwrap().push(layer);
        project["assets"] = serde_json::json!({"Aimage": {"name": "gone.png", "type": "image", "w": 200, "h": 120}});
        let offline = filled_frame(&scene_from(&project.to_string()));
        assert_ne!(
            offline,
            filled_frame(&scene()),
            "the offline clip's card is drawn"
        );
    }

    /// A cached frame is the exported frame at the viewer's precision: the
    /// cache's half floats through the export's own 16-bit conversion (its
    /// dither too) are the samples of a 16-bit PNG export of the same time,
    /// both through the real children, to within a half float's rounding
    /// ([`HALF_ROUNDING`]). An animated layer with motion blur (the paused
    /// viewer's samples and the export's differ) and a 32-bit working depth.
    #[test]
    fn a_cached_frame_is_the_exported_frame() {
        let mut project: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
        project["layers"][0]["mblur"] = true.into();
        project["colorDepth"] = 32.into();
        assert_cache_is_export(&project, false);
    }

    /// The same through background rendering, with a glow that reaches
    /// across bands: a background fill renders whole frames, as the export.
    #[test]
    fn a_background_cached_frame_is_the_exported_frame() {
        let mut project: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
        project["layers"][0]["fx"] =
            serde_json::json!([{"id": "fx-glow", "type": "glow", "on": true, "p": {}}]);
        assert_cache_is_export(&project, true);
    }

    fn assert_cache_is_export(project: &serde_json::Value, background: bool) {
        use pm_export::convert::{ConvertParams, WorkingFrame, convert};
        use pm_export::settings::{AlphaOutput, ColorTags, InputFormat, WorkingEncoding};
        let exe = job_binary();
        let scene = scene_from(&project.to_string());
        let at = Ticks::new(Rate::FPS_30.frame_start(30).unwrap()).unwrap();
        let cached = if background {
            background_values(&scene)
        } else {
            filled_values(&scene)
        };
        // The same frame exported as a 16-bit PNG, as the app exports it
        // (opaque, dithered, the project's motion blur).
        let out = tempfile::tempdir().unwrap();
        let frames = {
            let (w, h) = (640, 360);
            CacheFill {
                dir: out.path().into(),
                quota: 0,
                width: w,
                height: h,
                frames: vec![CacheFrame {
                    ticks: at.get(),
                    key: RenderKey {
                        graph: [0; 32],
                        media: [0; 32],
                        ticks: at.get(),
                        width: w,
                        height: h,
                        format: FORMAT.into(),
                        renderer_version: String::new(),
                    },
                    chunk: 0,
                    probe: false,
                }],
                budget_us: None,
                frame_ticks: 0,
                probe_scale: 1.,
            }
        };
        let source = Source {
            scene: scene.clone(),
            definitions: vec![],
            doc_id: 1,
        };
        let mut request = fill_request(&source, Rate::FPS_30, frames, 0).unwrap();
        request.cache = None;
        request.destination = out.path().join("still");
        if let pm_protocol::priority::JobSpec::Export { settings, .. } = &mut request.spec {
            settings.format =
                pm_protocol::priority::ExportFormat::PngSequence { sixteen_bit: true };
            settings.dither = true;
        }
        let export = crate::jobs::ExportHandle::start(&exe, request).unwrap();
        let status = export.wait().unwrap();
        assert_eq!(
            status.state,
            pm_protocol::priority::JobState::Done,
            "{status:?}"
        );
        let png = walk_png(out.path()).expect("the exported frame");
        let (_, decoded) = pm_export::png::read(&png, true).unwrap();
        let decoded = decoded.unwrap();
        let exported = decoded.planes[0].u16().unwrap();
        let expected = convert(
            &WorkingFrame {
                width: 640,
                height: 360,
                rgba: &cached,
            },
            InputFormat::Rgba16,
            &ConvertParams {
                working: WorkingEncoding::Standard,
                alpha: AlphaOutput::None,
                tags: ColorTags::SrgbImage,
                dither: true,
                frame: 0,
            },
        )
        .unwrap();
        let expected = expected.planes[0].u16().unwrap();
        let worst = expected
            .iter()
            .zip(exported)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= HALF_ROUNDING, "{worst} of 65535");
        // The comparison would see a missing layer or sample.
        assert!(exported.iter().any(|v| *v > 4096) && exported.iter().any(|v| *v < 60000));
    }

    /// The most a 16-bit export sample moves when its working value is
    /// rounded to a half float first: half a unit in the last of a half
    /// float's 11 significant bits is 2⁻¹² of a value below 1.0, 16 of
    /// 65535, and the transfer and the dither's rounding add at most one.
    const HALF_ROUNDING: u16 = 17;

    /// Frame 30 rendered by background rendering (its chunk known not to
    /// play), as stored.
    fn background_values(scene: &Scene) -> Vec<f32> {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        disk.set_activity(crate::jobs::Activity::default());
        disk.observe(scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        let at = Ticks::new(Rate::FPS_30.frame_start(30).unwrap()).unwrap();
        disk.measured(disk.frame_id(scene, at).unwrap(), DROPPED_US);
        disk.set_background(Background {
            enabled: true,
            delay: Duration::from_millis(100),
            floor: 0,
        });
        disk.set_background_host(Some(BackgroundHost {
            executable: job_binary(),
            pin: Arc::new(|_, _| {}),
            busy: Arc::new(|| false),
        }));
        settle(&disk, |_| present(&disk).contains(&30));
        disk.set_background_host(None);
        stored_values(&disk, 30)
    }

    fn stored_values(disk: &DiskFrames, n: i64) -> Vec<f32> {
        let (key, cache) = {
            let state = disk.inner.state.lock().unwrap();
            (
                state.frames.as_ref().unwrap().key(n).unwrap(),
                state.cache.clone().unwrap(),
            )
        };
        let coded = cache.get(&key).unwrap().expect("the stored frame");
        crate::frame_codec::decode(&coded, 640, 360)
            .unwrap()
            .chunks_exact(2)
            .map(|b| half::f16::from_le_bytes(b.try_into().unwrap()).to_f32())
            .collect()
    }

    fn walk_png(dir: &Path) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk_png(&path) {
                    return Some(found);
                }
            } else if path.extension().is_some_and(|e| e == "png") {
                return Some(path);
            }
        }
        None
    }

    /// Delete Rendered Frames while a fill runs: the fill stops first, so no
    /// frame is written after the deletion and the bar stays empty.
    #[test]
    fn deleting_during_a_fill_leaves_nothing_behind() {
        let exe = std::env::current_exe().unwrap();
        let exe = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("pm-engine-service");
        let dir = tempfile::tempdir().unwrap();
        let frames_dir = dir.path().join("frames");
        let disk = DiskFrames::default();
        disk.set_storage(Some((frames_dir.clone(), 1 << 30)))
            .unwrap();
        let scene = scene();
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        disk.start_fill(0., 3., &exe, crate::jobs::Activity::default(), |_| {})
            .unwrap();
        settle(&disk, |b| b.done >= 1);
        disk.delete_all().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let left = std::fs::read_dir(&frames_dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "frame"))
            })
            .count();
        assert_eq!(left, 0);
        let bars = disk.bars();
        assert!(bars.bars.is_empty() && !bars.running, "{bars:?}");
    }

    /// The fixture with its layers in sequence: L001 at 0-1 s, L002 at
    /// 1-2 s, the rest at 2-3 s.
    fn sequenced(edit: impl FnOnce(&mut serde_json::Value)) -> Scene {
        let mut project: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
        for layer in project["layers"].as_array_mut().unwrap() {
            let from = match layer["id"].as_str().unwrap() {
                "L001" => 0,
                "L002" => 1,
                _ => 2,
            };
            layer["from"] = from.into();
            layer["dur"] = 1.into();
        }
        edit(&mut project);
        scene_from(&project.to_string())
    }
    fn job_binary() -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        exe.parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("pm-engine-service")
    }
    fn present(disk: &DiskFrames) -> BTreeSet<i64> {
        let state = disk.inner.state.lock().unwrap();
        state
            .frames
            .as_ref()
            .unwrap()
            .present
            .iter()
            .copied()
            .collect()
    }

    /// An edit misses only the frames it changed: a frame drawn from
    /// other layers is still read after it.
    #[test]
    fn an_edit_keeps_the_frames_it_did_not_touch() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = sequenced(|_| {});
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        let (early, late) = (write_frame(&disk, 10), write_frame(&disk, 80));
        assert_eq!(present(&disk), BTreeSet::from([10, 80]));
        // L002 (1-2 s) moves: frame 80 (2.67 s) and 10 (0.33 s) don't show it.
        let mut edited = sequenced(|p| p["layers"][7]["p"]["opacity"]["v"] = 10.into());
        edited.content = 2;
        assert!(disk.observe(&edited, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        assert!(read_eventually(&disk, &edited, early).is_some());
        assert!(read_eventually(&disk, &edited, late).is_some());
        // The layers at 2-3 s change: frame 80 misses, frame 10 doesn't.
        let mut edited = sequenced(|p| p["layers"][0]["p"]["opacity"]["v"] = 10.into());
        edited.content = 3;
        assert!(disk.observe(&edited, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        assert_eq!(present(&disk), BTreeSet::from([10]));
        assert!(read_eventually(&disk, &edited, early).is_some());
        assert!(read_eventually(&disk, &edited, late).is_none());
    }

    /// With raster overrides producing, an edit anywhere misses every
    /// frame: the override's reads of the document aren't tracked.
    #[test]
    fn with_overrides_an_edit_misses_every_frame() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let t0 = Ticks::new(0).unwrap();
        let scene = sequenced(|_| {});
        disk.observe(&scene, &[], Some(7), None, None, 1, t0);
        ready(&disk);
        write_frame(&disk, 10);
        assert_eq!(present(&disk), BTreeSet::from([10]));
        // The layers at 2-3 s change; frame 10 (0.33 s) doesn't draw them.
        let mut edited = sequenced(|p| p["layers"][0]["p"]["opacity"]["v"] = 10.into());
        edited.content = 2;
        assert!(disk.observe(&edited, &[], Some(7), None, None, 1, t0));
        ready(&disk);
        assert!(present(&disk).is_empty());
    }

    /// A font installed or removed (the engine's font generation moves)
    /// is observed again, so frames are fingerprinted against the fonts
    /// installed now.
    #[test]
    fn a_font_change_is_observed_again() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let t0 = Ticks::new(0).unwrap();
        let scene = scene();
        assert!(disk.observe(&scene, &[], None, None, Some(1), 1, t0));
        ready(&disk);
        assert!(!disk.observe(&scene, &[], None, None, Some(1), 1, t0));
        assert!(disk.observe(&scene, &[], None, None, Some(2), 1, t0));
    }

    /// A chunk needs render when all its measured probes cost the budget
    /// (90 % of a frame) or more; one slow probe among fast ones is noise.
    #[test]
    fn a_chunk_is_heavy_when_all_its_probes_are_slow() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = scene();
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        let at = |n: i64| Ticks::new(Rate::FPS_30.frame_start(n).unwrap()).unwrap();
        let measure =
            |n: i64, ms: f64| disk.measured(disk.frame_id(&scene, at(n)).unwrap(), ms * 1e3);
        // Chunk 0-14: one probe slow, one fast.
        measure(0, 40.);
        measure(14, 5.);
        // Chunk 15-29: both slow.
        measure(15, 31.);
        measure(29, 45.);
        assert_eq!(disk.bars().bars, vec![(0.5, 1., BarState::Missing)]);
        let state = disk.inner.state.lock().unwrap();
        let frames = state.frames.as_ref().unwrap();
        assert_eq!(state.known.need(frames, 0, 15), Need::Light);
        assert_eq!(state.known.need(frames, 15, 30), Need::Heavy);
    }

    /// Frames playback skipped need render; a skip reported for another
    /// state (frame 14's key from before an edit) is ignored.
    #[test]
    fn dropped_frames_need_render() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let t0 = Ticks::new(0).unwrap();
        let scene = scene();
        disk.observe(&scene, &[], None, None, None, 1, t0);
        ready(&disk);
        let at = |n: i64| Ticks::new(Rate::FPS_30.frame_start(n).unwrap()).unwrap();
        let (_, stale) = disk.frame_id(&scene, at(40)).unwrap();
        disk.dropped(10, (14, stale));
        assert!(disk.bars().bars.is_empty());
        disk.dropped(10, disk.frame_id(&scene, at(14)).unwrap());
        // Frames 11-13 are in the first 0.5 s chunk.
        assert_eq!(disk.bars().bars, vec![(0., 0.5, BarState::Missing)]);
    }

    /// Background rendering through the real child, once the viewer is
    /// idle: the chunk known not to play in real time renders and is stored
    /// whole, chunks known to play are left alone, and nothing more is
    /// asked for after.
    #[test]
    fn background_rendering_stores_only_what_cannot_play() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        let scene = scene();
        let activity = crate::jobs::Activity::default();
        disk.set_activity(activity.clone());
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        // Measured costs, injected so the outcome never depends on this
        // machine's speed: every 0.5 s chunk played in real time (two fast
        // frames each) except chunk 45-59, where playback dropped frame 50.
        let at = |n: i64| Ticks::new(Rate::FPS_30.frame_start(n).unwrap()).unwrap();
        for a in (0..90).step_by(15) {
            for n in [a, a + 14] {
                disk.measured(disk.frame_id(&scene, at(n)).unwrap(), 1_000.);
            }
        }
        disk.measured(disk.frame_id(&scene, at(50)).unwrap(), DROPPED_US);
        assert_eq!(disk.bars().bars, vec![(1.5, 2., BarState::Missing)]);
        disk.set_background(Background {
            enabled: true,
            delay: Duration::from_millis(200),
            floor: 0,
        });
        disk.set_background_host(Some(BackgroundHost {
            executable: job_binary(),
            pin: Arc::new(|_, _| {}),
            busy: Arc::new(|| false),
        }));
        // The chunk that can't play renders whole; nothing else is asked for.
        settle(&disk, |b| {
            b.bars == vec![(1.5, 2., BarState::Rendered)] && !b.running
        });
        assert_eq!(present(&disk), (45..60).collect());
        let fills = disk.inner.state.lock().unwrap().fill.serial;
        assert_eq!(fills, 1, "one fill, of that chunk only");
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(
            disk.inner.state.lock().unwrap().fill.serial,
            fills,
            "nothing left"
        );
    }

    /// Background rendering never takes the volume below its floor: with
    /// less free it starts nothing and the bars say why, and it goes on by
    /// itself once there is room.
    #[test]
    fn background_rendering_stops_at_its_free_space_floor() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        let scene = scene();
        disk.set_activity(crate::jobs::Activity::default());
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        let at = Ticks::new(Rate::FPS_30.frame_start(50).unwrap()).unwrap();
        disk.measured(disk.frame_id(&scene, at).unwrap(), DROPPED_US);
        // Just under the floor.
        let free = Arc::new(std::sync::atomic::AtomicU64::new((8 << 30) - 1));
        let says = free.clone();
        disk.inner.state.lock().unwrap().free =
            Some(Arc::new(move || says.load(Ordering::Acquire)));
        disk.set_background(Background {
            enabled: true,
            delay: Duration::from_millis(100),
            floor: 8 << 30,
        });
        disk.set_background_host(Some(BackgroundHost {
            executable: job_binary(),
            pin: Arc::new(|_, _| {}),
            busy: Arc::new(|| false),
        }));
        let bars = settle(&disk, |b| b.error.is_some());
        assert_eq!(bars.error.as_deref(), Some(BACKGROUND_LOW_DISK));
        assert!(
            bars.disk_full && bars.background && !bars.running,
            "{bars:?}"
        );
        std::thread::sleep(Duration::from_millis(500));
        assert!(present(&disk).is_empty(), "nothing was rendered");
        assert!(disk.inner.running.lock().unwrap().is_none(), "no child ran");
        // Room again: it starts rendering by itself, and the note goes.
        free.store(64 << 30, Ordering::Release);
        let bars = settle(&disk, |b| b.error.is_none() && b.running);
        assert!(!bars.disk_full, "{bars:?}");
        disk.set_background_host(None);
    }

    /// Any viewer activity pauses a background fill at once (no frame
    /// beyond the one in flight), and it goes on once idle again; an export
    /// stops it.
    #[test]
    fn interaction_pauses_background_rendering_and_an_export_stops_it() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        let scene = scene();
        let activity = crate::jobs::Activity::default();
        disk.set_activity(activity.clone());
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        // Every chunk measured too slow: all 90 frames render.
        for n in (0..90).step_by(15) {
            let at = Ticks::new(Rate::FPS_30.frame_start(n).unwrap()).unwrap();
            disk.measured(disk.frame_id(&scene, at).unwrap(), DROPPED_US);
        }
        disk.set_background(Background {
            enabled: true,
            delay: Duration::from_millis(300),
            floor: 0,
        });
        let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let exporting = busy.clone();
        activity.set(true);
        disk.set_background_host(Some(BackgroundHost {
            executable: job_binary(),
            pin: Arc::new(|_, _| {}),
            busy: Arc::new(move || exporting.load(Ordering::Acquire)),
        }));
        // Busy: nothing starts.
        std::thread::sleep(Duration::from_millis(800));
        assert!(present(&disk).is_empty() && !disk.bars().running);
        activity.set(false);
        settle(&disk, |_| present(&disk).len() >= 3);
        // The person scrubs: at most the frame in flight lands.
        activity.set(true);
        std::thread::sleep(Duration::from_millis(200));
        let paused = present(&disk).len();
        std::thread::sleep(Duration::from_millis(800));
        assert!(present(&disk).len() <= paused + 1, "paused");
        assert!(disk.bars().running, "paused, not stopped");
        // Idle again: it goes on.
        activity.set(false);
        settle(&disk, |_| present(&disk).len() > paused + 3);
        // An export starts: the fill stops and nothing starts again.
        busy.store(true, Ordering::Release);
        settle(&disk, |b| !b.running);
        let stopped = present(&disk).len();
        assert!(stopped < 90);
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(present(&disk).len(), stopped);
        busy.store(false, Ordering::Release);
        settle(&disk, |_| present(&disk).len() == 90);
    }

    /// A fingerprint taken for an observation that something newer replaced
    /// never lands (here: storage changed while it was taken).
    #[test]
    fn a_stale_fingerprint_never_lands() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("a"), u64::MAX)))
            .unwrap();
        let scene = scene();
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        disk.set_storage(Some((dir.path().join("b"), u64::MAX)))
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(disk.inner.state.lock().unwrap().frames.is_none());
    }
}
