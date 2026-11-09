//! Disk render cache (plan section 1, pro media): a composition's frames as
//! the paused viewer draws them, rendered in the background (View › Render
//! Selection), kept in Storage › Renders, and shown by playback and scrubbing
//! instead of rendering them again.
//!
//! - **Identity.** Frames are keyed by a fingerprint of everything that draws
//!   them: the canonical document, the contributed definitions, each media
//!   file's content (sampled) and stamp, the installed fonts, the viewer's
//!   output tone, the running renderer binary, and (with raster overrides)
//!   this session's override wrappers. Any change misses every frame; frames
//!   of an earlier state stay until the limit or Clear… removes them.
//! - **Rendering** runs in an `--engine-job` child (`jobs::fill_cache`) with
//!   the export's admission gate and its exact override answers, writing to
//!   the folder captured when it started. The viewer's engine thread never
//!   renders, reads or writes cache frames itself: reads come from a reader
//!   thread ahead of the playhead.
//! - **Generations** fence everything asynchronous: a fingerprint, a fill's
//!   progress or a read lands only if no observation, storage change or
//!   deletion came after it started.
//! - The shared state is never held across disk IO; the render bar reads a
//!   snapshot published on every change.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use pm_promedia::cache::{BarState, RenderCache, RenderKey};
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::definitions::DefinitionSnapshot;
use pm_time::Rate;
use serde::{Deserialize, Serialize};

use crate::render::Scene;

/// Frames on disk: RGBA16F rows, top-down, premultiplied working values.
pub const FORMAT: &str = "rgba16f-viewer-v1";
/// Media stamps are looked at again at most this often.
const MEDIA_RECHECK: Duration = Duration::from_secs(2);
/// The same storage is indexed again at most this often.
const REINDEX_EVERY: Duration = Duration::from_secs(2);
/// Frames read ahead of the playhead, and kept read.
const AHEAD: i64 = 6;
const READY_FRAMES: usize = 8;

/// What a fill child renders: frames by time and key, into `dir`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheFill {
    pub dir: PathBuf,
    pub quota: u64,
    pub width: u32,
    pub height: u32,
    pub frames: Vec<CacheFrame>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheFrame {
    pub ticks: i64,
    pub key: RenderKey,
}

/// A fill child's failure when the volume is nearly full.
pub const DISK_FULL: &str =
    "Rendering stopped because the disk is almost full. Free some space and render again.";
/// A fill child's failure when kept frames fill Settings › Media › Render Files Limit.
pub const LIMIT_FULL: &str =
    "Kept rendered frames fill the Render Files Limit. Raise the limit or delete rendered frames.";

/// One media file's stamp: path, length, modification time, inode.
type Stamp = (PathBuf, u64, u128, u64);

/// The composition state a fingerprint is taken of.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    doc: usize,
    revision: u64,
    content: u64,
    comp: String,
    tone: Option<String>,
    overrides: Option<u64>,
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
    fn key(&self, n: i64) -> Option<RenderKey> {
        let fingerprint = self.fingerprint?;
        Some(RenderKey {
            graph: fingerprint,
            media: fingerprint,
            ticks: self.ticks(n)?.get(),
            width: self.size.width,
            height: self.size.height,
            format: FORMAT.into(),
            renderer_version: renderer_identity().into(),
        })
    }
}

/// What the timeline's render bar and the Render task show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderBars {
    /// Rises whenever anything below changes.
    pub revision: u64,
    pub composition: Option<String>,
    /// Seconds, half-open, ordered; unrendered stretches are left out.
    pub bars: Vec<(f64, f64, BarState)>,
    /// The current fill: frames queued and rendered, whether it runs, and why
    /// it stopped (`disk_full`: the volume or the limit was full).
    pub queued: u64,
    pub done: u64,
    pub running: bool,
    pub error: Option<String>,
    pub disk_full: bool,
}

#[derive(Default)]
struct Fill {
    generation: u64,
    queued: BTreeSet<i64>,
    total: u64,
    done: u64,
    running: bool,
    error: Option<String>,
    disk_full: bool,
}

