//! Disk render cache (plan section 1, pro media): a composition's frames as
//! the paused viewer draws them, rendered in the background, kept in
//! Storage › Renders, and shown by playback and scrubbing instead of
//! rendering them again.
//!
//! - **Identity.** A frame is keyed by a global fingerprint (the contributed
//!   definitions, each media file as a complete file version (its own, or
//!   the first one proved to hold all of the same bytes: `MediaIds`), the
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
use pm_text::file_version::FileVersion;
use pm_time::Rate;
use serde::{Deserialize, Serialize};

use crate::render::Scene;

/// Frames on disk: RGBA16F rows, top-down, the export's premultiplied
/// working values rounded to the half floats the viewer's surface holds,
/// coded losslessly (`crate::frame_codec`).
pub const FORMAT: &str = "rgba16f-export-lzfse-v4";
/// Media stamps are looked at again at most this often.
pub(crate) const MEDIA_RECHECK: Duration = Duration::from_secs(2);
/// The same storage is indexed again at most this often.
const REINDEX_EVERY: Duration = Duration::from_secs(2);
/// Frames read ahead of the playhead, and kept read.
const AHEAD: i64 = 6;
const READY_FRAMES: usize = 8;
/// Reader threads.
const READERS: usize = 3;
/// Frames kept read at a paused playhead, so a play starts from them.
const WARM: i64 = 3;
/// The most the reader holds: read frames, and for every read asked for or
/// in flight its decoded frame and its coded bytes (twice the frame at
/// most). Look-ahead is what fits: six frames ahead at 1080p (as measured
/// for playback without a dropped frame), three at 4K, one frame at a time
/// at 8K. Half under memory pressure; at Critical only the frame the
/// playhead is on.
pub(crate) const READ_AHEAD_BYTES: u64 = 256 << 20;
/// The most kept read while stopped ([`WARM`] frames of 1080p).
const WARM_BYTES: u64 = 64 << 20;
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
    /// The media files as the frames' keys took them. A frame is stored
    /// only when every file it read is the version named here
    /// ([`reads_are`]).
    #[serde(default)]
    pub media: Vec<Stamp>,
    /// The folder of files named by their bytes ([`keyed`]).
    #[serde(default)]
    pub content_named: Option<PathBuf>,
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
    /// Stored, and where its bytes came from is established: every media
    /// file it read is one its key names, at the version named
    /// ([`reads_listed`]), and every font file read is among the fonts
    /// asked for, at its version (`pm_text::system_fonts::Fence`).
    #[serde(default)]
    pub proved: bool,
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
/// A fill child's end when a media file it read is not the version its
/// frames' keys were taken from: the frame is not stored, and the fill is
/// planned again from the files as they are.
pub const MEDIA_CHANGED: &str = "Rendering stopped because a media file changed. Render again.";
/// A fill child's failure when kept frames fill Settings › Media › Render Files Limit.
pub const LIMIT_FULL: &str =
    "Kept rendered frames fill the Render Files Limit. Raise the limit or delete rendered frames.";

/// One media file as a frame's key takes it: its path and its version
/// (`pm_text::file_version`: length, modification time, device, inode and
/// status-change time, the identity the renderer keys what it reads on). A
/// file named by its bytes is its length alone, and one that cannot be
/// looked at the empty version ([`keyed`]).
pub(crate) type Stamp = (PathBuf, FileVersion);
/// A frame's key digest: the global fingerprint, its inputs' digest and its
/// time.
type FrameId = [u8; 32];

/// What a read frame's pixels are of: the frame's key digest, and the cache
/// generation it was read under. Whoever holds the pixels past the read (an
/// upload made ahead of its turn) shows them only while both still are the
/// frame's ([`DiskFrames::held`]): an observation of anything the fingerprint
/// covers, a storage change or a deletion moves the generation on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Held {
    generation: u64,
    id: FrameId,
}

/// A scene's document, revision, content and composition.
type SceneState = (usize, u64, u64, String);

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
    /// The fonts its frames' keys were taken from (`None` until they are
    /// fingerprinted): a fill draws with exactly these.
    fonts: Option<Arc<pm_text::system_fonts::Inventory>>,
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
    /// `generation` as of the last published change, for the indexer to
    /// read without the lock ([`Indexer`]).
    live: Arc<AtomicU64>,
    /// Rises with every storage change or deletion: fences fills' frames.
    storage: u64,
    frames: Option<Frames>,
    /// The scene state last looked at, and whether the path-traced 3D
    /// renderer draws it ([`crate::frame_inputs::draws_studio`]): such a
    /// composition has no `frames`, so none is rendered to disk or shown
    /// from it.
    studio: Option<(SceneState, bool)>,
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
    indexer: Indexer,
    /// What each media file's bytes are, by the version they were read at.
    media_ids: Arc<MediaIds>,
    /// Test hook: the next fills name no media file to their child, as a
    /// fill whose frames read a file their keys do not name.
    #[cfg(test)]
    unnamed_media: std::sync::atomic::AtomicBool,
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

/// What a Render Selection is asked of: a document at a revision, one of its
/// compositions, drawn with these definitions.
#[derive(Clone, Copy)]
pub struct Observation<'a> {
    pub doc_id: u64,
    pub revision: u64,
    pub comp: &'a str,
    pub definitions: &'a [DefinitionSnapshot],
}
impl Observation<'_> {
    fn is(&self, frames: &Frames) -> bool {
        frames.source.doc_id == self.doc_id
            && frames.identity.revision == self.revision
            && frames.identity.comp == self.comp
            && frames.source.definitions == self.definitions
    }
}

/// The engine's render cache, shared by the engine thread (observation,
/// reads) and the app (commands, the render bar).
#[derive(Clone)]
pub struct DiskFrames {
    inner: Arc<Inner>,
}
impl Default for DiskFrames {
    fn default() -> Self {
        let state = State::default();
        let indexer = Indexer::start(state.live.clone());
        let disk = Self {
            inner: Arc::new(Inner {
                state: Mutex::new(state),
                published: Mutex::default(),
                running: Mutex::default(),
                reader: Reader::start(),
                indexer,
                media_ids: Arc::default(),
                #[cfg(test)]
                unnamed_media: Default::default(),
                delay_ms: Arc::new(AtomicU64::new(
                    Background::default().delay.as_millis() as u64
                )),
                scheduling: std::sync::Once::new(),
            }),
        };
        // A proof that a file's bytes are another version's: the open
        // state's frames are those of the other version from now on.
        let inner = Arc::downgrade(&disk.inner);
        let _ = disk.inner.media_ids.proved.set(Box::new(move || {
            if let Some(inner) = inner.upgrade() {
                DiskFrames { inner }.fingerprint_again();
            }
        }));
        disk
    }
}