type Notify = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    notify: Option<Notify>,
    wake: Option<std::sync::mpsc::SyncSender<()>>,
    root: Option<(PathBuf, u64)>,
    cache: Option<Arc<RenderCache>>,
    /// Rises with every new observation, storage or deletion: fences
    /// fingerprints, fills and reads that started before it.
    generation: u64,
    frames: Option<Frames>,
    media_checked: Option<Instant>,
    indexed: Option<Instant>,
    fill: Fill,
    revision: u64,
}

struct Running {
    cancel: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

struct Inner {
    state: Mutex<State>,
    published: Mutex<Arc<RenderBars>>,
    running: Mutex<Option<Running>>,
    reader: Reader,
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
        error: fill.error.clone(),
        disk_full: fill.disk_full,
        ..RenderBars::default()
    };
    let Some(frames) = state.frames.as_ref() else {
        return out;
    };
    out.composition = Some(frames.identity.comp.clone());
    let seconds = |n: i64| {
        frames
            .ticks(n)
            .map_or(0., |t| t.get() as f64 / TICKS_PER_SECOND as f64)
    };
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
    marks.sort_by_key(|m| m.0);
    for (n, bar) in marks {
        let (from, to) = (seconds(n), seconds(n + 1));
        match out.bars.last_mut() {
            Some(last) if last.2 == bar && (last.1 - from).abs() < 1e-9 => last.1 = to,
            _ => out.bars.push((from, to, bar)),
        }
    }
    out
}

impl DiskFrames {
    /// Called (on any thread, never blocking) whenever the render bar changes.
    pub fn set_notify(&self, notify: Option<Notify>) {
        self.inner.state.lock().unwrap().notify = notify;
    }
    /// Wakes the engine thread when frames land (playback reads them).
    pub(crate) fn set_wake(&self, wake: std::sync::mpsc::SyncSender<()>) {
        self.inner.state.lock().unwrap().wake = Some(wake);
    }
    fn with<R>(&self, f: impl FnOnce(&mut State) -> (R, bool)) -> R {
        let mut state = self.inner.state.lock().unwrap();
        let (r, changed) = f(&mut state);
        if changed {
            bump(&mut state, &self.inner.published);
        }
        r
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
        self.with(|s| {
            s.root = root;
            s.cache = cache;
            s.generation += 1;
            // Observed again (and fingerprinted against the new folder).
            s.frames = None;
            s.fill = Fill::default();
            ((), true)
        });
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

    /// The engine thread's current scene. A new composition state (scene,
    /// output tone, override wrappers, or, looked at every 2 s, a media
    /// file's stamp) takes a new fingerprint off this thread and finds its
    /// frames on disk. Returns whether it did.
    pub(crate) fn observe(
        &self,
        scene: &Scene,
        definitions: &[DefinitionSnapshot],
        overrides: Option<u64>,
        tone: Option<String>,
        doc_id: u64,
    ) -> bool {
        let state = self.inner.state.lock().unwrap();
        if state.cache.is_none() {
            return false;
        }
        let unchanged = state.frames.as_ref().is_some_and(|f| {
            f.identity.scene_matches(scene)
                && f.identity.tone == tone
                && f.identity.overrides == overrides
        });
        let recheck = state
            .media_checked
            .is_none_or(|at| at.elapsed() >= MEDIA_RECHECK);
        if unchanged && !recheck {
            return false;
        }
        drop(state);
        // A few stats, every 2 s at most, outside the lock.
        let media = stamps(&scene.media);
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
            media,
        };
        let source = Source {
            scene: scene.clone(),
            definitions: definitions.to_vec(),
            doc_id,
        };
        state.generation += 1;
        let generation = state.generation;
        state.frames = Some(Frames {
            identity: identity.clone(),
            source: source.clone(),
            fingerprint: None,
            size,
            rate,
            count,
            present: HashSet::new(),
            kept: HashSet::new(),
        });
        bump(&mut state, &self.inner.published);
        let cache = state.cache.clone().expect("checked above");
        drop(state);
        let weak = Arc::downgrade(&self.inner);
        std::thread::Builder::new()
            .name("pm-render-cache".into())
            .spawn(move || {
                let fingerprint = fingerprint(&source, &identity);
                let template = Frames {
                    identity,
                    source,
                    fingerprint: Some(fingerprint),
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

    /// Render Selection: the composition's frames in `[from, to)` seconds that
    /// are not on disk render in an `--engine-job` child (`executable`),
    /// admitted around viewer `activity` as exports are. Returns the frames
    /// queued.
    pub fn start_fill(
        &self,
        from: f64,
        to: f64,
        executable: &Path,
        activity: crate::jobs::Activity,
    ) -> Result<u64, String> {
        let mut running = self.inner.running.lock().unwrap();
        if running.as_ref().is_some_and(|r| !r.thread.is_finished()) {
            return Err("A render is already running.".into());
        }
        let mut state = self.inner.state.lock().unwrap();
        let frames = state
            .frames
            .as_ref()
            .ok_or("Open a composition to render.")?;
        if frames.fingerprint.is_none() {
            return Err("Still reading the composition. Try again in a moment.".into());
        }
        if frames.identity.tone.is_some() {
            return Err(
                "Rendering to disk isn't available while the viewer tone-maps HDR for this display."
                    .into(),
            );
        }
        let (dir, quota) = state
            .root
            .clone()
            .ok_or("No storage for rendered frames.")?;
        let missing: Vec<(i64, CacheFrame)> = frame_range(frames, from, to)
            .filter(|n| !frames.present.contains(n))
            .filter_map(|n| {
                Some((
                    n,
                    CacheFrame {
                        ticks: frames.ticks(n)?.get(),
                        key: frames.key(n)?,
                    },
                ))
            })
            .collect();
        if missing.is_empty() {
            return Ok(0);
        }
        let (source, size, rate) = (frames.source.clone(), frames.size, frames.rate);
        let generation = state.generation;
        state.fill = Fill {
            generation,
            queued: missing.iter().map(|m| m.0).collect(),
            total: missing.len() as u64,
            running: true,
            ..Fill::default()
        };
        bump(&mut state, &self.inner.published);
        drop(state);
        let order: Vec<i64> = missing.iter().map(|m| m.0).collect();
        let fill = CacheFill {
            dir,
            quota,
            width: size.width,
            height: size.height,
            frames: missing.into_iter().map(|m| m.1).collect(),
        };
        let request = fill_request(&source, rate, fill).map_err(|e| e.to_string());
        let cancel = Arc::new(AtomicBool::new(false));
        let (stop, weak, exe) = (
            cancel.clone(),
            Arc::downgrade(&self.inner),
            executable.to_owned(),
        );
        let thread = std::thread::Builder::new()
            .name("pm-render-fill".into())
            .spawn(move || run_fill(weak, generation, order, request, &exe, activity, &stop))
            .map_err(|e| e.to_string())?;
        *running = Some(Running { cancel, thread });
        Ok(self.inner.published.lock().unwrap().queued)
    }

    /// Stops the running fill and waits for its child to exit (frames it
    /// finished stay). Blocks: call off the main thread.
    pub fn cancel_fill(&self) {
        let running = self.inner.running.lock().unwrap().take();
        if let Some(running) = running {
            running.cancel.store(true, Ordering::Release);
            let _ = running.thread.join();
        }
    }

    /// Keep Rendered: the rendered frames in `[from, to)` are pinned against
    /// the limit and Clear… (`keep`), or unpinned. Returns frames changed.
    /// Disk work: off the main thread.
    pub fn keep_range(&self, from: f64, to: f64, keep: bool) -> std::io::Result<u64> {
        let plan = self.with(|s| {
            let (Some(cache), Some(frames)) = (s.cache.clone(), s.frames.as_ref()) else {
                return (None, false);
            };
            let keys: Vec<(i64, RenderKey)> = frame_range(frames, from, to)
                .filter(|n| frames.present.contains(n) && frames.kept.contains(n) != keep)
                .filter_map(|n| Some((n, frames.key(n)?)))
                .collect();
            (Some((cache, keys, s.generation)), false)
        });
        let Some((cache, keys, generation)) = plan else {
            return Ok(0);
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
        Ok(count)
    }

    /// Delete Rendered Frames: every frame of the open composition's current
    /// state, kept ones too. A running fill stops first, so no write lands
    /// after the deletion. Returns frames deleted. Disk work: off the main
    /// thread.
    pub fn delete_all(&self) -> std::io::Result<u64> {
        self.cancel_fill();
        let plan = self.with(|s| {
            s.generation += 1;
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
    pub(crate) fn read(&self, scene: &Scene, time: Ticks) -> Option<(Size, Arc<Vec<u8>>)> {
        let state = self.inner.state.lock().unwrap();
        let frames = state.frames.as_ref()?;
        if !frames.identity.scene_matches(scene) || frames.fingerprint.is_none() {
            return None;
        }
        let n = frames.index(time)?;
        let size = frames.size;
        let bytes = u64::from(size.width) * u64::from(size.height) * 8;
        let ahead: Vec<(i64, RenderKey)> = (n..(n + AHEAD).min(frames.count))
            .filter(|k| frames.present.contains(k))
            .filter_map(|k| Some((k, frames.key(k)?)))
            .collect();
        let (generation, cache) = (state.generation, state.cache.clone()?);
        drop(state);
        let found = self.inner.reader.take(generation, n);
        for (k, key) in ahead {
            self.inner.reader.ask(generation, k, key, &cache, bytes);
        }
        found.map(|bytes| (size, bytes))
    }
}

/// The fill child's request: the viewer's scene as an export job (png
/// settings stand in for delivery; the child renders `fill`'s frames only).
fn fill_request(
    source: &Source,
    rate: Rate,
    fill: CacheFill,
) -> anyhow::Result<crate::jobs::ExportRequest> {
    use pm_protocol::{
        color::{OutputColor, Precision},
        common::{FrameRate, SessionId, TickRange},
        priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, QualityTier},
    };
    let first = fill.frames.first().map_or(0, |f| f.ticks);
    let last = fill.frames.last().map_or(0, |f| f.ticks);
    let step = rate.frame_start(1).unwrap_or(1).max(1);
    let fps = (TICKS_PER_SECOND as f64 / step as f64 * 1000.).round() as u32;
    let settings = ExportSettings {
        format: ExportFormat::PngSequence { sixteen_bit: true },
        alpha: AlphaOutput::Straight,
        quality: QualityTier::High,
        size: Size::new(fill.width, fill.height),
        frame_rate: FrameRate::new(fps, 1000),
        range: TickRange {
            start: Ticks::new(first)?,
            end: Ticks::new(last + step)?,
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
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
        disk_reserve: crate::jobs::DISK_RESERVE,
        bypass_missing_effects: true,
        without_offline: true,
        preview: false,
        cache: Some(fill),
        frozen: None,
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

/// The fill thread: runs the child, lands its progress while the generation
/// it started at is current, and records how it ended.
fn run_fill(
    inner: Weak<Inner>,
    generation: u64,
    order: Vec<i64>,
    request: Result<crate::jobs::ExportRequest, String>,
    executable: &Path,
    activity: crate::jobs::Activity,
    cancel: &AtomicBool,
) {
    use pm_protocol::priority::JobState;
    let land = |done: usize, end: Option<(Option<String>, bool)>| {
        let Some(inner) = inner.upgrade() else { return };
        let mut state = inner.state.lock().unwrap();
        if state.generation != generation || state.fill.generation != generation {
            return;
        }
        if let Some(frames) = state.frames.as_mut() {
            frames.present.extend(order.iter().take(done));
        }
        for n in order.iter().take(done) {
            state.fill.queued.remove(n);
        }
        state.fill.done = state.fill.done.max(done as u64);
        if let Some((error, disk_full)) = end {
            state.fill.running = false;
            state.fill.queued.clear();
            state.fill.error = error;
            state.fill.disk_full = disk_full;
        }
        bump(&mut state, &inner.published);
    };
    let started = request.and_then(|request| {
        crate::jobs::ExportHandle::start_scheduled(executable, request, activity)
            .map_err(|e| e.to_string())
    });
    let mut handle = match started {
        Ok(handle) => handle,
        Err(error) => return land(0, Some((Some(error), false))),
    };
    let total = order.len() as f64;
    let mut cancelled = false;
    let status = loop {
        if cancel.load(Ordering::Acquire) && !cancelled {
            cancelled = true;
            let _ = handle.cancel();
        }
        let status = handle.status();
        if status.state.is_terminal() {
            break status;
        }
        land((status.progress * total).round() as usize, None);
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = handle.wait();
    let end = match status.state {
        JobState::Done => (order.len(), None, false),
        JobState::Cancelled => ((status.progress * total).floor() as usize, None, false),
        _ => {
            let message = status.error.map(|e| e.message).unwrap_or_default();
            let full =
                message == DISK_FULL || message == LIMIT_FULL || message == crate::jobs::LOW_DISK;
            (
                (status.progress * total).floor() as usize,
                Some(message),
                full,
            )
        }
    };
    land(end.0, Some((end.1, end.2)));
    // The limit may have evicted older frames as these landed.
    if let Some(inner) = inner.upgrade() {
        let _ = DiskFrames { inner }.reindex(true);
    }
}

/// Reads frames on a thread of its own, ahead of the playhead.
struct Reader {
    asks: std::sync::mpsc::SyncSender<Ask>,
    ready: Arc<Mutex<Ready>>,
}
struct Ask {
    id: (u64, i64),
    key: RenderKey,
    cache: Arc<RenderCache>,
    bytes: u64,
}
/// A read frame: the generation it was asked at, and its index.
type ReadId = (u64, i64);
#[derive(Default)]
struct Ready {
    frames: VecDeque<(ReadId, Arc<Vec<u8>>)>,
    asked: HashSet<(u64, i64)>,
}
impl Reader {
    /// The thread holds only the ready list: dropping the cache's last
    /// owner closes the channel and ends it.
    fn start() -> Self {
        let (asks, rx) = std::sync::mpsc::sync_channel::<Ask>(2 * AHEAD as usize);
        let ready = Arc::new(Mutex::new(Ready::default()));
        let shared = ready.clone();
        std::thread::Builder::new()
            .name("pm-render-cache-read".into())
            .spawn(move || {
                for ask in rx {
                    let data = ask
                        .cache
                        .get(&ask.key)
                        .ok()
                        .flatten()
                        .filter(|d| d.len() as u64 == ask.bytes);
                    let mut ready = shared.lock().unwrap();
                    ready.asked.remove(&ask.id);
                    if let Some(data) = data {
                        ready.frames.push_back((ask.id, Arc::new(data)));
                        while ready.frames.len() > READY_FRAMES {
                            ready.frames.pop_front();
                        }
                    }
                }
            })
            .expect("render cache reader thread");
        Self { asks, ready }
    }
    fn take(&self, generation: u64, n: i64) -> Option<Arc<Vec<u8>>> {
        let ready = self.ready.lock().unwrap();
        ready
            .frames
            .iter()
            .find(|(id, _)| *id == (generation, n))
            .map(|(_, bytes)| bytes.clone())
    }
    fn ask(&self, generation: u64, n: i64, key: RenderKey, cache: &Arc<RenderCache>, bytes: u64) {
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
            bytes,
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

fn stamp(path: &Path) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok();
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

fn stamps(media: &pm_export::render::MediaPaths) -> Vec<Stamp> {
    media.paths.values().map(|p| stamp(p)).collect()
}

/// The running renderer: a digest of this executable (any rebuild is
/// another renderer), taken once.
fn renderer_identity() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
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
                Ok(t) if t.is_file() => out.push(stamp(&path)),
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

/// The fingerprint of everything that draws the composition's frames.
fn fingerprint(source: &Source, identity: &Identity) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key("powermove render cache v2");
    if let Some(root) = source.scene.doc.root() {
        hash.update(&pm_model::hash::canonical_digest_parallel(root));
    }
    hash.update(identity.comp.as_bytes());
    hash.update(&serde_json::to_vec(&source.definitions).unwrap_or_default());
    for (asset, path) in &source.scene.media.paths {
        hash.update(asset.as_bytes());
        hash.update(path.as_os_str().as_encoded_bytes());
        media_content(path, &mut hash);
    }
    for (path, len, modified, inode) in &identity.media {
        hash.update(path.as_os_str().as_encoded_bytes());
        hash.update(&len.to_le_bytes());
        hash.update(&modified.to_le_bytes());
        hash.update(&inode.to_le_bytes());
    }
    fonts_identity(&mut hash);
    hash.update(format!("{:?}", identity.tone).as_bytes());
    hash.update(renderer_identity().as_bytes());
    if let Some(epoch) = identity.overrides {
        hash.update(&std::process::id().to_le_bytes());
        hash.update(&epoch.to_le_bytes());
    }
    *hash.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        cache.put(&key, &bytes).unwrap();
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
        assert!(disk.observe(&scene, &[], None, None, 1));
        assert!(!disk.observe(&scene, &[], None, None, 1));
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
        assert_eq!(disk.keep_range(0., 3., true).unwrap(), 1);
        assert_eq!(disk.bars().bars[0].2, BarState::Kept);
        assert_eq!(disk.delete_all().unwrap(), 1);
        assert!(disk.bars().bars.is_empty());
        assert!(read_eventually(&disk, &scene, time).is_none());
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
        disk.observe(&scene, &[], None, None, 1);
        ready(&disk);
        let time = write_frame(&disk, 30);
        assert!(read_eventually(&disk, &scene, time).is_some());
        assert!(disk.observe(&scene, &[], None, Some("Pq".into()), 1));
        ready(&disk);
        assert!(read_eventually(&disk, &scene, time).is_none());
        assert!(disk.observe(&scene, &[], Some(3), None, 1));
        ready(&disk);
        assert!(read_eventually(&disk, &scene, time).is_none());
        // Back to the first state: its frame is found again.
        assert!(disk.observe(&scene, &[], None, None, 1));
        ready(&disk);
        assert!(read_eventually(&disk, &scene, time).is_some());
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
        disk.observe(&scene, &[], None, None, 1);
        ready(&disk);
        let queued = disk
            .start_fill(1., 1.2, &exe, crate::jobs::Activity::default())
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
        let again = disk.start_fill(1., 1.2, &exe, crate::jobs::Activity::default());
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
        disk.observe(scene, &[], None, None, 1);
        ready(&disk);
        disk.start_fill(1., 1.03, &exe, crate::jobs::Activity::default())
            .unwrap();
        let bars = settle(&disk, |b| !b.running);
        assert_eq!(bars.error.as_deref(), None, "{bars:?}");
        let time = Ticks::new(pm_time::Rate::FPS_30.frame_start(30).unwrap()).unwrap();
        read_eventually(&disk, scene, time).expect("rendered frame")
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
        disk.observe(&scene, &[], None, None, 1);
        ready(&disk);
        disk.start_fill(0., 3., &exe, crate::jobs::Activity::default())
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

    /// A fingerprint taken for an observation that something newer replaced
    /// never lands (here: storage changed while it was taken).
    #[test]
    fn a_stale_fingerprint_never_lands() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("a"), u64::MAX)))
            .unwrap();
        let scene = scene();
        disk.observe(&scene, &[], None, None, 1);
        disk.set_storage(Some((dir.path().join("b"), u64::MAX)))
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(disk.inner.state.lock().unwrap().frames.is_none());
    }
}