/// Something the render bar shows changed: publish a snapshot and tell
/// whoever listens (never blocking).
fn bump(state: &mut State, published: &Mutex<Arc<RenderBars>>) {
    state.live.store(state.generation, Ordering::Release);
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
        let (generation, at, size) = self.with(|s| {
            (
                (s.generation, s.last_read, s.frames.as_ref().map(|f| f.size)),
                false,
            )
        });
        match (at, size) {
            (Some(n), Some(size)) => {
                let warm = WARM.min((WARM_BYTES / frame_bytes(size)) as i64);
                self.inner.reader.keep(generation, n, n + warm - 1)
            }
            _ => self.inner.reader.clear(generation),
        }
    }

    /// Every byte the reader holds or is about to: read frames, and reads
    /// asked for or in flight ([`READ_AHEAD_BYTES`]).
    pub(crate) fn ready_bytes(&self) -> u64 {
        self.inner.reader.ready.lock().unwrap().held()
    }

    /// The process's memory pressure, as the engine sees it at every frame:
    /// the reader's budget follows it at once (frames beyond it are
    /// dropped, reads not yet started are cancelled).
    pub(crate) fn memory_pressure(&self, level: crate::memory::Pressure) {
        self.inner.reader.pressure(level);
    }

    /// The reader's budget at no pressure (tests: a small lease).
    #[cfg(test)]
    pub(crate) fn set_read_budget(&self, bytes: u64) {
        let mut ready = self.inner.reader.ready.lock().unwrap();
        ready.cap = bytes;
        ready.trim();
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
            Some((dir, quota)) => {
                let cache = RenderCache::open(dir, *quota)?;
                // Frames keyed by another version of the keys, the format
                // or the renderer can never be found again, and the kept
                // ones among them were never evicted: they go now, before
                // anything is written under this one.
                match cache.retire_other_schemas(&schema()) {
                    Ok((0, _)) => {}
                    Ok((entries, bytes)) => {
                        eprintln!(
                            "Removed {entries} rendered frames of an earlier version ({bytes} bytes)."
                        );
                        diagnostic(serde_json::json!({
                            "retired_entries": entries, "retired_bytes": bytes,
                        }));
                    }
                    Err(error) => {
                        eprintln!("Rendered frames of earlier versions were not removed: {error}")
                    }
                }
                // What its frames' media were, known across launches.
                self.inner.media_ids.open(dir);
                Some(Arc::new(cache))
            }
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

    /// Whether the path-traced 3D renderer draws `scene`
    /// ([`crate::frame_inputs::draws_studio`]), looked up once per scene
    /// state: the frame loop asks again for every frame.
    pub(crate) fn draws_studio(&self, scene: &Scene) -> bool {
        let at: SceneState = (
            Arc::as_ptr(&scene.doc) as usize,
            scene.doc.revision(),
            scene.content,
            scene.comp.clone(),
        );
        if let Some((known, studio)) = &self.inner.state.lock().unwrap().studio
            && *known == at
        {
            return *studio;
        }
        let studio = crate::frame_inputs::draws_studio(&scene.doc, &scene.comp);
        self.inner.state.lock().unwrap().studio = Some((at, studio));
        studio
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
        drop(state);
        let studio = self.draws_studio(scene);
        let mut state = self.inner.state.lock().unwrap();
        if studio {
            // Path-traced frames are never kept on disk: whatever was
            // rendered for an earlier state of this composition stops
            // showing, and nothing new is rendered.
            if state.frames.take().is_none() {
                return false;
            }
            state.generation += 1;
            self.inner.reader.clear(state.generation);
            bump(&mut state, &self.inner.published);
            return true;
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
        self.install(state, scene, definitions, identity, doc_id);
        true
    }

    /// A new composition state: its frames are fingerprinted and found on
    /// disk off this thread, and whatever was read or is being rendered
    /// for the state before is not this one's.
    fn install(
        &self,
        mut state: std::sync::MutexGuard<'_, State>,
        scene: &Scene,
        definitions: &[DefinitionSnapshot],
        identity: Identity,
        doc_id: u64,
    ) {
        let Some((size, rate, count)) = composition(scene) else {
            state.frames = None;
            state.generation += 1;
            self.inner.reader.clear(state.generation);
            bump(&mut state, &self.inner.published);
            return;
        };
        let source = Source {
            scene: scene.clone(),
            definitions: definitions.to_vec(),
            doc_id,
            fonts: None,
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
        let cache = state
            .cache
            .clone()
            .expect("a state is observed with storage");
        drop(state);
        // One worker takes it: a newer observation replaces one still
        // waiting, and the one being worked on stops at its next stage.
        self.inner.indexer.submit(Indexing {
            inner: Arc::downgrade(&self.inner),
            media_ids: self.inner.media_ids.clone(),
            content_named: self.content_named(),
            generation,
            source,
            identity,
            cache,
            size,
            rate,
            count,
        });
    }

    /// Looks at the open state's media again without a frame asking: a
    /// paused viewer draws nothing, yet a file saved again by another app
    /// retires the frames rendered from it and stops a fill of them.
    /// Looked at every [`MEDIA_RECHECK`] while there is something to
    /// retire (`now`: at once, a fill child met a changed file).
    fn recheck_media(&self, now: bool) {
        let looked = {
            let mut state = self.inner.state.lock().unwrap();
            let due = now
                || state
                    .media_checked
                    .is_none_or(|at| at.elapsed() >= MEDIA_RECHECK);
            let concerned = state.cache.is_some()
                && state
                    .frames
                    .as_ref()
                    .is_some_and(|f| now || state.fill.running || !f.present.is_empty());
            if !due || !concerned {
                return;
            }
            state.media_checked = Some(Instant::now());
            let frames = state.frames.as_ref().expect("checked above");
            (
                frames.source.clone(),
                frames.identity.clone(),
                state.generation,
                state.content_named.clone(),
            )
        };
        let (source, identity, generation, content_named) = looked;
        let media = stamps(&source.scene.media, content_named.as_deref());
        if media == identity.media {
            return;
        }
        let state = self.inner.state.lock().unwrap();
        // Only the state that was looked at: a newer observation has its own.
        if state.generation == generation && state.cache.is_some() {
            self.install(
                state,
                &source.scene,
                &source.definitions,
                Identity { media, ..identity },
                source.doc_id,
            );
        }
    }

    /// Takes the open state's fingerprint again (what names its media
    /// changed: [`MediaIds::named`]).
    fn fingerprint_again(&self) {
        let state = self.inner.state.lock().unwrap();
        let Some(frames) = state.frames.as_ref().filter(|_| state.cache.is_some()) else {
            return;
        };
        let (source, identity) = (frames.source.clone(), frames.identity.clone());
        self.install(
            state,
            &source.scene,
            &source.definitions,
            identity,
            source.doc_id,
        );
    }

    /// The versions of `media`'s files as this cache's keys take them.
    pub(crate) fn media_stamps(&self, media: &pm_export::render::MediaPaths) -> Vec<Stamp> {
        let content_named = self.content_named();
        stamps(media, content_named.as_deref())
    }

    /// The folder of files named by their bytes ([`Self::set_content_named`]).
    pub(crate) fn content_named(&self) -> Option<PathBuf> {
        self.with(|s| (s.content_named.clone(), false))
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

    /// The viewer has not yet given this cache a composition to render (no
    /// settled frame observed): a Render Selection asked now waits for one.
    pub fn reading(&self) -> bool {
        let state = self.inner.state.lock().unwrap();
        state
            .frames
            .as_ref()
            .is_none_or(|frames| frames.fingerprint.is_none() || frames.ids.is_empty())
    }
    /// The viewer has drawn a settled frame of exactly `of`. An observation
    /// of anything else (another project open in the window before, an
    /// earlier revision, another composition, an effect's code before its
    /// extension reloaded) is not it, so a Render Selection of this does
    /// not start from that.
    pub fn observed(&self, of: Observation) -> bool {
        let state = self.inner.state.lock().unwrap();
        state.frames.as_ref().is_some_and(|frames| {
            frames.fingerprint.is_some() && !frames.ids.is_empty() && of.is(frames)
        })
    }
    /// [`Self::start_fill`] of exactly `of`, checked as the frames are
    /// chosen: refused when the cache holds anything else (the window moved
    /// on while the Render Selection waited, or the viewer never drew this).
    pub fn start_fill_of(
        &self,
        of: Observation,
        from: f64,
        to: f64,
        executable: &Path,
        activity: crate::jobs::Activity,
        overrides: impl FnOnce(&mut crate::jobs::JobSession),
    ) -> Result<u64, String> {
        let frames = {
            let state = self.inner.state.lock().unwrap();
            let frames = ready(&state)?;
            if !of.is(frames) {
                return Err("The composition changed before rendering started.".into());
            }
            frame_range(frames, from, to)
                .filter(|n| !frames.present.contains(n))
                .filter_map(|n| Some((n, cache_frame(frames, n, 0, false)?)))
                .collect::<Vec<_>>()
        };
        self.launch(frames, None, executable, activity, overrides)
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
        let order: Vec<(i64, RenderKey)> =
            frames.iter().map(|(n, f)| (*n, f.key.clone())).collect();
        let (source, size, rate) = (frames_now.source.clone(), frames_now.size, frames_now.rate);
        let media = frames_now.identity.media.clone();
        let answering = frames_now.identity.overrides;
        let (cache, content_named) = (state.cache.clone(), state.content_named.clone());
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
            media,
            content_named,
        };
        #[cfg(test)]
        let fill = CacheFill {
            media: if self.inner.unnamed_media.load(Ordering::Acquire) {
                Vec::new()
            } else {
                fill.media
            },
            ..fill
        };
        let unbound = answering;
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
            .spawn(move || run_fill(weak, serial, storage, cache, unbound, order, started))
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
        self.retire_gone();
        self.recheck_media(false);
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
            Read::Frame(size, bytes, _) => Some((size, bytes)),
            Read::Late(_) | Read::Absent => None,
        }
    }

    /// Frames a read found missing or damaged ([`Reader::read_asks`]) are
    /// not rendered: they leave the index, the render bar shows them
    /// unrendered and background rendering renders them again. A frame
    /// written again in the meantime (a stat, outside the lock) stays.
    fn retire_gone(&self) {
        let gone = {
            let state = self.inner.state.lock().unwrap();
            let Some(frames) = state.frames.as_ref() else {
                return;
            };
            let gone = self.inner.reader.gone(state.generation);
            if gone.is_empty() {
                return;
            }
            let Some(cache) = state.cache.clone() else {
                return;
            };
            let keys: Vec<(i64, RenderKey)> = gone
                .into_iter()
                .filter_map(|n| Some((n, frames.key(n)?)))
                .collect();
            (state.generation, cache, keys)
        };
        let (generation, cache, keys) = gone;
        let absent: Vec<i64> = keys
            .iter()
            .filter(|(_, key)| !matches!(cache.presence(key), Ok(Some(_))))
            .map(|(n, _)| *n)
            .collect();
        self.with(|s| {
            let Some(frames) = s.frames.as_mut().filter(|_| s.generation == generation) else {
                return ((), false);
            };
            let mut changed = false;
            for n in absent {
                changed |= frames.present.remove(&n);
                frames.kept.remove(&n);
            }
            ((), changed)
        });
    }

    /// [`Self::read`], waiting up to `patience` for the frame when it is on
    /// disk and still being read (a playing frame: [`PATIENCE`]).
    pub(crate) fn read_within(&self, scene: &Scene, time: Ticks, patience: Duration) -> Read {
        self.retire_gone();
        let Some((size, held, n, found)) = self.ask_ahead(scene, time, AHEAD, false) else {
            return Read::Absent;
        };
        match found {
            Some(bytes) => Read::Frame(size, bytes, held),
            None => match self.inner.reader.wait(held.generation, n, patience) {
                Ok(Some(bytes)) => Read::Frame(size, bytes, held),
                Ok(None) => Read::Absent,
                Err(Late) => Read::Late(size),
            },
        }
    }

    /// What `scene`'s rendered frame at `time` is now ([`Held`]): `None` when
    /// this state has no such frame on disk.
    pub(crate) fn held(&self, scene: &Scene, time: Ticks) -> Option<Held> {
        let state = self.inner.state.lock().unwrap();
        let frames = state.frames.as_ref()?;
        if !frames.identity.scene_matches(scene) {
            return None;
        }
        let n = frames.index(time)?;
        frames.present.contains(&n).then_some(Held {
            generation: state.generation,
            id: frames.id(n)?,
        })
    }

    /// The scene the open composition's rendered frames belong to: the
    /// viewer's own, for reading them back through a [`crate::Headless`].
    pub fn observed_scene(&self) -> Option<Scene> {
        let state = self.inner.state.lock().unwrap();
        Some(state.frames.as_ref()?.source.scene.clone())
    }

    /// `scene`'s rendered frame at `time` as playback reads it (RGBA half
    /// floats, row by row), waiting up to ten seconds for the read.
    pub fn stored_frame(&self, scene: &Scene, time: Ticks) -> Option<Arc<Vec<u8>>> {
        match self.read_within(scene, time, Duration::from_secs(10)) {
            Read::Frame(_, bytes, _) => Some(bytes),
            Read::Late(_) | Read::Absent => None,
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
        self.retire_gone();
        self.ask_ahead(scene, time, WARM, true);
    }

    /// Asks the reader for the frame at `time` and the `ahead - 1` after it
    /// in the direction of motion (forward when `still`: a play starts
    /// forward): the stored size, what the frame is of, its index, and the
    /// frame if it is read already.
    #[allow(clippy::type_complexity)]
    fn ask_ahead(
        &self,
        scene: &Scene,
        time: Ticks,
        ahead: i64,
        still: bool,
    ) -> Option<(Size, Held, i64, Option<Arc<Vec<u8>>>)> {
        let mut state = self.inner.state.lock().unwrap();
        let state = &mut *state;
        let frames = state.frames.as_ref()?;
        if !frames.identity.scene_matches(scene) || frames.fingerprint.is_none() {
            return None;
        }
        let n = frames.index(time)?;
        let id = frames.id(n)?;
        if let Some(last) = state.last_read.replace(n)
            && last != n
        {
            state.backward = n < last;
        }
        state.backward &= !still;
        let size = frames.size;
        // As far ahead as the reader's budget holds frames of this size.
        let ahead = ahead.min(self.inner.reader.fit(size));
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
            self.inner
                .reader
                .ask(generation, k, key, &cache, size, k == n);
        }
        Some((size, Held { generation, id }, n, found))
    }
}

/// What tests elsewhere in the crate need of the cache's state.
#[cfg(test)]
impl DiskFrames {
    /// Waits until the observed state's fingerprint is taken.
    pub(crate) fn test_ready(&self) {
        let start = Instant::now();
        while self
            .inner
            .state
            .lock()
            .unwrap()
            .frames
            .as_ref()
            .is_none_or(|f| f.fingerprint.is_none())
        {
            // A liveness bound: the process's first look at the installed
            // fonts alone takes tens of seconds on a loaded machine.
            assert!(start.elapsed() < Duration::from_secs(120));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// Frame `n` of the observed state written as a fill child writes it,
    /// every sample `value`: its time.
    pub(crate) fn test_put(&self, n: i64, value: f32) -> Ticks {
        let (key, time, size, cache) = {
            let state = self.inner.state.lock().unwrap();
            let frames = state.frames.as_ref().unwrap();
            (
                frames.key(n).unwrap(),
                frames.ticks(n).unwrap(),
                frames.size,
                state.cache.clone().unwrap(),
            )
        };
        let bytes: Vec<u8> = std::iter::repeat_n(
            half::f16::from_f32(value).to_le_bytes(),
            (size.width * size.height * 4) as usize,
        )
        .flatten()
        .collect();
        cache
            .put(
                &key,
                &crate::frame_codec::encode(&bytes, size.width, size.height),
            )
            .unwrap();
        self.reindex(true).unwrap();
        time
    }
    /// Media stamps are looked at again at the next observation.
    pub(crate) fn test_recheck_media(&self) {
        self.inner.state.lock().unwrap().media_checked = None;
    }
}

/// The open composition's frames, if a fill can start from them.
fn ready(state: &State) -> Result<&Frames, String> {
    if state.frames.is_none() && state.studio.as_ref().is_some_and(|(_, studio)| *studio) {
        return Err("Rendering to disk isn't available for a composition with 3D objects.".into());
    }
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
        // background).
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
        // The viewer's motion blur, not the Export sheet's default: every
        // viewer frame draws the blur of the layers that have it on, and a
        // rendered frame is the paused viewer's picture (drawn with the
        // export's sample count, as a paused frame is).
        motion_blur: true,
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
        copies_of: source.scene.media.originals.clone(),
        destination: dir,
        shader_cache: crate::jobs::shader_cache(),
        faces_of: crate::jobs::font_broker().map(|broker| broker.generation()),
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
        studio_quality: None,
        studio_render_scale:None,
        fonts: source.fonts.as_deref().cloned(),
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
///
/// A stored frame whose report is not taken (the state has moved on) stays
/// on disk only when where it came from is established: the child proved
/// that every media file it read is one its key names, at the version
/// named, and that every font file it read is among the fonts asked for,
/// at its version (`FrameReport::proved`), and the override wrappers
/// answering are still the ones the fill began with (`unbound`: their
/// answers come from the app as it is now, nothing the child can hold).
/// Otherwise, or when that state is gone, it is removed from `cache`, the
/// store it was written to: an entry of unknown provenance, found again
/// when its key comes back after an undo, would be shown as the key's
/// picture. (A proved frame rendered while an edit, a font change or a
/// media change was in the way stays: it is its key's picture.)
fn run_fill(
    inner: Weak<Inner>,
    serial: u64,
    storage: u64,
    cache: Option<Arc<RenderCache>>,
    unbound: Option<u64>,
    order: Vec<(i64, RenderKey)>,
    started: Result<crate::jobs::ExportHandle, String>,
) {
    use pm_protocol::priority::JobState;
    let land = |reports: Vec<FrameReport>, done: u64, end: Option<(Option<String>, bool)>| {
        let Some(inner) = inner.upgrade() else {
            // The cache is gone with its window: nothing vouches for them.
            for report in reports.iter().filter(|report| report.stored) {
                if let (Some(cache), Some((_, key))) = (&cache, order.get(report.at as usize)) {
                    let _ = cache.discard(key);
                }
            }
            return;
        };
        let mut not_taken = Vec::new();
        {
            let mut state = inner.state.lock().unwrap();
            let state = &mut *state;
            let current = state.fill.serial == serial;
            for report in reports {
                let Some((n, key)) = order.get(report.at as usize) else {
                    continue;
                };
                let (n, id) = (*n, key.graph);
                let mut taken = false;
                if let Some(frames) = state.frames.as_mut() {
                    if let Some(cost) = report.cost_us {
                        state.known.cost(frames, Some(n), id, cost);
                    }
                    if report.stored && state.storage == storage && frames.id(n) == Some(id) {
                        frames.present.insert(n);
                        taken = true;
                    }
                }
                // Kept only when where it came from is established: the
                // child proved the media and fonts it read are its key's
                // (an entry of unknown provenance found again when the key
                // comes back, after an undo, would be shown as that key's
                // picture), and the override wrappers answering, which the
                // child cannot hold itself, are the ones the fill began
                // with.
                let held = report.proved
                    && state
                        .frames
                        .as_ref()
                        .is_some_and(|f| f.identity.overrides == unbound);
                if report.stored && !taken && !held {
                    not_taken.push(key);
                }
                if report.refused {
                    state.known.refuse(id);
                }
                if current {
                    state.fill.queued.remove(&n);
                }
            }
            if current {
                state.fill.done = state.fill.done.max(done);
                if let Some((error, disk_full)) = end {
                    state.fill.running = false;
                    state.fill.queued.clear();
                    // Background rendering stopped at its floor says so in
                    // its own words (the child's are an export's).
                    let error = if state.fill.background
                        && disk_full
                        && error.as_deref() != Some(LIMIT_FULL)
                    {
                        Some(BACKGROUND_LOW_DISK.to_owned())
                    } else {
                        // A fill that met a changed file keeps its reason:
                        // the state is taken again at once (below), and
                        // rendering starts over for the new one; were the
                        // files found unchanged after all, the same fill is
                        // not started again and again.
                        error
                    };
                    state.fill.error = error;
                    state.fill.disk_full = disk_full;
                    state.fill.ended = Some(Instant::now());
                }
                bump(state, &inner.published);
            }
        }
        // Off the lock (the viewer's worker takes it every frame). No other
        // fill can have stored these keys again meanwhile: one fill runs at
        // a time, and the next starts only when this thread has ended.
        for key in not_taken {
            if let Some(cache) = &cache {
                let _ = cache.discard(key);
            }
        }
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
    let media_changed = end.1.as_deref() == Some(MEDIA_CHANGED);
    land(reports, end.0, Some((end.1, end.2)));
    if let Some(inner) = inner.upgrade() {
        let disk = DiskFrames { inner };
        // The child met a file that is not the one its keys name: the
        // state is taken again from the files as they are.
        if media_changed {
            disk.recheck_media(true);
        }
        // The limit may have evicted older frames as these landed.
        let _ = disk.reindex(true);
    }
}

/// What [`DiskFrames::read_within`] found.
pub(crate) enum Read {
    /// The frame, at its stored size, and what it is of.
    Frame(Size, Arc<Vec<u8>>, Held),
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
struct Ready {
    frames: VecDeque<(ReadId, Arc<Vec<u8>>)>,
    asked: HashSet<(u64, i64)>,
    /// The frames the latest read wants (generation, first, last): asks
    /// queued before a reversal or a jump are dropped, not read.
    wanted: Option<(u64, i64, i64)>,
    /// Frames a read found missing or damaged on disk.
    gone: Vec<ReadId>,
    /// [`READ_AHEAD_BYTES`], and the memory pressure that scales it.
    cap: u64,
    pressure: crate::memory::Pressure,
    /// One frame's bytes at the size last asked for.
    frame: u64,
    /// The most ever held (tests).
    peak: u64,
}
impl Default for Ready {
    fn default() -> Self {
        Self {
            frames: VecDeque::new(),
            asked: HashSet::new(),
            wanted: None,
            gone: Vec::new(),
            cap: READ_AHEAD_BYTES,
            pressure: crate::memory::Pressure::Normal,
            frame: 0,
            peak: 0,
        }
    }
}
/// A frame's decoded bytes (RGBA16F).
fn frame_bytes(size: Size) -> u64 {
    (u64::from(size.width) * u64::from(size.height) * 8).max(1)
}
impl Ready {
    fn budget(&self) -> u64 {
        match self.pressure {
            crate::memory::Pressure::Normal => self.cap,
            crate::memory::Pressure::Warning => self.cap / 2,
            crate::memory::Pressure::Critical => 0,
        }
    }
    /// Read frames, and twice a frame (decoded, and its coded bytes at
    /// most) for every read asked for or in flight.
    fn held(&self) -> u64 {
        self.frames
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .sum::<u64>()
            + self.asked.len() as u64 * 2 * self.frame
    }
    fn in_window(&self, (generation, n): ReadId) -> bool {
        self.wanted
            .is_some_and(|(g, first, last)| g == generation && (first..=last).contains(&n))
    }
    /// Back within the budget: frames outside the wanted range go first,
    /// then the furthest from where it starts. The frame it starts at stays
    /// (the picture on screen is read from it).
    fn trim(&mut self) {
        let budget = self.budget();
        if self.held() <= budget {
            return;
        }
        let keep: Vec<bool> = self
            .frames
            .iter()
            .map(|(id, _)| self.in_window(*id))
            .collect();
        let mut keep = keep.into_iter();
        self.frames.retain(|_| keep.next().unwrap_or(false));
        while self.held() > budget && self.frames.len() > 1 {
            let first = self.wanted.map_or(0, |(_, first, _)| first);
            let furthest = self
                .frames
                .iter()
                .enumerate()
                .max_by_key(|(_, ((_, n), _))| (n - first).abs())
                .map(|(i, _)| i);
            match furthest {
                Some(i) => self.frames.remove(i),
                None => break,
            };
        }
    }
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
            let stale = {
                let mut ready = shared.lock().unwrap();
                let (generation, n) = ask.id;
                let stale = ready
                    .wanted
                    .is_some_and(|(g, first, last)| g != generation || n < first || n > last);
                if stale {
                    ready.asked.remove(&ask.id);
                }
                stale
            };
            if stale {
                end.notify_all();
                continue;
            }
            // Stored as the viewer's surface holds it. An entry that is not
            // there, or whose bytes are wrong (damaged on disk, or not a
            // frame of this size), is not a rendered frame: it leaves the
            // index ([`DiskFrames::retire_gone`]) and a damaged file is
            // removed, so the frame shows as unrendered and is rendered
            // again. A read that merely failed is asked again next time.
            let data = match ask.cache.get(&ask.key) {
                Ok(Some(d)) => {
                    crate::frame_codec::decode(&d, ask.size.width, ask.size.height).ok_or(true)
                }
                Ok(None) => Err(true),
                Err(e) => Err(pm_promedia::cache::corrupt(&e)),
            };
            match &data {
                // Played frames are the last the limit removes.
                Ok(_) => {
                    let _ = ask.cache.touch(&ask.key);
                }
                Err(true) => {
                    let _ = ask.cache.discard(&ask.key);
                }
                Err(false) => {}
            }
            let mut ready = shared.lock().unwrap();
            ready.asked.remove(&ask.id);
            end.notify_all();
            if matches!(data, Err(true)) {
                ready.gone.push(ask.id);
            }
            if ready.wanted.is_some_and(|(g, first, last)| {
                ask.id.0 != g || ask.id.1 < first || ask.id.1 > last
            }) {
                continue;
            }
            if let Ok(data) = data {
                ready.frames.push_back((ask.id, Arc::new(data)));
                while ready.frames.len() > READY_FRAMES {
                    ready.frames.pop_front();
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
            cap: ready.cap,
            pressure: ready.pressure,
            peak: ready.peak,
            ..Default::default()
        };
    }
    /// How many frames of `size` the budget holds read (at least the one
    /// the playhead is on).
    fn fit(&self, size: Size) -> i64 {
        let ready = self.ready.lock().unwrap();
        ((ready.budget() / frame_bytes(size)) as i64).max(1)
    }
    fn pressure(&self, level: crate::memory::Pressure) {
        let mut ready = self.ready.lock().unwrap();
        if ready.pressure == level {
            return;
        }
        ready.pressure = level;
        // Reads asked for and not started are cancelled by the range: at
        // Critical only the frame it starts at is wanted.
        if level == crate::memory::Pressure::Critical
            && let Some((g, first, last)) = ready.wanted
            && last > first
        {
            ready.wanted = Some((g, first, first));
        }
        ready.trim();
        drop(ready);
        self.ended.notify_all();
    }
    /// The frames of `generation` reads found missing or damaged since the
    /// last call.
    fn gone(&self, generation: u64) -> Vec<i64> {
        let mut ready = self.ready.lock().unwrap();
        std::mem::take(&mut ready.gone)
            .into_iter()
            .filter(|(g, _)| *g == generation)
            .map(|(_, n)| n)
            .collect()
    }
    fn want(&self, generation: u64, first: i64, last: i64) {
        self.ready.lock().unwrap().wanted = Some((generation, first, last));
    }
    /// Asks for frame `n` if the budget admits another read (`head`: the
    /// frame the playhead is on, read whatever the budget: everything else
    /// read gives way to it). A frame not admitted is asked again at the
    /// next frame.
    fn ask(
        &self,
        generation: u64,
        n: i64,
        key: RenderKey,
        cache: &Arc<RenderCache>,
        size: Size,
        head: bool,
    ) {
        let id = (generation, n);
        {
            let mut ready = self.ready.lock().unwrap();
            if ready.asked.contains(&id) || ready.frames.iter().any(|(i, _)| *i == id) {
                return;
            }
            ready.frame = frame_bytes(size);
            let need = 2 * ready.frame;
            if ready.held() + need > ready.budget() {
                // Frames outside the wanted range make room first.
                let keep: Vec<bool> = ready
                    .frames
                    .iter()
                    .map(|(i, _)| ready.in_window(*i))
                    .collect();
                let mut keep = keep.into_iter();
                ready.frames.retain(|_| keep.next().unwrap_or(false));
            }
            if ready.held() + need > ready.budget() {
                if !head || !ready.asked.is_empty() {
                    return;
                }
                ready.frames.clear();
            }
            ready.asked.insert(id);
            ready.peak = ready.peak.max(ready.held());
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
    index_until(cache, keys, &|| false).expect("never stopped")
}
/// [`index`], given up (`None`) as soon as `stop` says so, asked every 64
/// frames: a long composition on a slow volume is not looked through for an
/// observation nobody waits for any more.
fn index_until(
    cache: &RenderCache,
    keys: &[(i64, RenderKey)],
    stop: &dyn Fn() -> bool,
) -> Option<(HashSet<i64>, HashSet<i64>)> {
    let (mut present, mut kept) = (HashSet::new(), HashSet::new());
    for chunk in keys.chunks(64) {
        if stop() {
            return None;
        }
        for (n, key) in chunk {
            if let Ok(Some(keep)) = cache.presence(key) {
                present.insert(*n);
                if keep {
                    kept.insert(*n);
                }
            }
        }
    }
    Some((present, kept))
}

/// One observation to fingerprint and find on disk.
struct Indexing {
    inner: Weak<Inner>,
    media_ids: Arc<MediaIds>,
    content_named: Option<PathBuf>,
    generation: u64,
    source: Source,
    identity: Identity,
    cache: Arc<RenderCache>,
    size: Size,
    rate: Rate,
    count: i64,
}
/// Takes fingerprints and finds frames on disk on one thread, one
/// observation at a time. A newer observation replaces the one waiting (and
/// frees its document snapshot); the one being worked on gives up at its
/// next stage once the cache's generation has moved on (another
/// observation, a storage change, a deletion, a closed project). Rapid
/// edits or alternating viewers on a slow volume therefore cost one job in
/// flight and one waiting, never a thread and a snapshot each.
struct Indexer {
    shared: Arc<IndexerShared>,
}
#[derive(Default)]
struct IndexerShared {
    slot: Mutex<(Option<Indexing>, bool)>,
    wake: std::sync::Condvar,
    live: Arc<AtomicU64>,
    /// Jobs begun (tests).
    begun: AtomicU64,
    /// Held by a test to keep the worker at the start of a job.
    #[cfg(test)]
    gate: Mutex<()>,
}
impl Indexer {
    fn start(live: Arc<AtomicU64>) -> Self {
        let shared = Arc::new(IndexerShared {
            live,
            ..Default::default()
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("pm-render-cache".into())
            .spawn(move || {
                loop {
                    let job = {
                        let mut slot = worker.slot.lock().unwrap();
                        loop {
                            if slot.1 {
                                return;
                            }
                            if let Some(job) = slot.0.take() {
                                break job;
                            }
                            slot = worker.wake.wait(slot).unwrap();
                        }
                    };
                    worker.run(job);
                }
            })
            .expect("render cache indexer thread");
        Self { shared }
    }
    fn submit(&self, job: Indexing) {
        // The job it replaces is dropped outside the lock.
        let replaced = self.shared.slot.lock().unwrap().0.replace(job);
        drop(replaced);
        self.shared.wake.notify_one();
    }
}
impl Drop for Indexer {
    fn drop(&mut self) {
        self.shared.slot.lock().unwrap().1 = true;
        self.shared.wake.notify_one();
    }
}
impl IndexerShared {
    fn run(&self, job: Indexing) {
        self.begun.fetch_add(1, Ordering::Relaxed);
        #[cfg(test)]
        drop(self.gate.lock().unwrap());
        let Indexing {
            inner,
            media_ids,
            content_named,
            generation,
            source,
            identity,
            cache,
            size,
            rate,
            count,
        } = job;
        // Anything since (another observation, storage, a deletion) wins.
        let stale = || self.live.load(Ordering::Acquire) != generation;
        if stale() {
            return;
        }
        // A font file rewritten under this process is read again first (a
        // change of the installed fonts like any other: the font generation
        // moves and the state is observed again).
        pm_text::system_fonts::refresh();
        let fonts = pm_text::system_fonts::Inventory::current();
        // Reads the media files it has not read at their versions yet:
        // given up between files once something newer is observed.
        let media = (&media_ids, content_named.as_deref());
        let Some(fingerprint) = fingerprint(&source, &identity, &fonts, media, &stale) else {
            return;
        };
        let source = Source {
            fonts: Some(Arc::new(fonts)),
            ..source
        };
        if stale() {
            return;
        }
        let inputs = crate::frame_inputs::frame_digests(
            &source.scene.doc,
            &identity.comp,
            rate,
            count,
            // Override code can read anything in the document.
            identity.overrides.is_some(),
        )
        .unwrap_or_default();
        if stale() {
            return;
        }
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
        let Some((present, kept)) = index_until(&cache, &keys, &stale) else {
            return;
        };
        let Some(inner) = inner.upgrade() else { return };
        let mut state = inner.state.lock().unwrap();
        // Only the observation it was taken for.
        if state.generation == generation {
            state.frames = Some(Frames {
                present,
                kept,
                ..template
            });
            bump(&mut state, &inner.published);
        }
    }
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
    (
        path.to_owned(),
        keyed(FileVersion::of(path), path, content_named),
    )
}

/// A file's version as a frame's key takes it ([`Stamp`]): the version
/// read; for a file directly in `content_named` (named by its bytes: its
/// name and length say it all, and the app touches it at every open) its
/// length alone; for a file that could not be looked at, the empty version.
pub(crate) fn keyed(
    read: Option<FileVersion>,
    path: &Path,
    content_named: Option<&Path>,
) -> FileVersion {
    let read = read.unwrap_or_default();
    if content_named.is_some() && path.parent() == content_named {
        FileVersion {
            len: read.len,
            ..Default::default()
        }
    } else {
        read
    }
}

/// Whether every file a frame read is one its key names (`expected`), so
/// that [`reads_are`] compared each of them with a version: a frame that
/// read a file the key does not name was drawn from something the key says
/// nothing about.
pub(crate) fn reads_listed(reads: &[(PathBuf, Option<FileVersion>)], expected: &[Stamp]) -> bool {
    reads
        .iter()
        .all(|(path, _)| expected.iter().any(|(named, _)| named == path))
}

/// Whether what a frame read is what its key says. `reads` are the file
/// versions its render looked at (`pm_render::media::take_reads`, every
/// band of it): each file among `expected` was the version named there,
/// every file kept one version through the frame, and each is still that
/// version now, after the read (a file that changes while it is read is
/// another version from then on: its status-change time cannot be put
/// back). `Err`: the first file that is not.
pub(crate) fn reads_are(
    reads: &[(PathBuf, Option<FileVersion>)],
    expected: &[Stamp],
    content_named: Option<&Path>,
) -> Result<(), PathBuf> {
    let expected: std::collections::HashMap<&Path, FileVersion> = expected
        .iter()
        .map(|(path, version)| (path.as_path(), *version))
        .collect();
    let mut read: std::collections::HashMap<&Path, FileVersion> = Default::default();
    for (path, version) in reads {
        let version = keyed(*version, path, content_named);
        if expected.get(path.as_path()).is_some_and(|e| *e != version)
            || *read.entry(path).or_insert(version) != version
        {
            return Err(path.clone());
        }
    }
    match read
        .iter()
        .find(|(path, version)| keyed(FileVersion::of(path), path, content_named) != **version)
    {
        Some((path, _)) => Err(path.to_path_buf()),
        None => Ok(()),
    }
}

pub(crate) fn stamps(
    media: &pm_export::render::MediaPaths,
    content_named: Option<&Path>,
) -> Vec<Stamp> {
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

/// What a media file's bytes are: the BLAKE3 of ALL of them (and the
/// length), read in pieces; `None` when it cannot be read, or when
/// `go_on` says to give up (asked between pieces). Equality of these is
/// what lets two file versions share frames; nothing short of every byte
/// does ([`CONTENT`]).
fn content_of(path: &Path, go_on: &dyn Fn() -> bool) -> Option<[u8; 32]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut hash = blake3::Hasher::new_derive_key(CONTENT);
    hash.update(&len.to_le_bytes());
    let mut buf = vec![0u8; 4 << 20];
    loop {
        if !go_on() {
            return None;
        }
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Some(*hash.finalize().as_bytes())
}
/// How a media file's bytes are digested ([`content_of`]): part of the
/// cache's schema, so that frames keyed under another way of telling files
/// apart (the 64 KiB-every-2 MiB samples before) are retired.
const CONTENT: &str = "powermove media content v2: blake3 of every byte";
/// Files up to this size are read by the indexer when it meets them
/// (every still, most clips: 512 MiB is a fraction of a second on an
/// internal disk); larger ones by the prover, in the background.
const READ_INLINE: u64 = 512 << 20;

/// The versions whose bytes are known, in the order they were first read.
#[derive(Default)]
struct Contents {
    /// The file they are kept in.
    record: Option<PathBuf>,
    rows: Vec<(Stamp, [u8; 32])>,
    by_stamp: HashMap<Stamp, [u8; 32]>,
    /// The first version read with each content: the one whose frames
    /// every version with those bytes shares.
    first: HashMap<[u8; 32], Stamp>,
}
impl Contents {
    fn add(&mut self, stamp: Stamp, content: [u8; 32]) {
        if self.by_stamp.insert(stamp.clone(), content).is_none() {
            self.first.entry(content).or_insert_with(|| stamp.clone());
            self.rows.push((stamp, content));
        }
    }
}

/// A large file whose bytes are being proved (or wait to be).
type Proof = (Stamp, Option<PathBuf>);

/// What each media file's bytes are ([`content_of`]), remembered by the
/// version of the file they were read at ([`Stamp`]), across launches (in
/// the folder of rendered frames).
///
/// A frame's key names each media file by a file VERSION: its own, or,
/// once every byte of it has been read, the first version met with the
/// same bytes. So a file that is another version with the same bytes (its
/// permissions or times touched, replaced by an identical copy, the same
/// bytes under another path: an embedded clip's cache object is named
/// again when its project is first saved in the current format) keeps its
/// rendered frames, and a file whose bytes changed anywhere never does,
/// whatever its length and modification time say. Equality is granted by
/// the whole file only, never by part of it. A file larger than
/// [`READ_INLINE`] is read by the prover thread instead of the indexer:
/// until it is, the file's own version names it (its frames are its own),
/// and when the proof finds a first version other than its own, the open
/// state is fingerprinted again ([`MediaIds::proved`]). A file with new
/// bytes is its own first version: its key does not change when the proof
/// comes in.
#[derive(Default)]
struct MediaIds {
    known: Mutex<Contents>,
    /// Files read, inline or proved (tests: a known version is not read
    /// again).
    read: AtomicU64,
    /// Files above this size are proved in the background (tests lower it;
    /// 0 = [`READ_INLINE`]).
    read_inline: AtomicU64,
    /// The prover's queue (started with its first large file), and what is
    /// queued or being read.
    prover: Mutex<Option<std::sync::mpsc::Sender<Proof>>>,
    proving: Mutex<std::collections::HashSet<Stamp>>,
    /// Told when a proof found a version's bytes under another version
    /// (the open state is fingerprinted again).
    proved: OnceLock<Box<dyn Fn() + Send + Sync>>,
}
impl MediaIds {
    /// Kept in the frames folder (earlier versions' files are removed: their
    /// sampled contents must never be taken as proved).
    const FILE: &'static str = ".pm-media-content-v2";
    const EARLIER: [&'static str; 1] = [".pm-media-content"];
    /// At most this many versions are remembered (a version is ~200 bytes);
    /// the earliest are forgotten first.
    const AT_MOST: usize = 20_000;

    /// Remembers in (and from) `folder` from now on.
    fn open(&self, folder: &Path) {
        for earlier in Self::EARLIER {
            let _ = std::fs::remove_file(folder.join(earlier));
        }
        let record = folder.join(Self::FILE);
        let found: Vec<(Stamp, String)> = std::fs::read(&record)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut known = self.known.lock().unwrap();
        known.record = Some(record);
        for (stamp, content) in found.into_iter().take(Self::AT_MOST) {
            if let Ok(content) = blake3::Hash::from_hex(&content) {
                known.add(stamp, *content.as_bytes());
            }
        }
    }

    /// The version that names the file `stamp` names in a frame's key: the
    /// first version known with its bytes, or its own while they are not
    /// known (being proved, changing under the look, unreadable).
    fn named(self: &Arc<Self>, stamp: &Stamp, content_named: Option<&Path>) -> Stamp {
        let first_of = |this: &Self| {
            let known = this.known.lock().unwrap();
            let content = known.by_stamp.get(stamp)?;
            known.first.get(content).cloned()
        };
        if let Some(first) = first_of(self) {
            return first;
        }
        let (path, version) = stamp;
        let now = || keyed(FileVersion::of(path), path, content_named);
        if now() != *version {
            return stamp.clone();
        }
        let inline = match self.read_inline.load(Ordering::Relaxed) {
            0 => READ_INLINE,
            n => n,
        };
        if version.len > inline {
            self.prove(stamp.clone(), content_named.map(Path::to_owned));
            return stamp.clone();
        }
        self.read.fetch_add(1, Ordering::Relaxed);
        // Read while it was that version throughout.
        match content_of(path, &|| true) {
            Some(content) if now() == *version => {
                self.learn(stamp.clone(), content);
                first_of(self).unwrap_or_else(|| stamp.clone())
            }
            _ => stamp.clone(),
        }
    }

    fn learn(&self, stamp: Stamp, content: [u8; 32]) {
        let mut known = self.known.lock().unwrap();
        if known.rows.len() >= Self::AT_MOST {
            let rows = std::mem::take(&mut known.rows);
            known.by_stamp.clear();
            known.first.clear();
            for (stamp, content) in rows.into_iter().skip(Self::AT_MOST / 2) {
                known.add(stamp, content);
            }
        }
        known.add(stamp, content);
    }

    /// Queues a large file's version to be read whole on the prover thread
    /// (utility class: it gives way to playback's reads), once.
    fn prove(self: &Arc<Self>, stamp: Stamp, content_named: Option<PathBuf>) {
        if !self.proving.lock().unwrap().insert(stamp.clone()) {
            return;
        }
        let mut prover = self.prover.lock().unwrap();
        let sender = prover.get_or_insert_with(|| {
            let (send, take) = std::sync::mpsc::channel::<Proof>();
            let ids = Arc::downgrade(self);
            std::thread::Builder::new()
                .name("pm-media-prover".into())
                .spawn(move || {
                    let _ = crate::qos::set_job_qos(crate::qos::JobQos::Utility);
                    // Ends when the ids are dropped (the sender with them).
                    while let Ok((stamp, content_named)) = take.recv() {
                        let (path, version) = &stamp;
                        let same = || {
                            ids.strong_count() > 0
                                && keyed(FileVersion::of(path), path, content_named.as_deref())
                                    == *version
                        };
                        let started = Instant::now();
                        let content = content_of(path, &same).filter(|_| same());
                        let Some(ids) = ids.upgrade() else { return };
                        ids.proving.lock().unwrap().remove(&stamp);
                        let Some(content) = content else { continue };
                        ids.read.fetch_add(1, Ordering::Relaxed);
                        ids.learn(stamp.clone(), content);
                        ids.save();
                        let first = ids.known.lock().unwrap().first.get(&content).cloned();
                        diagnostic(serde_json::json!({
                            "media_proved": path.to_string_lossy(),
                            "len": version.len,
                            "ms": started.elapsed().as_millis() as u64,
                            "shared": first.as_ref() != Some(&stamp),
                        }));
                        if first.as_ref() != Some(&stamp)
                            && let Some(proved) = ids.proved.get()
                        {
                            proved();
                        }
                    }
                })
                .expect("media prover thread");
            send
        });
        let _ = sender.send((stamp, content_named));
    }

    /// Whether the bytes of the file `stamp` names are known (tests).
    #[cfg(test)]
    fn knows(&self, stamp: &Stamp) -> bool {
        self.known.lock().unwrap().by_stamp.contains_key(stamp)
    }

    /// Writes what is known next to the rendered frames.
    fn save(&self) {
        let known = self.known.lock().unwrap();
        let Some(record) = &known.record else {
            return;
        };
        let rows: Vec<(&Stamp, String)> = known
            .rows
            .iter()
            .map(|(stamp, content)| {
                (
                    stamp,
                    blake3::Hash::from_bytes(*content).to_hex().to_string(),
                )
            })
            .collect();
        let partial = record.with_extension(format!("{}", std::process::id()));
        if let Ok(bytes) = serde_json::to_vec(&rows)
            && std::fs::write(&partial, bytes).is_ok()
        {
            let _ = std::fs::rename(&partial, record);
        }
    }
}

/// The fonts text can resolve to (`pm_text::system_fonts::Inventory`):
/// every font file CoreText has registered (installed for the system or
/// the user, activated from anywhere else, or registered by the process),
/// with its version. The list is current as soon as a
/// font is installed or removed; the font generation observes again when
/// it changes, and a fill draws with the very list its keys were taken
/// from (`Source::fonts`).
fn fonts_identity(fonts: &pm_text::system_fonts::Inventory, hash: &mut blake3::Hasher) {
    for (path, version) in &fonts.0 {
        hash.update(path.as_os_str().as_encoded_bytes());
        hash.update(&[0]);
        hash.update(&version.len.to_le_bytes());
        hash.update(&version.modified.to_le_bytes());
        hash.update(&version.device.to_le_bytes());
        hash.update(&version.inode.to_le_bytes());
        hash.update(&version.changed.0.to_le_bytes());
        hash.update(&version.changed.1.to_le_bytes());
    }
}

/// The version of the keys' layout: what a fingerprint is derived under.
const KEYS: &str = "powermove render cache v9";

/// How this build keys and stores its frames: the keys' layout, the stored
/// format and the renderer. A folder's entries are of one schema
/// (`RenderCache::retire_other_schemas`).
fn schema() -> String {
    format!("{KEYS}\n{CONTENT}\n{FORMAT}\n{}", renderer_identity())
}

/// A row in `$PM_ENGINE_TRACE.cache.jsonl` (diagnostics only).
fn diagnostic(row: serde_json::Value) {
    let Some(trace) = std::env::var_os("PM_ENGINE_TRACE") else {
        return;
    };
    let mut path = PathBuf::from(trace).into_os_string();
    path.push(".cache.jsonl");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{row}");
    }
}

/// The fingerprint of everything that draws the composition's frames
/// besides the document, whose parts each frame draws from are in its key
/// (`crate::frame_inputs`).
fn fingerprint(
    source: &Source,
    identity: &Identity,
    fonts: &pm_text::system_fonts::Inventory,
    (media_ids, content_named): (&Arc<MediaIds>, Option<&Path>),
    stale: &dyn Fn() -> bool,
) -> Option<[u8; 32]> {
    // Each media file as the version that names its bytes ([`MediaIds`]).
    let before = media_ids.read.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut media = Vec::with_capacity(source.scene.media.paths.len());
    for (asset, path) in &source.scene.media.paths {
        if stale() {
            return None;
        }
        let stamp = identity
            .media
            .iter()
            .find(|(named, _)| named == path)
            .cloned()
            .unwrap_or_else(|| (path.clone(), FileVersion::default()));
        let named = media_ids.named(&stamp, content_named);
        media.push((asset.as_str(), stamp, named));
    }
    let read_ms = started.elapsed().as_millis() as u64;
    if media_ids.read.load(Ordering::Relaxed) != before {
        media_ids.save();
    }
    let parts = fingerprint_parts(source, identity, fonts, &media);
    let mut hash = blake3::Hasher::new_derive_key(KEYS);
    for (_, part) in &parts {
        hash.update(part);
    }
    let fingerprint = *hash.finalize().as_bytes();
    // Which part of a state's identity changed (diagnostics only).
    if std::env::var_os("PM_ENGINE_TRACE").is_some() {
        let short = |digest: &[u8; 32]| -> String {
            digest[..6].iter().map(|b| format!("{b:02x}")).collect()
        };
        let mut row: serde_json::Map<String, serde_json::Value> = parts
            .iter()
            .map(|(name, part)| ((*name).to_owned(), serde_json::Value::String(short(part))))
            .collect();
        row.insert(
            "fingerprint".into(),
            serde_json::Value::String(short(&fingerprint)),
        );
        row.insert("revision".into(), identity.revision.into());
        // Each media file as the key took it: which of them moved between
        // two rows (its path, a field of its version, or its bytes).
        row.insert(
            "media_files".into(),
            media
                .iter()
                .map(|(asset, (path, version), named)| {
                    serde_json::json!([
                        asset,
                        path.to_string_lossy(),
                        version.len,
                        version.modified,
                        version.device,
                        version.inode,
                        version.changed.0,
                        version.changed.1,
                        // The version naming it, when it is another one.
                        (named.0 != *path || named.1 != *version)
                            .then(|| named.0.to_string_lossy()),
                    ])
                })
                .collect::<Vec<_>>()
                .into(),
        );
        row.insert(
            "media_read".into(),
            (media_ids.read.load(Ordering::Relaxed) - before).into(),
        );
        row.insert("media_read_ms".into(), read_ms.into());
        diagnostic(serde_json::Value::Object(row));
    }
    Some(fingerprint)
}

/// What a composition state's frames depend on beyond its document, each
/// part digested on its own.
fn fingerprint_parts(
    source: &Source,
    identity: &Identity,
    fonts: &pm_text::system_fonts::Inventory,
    media: &[(&str, Stamp, Stamp)],
) -> Vec<(&'static str, [u8; 32])> {
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
            }),
        ),
        (
            "definitions",
            part(&|hash| {
                hash.update(&serde_json::to_vec(&source.definitions).unwrap_or_default());
            }),
        ),
        // Each asset by the complete file version that names its bytes
        // ([`MediaIds::named`]): its own until all of its bytes are known,
        // then the first version met with the same bytes.
        (
            "media",
            part(&|hash| {
                for (asset, _, (path, version)) in media {
                    hash.update(asset.as_bytes());
                    hash.update(&[0]);
                    hash.update(path.as_os_str().as_encoded_bytes());
                    hash.update(&[0]);
                    hash.update(&version.len.to_le_bytes());
                    hash.update(&version.modified.to_le_bytes());
                    hash.update(&version.device.to_le_bytes());
                    hash.update(&version.inode.to_le_bytes());
                    hash.update(&version.changed.0.to_le_bytes());
                    hash.update(&version.changed.1.to_le_bytes());
                }
            }),
        ),
        ("fonts", part(&|hash| fonts_identity(fonts, hash))),
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
    /// Waits for `until` on the cache's own reports: it fails only when
    /// nothing was published for 30 s (a fill that stalled), however long a
    /// loaded machine takes to render.
    fn settle(disk: &DiskFrames, until: impl Fn(&RenderBars) -> bool) -> Arc<RenderBars> {
        let (mut last, mut heard) = (disk.bars(), Instant::now());
        loop {
            let bars = disk.bars();
            if until(&bars) {
                return bars;
            }
            if !Arc::ptr_eq(&bars, &last) {
                (last, heard) = (bars.clone(), Instant::now());
            }
            assert!(heard.elapsed() < Duration::from_secs(30), "{bars:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn ready(disk: &DiskFrames) {
        disk.test_ready();
    }
    /// Frame `n` of the fixture written as a fill child writes it.
    fn write_frame(disk: &DiskFrames, n: i64) -> Ticks {
        disk.test_put(n, 0.25)
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
    /// one's), and its bytes are not read again for it.
    #[test]
    fn a_touched_media_cache_object_keeps_its_frames() {
        let objects = tempfile::tempdir().unwrap();
        let object = objects.path().join("0a1b2c.mp4");
        std::fs::write(&object, b"media bytes").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_content_named(Some(objects.path().to_owned()));
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        let scene = media_scene(&object);
        let before = fingerprint_of(&disk, &scene);
        let read = disk.inner.media_ids.read.load(Ordering::Relaxed);
        // The next open: the recency mark moves, the bytes don't.
        std::fs::File::options()
            .append(true)
            .open(&object)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
            .unwrap();
        assert_eq!(fingerprint_of(&disk, &scene), before);
        assert_eq!(disk.inner.media_ids.read.load(Ordering::Relaxed), read);
    }

    /// A scene drawing from one media file.
    fn media_scene(file: &Path) -> Scene {
        Scene {
            media: Arc::new(pm_export::render::MediaPaths {
                paths: [("clip".to_owned(), file.to_owned())].into(),
                ..Default::default()
            }),
            ..scene()
        }
    }

    /// The fingerprint of `scene`'s frames with its media looked at now.
    fn fingerprint_of(disk: &DiskFrames, scene: &Scene) -> [u8; 32] {
        disk.inner.state.lock().unwrap().media_checked = None;
        disk.observe(scene, &[], None, None, None, 1, Ticks::ZERO);
        ready(disk);
        let state = disk.inner.state.lock().unwrap();
        state.frames.as_ref().unwrap().fingerprint.unwrap()
    }

    /// Rendered frames are of a media file's bytes, not of the file. A
    /// linked file whose metadata alone changed (its permissions, its
    /// modification time), one replaced by an identical copy, and the same
    /// bytes under another path (an embedded clip's cache object is named
    /// again when its project is first saved in the current format) keep
    /// the frames rendered from them: a person does not lose a rendered
    /// cache by saving, or to a backup tool. Other bytes of the same length
    /// with the modification time put back are another picture. A file
    /// read at a version is not read again, in this launch or the next.
    #[test]
    fn rendered_frames_follow_a_media_files_bytes_not_its_file() {
        use std::os::unix::fs::PermissionsExt;
        let media = tempfile::tempdir().unwrap();
        let still = media.path().join("still.bmp");
        grey_still(&still, 40);
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        let disk = DiskFrames::default();
        disk.set_storage(Some((frames.clone(), 1 << 30))).unwrap();
        let scene = media_scene(&still);
        let reads = |disk: &DiskFrames| disk.inner.media_ids.read.load(Ordering::Relaxed);
        let first = fingerprint_of(&disk, &scene);
        assert_eq!(reads(&disk), 1);
        assert_eq!(fingerprint_of(&disk, &scene), first);
        assert_eq!(reads(&disk), 1, "the same version is not read again");
        // The status-change time has the file system's resolution.
        let tick = || std::thread::sleep(Duration::from_millis(20));
        let version = || FileVersion::of(&still).unwrap();
        // Its permissions change and change back.
        let was = version();
        tick();
        std::fs::set_permissions(&still, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&still, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_ne!(version(), was, "another version of the file");
        assert_eq!(fingerprint_of(&disk, &scene), first, "permissions");
        // Its modification time moves.
        let was = version();
        std::fs::File::options()
            .append(true)
            .open(&still)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
            .unwrap();
        assert_ne!(version(), was);
        assert_eq!(fingerprint_of(&disk, &scene), first, "a touched file");
        // An identical copy takes its place (another inode).
        let was = version();
        let copy = media.path().join("copy.bmp");
        std::fs::copy(&still, &copy).unwrap();
        std::fs::rename(&copy, &still).unwrap();
        assert_ne!(version().inode, was.inode);
        assert_eq!(fingerprint_of(&disk, &scene), first, "an identical copy");
        // The same bytes under another path.
        let renamed = media.path().join("renamed.bmp");
        std::fs::copy(&still, &renamed).unwrap();
        assert_eq!(
            fingerprint_of(&disk, &media_scene(&renamed)),
            first,
            "the same bytes under another path"
        );
        // The next launch reads none of them again.
        let known = reads(&disk);
        assert_eq!(known, 5, "once per version met");
        let relaunched = DiskFrames::default();
        relaunched.set_storage(Some((frames, 1 << 30))).unwrap();
        assert_eq!(fingerprint_of(&relaunched, &scene), first);
        assert_eq!(reads(&relaunched), 0, "known from the launch before");
        // Other bytes of the same length, the modification time put back.
        let modified = std::fs::metadata(&still).unwrap().modified().unwrap();
        tick();
        grey_still(&still, 220);
        std::fs::File::options()
            .write(true)
            .open(&still)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_ne!(
            fingerprint_of(&relaunched, &scene),
            first,
            "other bytes of the same length"
        );
    }

    /// A large media file (a long clip) is the same picture as another file
    /// only once ALL of its bytes are proved equal: a change anywhere in
    /// it, the same length kept and the modification time put back, is
    /// another picture, however little of it changed. Until the proof is
    /// in (read off the indexer, it takes a while), the file's own complete
    /// version names it; then the same bytes under another path share the
    /// frames, in this launch (found again by themselves) and the next.
    #[test]
    fn a_large_media_file_shares_frames_only_once_all_its_bytes_are_proved_equal() {
        // Above the size whose bytes were once sampled (a 64 KiB piece
        // every 2 MiB); sparse, so it takes no disk.
        const LEN: u64 = 600 << 20;
        let media = tempfile::tempdir().unwrap();
        let clip = media.path().join("clip.mov");
        let file = std::fs::File::create(&clip).unwrap();
        file.set_len(LEN).unwrap();
        drop(file);
        let poke = |path: &Path, at: u64, byte: u8| {
            use std::os::unix::fs::FileExt;
            let modified = std::fs::metadata(path).unwrap().modified().unwrap();
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.write_all_at(&[byte; 16], at).unwrap();
            file.set_modified(modified).unwrap();
        };
        poke(&clip, 0, 1);
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        let disk = DiskFrames::default();
        disk.set_storage(Some((frames.clone(), 1 << 30))).unwrap();
        let scene = media_scene(&clip);
        let first = fingerprint_of(&disk, &scene);
        // The same frames while nothing changed.
        assert_eq!(fingerprint_of(&disk, &scene), first);
        // A pixel corrected in place, between the pieces once sampled.
        std::thread::sleep(Duration::from_millis(20));
        poke(&clip, (1 << 20) + 4096, 2);
        assert_eq!(std::fs::metadata(&clip).unwrap().len(), LEN);
        let corrected = fingerprint_of(&disk, &scene);
        assert_ne!(corrected, first, "a change between the sampled pieces");
        // Another file of the same length rendered over it.
        let rendered = media.path().join("rendered.mov");
        std::fs::copy(&clip, &rendered).unwrap();
        poke(&rendered, (5 << 20) + 9, 3);
        std::fs::rename(&rendered, &clip).unwrap();
        let replaced = fingerprint_of(&disk, &scene);
        assert_ne!(replaced, corrected, "a same-length file rendered over it");
        assert_ne!(replaced, first);
        // Once its bytes are proved, an identical copy under another path
        // is the same picture: its state is found again by itself.
        let copy = media.path().join("copy.mov");
        std::fs::copy(&clip, &copy).unwrap();
        let proved = |disk: &DiskFrames, path: &Path| {
            let stamp = stamp(path, None);
            let start = Instant::now();
            while !disk.inner.media_ids.knows(&stamp) {
                assert!(start.elapsed() < Duration::from_secs(60), "not proved");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        proved(&disk, &clip);
        let copied = media_scene(&copy);
        disk.inner.state.lock().unwrap().media_checked = None;
        disk.observe(&copied, &[], None, None, None, 1, Ticks::ZERO);
        let start = Instant::now();
        loop {
            ready(&disk);
            let now = disk
                .inner
                .state
                .lock()
                .unwrap()
                .frames
                .as_ref()
                .unwrap()
                .fingerprint;
            if now == Some(replaced) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "the copy never became the same picture"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        // The next launch knows both without reading either again.
        let relaunched = DiskFrames::default();
        relaunched.set_storage(Some((frames, 1 << 30))).unwrap();
        assert_eq!(fingerprint_of(&relaunched, &copied), replaced);
        assert_eq!(fingerprint_of(&relaunched, &scene), replaced);
        assert_eq!(relaunched.inner.media_ids.read.load(Ordering::Relaxed), 0);
        // And a copy changed between the pieces is not.
        poke(&copy, (7 << 20) + 77, 4);
        assert_ne!(fingerprint_of(&relaunched, &copied), replaced);
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

    /// A Render Selection is of one project's composition as the viewer
    /// drew it: what the cache observed for another project, another
    /// composition, an earlier revision or other effect code is not it, and
    /// a fill asked for one never starts from another's frames.
    #[test]
    fn an_observation_is_of_one_project_composition_revision_and_definitions() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        let scene = scene();
        let revision = scene.doc.revision();
        let observed = |doc_id, revision, comp: &str, definitions: &[DefinitionSnapshot]| {
            disk.observed(Observation {
                doc_id,
                revision,
                comp,
                definitions,
            })
        };
        assert!(!observed(1, revision, &scene.comp, &[]), "nothing yet");
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        assert!(observed(1, revision, &scene.comp, &[]));
        assert!(!observed(2, revision, &scene.comp, &[]), "another project");
        assert!(
            !observed(1, revision, "another", &[]),
            "another composition"
        );
        assert!(
            !observed(1, revision + 1, &scene.comp, &[]),
            "a later revision"
        );
        let definition = pm_render::all_builtin_definitions()
            .definitions
            .into_iter()
            .next()
            .unwrap();
        assert!(
            !observed(1, revision, &scene.comp, std::slice::from_ref(&definition)),
            "other definitions"
        );
        let nowhere = Path::new("/nonexistent/engine-job");
        let other = [definition];
        let fill = |doc, at, comp: &str, definitions: &[DefinitionSnapshot]| {
            let of = Observation {
                doc_id: doc,
                revision: at,
                comp,
                definitions,
            };
            disk.start_fill_of(of, 1., 1.2, nowhere, Default::default(), |_| {})
        };
        let changed = Err("The composition changed before rendering started.".to_owned());
        assert_eq!(fill(2, revision, &scene.comp, &[]), changed);
        assert_eq!(fill(1, revision, "another", &[]), changed);
        // The observation it has is of an earlier revision or other effect
        // code than the one asked for: its frames are not rendered instead.
        assert_eq!(fill(1, revision + 1, &scene.comp, &[]), changed);
        assert_eq!(fill(1, revision, &scene.comp, &other), changed);
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
        let (_dir, disk) = filled(scene);
        read_eventually(&disk, scene, second()).expect("rendered frame")
    }
    /// Frame 30's time (1 s).
    fn second() -> Ticks {
        Ticks::new(pm_time::Rate::FPS_30.frame_start(30).unwrap()).unwrap()
    }
    /// A cache with `scene`'s frame 30 rendered by a fill child.
    fn filled(scene: &Scene) -> (tempfile::TempDir, DiskFrames) {
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
        (dir, disk)
    }
    /// What a paused viewer (not a capture) settles on for `scene` at 1 s
    /// on a `width` × `height` drawable: whether it was the frame on disk,
    /// and the presented picture.
    fn paused_picture(
        disk: Option<&DiskFrames>,
        scene: &Scene,
        (width, height): (u32, u32),
    ) -> (bool, Vec<[f64; 3]>) {
        use crate::render::{Engine, FrameSpec};
        let mut engine = Engine::new(pm_gpu::Gpu::new("paused-picture").unwrap()).unwrap();
        engine.disk = disk.cloned();
        let output = crate::Output {
            width,
            height,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        // The settled paused picture: the refinement at the Full scale.
        let spec = FrameSpec {
            exact: false,
            scale: FrameSpec::paused_full_scale(scene, &output),
            ..FrameSpec::exact(second(), output)
        };
        let start = Instant::now();
        loop {
            let trace = Arc::new(Mutex::new(crate::trace::Trace::default()));
            let sub = engine
                .frame_settled(scene, spec, None, &Default::default(), true, trace.clone())
                .unwrap()
                .unwrap();
            sub.commit();
            sub.complete().unwrap();
            let from_disk = trace.lock().unwrap().disk == 2;
            // A frame on disk is shown once its read lands.
            if !sub.stand_ins && (from_disk || disk.is_none()) {
                return (from_disk, sub.pixels());
            }
            assert!(start.elapsed() < Duration::from_secs(20), "never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// The largest difference between two pictures' samples.
    fn apart(a: &[[f64; 3]], b: &[[f64; 3]]) -> f64 {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b)
            .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs()))
            .fold(0., f64::max)
    }

    /// Motion blur is the same in a paused picture drawn and one read from
    /// disk, whatever the Export sheet's default says: a thin line crossing
    /// the frame in a fifth of a second, its layer's blur on, with the
    /// project's export default on and off, at the composition's size and
    /// reduced. (Before: the drawn picture had 12 samples, the rendered one
    /// 16, or none at all with the export default off.)
    #[test]
    fn motion_blur_is_the_same_drawn_and_rendered() {
        for export_default in [true, false] {
            let mut project: serde_json::Value =
                serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
            let layers = project["layers"].as_array_mut().unwrap();
            let line = layers.iter_mut().find(|l| l["id"] == "L007").unwrap();
            line["mblur"] = true.into();
            let key = |t: f64, v: f64, id: &str| {
                serde_json::json!({"t": t, "v": v, "i": id,
                    "inInterp": "linear", "outInterp": "linear",
                    "inEase": {"speed": 0, "influence": 33.333333333333336},
                    "outEase": {"speed": 0, "influence": 33.333333333333336},
                    "autoBezier": false, "continuous": false})
            };
            line["p"]["position.y"]["kf"] =
                serde_json::json!([key(0.9, 20., "kfast1"), key(1.1, 340., "kfast2")]);
            project["exportDefaults"] = serde_json::json!({ "mblur": export_default });
            let scene = scene_from(&project.to_string());
            let (_dir, disk) = filled(&scene);
            for drawable in [(640, 360), (320, 180)] {
                let (_, drawn) = paused_picture(None, &scene, drawable);
                let (from_disk, rendered) = paused_picture(Some(&disk), &scene, drawable);
                assert!(from_disk, "{drawable:?}: the frame on disk is shown");
                assert_eq!(
                    apart(&rendered, &drawn),
                    0.,
                    "export default {export_default}, {drawable:?}: the blur differs"
                );
            }
            // The blur is there: the same frame with the layer's switch off
            // is another picture.
            let line = project["layers"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|l| l["id"] == "L007")
                .unwrap();
            line["mblur"] = false.into();
            let sharp = scene_from(&project.to_string());
            let (_, blurred) = paused_picture(None, &scene, (640, 360));
            let (_, unblurred) = paused_picture(None, &sharp, (640, 360));
            assert!(apart(&blurred, &unblurred) > 0.05);
        }
    }

    /// A paused picture is the same whether its frame is rendered on disk
    /// or drawn: the composition's frame reduced to the drawable through
    /// one footprint filter, at a quarter of its size and at a third (not a
    /// whole reduction). Thin strokes and a 1-px grid would alias through a
    /// single bilinear tap; motion blur is off.
    #[test]
    fn a_rendered_paused_frame_is_presented_as_a_drawn_one() {
        let scene = scene();
        let (_dir, disk) = filled(&scene);
        for drawable in [(160, 90), (213, 120)] {
            let (from_disk, drawn) = paused_picture(None, &scene, drawable);
            assert!(!from_disk);
            let (from_disk, rendered) = paused_picture(Some(&disk), &scene, drawable);
            assert!(from_disk, "{drawable:?}: the frame on disk is shown");
            assert_eq!(
                apart(&rendered, &drawn),
                0.,
                "{drawable:?}: the rendered frame is presented differently from the drawn one"
            );
        }
    }

    /// What a paused viewer settles on at 1 s as its drawable takes each of
    /// `drawables` in turn (one engine: a window resized), drawn as the
    /// worker draws it: the picture at the drawable's size at once, then
    /// its refinement at the composition's size in bands. With `disk`, the
    /// frame on disk is shown instead; whether it was comes with each
    /// picture.
    fn settled_pictures(
        disk: Option<&DiskFrames>,
        scene: &Scene,
        definitions: &[DefinitionSnapshot],
        drawables: &[(u32, u32)],
    ) -> Vec<(bool, Vec<[f64; 3]>)> {
        use crate::render::{Engine, FrameSpec};
        let mut engine = Engine::new(pm_gpu::Gpu::new("settled-pictures").unwrap()).unwrap();
        engine.set_resources(Arc::new(crate::resources::ResourceSnapshot {
            generation: 1,
            definitions: definitions.to_vec(),
            ..Default::default()
        }));
        engine.disk = disk.cloned();
        let shown = |engine: &mut Engine, spec: FrameSpec| {
            let start = Instant::now();
            loop {
                let trace = Arc::new(Mutex::new(crate::trace::Trace::default()));
                let sub = engine
                    .frame(scene, spec, None, &Default::default(), true, trace.clone())
                    .unwrap()
                    .unwrap();
                sub.commit();
                sub.complete().unwrap();
                assert!(sub.failures.is_empty(), "{:?}", sub.failures);
                let from_disk = trace.lock().unwrap().disk == 2;
                // A frame on disk is shown once its read lands.
                if !sub.stand_ins && (from_disk || disk.is_none()) {
                    return (from_disk, sub.pixels());
                }
                assert!(start.elapsed() < Duration::from_secs(20), "never settled");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        drawables
            .iter()
            .map(|&(width, height)| {
                let output = crate::Output {
                    width,
                    height,
                    display: 0,
                    headroom: 1.,
                    crop: None,
                };
                let fast = FrameSpec {
                    exact: false,
                    ..FrameSpec::exact(second(), output)
                };
                // As the worker draws a paused frame: its preview at once
                // (or the frame on disk), then its settled picture.
                let first = shown(&mut engine, fast);
                if first.0 || !fast.settles(scene) {
                    return first;
                }
                let settled = FrameSpec {
                    scale: FrameSpec::paused_full_scale(scene, &output),
                    settled: true,
                    ..fast
                };
                assert!(engine.settle_now(scene, settled, || false).unwrap());
                shown(&mut engine, settled)
            })
            .collect()
    }

    /// The fixture with `layer` on top, its still linked to `path`.
    fn scene_with(layer: serde_json::Value, still: Option<&Path>) -> Scene {
        let mut project: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
        project["layers"].as_array_mut().unwrap().insert(0, layer);
        if still.is_some() {
            project["assets"]["pic"] = serde_json::json!(
                {"id":"pic","type":"image","name":"Texture","w":2048,"h":2048,"dur":0});
        }
        let mut scene = scene_from(&project.to_string());
        if let Some(still) = still {
            scene.media = Arc::new(pm_export::render::MediaPaths {
                paths: [("pic".to_owned(), still.to_owned())].into(),
                ..Default::default()
            });
        }
        scene
    }

    /// A flat grey still (a BMP: one length whatever the grey).
    fn grey_still(path: &Path, level: u8) {
        let (w, h) = (64u32, 64u32);
        let size = 54 + w * 3 * h;
        let mut b = Vec::with_capacity(size as usize);
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(w as i32).to_le_bytes());
        b.extend_from_slice(&(h as i32).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        b.resize(size as usize, level);
        std::fs::write(path, b).unwrap();
    }
    /// The fixture under one linked still that covers the frame.
    fn covered_by(still: &Path) -> Scene {
        let ch = |v: f64| serde_json::json!({"v": v, "kf": [], "expr": null});
        scene_with(
            serde_json::json!({
                "id":"still","type":"image","name":"Still","from":0,"dur":3,"on":true,
                "solo":false,"mblur":false,"fx":[],"masks":[],
                "p":{"position.x":ch(0.),"position.y":ch(0.),"anchor.x":ch(0.),"anchor.y":ch(0.),
                     "scale.x":ch(1000.),"scale.y":ch(1000.),"rotation":ch(0.),"opacity":ch(100.)},
                "d":{"asset":"pic","fit":"stretch","w":64,"h":64}
            }),
            Some(still),
        )
    }
    /// The keys of the open state's frames in `[from, to)` seconds, and the
    /// store they are kept in.
    fn keys_of(disk: &DiskFrames, from: f64, to: f64) -> (Arc<RenderCache>, Vec<RenderKey>) {
        let state = disk.inner.state.lock().unwrap();
        let frames = state.frames.as_ref().unwrap();
        (
            state.cache.clone().unwrap(),
            frame_range(frames, from, to)
                .filter_map(|n| frames.key(n))
                .collect(),
        )
    }
    fn stored(cache: &RenderCache, keys: &[RenderKey]) -> usize {
        keys.iter()
            .filter(|key| cache.presence(key).unwrap().is_some())
            .count()
    }

    /// A linked still saved again by another app after a fill's keys were
    /// taken and before the fill reads it: the fill stores nothing. (Its
    /// frames would be the new file's pixels under the keys of the file as
    /// it was, found again whenever that file comes back.)
    #[test]
    fn a_fill_stores_nothing_drawn_from_a_file_that_changed() {
        let dir = tempfile::tempdir().unwrap();
        let still = dir.path().join("still.bmp");
        grey_still(&still, 40);
        let scene = covered_by(&still);
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        disk.observe(&scene, &[], None, None, None, 1, Ticks::ZERO);
        ready(&disk);
        let (cache, keys) = keys_of(&disk, 1., 1.1);
        assert!(!keys.is_empty());
        grey_still(&still, 220);
        disk.start_fill(
            1.,
            1.1,
            &job_binary(),
            crate::jobs::Activity::default(),
            |_| {},
        )
        .unwrap();
        settle(&disk, |b| !b.running);
        assert_eq!(
            stored(&cache, &keys),
            0,
            "frames drawn from the changed file are stored under the keys of the file as it was"
        );
    }

    /// A frame a fill stored for a state the cache has moved on from (its
    /// report is not taken) stays on disk only when where it came from is
    /// established. After an edit it stays (the frame is the old state's
    /// picture, found again on undo) when the child proved that the media
    /// it read are the ones its key names; a frame that read a file its
    /// key does not name is removed, and so is every frame when the
    /// override wrappers answering are others (the child cannot hold them).
    #[test]
    fn a_frame_whose_report_is_not_taken_stays_only_with_its_provenance() {
        let ch = |v: f64| serde_json::json!({"v": v, "kf": [], "expr": null});
        let dir = tempfile::tempdir().unwrap();
        let still = dir.path().join("still.bmp");
        grey_still(&still, 40);
        // The still at `x`: another `x` is the same picture after an edit.
        let picture = |x: f64| {
            scene_with(
                serde_json::json!({
                    "id":"still","type":"image","name":"Still","from":0,"dur":3,"on":true,
                    "solo":false,"mblur":false,"fx":[],"masks":[],
                    "p":{"position.x":ch(x),"position.y":ch(30.),"anchor.x":ch(0.),"anchor.y":ch(0.),
                         "scale.x":ch(100.),"scale.y":ch(100.),"rotation":ch(0.),"opacity":ch(100.)},
                    "d":{"asset":"pic","fit":"stretch","w":64,"h":64}
                }),
                Some(&still),
            )
        };
        // The state the cache moves on to while the child waits, whether
        // the fill names its media to the child, and how many of the
        // frames stored for the state before stay.
        for (moved_on, overrides, named, stay) in [
            (picture(180.), None, true, true),
            (picture(180.), None, false, false),
            (picture(170.), Some(7), true, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let disk = DiskFrames::default();
            disk.inner.unnamed_media.store(!named, Ordering::Release);
            disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
                .unwrap();
            disk.observe(&picture(170.), &[], None, None, None, 1, Ticks::ZERO);
            ready(&disk);
            let (cache, keys) = keys_of(&disk, 1., 1.1);
            // The viewer is busy: the child waits at its first submission.
            let activity = crate::jobs::Activity::default();
            activity.set(true);
            disk.start_fill(1., 1.1, &job_binary(), activity.clone(), |_| {})
                .unwrap();
            assert!(disk.observe(&moved_on, &[], overrides, None, None, 1, Ticks::ZERO));
            ready(&disk);
            assert_ne!(keys_of(&disk, 1., 1.1).1, keys, "the state moved on");
            activity.set(false);
            settle(&disk, |b| !b.running);
            assert_eq!(
                stored(&cache, &keys),
                if stay { keys.len() } else { 0 },
                "after {}",
                match (overrides, named) {
                    (Some(_), _) => "other override wrappers",
                    (None, true) => "an edit, its media proved",
                    (None, false) => "an edit, a media file read that its keys do not name",
                }
            );
        }
    }

    /// A same-length rewrite of a media file with its modification time put
    /// back (its inode unchanged) is another media identity: the frames
    /// kept in memory and on disk from it are not the file's any more.
    #[test]
    fn a_same_length_rewrite_with_its_time_put_back_is_another_media_identity() {
        let dir = tempfile::tempdir().unwrap();
        let still = dir.path().join("still.bmp");
        grey_still(&still, 40);
        let media = pm_export::render::MediaPaths {
            paths: [("pic".to_owned(), still.clone())].into(),
            ..Default::default()
        };
        let before = stamps(&media, None);
        let modified = std::fs::metadata(&still).unwrap().modified().unwrap();
        // The status-change time has the file system's resolution.
        std::thread::sleep(Duration::from_millis(20));
        grey_still(&still, 220);
        std::fs::File::options()
            .write(true)
            .open(&still)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_eq!(
            std::fs::metadata(&still).unwrap().modified().unwrap(),
            modified
        );
        assert_ne!(stamps(&media, None), before);
    }

    /// Between two looks at the media (two seconds apart; the test holds
    /// the first look open), a frame not drawn before reads a file another
    /// app has just saved again: it is shown, and not kept as a frame of
    /// the file as it was. When that file comes back (same inode, length
    /// and modification time), the frame is drawn from it again.
    #[test]
    fn a_frame_drawn_from_a_changed_file_is_not_kept_as_the_file_it_was() {
        use crate::render::{Engine, FrameSpec};
        let dir = tempfile::tempdir().unwrap();
        let still = dir.path().join("still.bmp");
        grey_still(&still, 40);
        let modified = std::fs::metadata(&still).unwrap().modified().unwrap();
        let scene = covered_by(&still);
        let mut engine = Engine::new(pm_gpu::Gpu::new("live-read-binding").unwrap()).unwrap();
        engine.media_look = Duration::from_secs(3600);
        let spec = |seconds: f64| FrameSpec {
            exact: false,
            ..FrameSpec::exact(
                Ticks::new((seconds * TICKS_PER_SECOND as f64) as i64).unwrap(),
                crate::Output {
                    width: 640,
                    height: 360,
                    display: 0,
                    headroom: 1.,
                    crop: None,
                },
            )
        };
        // The picture once the still has landed (a preview's still is
        // decoded beside the frame, which stands in for it meanwhile).
        let mut shown = |spec: FrameSpec| {
            let start = Instant::now();
            loop {
                let trace = Arc::new(Mutex::new(crate::trace::Trace::default()));
                let sub = engine
                    .frame(&scene, spec, None, &Default::default(), true, trace)
                    .unwrap()
                    .unwrap();
                sub.commit();
                sub.complete().unwrap();
                if !sub.stand_ins {
                    return sub.pixels();
                }
                assert!(
                    start.elapsed() < Duration::from_secs(60),
                    "the still never landed"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let dark = shown(spec(1.));
        grey_still(&still, 220);
        let bright = shown(spec(2.));
        assert!(apart(&dark, &bright) > 0.1, "the new file is drawn");
        grey_still(&still, 40);
        std::fs::File::options()
            .write(true)
            .open(&still)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let again = shown(spec(2.));
        // The same frame from an engine that never saw the file change.
        let mut fresh = Engine::new(pm_gpu::Gpu::new("live-read-fresh").unwrap()).unwrap();
        let mut expected = None;
        let start = Instant::now();
        while expected.is_none() {
            let sub = fresh
                .frame(
                    &scene,
                    spec(2.),
                    None,
                    &Default::default(),
                    true,
                    Arc::default(),
                )
                .unwrap()
                .unwrap();
            sub.commit();
            sub.complete().unwrap();
            expected = (!sub.stand_ins).then(|| sub.pixels());
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "the still never landed"
            );
        }
        assert_eq!(
            apart(&again, &expected.unwrap()),
            0.,
            "a frame drawn from the changed file was kept as a frame of the file as it was"
        );
    }

    /// A paused picture drawn and the same frame read from disk, at a
    /// drawable of the composition's size, at smaller ones, and again at
    /// the first after the window was resized: one picture.
    fn assert_drawn_is_rendered(scene: &Scene, definitions: &[DefinitionSnapshot]) {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        disk.observe(scene, definitions, None, None, None, 1, Ticks::ZERO);
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
        let drawables = [(640, 360), (320, 180), (213, 120), (640, 360)];
        let drawn = settled_pictures(None, scene, definitions, &drawables);
        let rendered = settled_pictures(Some(&disk), scene, definitions, &drawables);
        for ((drawable, drawn), rendered) in drawables.iter().zip(&drawn).zip(&rendered) {
            assert!(rendered.0, "{drawable:?}: the frame on disk is shown");
            let differing = drawn
                .1
                .iter()
                .zip(&rendered.1)
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                (differing, apart(&rendered.1, &drawn.1)),
                (0, 0.),
                "{drawable:?}: the paused picture changes when its frame is rendered"
            );
        }
    }

    /// A square still, every pixel its own grey: texture at the finest
    /// scale.
    fn noise_still(path: &Path, side: u32) {
        let (w, h) = (side, side);
        let mut bmp = Vec::with_capacity((54 + w * h * 3) as usize);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54 + w * h * 3).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(w as i32).to_le_bytes());
        bmp.extend_from_slice(&(h as i32).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        let mut seed = 0x2545_F491u32;
        for _ in 0..w * h {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            bmp.extend_from_slice(&[(seed >> 24) as u8; 3]);
        }
        std::fs::write(path, bmp).unwrap();
    }

    /// A settled paused frame reads a still whole, here fitted small:
    /// drawn through the still's own mip chain. Once the frame is drawn the
    /// engine holds none of the original: not the still, and not the views
    /// of its mips kept for its draws (they kept the whole texture
    /// allocated, and uncounted, until they aged out).
    #[test]
    fn a_settled_frame_keeps_none_of_its_original_stills() {
        use crate::render::{Engine, FrameSpec};
        let dir = tempfile::tempdir().unwrap();
        let still = dir.path().join("texture.bmp");
        noise_still(&still, 1024);
        let ch = |v: f64| serde_json::json!({"v": v, "kf": [], "expr": null});
        let scene = scene_with(
            serde_json::json!({
                "id":"still","type":"image","name":"Texture","from":0,"dur":3,"on":true,
                "solo":false,"mblur":false,"fx":[],"masks":[],
                "p":{"position.x":ch(170.),"position.y":ch(30.),"anchor.x":ch(0.),"anchor.y":ch(0.),
                     "scale.x":ch(30.),"scale.y":ch(30.),"rotation":ch(0.),"opacity":ch(100.)},
                "d":{"asset":"pic","fit":"stretch","w":1024,"h":1024}
            }),
            Some(&still),
        );
        let mut engine = Engine::new(pm_gpu::Gpu::new("settled-stills").unwrap()).unwrap();
        let settled = FrameSpec {
            exact: false,
            settled: true,
            ..FrameSpec::exact(
                second(),
                crate::Output {
                    width: 640,
                    height: 360,
                    display: 0,
                    headroom: 1.,
                    crop: None,
                },
            )
        };
        assert!(settled.final_picture(&scene));
        let sub = engine
            .frame_settled(
                &scene,
                settled,
                None,
                &Default::default(),
                true,
                Arc::default(),
            )
            .unwrap()
            .unwrap();
        sub.commit();
        sub.complete().unwrap();
        assert!(sub.failures.is_empty(), "{:?}", sub.failures);
        let usage = engine.memory_usage();
        // The least the original takes as a texture, without its mips.
        let original = 1024 * 1024 * 4;
        assert!(
            usage.image_bytes + usage.pyramid_bytes < original,
            "the original still is held after its frame: {} bytes of stills, {} of pyramids",
            usage.image_bytes,
            usage.pyramid_bytes
        );
    }

    /// A large detailed still fitted small is the same in a paused picture
    /// drawn and one read from disk: a settled paused frame is drawn from
    /// the original, as an export and a rendered frame are. (Before: the
    /// viewer drew it from a thumbnail at the size it is shown, and its
    /// fine texture changed when the render bar turned green.)
    #[test]
    fn a_detailed_still_fitted_small_is_the_same_drawn_and_rendered() {
        let dir = tempfile::tempdir().unwrap();
        let still = dir.path().join("texture.bmp");
        noise_still(&still, 2048);
        let ch = |v: f64| serde_json::json!({"v": v, "kf": [], "expr": null});
        let scene = scene_with(
            serde_json::json!({
                "id":"still","type":"image","name":"Texture","from":0,"dur":3,"on":true,
                "solo":false,"mblur":false,"fx":[],"masks":[],
                "p":{"position.x":ch(170.),"position.y":ch(30.),"anchor.x":ch(0.),"anchor.y":ch(0.),
                     "scale.x":ch(14.7),"scale.y":ch(14.7),"rotation":ch(0.),"opacity":ch(100.)},
                "d":{"asset":"pic","fit":"stretch","w":2048,"h":2048}
            }),
            Some(&still),
        );
        assert_drawn_is_rendered(&scene, &[]);
    }

    /// An extension's glow in the shape of a multi-pass bloom (a
    /// half-resolution tile packed into its target, blurred and upsampled;
    /// it declares a spread of 10 px and reaches 40) is
    /// the same in a paused picture drawn and one read from disk: a settled
    /// paused frame draws its effects over the whole frame, as an export
    /// and a rendered frame do. (Before: its refinement drew the effect
    /// band by band within the declared spread.)
    #[test]
    fn a_reduced_pass_effect_is_the_same_drawn_and_rendered() {
        const HALO: &str = "#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform sampler2D u_orig;
uniform vec2 u_texel;
uniform int u_pass;
uniform float u_radius;
out vec4 o;
void main() {
  vec2 halfTexel = 0.5 * u_texel;
  bool tile = v_st.x < 0.5 && v_st.y < 0.5;
  vec2 logicalUV = v_st / 0.5;
  if (u_pass == 0) {
    vec4 sum = vec4(0.0);
    if (tile) {
      for (int dy = 0; dy < 2; dy++) {
        for (int dx = 0; dx < 2; dx++) {
          sum += texture(u_orig, logicalUV + (vec2(float(dx), float(dy)) - 0.5) * u_texel) * 0.25;
        }
      }
    }
    o = sum;
  } else if (u_pass < 3) {
    vec4 sum = vec4(0.0);
    float weights = 0.0;
    if (tile) {
      vec2 axis = u_pass == 1 ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
      for (int i = -16; i <= 16; i++) {
        float t = float(i) / 16.0;
        float w = exp(-4.5 * t * t);
        vec2 uv = logicalUV + axis * (u_radius * t) * u_texel;
        if (all(greaterThanEqual(uv, vec2(0.0))) && all(lessThanEqual(uv, vec2(1.0)))) {
          sum += texture(u_tex, clamp(uv * 0.5, halfTexel, vec2(0.5) - halfTexel)) * w;
        }
        weights += w;
      }
    }
    o = sum / max(weights, 0.00001);
  } else {
    vec4 bloom = texture(u_tex, clamp(v_st * 0.5, halfTexel, vec2(0.5) - halfTexel));
    o = texture(u_orig, v_st) + bloom * 2.0;
  }
}
";
        let halo: DefinitionSnapshot = serde_json::from_value(serde_json::json!({
            "id": "acme.halo", "kind": "effect", "version": 0, "origin": "local", "renderer": null,
            "glsl": { "source": HALO, "bodyLineOffset": 0,
                      "bodyUtf16Len": HALO.encode_utf16().count(), "bodyUtf8Len": HALO.len() },
            "params": [
                { "type": "number", "key": "radius", "default": 40, "min": 0, "max": 500, "step": null },
                { "type": "number", "key": "reach", "default": 40, "min": 0, "max": 500, "step": null }
            ],
            "flags": { "passes": 4, "keepOrig": true, "backdrop": false, "rawShader": true,
                       "viewportSafe": true, "viewportPadding": ["reach"], "clamp": false }
        }))
        .unwrap();
        let ch = |v: f64| serde_json::json!({"v": v, "kf": [], "expr": null});
        let scene = scene_with(
            serde_json::json!({
                "id":"lit","type":"shape","name":"Lit","from":0,"dur":3,"on":true,
                "solo":false,"mblur":false,"masks":[],
                "fx":[{"id":"e1","type":"acme.halo","on":true,
                       "p":{"radius":ch(40.),"reach":ch(10.)}}],
                "p":{"position.x":ch(320.),"position.y":ch(125.),"anchor.x":ch(0.),"anchor.y":ch(0.),
                     "scale.x":ch(100.),"scale.y":ch(100.),"rotation":ch(0.),"opacity":ch(100.)},
                "d":{"shape":"rect","w":180,"h":60,"radius":0,"color":"#FFE9A0"}
            }),
            None,
        );
        assert_drawn_is_rendered(&scene, &[halo]);
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
                media: Vec::new(),
                content_named: None,
            }
        };
        let source = Source {
            scene: scene.clone(),
            definitions: vec![],
            doc_id: 1,
            fonts: None,
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

    /// A rendered frame is missed when a layer its expression samples
    /// changes, though that layer is drawn seconds away and named only
    /// inside the expression; and when a layer added before it changes the
    /// index its expression reads.
    #[test]
    fn an_edit_an_expression_reads_misses_the_rendered_frame() {
        for (source, edit) in [
            // L001 (0-1 s) samples L00d (2-3 s), whose value changes.
            (
                "sample('L00d', 'opacity', 0)",
                (|p| p["layers"][0]["p"]["opacity"]["v"] = 10.into()) as fn(&mut serde_json::Value),
            ),
            // L001 reads its index; a layer drawn at 2-3 s is added before it.
            ("idx * 10", |p| {
                let mut added = p["layers"][0].clone();
                added["id"] = "Ladded".into();
                p["layers"].as_array_mut().unwrap().insert(0, added);
            }),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let disk = DiskFrames::default();
            disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
                .unwrap();
            let reading = |p: &mut serde_json::Value| {
                let layers = p["layers"].as_array_mut().unwrap();
                let l001 = layers.iter_mut().find(|l| l["id"] == "L001").unwrap();
                l001["p"]["opacity"]["expr"] = source.into();
            };
            let scene = sequenced(reading);
            disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
            ready(&disk);
            // Frame 10 (0.33 s) draws L001 alone.
            let early = write_frame(&disk, 10);
            assert!(read_eventually(&disk, &scene, early).is_some());
            let mut edited = sequenced(|p| {
                reading(p);
                edit(p);
            });
            edited.content = 2;
            assert!(disk.observe(&edited, &[], None, None, None, 1, Ticks::new(0).unwrap()));
            ready(&disk);
            assert!(
                read_eventually(&disk, &edited, early).is_none(),
                "{source}: the frame rendered before the edit is shown after it"
            );
            assert!(present(&disk).is_empty(), "{source}");
        }
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

    /// A hundred edits while the worker is busy (a slow volume being
    /// looked through) cost one job in flight and one waiting: each newer
    /// observation replaces the one waiting and frees its document
    /// snapshot, the job in flight gives up at its next stage, and only the
    /// latest observation is fingerprinted and lands.
    #[test]
    fn observations_coalesce_into_one_worker() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let indexer = disk.inner.indexer.shared.clone();
        // The worker stays at the start of whatever job it takes.
        let gate = indexer.gate.lock().unwrap();
        let mut snapshots = Vec::new();
        let mut latest = None;
        for revision in 1..=100 {
            let mut scene = scene();
            scene.content = revision;
            snapshots.push(Arc::downgrade(&scene.doc));
            assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap()));
            latest = Some(scene);
        }
        let latest = latest.unwrap();
        // The job in flight and the latest one: nothing in between is kept.
        let alive = snapshots.iter().filter(|d| d.upgrade().is_some()).count();
        assert!(alive <= 2, "{alive} document snapshots are held");
        assert!(indexer.begun.load(Ordering::Relaxed) <= 1);
        drop(gate);
        ready(&disk);
        {
            let state = disk.inner.state.lock().unwrap();
            let frames = state.frames.as_ref().unwrap();
            assert!(frames.identity.scene_matches(&latest), "the latest landed");
        }
        assert!(
            indexer.begun.load(Ordering::Relaxed) <= 2,
            "the job in flight, then the latest: nothing in between ran"
        );
    }

    /// Reading rendered frames ahead is bounded in bytes, not in frames:
    /// what the reader holds (read frames, and for each read asked for or in
    /// flight its decoded frame and coded bytes) stays within its budget
    /// whatever the frame's size. A frame as large as the budget (an 8K
    /// frame is 265 MB of the 256 MiB) plays one frame at a time; under
    /// Critical memory pressure only the frame the playhead is on is read,
    /// what was read ahead goes and queued reads are cancelled.
    #[test]
    fn read_ahead_is_bounded_in_bytes() {
        let size = |w, h| Size::new(w, h);
        let reader = Reader::start();
        assert_eq!(reader.fit(size(1920, 1080)).min(AHEAD), AHEAD);
        assert_eq!(reader.fit(size(3840, 2160)), 4);
        assert_eq!(reader.fit(size(7680, 4320)), 1);

        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = scene();
        assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap()));
        ready(&disk);
        let times: Vec<Ticks> = (10..22).map(|n| write_frame(&disk, n)).collect();
        let frame = frame_bytes(size(640, 360));
        let held = || disk.ready_bytes();
        let peak = || disk.inner.reader.ready.lock().unwrap().peak;
        let frames = || disk.inner.reader.ready.lock().unwrap().frames.len();
        let play = |range: std::ops::Range<usize>| {
            for at in range {
                let start = Instant::now();
                while !matches!(
                    disk.read_within(&scene, times[at], PATIENCE),
                    Read::Frame(..)
                ) {
                    assert!(start.elapsed() < Duration::from_secs(5), "frame {at}");
                }
            }
        };
        // A budget a little under one frame, as 256 MiB is to an 8K frame.
        disk.set_read_budget(frame * 97 / 100);
        play(0..4);
        assert!(peak() <= 2 * frame, "{} of {frame}", peak());
        assert_eq!(frames(), 1, "one frame at a time");
        // Room for four frames: never more held than that.
        disk.set_read_budget(4 * frame);
        disk.inner.reader.ready.lock().unwrap().peak = 0;
        play(4..8);
        assert!(peak() <= 4 * frame, "{} of {}", peak(), 4 * frame);
        // The default budget reads this size six ahead, as before.
        disk.set_read_budget(READ_AHEAD_BYTES);
        play(8..9);
        let start = Instant::now();
        while frames() < 4 {
            assert!(start.elapsed() < Duration::from_secs(5), "read ahead");
            std::thread::sleep(Duration::from_millis(2));
        }
        // Critical pressure: only the frame the playhead is on stays, and
        // nothing more is read ahead of it.
        disk.memory_pressure(crate::memory::Pressure::Critical);
        assert!(frames() <= 1, "what was read ahead goes at once");
        // A read already in flight ends and is dropped; queued ones never
        // start.
        let start = Instant::now();
        while held() > frame {
            assert!(start.elapsed() < Duration::from_secs(5), "{} held", held());
            std::thread::sleep(Duration::from_millis(2));
        }
        play(9..11);
        std::thread::sleep(Duration::from_millis(100));
        assert!(frames() <= 1 && held() <= 2 * frame, "{} held", held());
        // Pressure over: it reads ahead again.
        disk.memory_pressure(crate::memory::Pressure::Normal);
        play(10..11);
        let start = Instant::now();
        while frames() < 2 {
            assert!(start.elapsed() < Duration::from_secs(5), "read ahead again");
            disk.read_within(&scene, times[10], Duration::ZERO);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A rendered frame damaged on disk (a flipped byte, a cut payload, a
    /// file cut inside its header) is not shown and does not stay
    /// "rendered": the read that finds it removes it, the bar shows the
    /// stretch unrendered, background rendering renders it again by itself,
    /// and the frames then read from disk.
    #[test]
    fn a_damaged_rendered_frame_is_rendered_again() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
            .unwrap();
        let scene = scene();
        disk.set_activity(crate::jobs::Activity::default());
        disk.observe(&scene, &[], None, None, None, 1, Ticks::new(0).unwrap());
        ready(&disk);
        // Every chunk plays in real time (injected costs) except 45-59,
        // where playback dropped frame 50.
        let at = |n: i64| Ticks::new(Rate::FPS_30.frame_start(n).unwrap()).unwrap();
        for a in (0..90).step_by(15) {
            for n in [a, a + 14] {
                disk.measured(disk.frame_id(&scene, at(n)).unwrap(), 1_000.);
            }
        }
        disk.measured(disk.frame_id(&scene, at(50)).unwrap(), DROPPED_US);
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
        let whole = |b: &RenderBars| b.bars == vec![(1.5, 2., BarState::Rendered)] && !b.running;
        settle(&disk, whole);
        let fills = disk.inner.state.lock().unwrap().fill.serial;
        let file = |n: i64| {
            let state = disk.inner.state.lock().unwrap();
            let key = state.frames.as_ref().unwrap().key(n).unwrap();
            dir.path()
                .join("frames")
                .join(format!("{}.frame", key.digest().unwrap()))
        };
        for (n, damage) in [(50, 0), (51, 1), (52, 2)] {
            let mut bytes = std::fs::read(file(n)).unwrap();
            match damage {
                0 => *bytes.last_mut().unwrap() ^= 1,
                1 => bytes.truncate(bytes.len() / 2),
                _ => bytes.truncate(10),
            }
            std::fs::write(file(n), bytes).unwrap();
        }
        // Playback reads them: none is shown, each leaves the index.
        let start = Instant::now();
        for n in [50, 51, 52] {
            while present(&disk).contains(&n)
                && disk.inner.state.lock().unwrap().fill.serial == fills
            {
                assert!(
                    !matches!(disk.read_within(&scene, at(n), PATIENCE), Read::Frame(..)),
                    "a damaged frame was shown"
                );
                assert!(
                    start.elapsed() < Duration::from_secs(20),
                    "frame {n} is damaged on disk and still counts as rendered"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        // Rendered again without being asked, and read from disk after.
        settle(&disk, |b| {
            whole(b) && disk.inner.state.lock().unwrap().fill.serial > fills
        });
        assert_eq!(present(&disk), (45..60).collect());
        for n in [50, 51, 52] {
            assert!(read_eventually(&disk, &scene, at(n)).is_some(), "frame {n}");
        }
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
