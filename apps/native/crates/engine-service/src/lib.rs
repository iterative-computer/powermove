//! In-process native rendering. No Node, napi entrypoints, bridge or GPUI.
//! Callers own below-slot placement and pump native layers on the main thread.
pub mod audio;
pub mod clock;
pub mod color;
pub mod compat;
pub mod frame_codec;
mod frame_inputs;
pub mod jobs;
pub mod layers;
pub mod media;
pub mod memory;
pub mod policy;
mod present;
pub mod qos;
mod render;
pub mod render_cache;
pub mod resources;
mod scopes;
pub use scopes::ScopeResult;
pub mod studio;
pub mod trace;
pub mod transport;

use anyhow::{Result, ensure};
#[cfg(feature = "kernel")]
use pm_kernel::actor::Published;
#[cfg(feature = "kernel")]
pub use pm_kernel::document::Snapshot;
/// Standalone CLI snapshot when the optional kernel adapter is disabled.
#[cfg(not(feature = "kernel"))]
#[derive(Clone)]
pub struct Snapshot {
    pub doc_id: u64,
    pub revision: u64,
    pub document: std::sync::Arc<pm_model::Document>,
}
#[cfg(not(feature = "kernel"))]
impl Snapshot {
    pub fn open_comp(&self) -> Option<String> {
        self.document
            .to_value()?
            .as_object()?
            .get("compId")?
            .as_str()
            .map(str::to_owned)
    }
}
pub use crate::media::MediaMap;
use pm_protocol::{
    clock::{AudioLevels, ClockSample, TransportCommand},
    common::{Size, Ticks},
};
pub use policy::{Crop, Moment, Output, Quality, Stamp};
pub use render::{FrameSpec, Scene, metal_device};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

/// Geometry is normalized to the content viewport. Colours are linear P3 RGBA.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OverlayLine {
    pub ends: [f32; 4],
    pub color: [f32; 4],
}
#[derive(Clone, Debug, Default)]
pub struct OverlayGeometry {
    lines: Vec<OverlayLine>,
}
#[repr(C)]
pub(crate) struct OverlayParams {
    count: u32,
    reduced: u32,
    /// The source is the whole composition (a disk render cache frame).
    pub(crate) whole: u32,
    pad: u32,
    output: [f32; 4],
    canvas: [f32; 4],
    drawable: [f32; 2],
    pad2: [f32; 2],
    lines: [OverlayLine; OverlayGeometry::MAX_LINES],
}
impl OverlayGeometry {
    /// Must match `viewer.metal`; the parameters stay inside Metal's 4 KiB inline limit.
    pub const MAX_LINES: usize = 120;
    pub fn new(lines: Vec<OverlayLine>) -> Result<Self> {
        ensure!(
            lines.len() <= Self::MAX_LINES,
            "at most {} locked overlay lines",
            Self::MAX_LINES
        );
        ensure!(
            lines
                .iter()
                .all(|l| l.ends.iter().chain(&l.color).all(|v| v.is_finite())
                    && (0. ..=1.).contains(&l.color[3])),
            "invalid overlay geometry"
        );
        Ok(Self { lines })
    }
    /// The lines that can be drawn, at most [`Self::MAX_LINES`] of them: a
    /// line with a non-finite end or color (geometry divided by a zero-size
    /// composition, a transform beyond what a float holds) is left out, and
    /// lines past the limit are dropped. Never fails: overlay geometry is
    /// derived from the person's document on the main thread, where an
    /// error has nowhere to go but a crash.
    pub fn lossy(lines: impl IntoIterator<Item = OverlayLine>) -> Self {
        Self {
            lines: lines
                .into_iter()
                .filter(|l| {
                    l.ends.iter().chain(&l.color).all(|v| v.is_finite())
                        && (0. ..=1.).contains(&l.color[3])
                })
                .take(Self::MAX_LINES)
                .collect(),
        }
    }
    fn params(&self) -> OverlayParams {
        let mut p = OverlayParams {
            count: self.lines.len() as u32,
            reduced: 0,
            whole: 0,
            pad: 0,
            output: [0.; 4],
            canvas: [0.; 4],
            drawable: [0.; 2],
            pad2: [0.; 2],
            lines: [OverlayLine::default(); Self::MAX_LINES],
        };
        p.lines[..self.lines.len()].copy_from_slice(&self.lines);
        p
    }
}
/// GPUI supplies geometry/bindings once; this native Rust evaluator runs on
/// the render worker against the very document and time used for the picture.
/// It must never call GPUI, JS, or wait for the main thread.
pub type LockedOverlayProvider =
    Arc<dyn Fn(&pm_model::Document, &str, Ticks) -> Result<OverlayGeometry> + Send + Sync>;
#[derive(Clone, Default, PartialEq)]
pub struct MediaCatalog {
    pub originals: BTreeMap<String, std::path::PathBuf>,
    pub proxies: BTreeMap<String, std::path::PathBuf>,
    /// Optimized media: full-size high-quality transcodes the viewer plays
    /// in Automatic in place of their originals (Final Cut's optimized
    /// media). Exports never read them.
    pub optimized: BTreeMap<String, std::path::PathBuf>,
    /// High-resolution originals whose preview should use a ready HD proxy.
    /// Runtime-only policy; never serialized into the project.
    pub automatic_proxies: std::collections::BTreeSet<String>,
    /// Always Play Original: these assets never play a proxy or optimized
    /// media, in any mode.
    pub play_original: std::collections::BTreeSet<String>,
    /// Assets the app is still resolving (just imported or relinked): the
    /// viewer draws nothing for them until they land, never Media Offline.
    pub resolving: std::collections::BTreeSet<String>,
}
/// The files a viewer reads in `mode`: while playing and dragging, and
/// when paused. A paused frame shows the composition as an export draws
/// it, so Automatic's HD proxies (a playback convenience for large
/// sources) stand in only while playing or dragging; the proxies a person
/// chose (Proxy Preferred, Proxy Only) stay in every moment.
fn viewer_media(
    media: &MediaCatalog,
    mode: MediaMode,
) -> (
    Arc<pm_export::render::MediaPaths>,
    Arc<pm_export::render::MediaPaths>,
) {
    let paths = |m: &MediaCatalog| {
        Arc::new(pm_export::render::MediaPaths {
            paths: m.viewer_paths(mode),
            resolving: m.resolving.clone(),
        })
    };
    let moving = paths(media);
    let paused = if mode == MediaMode::Automatic && !media.automatic_proxies.is_empty() {
        paths(&MediaCatalog {
            automatic_proxies: Default::default(),
            ..media.clone()
        })
    } else {
        moving.clone()
    };
    (moving, paused)
}
/// What the viewer plays for each clip (View › Media). Exports read
/// originals whatever this says; only an explicit draft export uses proxies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaMode {
    /// HD proxies for large sources, otherwise originals/optimized media.
    #[default]
    Automatic,
    /// Originals only: a clip whose original is missing is offline.
    Originals,
    /// A proxy where one exists, the original otherwise.
    ProxyPreferred,
    /// Proxies only: a clip without one is offline.
    ProxyOnly,
}
impl MediaCatalog {
    /// Export safety: a draft is the only path that can substitute proxy pixels.
    pub fn export_paths(&self, draft: bool) -> pm_export::render::MediaPaths {
        let mut paths = self.originals.clone();
        if draft {
            paths.extend(self.proxies.clone());
        }
        pm_export::render::MediaPaths {
            paths,
            ..Default::default()
        }
    }
    /// The file the viewer reads for each asset in `mode`.
    pub fn viewer_paths(&self, mode: MediaMode) -> BTreeMap<String, std::path::PathBuf> {
        // Most preferred last: each later map overrides the earlier ones.
        let order: &[&BTreeMap<String, std::path::PathBuf>] = match mode {
            MediaMode::Automatic => &[&self.proxies, &self.originals, &self.optimized],
            MediaMode::Originals => &[&self.originals],
            MediaMode::ProxyPreferred => &[&self.originals, &self.optimized, &self.proxies],
            MediaMode::ProxyOnly => &[&self.proxies],
        };
        let mut paths = BTreeMap::new();
        for map in order {
            paths.extend(map.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        if mode == MediaMode::Automatic {
            for id in &self.automatic_proxies {
                if let Some(proxy) = self.proxies.get(id) {
                    paths.insert(id.clone(), proxy.clone());
                }
            }
        }
        for id in &self.play_original {
            match self.originals.get(id) {
                Some(original) => paths.insert(id.clone(), original.clone()),
                None => paths.remove(id),
            };
        }
        paths
    }
    /// Assets the viewer draws from a proxy in `mode`.
    pub fn proxied(&self, mode: MediaMode) -> usize {
        let paths = self.viewer_paths(mode);
        self.proxies
            .iter()
            .filter(|(id, path)| {
                paths.get(*id) == Some(path) && self.originals.get(*id) != Some(path)
            })
            .count()
    }
    pub fn audio_paths(&self, mode: MediaMode) -> MediaMap {
        MediaMap(Arc::new(
            self.viewer_paths(mode)
                .into_iter()
                .map(|(k, v)| (k, Some(v)))
                .collect(),
        ))
    }
}

#[derive(Clone)]
pub struct ViewerConfig {
    pub snapshot: Snapshot,
    pub composition: String,
    pub media: MediaCatalog,
    pub output: Output,
    /// Retained caller layer; configure_layer must have run on the main thread.
    /// None renders offscreen. The service never creates an NSWindow.
    pub layer: Option<metal::MetalLayer>,
    pub rate: pm_time::Rate,
    pub end: Option<i64>,
    pub audio: bool,
}
#[derive(Clone, Debug)]
pub struct FrameReport {
    pub stamp: Stamp,
    pub time: Ticks,
    pub cache_hit: bool,
    pub gpu_ok: bool,
    pub display_host_seconds: Option<f64>,
    pub completion_host_seconds: Option<f64>,
    pub pixels: Option<Vec<[f64; 3]>>,
    pub worker_to_completion_ms: Option<f64>,
    pub input_host_seconds: Option<f64>,
    pub audio_host_seconds: Option<f64>,
    pub moment: Moment,
    /// Render resolution as a share of the drawable.
    pub scale: f64,
    /// A video layer showed its nearest decoded frame.
    pub stand_ins: bool,
}
#[derive(Clone, Debug, Default)]
pub struct ViewerStatus {
    pub frame: Option<FrameReport>,
    pub clock: Option<ClockSample>,
    pub levels: Option<AudioLevels>,
    pub error: Option<String>,
    pub stale_rejections: u64,
    pub no_drawable: u64,
    pub audio_fallback: bool,
    pub skipped_targets: u64,
    /// Display ticks whose frame was identical to the one already on screen.
    /// The layer keeps showing it; no drawable is spent (see `run`).
    pub held_frames: u64,
    /// Paused pictures re-presented once because they never reached the glass.
    pub represented: u64,
    /// Frames completed for display under this viewer (Performance panel).
    pub presented: u64,
    /// Layers missing from the frame on screen, with why. The rest is drawn.
    pub layer_failures: Vec<pm_render::LayerFailure>,
    /// Effects the project uses whose provider isn't installed. Their
    /// layers draw without them (bypassed, as After Effects does).
    pub missing_effects: Vec<pm_render::effects::versions::MissingEffect>,
    /// Playing frames with a stand-in for a decode that missed its allowance.
    pub stand_in_frames: u64,
    /// Auto's current playing resolution, for the viewer chrome.
    pub effective_quality: f64,
    /// Playing or waiting for the audio epoch: the host should keep its
    /// display-phase source running so the start can align to vsync.
    pub wants_display: bool,
    /// GPU devices lost and rebuilt under this viewer.
    pub device_losses: u64,
    /// Times the worker started again after a panic (`supervise`).
    pub restarts: u64,
    /// A capture asked for (`capture_next`) that can't be delivered: the
    /// viewer stopped after an internal error. Waiters stop waiting on it.
    pub capture_failed: Option<String>,
    /// Clips the viewer plays from a proxy (View › Media). Never counted as
    /// originals.
    pub proxied: usize,
}
impl ViewerStatus {
    /// What the frame on screen leaves out, for the viewer's status line:
    /// the commonest reason and how many layers it hides. `None`: nothing.
    pub fn notice(&self) -> Option<String> {
        let mut reasons: Vec<(&str, BTreeSet<&str>)> = Vec::new();
        for f in &self.layer_failures {
            match reasons.iter_mut().find(|(m, _)| *m == f.message) {
                Some((_, layers)) => {
                    layers.insert(&f.layer);
                }
                None => reasons.push((&f.message, BTreeSet::from([f.layer.as_str()]))),
            }
        }
        reasons.sort_by_key(|(_, layers)| std::cmp::Reverse(layers.len()));
        let layers = |n: usize| match n {
            1 => "1 layer".to_owned(),
            n => format!("{n} layers"),
        };
        let more = |n: usize| match n {
            0 => String::new(),
            n => format!(" (and {n} more)"),
        };
        if let Some((message, failed)) = reasons.first() {
            let others = reasons.len() - 1 + self.missing_effects.len();
            return Some(format!(
                "Not drawn, {}: {message}{}",
                layers(failed.len()),
                more(others)
            ));
        }
        let missing = self.missing_effects.first()?;
        let what = match &missing.revision {
            Some(revision) => format!(
                "Effect saved by a newer Powermove: {} version {revision}",
                missing.effect
            ),
            None => format!("Effect not installed: {}", missing.effect),
        };
        Some(format!(
            "{what}, bypassed on {}{}",
            layers(missing.layers),
            more(self.missing_effects.len() - 1)
        ))
    }
}
struct View {
    config: ViewerConfig,
    stamp: Stamp,
    content: u64,
    dirty: bool,
    capture: bool,
    /// A capture taken into a frame whose pixels haven't landed yet; a
    /// restarted worker asks for it again.
    capture_owed: bool,
    /// The last `SetLoop` and `AudioScrub`, which a restarted worker's new
    /// transport takes again.
    looping: Option<pm_protocol::common::TickRange>,
    audio_scrub: bool,
    overlays: OverlayGeometry,
    overlay_time: Option<Ticks>,
    locked_provider: Option<LockedOverlayProvider>,
    commands: VecDeque<TransportCommand>,
    /// When the oldest pending command was sent (host s), for the
    /// diagnostic trace's command-to-frame latency.
    commands_sent: Option<f64>,
    /// A Mixer monitor solo not yet handed to the transport.
    monitor: Option<Vec<String>>,
    desired_rate: pm_protocol::common::PlaybackRate,
    status: ViewerStatus,
    phase: Option<(f64, f64)>,
    input: Option<f64>,
    pending_input: Option<f64>,
    pipeline: trace::Pipeline,
    mastering: color::Mastering,
    resources: Arc<resources::ResourceSnapshot>,
    quality: Quality,
    /// Auto draws dragged frames of a slow project reduced (an editor's
    /// viewer); otherwise every paused frame is exact.
    adapt_drags: bool,
    /// The skimmer (Final Cut's): while paused the viewer shows this time,
    /// and the playhead stays where it is.
    skim: Option<Ticks>,
    /// The skimmer's last position and pointer time, for its velocity.
    skim_input: Option<f64>,
    skim_rate: pm_protocol::common::PlaybackRate,
    /// The skimmer moved since the worker last looked.
    skim_moved: bool,
    skim_audio: Option<bool>,
    media_mode: MediaMode,
    paths: Arc<pm_export::render::MediaPaths>,
    /// What a paused frame reads: `paths` without Automatic's HD proxies.
    paused_paths: Arc<pm_export::render::MediaPaths>,
}
impl View {
    fn enqueue(&mut self, cmd: TransportCommand) -> Result<()> {
        if matches!(cmd, TransportCommand::Seek { .. })
            && matches!(self.commands.back(), Some(TransportCommand::Seek { .. }))
        {
            self.commands.pop_back();
        }
        ensure!(self.commands.len() < 64, "transport queue full");
        match cmd {
            TransportCommand::Play { rate } => self.desired_rate = rate,
            TransportCommand::Pause | TransportCommand::Step { .. } => {
                self.desired_rate = pm_protocol::common::PlaybackRate::STOPPED
            }
            TransportCommand::PlayAround { .. } => {
                self.desired_rate = pm_protocol::common::PlaybackRate::NORMAL
            }
            TransportCommand::SetLoop { range } => self.looping = range,
            TransportCommand::AudioScrub { enabled } => self.audio_scrub = enabled,
            _ => {}
        }
        self.commands.push_back(cmd);
        self.commands_sent.get_or_insert_with(trace::now);
        self.stamp.sequence += 1;
        self.dirty = true;
        Ok(())
    }
}
#[derive(Default)]
struct State {
    views: BTreeMap<u64, View>,
    retired_documents: BTreeSet<u64>,
    next: u64,
    content: u64,
    stop: bool,
    /// The worker stopped for good after repeated panics (`supervise`).
    halted: Option<String>,
    subscribers: Vec<async_channel::Sender<()>>,
}
impl State {
    /// Takes a view out. Its document is closed (its caches, programs and
    /// override answers go) only if no other view shows it by then.
    fn remove_view(&mut self, id: u64) {
        if let Some(view) = self.views.remove(&id) {
            let doc = view.stamp.doc_id;
            if !self.views.values().any(|v| v.stamp.doc_id == doc) {
                self.retired_documents.insert(doc);
            }
        }
    }
}
type ScopeTap = (
    pm_render::scopes::ScopeConfig,
    async_channel::Sender<ScopeResult>,
);
struct Shared {
    state: Mutex<State>,
    memory: Mutex<memory::CacheUsage>,
    /// Times a closed document's caches were released (`CacheUsage`).
    project_releases: std::sync::atomic::AtomicU64,
    wake: std::sync::mpsc::SyncSender<()>,
    interactive: jobs::Activity,
    /// The worker engine's current Metal device; a GPU loss replaces it.
    device: Mutex<metal::Device>,
    /// The viewer the Scopes panel analyzes, and where results go.
    scopes: Mutex<Option<(u64, ScopeTap)>>,
    disk: render_cache::DiskFrames,
    #[cfg(test)]
    before_submit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// Test record: each drawn frame's stamp, time and scale, in order.
    #[cfg(test)]
    drawn: Mutex<Vec<(Stamp, Ticks, f64)>>,
    #[cfg(test)]
    lose_device: std::sync::atomic::AtomicBool,
    /// Test hook: the worker panics once, as a bug in a frame would.
    #[cfg(test)]
    panic_once: std::sync::atomic::AtomicBool,
    /// Test hook: the next this many frames panic while rendering (a frame
    /// already holding its capture).
    #[cfg(test)]
    panic_frames: std::sync::atomic::AtomicU32,
    #[cfg(test)]
    invalid_call: std::sync::atomic::AtomicBool,
    /// Every frame costs more than its refresh (Auto resolution tests).
    #[cfg(test)]
    slow_frames: std::sync::atomic::AtomicBool,
}
impl Shared {
    /// The fonts changed: every viewer draws again, under a new stamp.
    fn fonts_changed(&self) {
        let mut state = self.state.lock().unwrap();
        for v in state.views.values_mut() {
            v.stamp.sequence += 1;
            v.dirty = true;
        }
        self.wake.try_send(()).ok();
    }
}
#[derive(Clone)]
pub struct Viewer {
    id: u64,
    shared: Arc<Shared>,
}
impl State {
    fn notify(&mut self) {
        self.subscribers.retain(|s| !s.is_closed());
        for s in &self.subscribers {
            let _ = s.try_send(());
        }
    }
}
impl Viewer {
    fn edit(&self, f: impl FnOnce(&mut View) -> Result<()>) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        let v = state
            .views
            .get_mut(&self.id)
            .ok_or_else(|| anyhow::anyhow!("viewer retired"))?;
        f(v)?;
        self.shared.interactive.set(true);
        self.shared.wake.try_send(()).ok();
        Ok(())
    }
    /// Immutable publication input, cheap COW tree clone. The snapshot is the
    /// authoritative fallback if the consumer missed any change events.
    #[cfg(feature = "kernel")]
    pub fn publish(&self, p: &Published) -> Result<()> {
        self.snapshot_traced(
            p.snapshot.clone(),
            Some((
                p.actor_work.as_secs_f64() * 1000.,
                p.publication_work.as_secs_f64() * 1000.,
            )),
        )
    }
    /// Attach an event origin before dispatching an asynchronous kernel edit.
    /// Keep it pending until a new revision; playback of the old revision
    /// must not consume the edit's measurement token. The oldest pending
    /// input wins: the next revision's frame is the first to show it.
    pub fn input_event(&self, input: f64) {
        if let Some(v) = self.shared.state.lock().unwrap().views.get_mut(&self.id) {
            v.pending_input.get_or_insert(input);
        }
    }
    pub fn snapshot(&self, snapshot: Snapshot) -> Result<()> {
        self.snapshot_traced(snapshot, None)
    }
    fn snapshot_traced(&self, snapshot: Snapshot, spans: Option<(f64, f64)>) -> Result<()> {
        self.edit(|v| {
            ensure!(
                snapshot.doc_id == v.stamp.doc_id,
                "open a new viewer for a new document session"
            );
            ensure!(
                snapshot.revision >= v.stamp.revision,
                "stale kernel publication"
            );
            if snapshot.revision > v.stamp.revision {
                v.input = v.pending_input.take();
                let (actor_ms, publication_ms) = spans.unwrap_or_default();
                v.pipeline = trace::Pipeline {
                    request: trace::now(),
                    actor_ms,
                    publication_ms,
                };
                v.config.snapshot = snapshot;
                v.stamp.revision = v.config.snapshot.revision;
                v.stamp.sequence += 1;
                // Paused gizmo geometry names one revision and time; safe
                // areas and guides are time-invariant and persist across edits.
                if v.overlay_time.take().is_some() {
                    v.overlays = OverlayGeometry::default();
                }
                v.dirty = true;
            }
            Ok(())
        })
    }
    /// Relink/asset resolver changes retire cached pixels even at one revision.
    pub fn media(&self, media: MediaCatalog) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        state.content += 1;
        let content = state.content;
        let v = state
            .views
            .get_mut(&self.id)
            .ok_or_else(|| anyhow::anyhow!("viewer retired"))?;
        (v.paths, v.paused_paths) = viewer_media(&media, v.media_mode);
        v.status.proxied = media.proxied(v.media_mode);
        v.config.media = media;
        v.content = content;
        v.stamp.sequence += 1;
        v.dirty = true;
        self.shared.wake.try_send(()).ok();
        Ok(())
    }
    /// Which file each clip plays from (View › Media). Audio follows it.
    pub fn media_mode(&self, mode: MediaMode) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        state.content += 1;
        let content = state.content;
        let v = state
            .views
            .get_mut(&self.id)
            .ok_or_else(|| anyhow::anyhow!("viewer retired"))?;
        if v.media_mode == mode {
            return Ok(());
        }
        v.media_mode = mode;
        (v.paths, v.paused_paths) = viewer_media(&v.config.media, mode);
        v.status.proxied = v.config.media.proxied(mode);
        v.content = content;
        v.stamp.sequence += 1;
        v.dirty = true;
        self.shared.wake.try_send(()).ok();
        Ok(())
    }
    /// Render resolution. Auto measures playback; stopped frames are full.
    pub fn quality(&self, quality: Quality) -> Result<()> {
        self.edit(|v| {
            if v.quality != quality {
                v.quality = quality;
                v.stamp.sequence += 1;
                v.dirty = true;
            }
            Ok(())
        })
    }
    /// Auto draws this viewer's dragged frames at reduced resolution while
    /// the project is slower than a refresh, and the frame exact once the
    /// playhead stops (`policy::SCRUB_SETTLE_SECONDS`): the editor's viewer.
    pub fn adapt_drags(&self, on: bool) -> Result<()> {
        self.edit(|v| {
            v.adapt_drags = on;
            Ok(())
        })
    }
    /// The skimmer: while paused the viewer shows `time` (on the drag path:
    /// reduced while it moves, exact once it rests) without moving the
    /// playhead; `None` returns to the playhead's frame. `input` is the
    /// pointer event's host time; the pointer's speed sets the skim
    /// audio's direction and speed.
    pub fn skim(&self, time: Option<Ticks>, input: f64) -> Result<()> {
        ensure!(input.is_finite() && input > 0., "invalid input timestamp");
        self.edit(|v| {
            if v.skim == time {
                return Ok(());
            }
            v.skim_rate = match (v.skim, time, v.skim_input) {
                (Some(a), Some(b), Some(at)) if input > at => skim_rate(
                    (b.get() - a.get()) as f64 / pm_protocol::common::TICKS_PER_SECOND as f64,
                    input - at,
                ),
                _ => pm_protocol::common::PlaybackRate::NORMAL,
            };
            v.skim = time;
            v.skim_input = Some(input);
            v.skim_moved = true;
            v.input = Some(input);
            v.pipeline.request = trace::now();
            v.stamp.sequence += 1;
            v.dirty = true;
            Ok(())
        })
    }
    /// Audio skimming: grains of the mix follow the skimmer.
    pub fn skim_audio(&self, on: bool) -> Result<()> {
        self.edit(|v| {
            v.skim_audio = Some(on);
            Ok(())
        })
    }
    /// Interactive scrubbing pauses and immediately requests the exact frame.
    pub fn scrub(&self, time: Ticks) -> Result<()> {
        self.scrub_at(time, trace::now())
    }
    /// NSEvent.timestamp in monotonic host seconds; preserve event origin.
    pub fn scrub_at(&self, time: Ticks, input: f64) -> Result<()> {
        ensure!(input.is_finite() && input > 0., "invalid input timestamp");
        self.edit(|v| {
            v.input = Some(input);
            v.pipeline.request = trace::now();
            // Replace an unconsumed scrub pair atomically, even under a burst
            // of HID input. A pause can never be accepted without its seek.
            if matches!(
                v.commands.back(),
                Some(TransportCommand::Seek {
                    mode: pm_protocol::clock::SeekMode::Scrub,
                    ..
                })
            ) && v.commands.len() >= 2
                && v.commands[v.commands.len() - 2] == TransportCommand::Pause
            {
                v.commands.pop_back();
                v.commands.pop_back();
            }
            ensure!(v.commands.len() <= 62, "transport queue full");
            v.enqueue(TransportCommand::Pause)?;
            v.enqueue(TransportCommand::Seek {
                time,
                mode: pm_protocol::clock::SeekMode::Scrub,
            })
        })
    }
    pub fn configure(&self, output: Output) -> Result<()> {
        output.validate()?;
        self.edit(|v| {
            let previous = v.config.output;
            v.config.output = output;
            // Headroom is display telemetry. The shader preserves extended
            // values and WindowServer adapts them; it does not change pixels.
            if output.width != previous.width
                || output.height != previous.height
                || output.display != previous.display
                || output.crop != previous.crop
            {
                v.stamp.generation += 1;
                v.stamp.sequence += 1;
                v.dirty = true;
            }
            Ok(())
        })
    }
    /// Geometry must name the revision AND exact media time it describes.
    /// Animated gizmos need new geometry per frame or engine-side evaluation.
    pub fn overlays(&self, stamp: Stamp, time: Ticks, geometry: OverlayGeometry) -> Result<()> {
        self.edit(|v| {
            ensure!(v.stamp == stamp, "stale overlay revision");
            ensure!(
                v.status
                    .clock
                    .as_ref()
                    .is_some_and(|c| c.media_time == time && c.rate.num == 0),
                "moving overlays require engine evaluation; static geometry is paused only"
            );
            v.overlays = geometry;
            v.overlay_time = Some(time);
            v.locked_provider = None;
            v.stamp.sequence += 1;
            v.dirty = true;
            Ok(())
        })
    }
    pub fn locked_overlays(&self, provider: LockedOverlayProvider) -> Result<()> {
        self.edit(|v| {
            v.locked_provider = Some(provider);
            v.stamp.sequence += 1;
            v.dirty = true;
            Ok(())
        })
    }
    /// Safe areas are time-invariant and are drawn with every video frame.
    pub fn safe_areas(&self, geometry: OverlayGeometry) -> Result<()> {
        self.edit(|v| {
            v.overlays = geometry;
            v.overlay_time = None;
            v.locked_provider = None;
            v.stamp.sequence += 1;
            v.dirty = true;
            Ok(())
        })
    }
    pub fn command(&self, cmd: TransportCommand) -> Result<()> {
        use pm_protocol::validate::Validate;
        cmd.validate().map_err(|e| anyhow::anyhow!("{e:?}"))?;
        self.edit(|v| v.enqueue(cmd))
    }
    /// The Mixer's monitor solo: only these top-level layers are heard
    /// (empty: every strip). Meters still read every strip.
    pub fn monitor_solo(&self, solo: Vec<String>) -> Result<()> {
        ensure!(solo.len() <= 4096, "too many solo strips");
        self.edit(|v| {
            v.monitor = Some(solo);
            v.dirty = true;
            Ok(())
        })
    }
    pub fn shuttle(&self, key: transport::ShuttleKey, from_pause: bool) -> Result<bool> {
        let mut consumed = false;
        self.edit(|v| {
            if let Some(cmd) = transport::shuttle(key, v.desired_rate, from_pause) {
                v.enqueue(cmd)?;
                consumed = true;
            }
            Ok(())
        })?;
        Ok(consumed)
    }
    /// The host's CADisplayLink timestamp/interval, in monotonic host seconds.
    pub fn display_tick(&self, target: f64, interval: f64) -> Result<()> {
        ensure!(
            target.is_finite() && interval.is_finite() && (0.001..=0.1).contains(&interval),
            "invalid display phase"
        );
        self.edit(|v| {
            v.phase = Some((target, interval));
            Ok(())
        })
    }
    /// Generation pin seam. Nonempty runtime requirements fail until P2 exists.
    pub fn resources(&self, snapshot: Arc<resources::ResourceSnapshot>) -> Result<()> {
        snapshot.validate()?;
        let mut state = self.shared.state.lock().unwrap();
        state.content += 1;
        let content = state.content;
        let v = state
            .views
            .get_mut(&self.id)
            .ok_or_else(|| anyhow::anyhow!("viewer retired"))?;
        ensure!(
            snapshot.generation >= v.resources.generation,
            "stale resource generation"
        );
        if v.resources.generation != snapshot.generation {
            v.resources = snapshot;
            v.content = content;
            v.stamp.generation += 1;
            v.stamp.sequence += 1;
            v.dirty = true;
            self.shared.interactive.set(true);
            self.shared.wake.try_send(()).ok();
        }
        Ok(())
    }

    /// Main-thread only: change the layer label and shader policy atomically.
    /// Use explicit mastering peaks; dynamic headroom never edits authored light.
    pub fn mastering(&self, main: objc2::MainThreadMarker, mode: color::Mastering) -> Result<()> {
        mode.params()?;
        self.edit(|v| {
            if let Some(layer) = &v.config.layer {
                layers::mastering_space(main, layer, mode)?;
            }
            v.mastering = mode;
            v.stamp.generation += 1;
            v.stamp.sequence += 1;
            v.dirty = true;
            Ok(())
        })
    }
    /// Sends this viewer's scope analysis while `tap` is set. Results arrive
    /// off the main thread, at most ten a second.
    pub fn scopes(&self, tap: Option<ScopeTap>) -> Result<()> {
        *self.shared.scopes.lock().unwrap() = tap.map(|tap| (self.id, tap));
        self.edit(|v| {
            v.stamp.sequence += 1;
            v.dirty = true;
            Ok(())
        })
    }
    pub fn capture_next(&self) -> Result<()> {
        if let Some(why) = &self.shared.state.lock().unwrap().halted {
            anyhow::bail!("The viewer stopped after an internal error: {why}");
        }
        self.edit(|v| {
            v.capture = true;
            v.stamp.sequence += 1;
            v.dirty = true;
            Ok(())
        })
    }
    pub fn status(&self) -> ViewerStatus {
        self.shared
            .state
            .lock()
            .unwrap()
            .views
            .get(&self.id)
            .map(|v| v.status.clone())
            .unwrap_or_default()
    }
    pub fn identity(&self) -> u64 {
        self.id
    }
    pub fn stamp(&self) -> Option<Stamp> {
        self.shared
            .state
            .lock()
            .unwrap()
            .views
            .get(&self.id)
            .map(|v| v.stamp)
    }
    pub fn retire(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.remove_view(self.id);
        state.notify();
        drop(state);
        self.retired();
    }
    /// This view left `State::views`: its scope tap goes, the worker wakes.
    fn retired(&self) {
        let mut scopes = self.shared.scopes.lock().unwrap();
        if scopes.as_ref().is_some_and(|(id, _)| *id == self.id) {
            *scopes = None;
        }
        drop(scopes);
        self.shared.wake.try_send(()).ok();
    }
}

pub struct EngineService {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    compat: Arc<compat::CompatStores>,
    layer_rasters: Option<Arc<compat::LayerRasters>>,
}
impl EngineService {
    /// Start off the UI thread: renderer/shader initialization may take time.
    pub fn start(gpu: pm_gpu::Gpu) -> Result<Self> {
        let device = metal_device(&gpu);
        let engine = render::Engine::new(gpu)?;
        Self::start_engine(device, engine)
    }
    /// `shader_cache` is a folder kept across launches for translated shaders.
    pub fn start_with_fonts(
        gpu: pm_gpu::Gpu,
        fonts: pm_render::text::host::RenderFonts,
        shader_cache: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let device = metal_device(&gpu);
        let layer_rasters = compat::LayerRasters::start(Some(fonts.clone()))?;
        // Built-in definitions load first thing on the service thread
        // (`run`): the app's window doesn't wait for a cold shader cache.
        let mut engine = render::Engine::without_definitions(gpu, shader_cache)?;
        engine.set_fonts(fonts);
        engine.builtins_pending = true;
        let mut service = Self::start_engine(device, engine)?;
        service.layer_rasters = Some(layer_rasters);
        Ok(service)
    }
    /// The base rasters raster overrides start from (`render.raster`).
    pub fn layer_rasters(&self) -> Option<Arc<compat::LayerRasters>> {
        self.layer_rasters.clone()
    }
    /// A document's raster override answers (`compat::CompatStore`).
    pub fn compat(&self, doc_id: u64) -> Arc<compat::CompatStore> {
        self.compat.document(doc_id)
    }
    /// A document closed: its answers go.
    pub fn forget_compat(&self, doc_id: u64) {
        self.compat.forget(doc_id);
    }
    /// The renderer's latest cache counters (not OS or allocator footprint).
    pub fn memory_usage(&self) -> memory::CacheUsage {
        memory::CacheUsage {
            project_releases: self
                .shared
                .project_releases
                .load(std::sync::atomic::Ordering::Relaxed),
            ..self.shared.memory.lock().unwrap().clone()
        }
    }
    fn start_engine(device: metal::Device, mut engine: render::Engine) -> Result<Self> {
        let (wake, wakes) = std::sync::mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            memory: Mutex::default(),
            project_releases: Default::default(),
            wake,
            interactive: jobs::Activity::default(),
            device: Mutex::new(device),
            scopes: Mutex::new(None),
            disk: render_cache::DiskFrames::default(),
            #[cfg(test)]
            before_submit: Mutex::new(None),
            #[cfg(test)]
            drawn: Mutex::new(Vec::new()),
            #[cfg(test)]
            lose_device: Default::default(),
            #[cfg(test)]
            panic_once: Default::default(),
            #[cfg(test)]
            panic_frames: Default::default(),
            #[cfg(test)]
            invalid_call: Default::default(),
            #[cfg(test)]
            slow_frames: Default::default(),
        });
        // A font installed or removed while a project is open: every viewer
        // draws again with fonts of the new generation, and the frame its
        // engine observes for the render cache has a new font identity, so
        // frames rendered with the fonts before are not this state's.
        let fonts = Arc::downgrade(&shared);
        pm_text::system_fonts::on_change(move || {
            fonts.upgrade().is_some_and(|shared| {
                shared.fonts_changed();
                true
            })
        });
        // Answers that land redraw every viewer, as a font change does.
        let compat = Arc::new(compat::CompatStores::default());
        let redraw = Arc::downgrade(&shared);
        compat.set_redraw(Arc::new(move || {
            if let Some(shared) = redraw.upgrade() {
                let mut state = shared.state.lock().unwrap();
                for v in state.views.values_mut() {
                    v.dirty = true;
                }
                drop(state);
                shared.wake.try_send(()).ok();
            }
        }));
        engine.set_compat(Some(compat.clone()));
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("pm-engine-service".into())
            .spawn(move || supervise(engine, worker, wakes))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            compat,
            layer_rasters: None,
        })
    }
    /// Call after the main-thread font broker's generation changes.
    pub fn fonts_changed(&self) {
        self.shared.fonts_changed();
    }
    /// The disk render cache: Render Selection, Keep Rendered, Delete
    /// Rendered Frames and the timeline's render bar.
    pub fn render_cache(&self) -> render_cache::DiskFrames {
        self.shared.disk.clone()
    }
    /// Share with ExportHandle::start_scheduled; true defers new export bands.
    pub fn export_activity(&self) -> jobs::Activity {
        self.shared.interactive.clone()
    }
    /// Coalesced status edges. Read status() after every wake; idle is silent.
    pub fn notifications(&self) -> async_channel::Receiver<()> {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(());
        self.shared.state.lock().unwrap().subscribers.push(tx);
        rx
    }
    /// The device viewer layers and popouts must use. It changes after a
    /// GPU loss; the worker re-points layers it already draws into.
    pub fn metal_device(&self) -> metal::Device {
        self.shared.device.lock().unwrap().clone()
    }
    pub fn viewer(&self, config: ViewerConfig) -> Result<Viewer> {
        self.place(config, None)
    }
    /// `old`'s place shows `config` from now on (another composition of its
    /// document, or another document). The two change places in one step: a
    /// document that stays open is never without a view, so it keeps its
    /// caches, programs, override answers and background fill, and the worker
    /// keeps its device. On an error `old` is still in place.
    pub fn replace_viewer(&self, old: &Viewer, config: ViewerConfig) -> Result<Viewer> {
        let viewer = self.place(config, Some(old.id))?;
        old.retired();
        Ok(viewer)
    }
    fn place(&self, config: ViewerConfig, replaces: Option<u64>) -> Result<Viewer> {
        config.output.validate()?;
        ensure!(
            config.rate.exact_frame_ticks().is_some(),
            "frame rate must divide engine ticks exactly"
        );
        ensure!(
            config.snapshot.revision == config.snapshot.document.revision(),
            "snapshot revision mismatch"
        );
        if let Some(layer) = &config.layer {
            use foreign_types::ForeignTypeRef;
            let attached: *mut objc2::runtime::AnyObject = unsafe {
                objc2::msg_send![layer.as_ptr() as *mut objc2::runtime::AnyObject, device]
            };
            ensure!(
                attached.cast::<metal::MTLDevice>() == self.metal_device().as_ptr(),
                "viewer layer must use the engine Metal device"
            );
            ensure!(
                layer.pixel_format() == metal::MTLPixelFormat::RGBA16Float,
                "viewer layer must be RGBA16Float"
            );
        }
        let mut state = self.shared.state.lock().unwrap();
        let replaced = replaces.filter(|id| state.views.contains_key(id));
        ensure!(
            state.views.len() - usize::from(replaced.is_some()) < 8,
            "at most eight concurrent viewers"
        );
        state.next += 1;
        state.content += 1;
        let id = state.next;
        let content = state.content;
        let stamp = Stamp {
            generation: 1,
            doc_id: config.snapshot.doc_id,
            revision: config.snapshot.revision,
            sequence: 1,
        };
        let (paths, paused_paths) = viewer_media(&config.media, MediaMode::default());
        let proxied = config.media.proxied(MediaMode::default());
        state.views.insert(
            id,
            View {
                config,
                stamp,
                content,
                dirty: true,
                capture: false,
                capture_owed: false,
                looping: None,
                audio_scrub: false,
                overlays: OverlayGeometry::default(),
                overlay_time: None,
                locked_provider: None,
                commands: VecDeque::new(),
                commands_sent: None,
                monitor: None,
                desired_rate: pm_protocol::common::PlaybackRate::STOPPED,
                status: ViewerStatus {
                    proxied,
                    ..Default::default()
                },
                phase: None,
                input: None,
                pending_input: None,
                pipeline: Default::default(),
                mastering: Default::default(),
                resources: Arc::default(),
                quality: Quality::Auto,
                adapt_drags: false,
                skim: None,
                skim_input: None,
                skim_rate: pm_protocol::common::PlaybackRate::NORMAL,
                skim_moved: false,
                skim_audio: None,
                media_mode: MediaMode::default(),
                paths,
                paused_paths,
            },
        );
        // After the new view is in: the document they share stays open.
        if let Some(old) = replaced {
            state.remove_view(old);
        }
        state.notify();
        self.shared.wake.try_send(()).ok();
        Ok(Viewer {
            id,
            shared: self.shared.clone(),
        })
    }
}
impl Drop for EngineService {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().stop = true;
        self.shared.interactive.set(false);
        self.shared.wake.try_send(()).ok();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
struct Runtime {
    transport: transport::Transport,
    revision: u64,
    content: u64,
    interval: f64,
    next: f64,
    /// Observed presentedTime − requested present time: the WindowServer/
    /// compositor delay the picture cannot avoid. Media time is chosen for
    /// requested + this lag, so a frame lands when its audio plays.
    lag: present::PresentLag,
    /// Most recent known vsync (host s) and refresh interval.
    grid: Option<(f64, f64)>,
    /// Refreshes ahead of the target at which a playing frame is encoded.
    ahead: f64,
    /// Stamp/time already submitted. Repeating identical pixels at the
    /// display rate only queues more drawables behind the compositor.
    last: Option<(Stamp, Ticks)>,
    /// A paused picture that never reached the glass was re-presented once
    /// for this stamp (bounded: a hidden window does not loop).
    represented: Option<Stamp>,
    /// The last seek was a drag: paused video may stand in until it lands.
    scrub: bool,
    /// The last frame showed stand-ins: the landed counter it was drawn at.
    stand_in: Option<u64>,
    /// A dragged frame was drawn below full resolution: draw it exact once
    /// the playhead has stayed put until then (host s).
    refine_at: Option<f64>,
    /// A paused picture drawn at the drawable's size that still owes its
    /// composition-size refinement ([`FrameSpec::paused_full_scale`]).
    full_owed: Option<Stamp>,
    /// The fast picture was submitted: its refinement is drawn next, unless
    /// something newer is asked for first.
    full_due: bool,
    /// The picture the refinement is drawn for; any newer one (a seek, an
    /// edit, play) is drawn fast again.
    full_for: Option<Stamp>,
    /// What the last paused picture at the drawable's size took, worker to
    /// GPU completion (µs): a refinement costing about this × the scale
    /// squared within one band's budget draws whole.
    paused_us: Option<f64>,
    /// What refinement bands cost, for the document state they measured.
    refine_costs: Option<((u64, u64, u64), crate::jobs::Costs)>,
    /// A clock that stops on its own still owes the exact stopped frame.
    was_playing: bool,
    auto: policy::AutoQuality,
    /// A transient refusal: draw again no earlier than this.
    not_before: f64,
    /// Frames in a row the layer gave no drawable (`nextDrawable` returned
    /// nil). Each retry renders a whole frame, so they back off until one lands.
    drawable_misses: u32,
    /// Transient refusals in a row (fonts, GPU watchdog); retries back off
    /// the same way, so a frame that keeps failing never renders at full rate.
    refusals: u32,
    /// When the frames started being refused as transient, or drawn with
    /// override layers from an earlier frame, while they are.
    waiting_since: Option<f64>,
    /// When the capture asked for started waiting for its exact frame.
    capture_waiting: Option<f64>,
    prepared: f64,
    lookahead: Lookahead,
}
/// Where media was last prepared ahead of the playhead, and the frames a
/// video entering since then still needs prepared.
#[derive(Default)]
struct Lookahead {
    /// The last look-ahead time and the video draws planned there.
    last: Option<(Ticks, std::collections::HashSet<(String, u64)>)>,
    /// Frames between two look-aheads where a video entered: its first
    /// frame is among them, and the shared cache should hold it before the
    /// playhead meets it. One is prepared per frame drawn.
    refine: VecDeque<Ticks>,
}
impl Lookahead {
    /// Notes what the look-ahead at `at` planned. A video new since the last
    /// one entered between the two: queue every frame between them.
    fn planned(&mut self, at: Ticks, videos: Vec<(String, u64)>, frame: i64) {
        let videos: std::collections::HashSet<_> = videos.into_iter().collect();
        if let Some((last, before)) = &self.last {
            let (lo, hi) = (last.get().min(at.get()), last.get().max(at.get()));
            let near = hi - lo
                <= (2. * PREPARE_EVERY_SECONDS * pm_protocol::common::TICKS_PER_SECOND as f64)
                    as i64;
            if near && frame > 0 && !videos.is_subset(before) {
                let mut t = lo - lo.rem_euclid(frame) + frame;
                while t < hi && self.refine.len() < MAX_REFINE {
                    self.refine.extend(Ticks::new(t).ok());
                    t += frame;
                }
            }
        }
        self.last = Some((at, videos));
    }
}
/// Entry frames queued at most (a quarter second at 120 fps).
const MAX_REFINE: usize = 30;
#[cfg(test)]
mod lookahead_tests {
    use super::*;
    use pm_protocol::common::TICKS_PER_SECOND;

    fn at(seconds: f64) -> Ticks {
        Ticks::new((seconds * TICKS_PER_SECOND as f64) as i64).unwrap()
    }

    /// A video first planned at one look-ahead entered since the last: each
    /// frame between them is queued, so its first frame is warmed. Known
    /// videos, a jump, or the first look-ahead queue nothing.
    #[test]
    fn a_video_entering_between_look_aheads_queues_the_frames_between() {
        let frame = TICKS_PER_SECOND / 24;
        let clip = ("clip".to_owned(), 1);
        let mut l = Lookahead::default();
        l.planned(at(1.0), vec![], frame);
        assert!(l.refine.is_empty());
        l.planned(at(1.25), vec![clip.clone()], frame);
        let queued: Vec<i64> = l.refine.drain(..).map(|t| t.get()).collect();
        assert_eq!(
            queued.len(),
            5,
            "the frames strictly between 1.0 s and 1.25 s"
        );
        assert!(
            queued
                .iter()
                .all(|t| t % frame == 0 && *t > at(1.0).get() && *t < at(1.25).get())
        );
        l.planned(at(1.5), vec![clip.clone()], frame);
        assert!(l.refine.is_empty(), "already planned");
        l.planned(at(9.0), vec![clip, ("other".to_owned(), 1)], frame);
        assert!(
            l.refine.is_empty(),
            "a jump says nothing about where it entered"
        );
    }
}
struct Flight {
    id: u64,
    stamp: Stamp,
    spec: FrameSpec,
    submission: render::Submission,
    input: Option<f64>,
    audio_host: Option<f64>,
    pipeline: trace::Pipeline,
    /// A playing frame's index and key in the disk render cache: gaps
    /// between played frames are dropped frames (`DiskFrames::dropped`).
    played: Option<(i64, [u8; 32])>,
}
/// A jump of more frames than this (2 s at 24 fps) between played frames
/// is a seek, not a stall; a short seek taken for one only renders more.
const DROPS_AT_MOST: i64 = 48;
/// One frame taken from the shared state, drawn without its lock.
struct Work {
    id: u64,
    stamp: Stamp,
    scene: Scene,
    /// What rendered frames are drawn from: a paused frame's media (never
    /// Automatic's HD proxies), whatever this frame's moment reads.
    rendered_media: Arc<pm_export::render::MediaPaths>,
    spec: FrameSpec,
    layer: Option<metal::MetalLayer>,
    overlays: OverlayGeometry,
    provider: Option<LockedOverlayProvider>,
    capture: bool,
    due: Option<f64>,
    mastering: color::Mastering,
    resources: Arc<resources::ResourceSnapshot>,
    input: Option<f64>,
    pipeline: trace::Pipeline,
    audio_host: Option<f64>,
    plan: Option<present::Plan>,
    /// The composition's frame duration (0: not a whole number of ticks).
    frame_ticks: i64,
}
/// How much of a frame's budget a video may wait for its exact frame.
const DECODE_SHARE: f64 = 0.35;
/// Clips this far ahead of the playhead open on background workers.
/// Playback asks the extension realm for raster override layers ahead of the
/// playhead, one batch at a time: far enough ahead that the answers land
/// before their frames (the realm's measured time), as many frames per batch
/// as cost about a tenth of a second. A realm slower than real time answers
/// every few frames; the frames between draw the nearest answer. A frame
/// rendered on disk is read, not drawn: nothing is asked for it.
fn ask_ahead(
    engine: &mut render::Engine,
    disk: &render_cache::DiskFrames,
    scene: &Scene,
    spec: FrameSpec,
    rate: Option<pm_protocol::common::PlaybackRate>,
) {
    let Some(store) = engine.compat().cloned() else {
        return;
    };
    let Some(rate) = rate.filter(|r| r.num != 0) else {
        return;
    };
    let frame = store.frame_ticks();
    if frame <= 0 {
        return;
    }
    let speed = rate.num as f64 / rate.den.max(1) as f64;
    let Some((lead, frames)) = store.ahead(spec.time.get(), speed) else {
        return;
    };
    let step = frame * i64::from(rate.num.signum());
    // Land a frame early, on the frame grid.
    let ahead = lead.as_secs_f64() * speed * pm_protocol::common::TICKS_PER_SECOND as f64;
    let frames_ahead = (ahead / frame as f64).abs().ceil() as i64 + 1;
    let at = (spec.time.get() + frames_ahead * step).max(0);
    let Ok(at) = Ticks::new(at) else {
        return;
    };
    if disk.rendered(scene, at) {
        return;
    }
    let needs = engine.compat_needs(scene, at, spec);
    store.request_ahead(needs, at.get(), frames, step);
}

const PREPARE_AHEAD_SECONDS: f64 = 0.75;
const PREPARE_EVERY_SECONDS: f64 = 0.25;
/// A transient refusal (fonts, GPU watchdog) waits this long to try again.
const RETRY_SECONDS: f64 = 0.016;
/// One frame, doubling per miss, to at most a second.
fn drawable_retry(misses: u32) -> f64 {
    (RETRY_SECONDS * f64::from(1u32 << misses.min(6))).min(1.)
}
/// A frame refused as transient this long is reported while it is retried.
const WAITING_SHOWN_SECONDS: f64 = 3.0;
/// A capture waiting this long for its exact frame fails, saying why.
const CAPTURE_WAIT_SECONDS: f64 = 10.0;
/// The viewer's note while extension layers keep showing an earlier frame.
fn late_note(layers: usize) -> String {
    match layers {
        1 => "Still waiting: 1 extension layer shows an earlier frame.".into(),
        n => format!("Still waiting: {n} extension layers show an earlier frame."),
    }
}
/// A GPU lost again within this long of a rebuild backs off between these.
const LOSS_WINDOW: Duration = Duration::from_secs(10);
const LOSS_BACKOFF: (Duration, Duration) = (Duration::from_millis(500), Duration::from_secs(4));
/// The skim audio's J/K/L rate for a skimmer that moved `seconds` of media
/// in `elapsed` seconds: its direction, and 2x once it moves faster than
/// one and a half times real time.
fn skim_rate(seconds: f64, elapsed: f64) -> pm_protocol::common::PlaybackRate {
    let speed = seconds / elapsed;
    let num = if speed.abs() > 1.5 { 2 } else { 1 };
    pm_protocol::common::PlaybackRate {
        num: if seconds < 0. { -num } else { num },
        den: 1,
    }
}
/// Runs the worker; a panic in it (a bug a frame hit) must not leave every
/// viewer blank, or poison the state the UI thread reads. The worker starts
/// again on a fresh engine, as after a lost GPU, every view draws again, and
/// the viewer reports what happened (its error line and the Error Log). Three
/// panics within a minute stop it with that error showing.
fn supervise(engine: render::Engine, shared: Arc<Shared>, wakes: std::sync::mpsc::Receiver<()>) {
    let (fonts, compat, cache) = (
        engine.fonts.clone(),
        engine.compat.clone(),
        engine.shader_cache.clone(),
    );
    let mut engine = Some(engine);
    let mut recent = VecDeque::<std::time::Instant>::new();
    while let Some(current) = engine.take() {
        let turn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(current, shared.clone(), &wakes)
        }));
        let Err(panic) = turn else { return };
        shared.state.clear_poison();
        shared.device.clear_poison();
        shared.scopes.clear_poison();
        let detail = panic
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown cause".into());
        eprintln!("Viewer worker panicked: {detail}");
        let now = std::time::Instant::now();
        recent.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        recent.push_back(now);
        let next = (recent.len() < 3)
            .then(|| {
                pm_gpu::Gpu::new("pm-engine-service")
                    .map_err(|e| anyhow::anyhow!("{e}"))
                    .and_then(|gpu| render::Engine::with_shader_cache(gpu, cache.clone()))
            })
            .and_then(Result::ok);
        let mut state = shared.state.lock().unwrap();
        let restarted = next.is_some();
        if !restarted {
            state.halted = Some(detail.clone());
        }
        let now = trace::now();
        for v in state.views.values_mut() {
            v.status.error = Some(if restarted {
                format!("The viewer restarted after an internal error.\n{detail}")
            } else {
                format!("The viewer stopped after an internal error.\n{detail}")
            });
            v.status.restarts += u64::from(restarted);
            v.stamp.sequence += 1;
            v.dirty = true;
            if restarted {
                resume(v, now);
            } else if std::mem::take(&mut v.capture) | std::mem::take(&mut v.capture_owed) {
                v.status.capture_failed = Some(format!(
                    "The viewer stopped after an internal error before the frame was captured.\n{detail}"
                ));
            }
        }
        state.notify();
        if state.stop {
            return;
        }
        drop(state);
        if let Some(mut next) = next {
            if let Some(fonts) = &fonts {
                next.set_fonts(fonts.clone());
            }
            next.set_compat(compat.clone());
            *shared.device.lock().unwrap() = next.metal().to_owned();
            engine = Some(next);
        }
    }
}

/// What a restarted worker owes a view its old worker held: the transport
/// (time, play state and rate, loop, audio scrub; the new transport opens
/// its audio from the view's config) ahead of any command still queued, and
/// a capture whose pixels never landed.
fn resume(v: &mut View, now: f64) {
    v.capture |= v.capture_owed;
    let Some(clock) = v.status.clock else {
        return;
    };
    let at = pm_protocol::common::HostMicros::new((now * 1e6) as u64)
        .unwrap_or(pm_protocol::common::HostMicros::ZERO);
    let mut restore = vec![
        TransportCommand::SetLoop { range: v.looping },
        TransportCommand::AudioScrub {
            enabled: v.audio_scrub,
        },
        TransportCommand::Seek {
            time: clock.media_time_at(at),
            mode: pm_protocol::clock::SeekMode::Exact,
        },
    ];
    // The rate the old transport was running or starting at.
    if v.desired_rate.num != 0 {
        restore.push(TransportCommand::Play {
            rate: v.desired_rate,
        });
    }
    for cmd in restore.into_iter().rev() {
        v.commands.push_front(cmd);
    }
}

/// An autorelease pool for one pass of a long-lived thread's loop, popped when
/// the pass ends however it ends (`continue`, `break`).
struct LoopPool(*mut std::ffi::c_void);
impl LoopPool {
    fn push() -> Self {
        unsafe extern "C" {
            fn objc_autoreleasePoolPush() -> *mut std::ffi::c_void;
        }
        // SAFETY: paired with the pop in Drop, on this thread, in LIFO order
        // (pools pushed during the pass are popped within it).
        Self(unsafe { objc_autoreleasePoolPush() })
    }
}
impl Drop for LoopPool {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn objc_autoreleasePoolPop(pool: *mut std::ffi::c_void);
        }
        // SAFETY: the token from `push` on this thread.
        unsafe { objc_autoreleasePoolPop(self.0) }
    }
}

fn run(mut engine: render::Engine, shared: Arc<Shared>, wakes: &std::sync::mpsc::Receiver<()>) {
    // Same interactive class as pm-node/present; export remains separately
    // governed. A default-class worker can miss display deadlines under load.
    objc2::rc::autoreleasepool(|_| {
        objc2_foundation::NSThread::currentThread()
            .setQualityOfService(objc2_foundation::NSQualityOfService::UserInteractive);
    });
    if std::mem::take(&mut engine.builtins_pending)
        && let Err(error) = engine.load_builtin_definitions()
    {
        // Frames still draw; effects report the failure where they're used.
        eprintln!("Built-in effect definitions failed to load: {error:#}");
        engine.definition_failure = Some(error.to_string());
    }
    engine.set_waker(Some(shared.wake.clone()));
    engine.disk = Some(shared.disk.clone());
    shared.disk.set_wake(shared.wake.clone());
    shared.disk.set_activity(shared.interactive.clone());
    let mut runtime = BTreeMap::<u64, Runtime>::new();
    let mut flights = VecDeque::<Flight>::new();
    // Each view's last played frame (disk render cache index).
    let mut played: std::collections::HashMap<u64, i64> = Default::default();
    let trace_path = trace::path().map(std::ffi::OsStr::to_os_string);
    let recorder = trace_path.and_then(|path| match trace::Recorder::open(path) {
        Ok(recorder) => Some(recorder),
        Err(error) => {
            eprintln!("Viewer diagnostic trace unavailable: {error}");
            None
        }
    });
    // Diagnostic A/B only: the pre-latency-pass schedule (re-present every
    // tick, picture for the target vsync, unaligned audio start).
    let legacy = std::env::var("PM_ENGINE_PRESENT").as_deref() == Ok("legacy");
    let mut cursor = 0;
    let (mut rebuilt_at, mut backoff) = (None::<std::time::Instant>, Duration::ZERO);
    loop {
        // Objects Metal and Core Animation autorelease on this thread outside
        // a frame's own pool (a finished flight returning its drawable to the
        // layer, device lookups after a rebuild) are drained every pass. This
        // thread's implicit pool is never drained: it kept every closed
        // project's viewer layer, and with it the layer's drawable pool.
        let _pool = LoopPool::push();
        #[cfg(test)]
        if shared
            .lose_device
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            engine.destroy_device();
        }
        #[cfg(test)]
        if shared
            .invalid_call
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            engine.raise_validation_error();
        }
        #[cfg(test)]
        if shared
            .panic_once
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            panic!("injected viewer failure");
        }
        // Delivers a pending device-lost callback even while idle.
        engine.govern_memory();
        engine.poll();
        *shared.memory.lock().unwrap() = engine.memory_usage();
        if engine.lost() {
            // Work on the lost device never completes: forget it, rebuild,
            // and draw every viewer again on the new device. A device lost
            // again soon after a rebuild waits longer each time, and says so.
            flights.clear();
            backoff = if rebuilt_at.is_some_and(|t| t.elapsed() < LOSS_WINDOW) {
                (backoff * 2).clamp(LOSS_BACKOFF.0, LOSS_BACKOFF.1)
            } else {
                Duration::ZERO
            };
            if !backoff.is_zero() {
                let mut state = shared.state.lock().unwrap();
                for v in state.views.values_mut() {
                    v.status.error = Some(format!(
                        "The GPU keeps failing. Trying again in {} s.",
                        backoff.as_secs_f64()
                    ));
                }
                state.notify();
                drop(state);
                // Wakes do not cut the wait short; quitting does.
                let until = std::time::Instant::now() + backoff;
                while let Some(left) = until.checked_duration_since(std::time::Instant::now())
                    && !shared.state.lock().unwrap().stop
                {
                    let _ = wakes.recv_timeout(left);
                }
            }
            let rebuilt = engine.rebuild();
            let mut state = shared.state.lock().unwrap();
            if state.stop {
                break;
            }
            match rebuilt {
                Ok(next) => {
                    engine = next;
                    rebuilt_at = Some(std::time::Instant::now());
                    *shared.device.lock().unwrap() = engine.metal().to_owned();
                    for (id, v) in state.views.iter_mut() {
                        v.status.device_losses += 1;
                        v.status.error = None;
                        v.stamp.sequence += 1;
                        v.dirty = true;
                        if let Some(r) = runtime.get_mut(id) {
                            r.last = None;
                        }
                    }
                }
                Err(e) => {
                    for v in state.views.values_mut() {
                        v.status.error = Some(format!("The GPU was lost and is not back yet: {e}"));
                    }
                }
            }
            state.notify();
            drop(state);
            if engine.lost() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        }
        let mut state = shared.state.lock().unwrap();
        // Completion is distinct from display. Never synthesize presentedTime.
        let mut i = 0;
        while i < flights.len() {
            // Retired once its completion handler has recorded the outcome:
            // Metal reports the status before that handler runs on its own
            // queue, and a frame read in between looked failed (gpu_ok still
            // false) and was never drawn again.
            if !matches!(
                flights[i].submission.command.status(),
                metal::MTLCommandBufferStatus::Completed | metal::MTLCommandBufferStatus::Error
            ) || flights[i].submission.trace.lock().unwrap().completed == 0.
            {
                i += 1;
                continue;
            }
            if flights[i].submission.drawable.is_some() {
                let trace = flights[i].submission.trace.lock().unwrap();
                if trace.display.is_none() && trace::now() - trace.commit < 0.25 {
                    i += 1;
                    continue;
                }
            }
            let f = flights.remove(i).unwrap();
            if f.submission.command.status() == metal::MTLCommandBufferStatus::Error {
                engine.clear_cache();
            }
            let mut represent = false;
            if let Some(r) = runtime.get_mut(&f.id) {
                let t = f.submission.trace.lock().unwrap();
                match t.display.filter(|d| *d > 0.) {
                    Some(display) => {
                        r.grid = Some((display, r.interval));
                        // Timed (playing) presents only; legacy requests the
                        // target itself and never reads the estimate.
                        if t.requested > 0. {
                            r.lag.record(t.requested, display, r.interval);
                        }
                    }
                    // Never shown on a layer: the screen may not hold it.
                    None => {
                        if f.submission.drawable.is_some() && r.last == Some((f.stamp, f.spec.time))
                        {
                            r.last = None;
                        }
                        // A paused (untimed) present dropped during window
                        // activation/reordering leaves an older picture on
                        // screen; nothing else repaints while paused.
                        if f.submission.drawable.is_some()
                            && t.target == 0.
                            && t.gpu_ok
                            && r.represented != Some(f.stamp)
                        {
                            r.represented = Some(f.stamp);
                            represent = true;
                        }
                    }
                }
                if !t.gpu_ok && r.last == Some((f.stamp, f.spec.time)) {
                    r.last = None;
                }
                if t.gpu_ok && !t.cache_hit && f.spec.moment != Moment::Playing {
                    let cost = (t.completed - t.worker_start) * 1000.;
                    #[cfg(test)]
                    let cost = if shared
                        .slow_frames
                        .load(std::sync::atomic::Ordering::Acquire)
                    {
                        f64::MAX
                    } else {
                        cost
                    };
                    r.auto.record_still(cost, r.interval * 1000., f.spec.scale);
                    if f.spec.moment == Moment::Paused && f.spec.scale == 1. {
                        r.paused_us = Some((t.completed - t.worker_start) * 1e6);
                    }
                }
                // Frames skipped between two played ones at normal speed
                // didn't play in real time: background rendering renders
                // them.
                match f.played {
                    Some((n, id))
                        if r.transport.rate() == pm_protocol::common::PlaybackRate::NORMAL =>
                    {
                        if let Some(before) = played.insert(f.id, n)
                            && n > before + 1
                            && n - before <= DROPS_AT_MOST
                        {
                            shared.disk.dropped(before, (n, id));
                        }
                    }
                    _ => {
                        played.remove(&f.id);
                    }
                }
            }
            if let Some(v) = state.views.get_mut(&f.id) {
                let t = f.submission.trace.lock().unwrap();
                if let Some(recorder) = &recorder && !recorder.record(serde_json::json!({"display":t.display,"worker":t.worker_start,"completed":t.completed,"commit":t.commit,
                        "input":f.input,"audio_host":f.audio_host,"time":f.spec.time.get(),"scale":f.spec.scale,"moment":format!("{:?}",f.spec.moment),"disk":t.disk,"disk_wait_ms":t.disk_wait_ms,"enter_ms":t.enter_ms,"disk_upload_ms":t.disk_upload_ms,"cache_hit":t.cache_hit,
                        "stale":v.stamp != f.stamp,"viewer_id":f.id,"generation":f.stamp.generation,"revision":f.stamp.revision,"sequence":f.stamp.sequence,"gpu_ok":t.gpu_ok,
                        "request":f.pipeline.request,"actor_ms":f.pipeline.actor_ms,"publication_ms":f.pipeline.publication_ms,
                        "render_start":t.render_start,"render_submitted":t.render_submitted,"render_cpu_ms":t.render_cpu_ms,"work":serde_json::from_str::<serde_json::Value>(&t.work.to_json()).unwrap_or_default(),"drawable_asked":t.drawable_asked,"drawable_acquired":t.drawable_acquired,
                        "encode_end":t.encode_end,"scheduled":t.scheduled,"gpu_start":t.gpu_start,"gpu_done":t.gpu_done,"callback":t.display_callback,"target":t.target,"requested":t.requested,"predicted":t.predicted})) {
                    v.status.error = Some("viewer diagnostic trace overflow/disconnected".into());
                }
                if v.stamp != f.stamp {
                    v.status.stale_rejections += 1;
                    // Its readback is for a stamp no one shows: ask again.
                    v.capture |= f.submission.readback.is_some();
                    continue;
                }
                if represent && v.commands.is_empty() {
                    // Same stamp: an exact-cache re-present, not new content.
                    v.dirty = true;
                    v.status.represented += 1;
                }
                v.status.layer_failures = f.submission.failures.to_vec();
                v.status.presented += 1;
                // A captured frame's pixels stay with its stamp and time: a
                // re-present of the same picture (no readback) keeps them.
                let kept = v
                    .status
                    .frame
                    .take()
                    .filter(|r| r.stamp == f.stamp && r.time == f.spec.time)
                    .and_then(|r| r.pixels);
                // Captured pixels are delivered only from an exact frame: a
                // paused, full-resolution picture with no stand-in and no
                // disk frame. Anything else is asked again.
                let readback = f.submission.readback.as_ref().filter(|_| {
                    let exact = f.spec.exact
                        && f.spec.moment == Moment::Paused
                        && f.spec.scale >= 1.
                        && !f.submission.stand_ins
                        && t.disk != 2;
                    if !exact {
                        v.capture = true;
                        v.dirty = true;
                    }
                    exact
                });
                v.status.frame = Some(FrameReport {
                    stamp: f.stamp,
                    time: f.spec.time,
                    moment: f.spec.moment,
                    scale: f.spec.scale,
                    stand_ins: f.submission.stand_ins,
                    cache_hit: t.cache_hit,
                    gpu_ok: f.submission.command.status()
                        == metal::MTLCommandBufferStatus::Completed,
                    display_host_seconds: t.display.filter(|t| *t > 0.),
                    completion_host_seconds: (t.completed > 0.).then_some(t.completed),
                    worker_to_completion_ms: (t.completed > 0.)
                        .then_some((t.completed - t.worker_start) * 1000.),
                    input_host_seconds: f.input,
                    audio_host_seconds: f.audio_host,
                    pixels: readback.map(|_| f.submission.pixels()).or(kept),
                });
                if readback.is_some() {
                    v.capture_owed = false;
                }
                // The fast paused picture is reported: its refinement, a
                // new picture of the same time, is drawn next. (Asked for
                // at submission, the new stamp made this report stale and
                // the fast picture was never reported.)
                // A picture read from disk is the composition-size frame
                // already: nothing to refine.
                if let Some(r) = runtime.get_mut(&f.id)
                    && r.full_owed == Some(f.stamp)
                {
                    r.full_owed = None;
                    r.full_due = t.disk != 2;
                }
            }
            state.notify();
        }
        if state.stop {
            break;
        }
        let retired: Vec<_> = runtime
            .keys()
            .filter(|id| !state.views.contains_key(id))
            .copied()
            .collect();
        let closed: Vec<_> = std::mem::take(&mut state.retired_documents)
            .into_iter()
            .filter(|doc| !state.views.values().any(|v| v.stamp.doc_id == *doc))
            .collect();
        if !retired.is_empty() || !closed.is_empty() {
            let last_view_closed = state.views.is_empty();
            if !closed.is_empty() {
                for v in state.views.values_mut() {
                    v.dirty = true;
                }
            }
            drop(state);
            for id in retired {
                runtime.remove(&id);
            }
            if !closed.is_empty() {
                for doc in closed {
                    if let Some(compat) = &engine.compat {
                        compat.forget(doc);
                    }
                    shared.disk.forget_document(doc);
                }
                engine.release_project_memory();
                shared
                    .project_releases
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if last_view_closed {
                    // Metal command queues and wgpu encoder pools retain
                    // their playback high-water storage after caches empty.
                    // No viewer uses this device now; retire those owners as
                    // well. The ordinary rebuild path preserves fonts/cache
                    // configuration and recreates programs on next use.
                    match engine.rebuild() {
                        Ok(next) => {
                            engine = next;
                            *shared.device.lock().unwrap() = engine.metal().to_owned();
                        }
                        Err(error) => {
                            eprintln!("Viewer cleanup could not renew the idle device: {error:#}")
                        }
                    }
                    crate::memory::pressure_relief();
                }
                *shared.memory.lock().unwrap() = engine.memory_usage();
            }
            continue;
        }
        // A new revision rebuilds the audio graph input, which walks the whole
        // document (O(layers)). Do it without the state lock: the UI thread
        // takes that lock to publish, configure and read status.
        let stale: Vec<_> = state
            .views
            .iter()
            .filter_map(|(id, v)| {
                let r = runtime.get(id)?;
                (r.revision != v.stamp.revision || r.content != v.content).then(|| {
                    (
                        *id,
                        v.config.snapshot.clone(),
                        v.config.media.audio_paths(v.media_mode),
                        v.config.composition.clone(),
                        v.stamp.revision,
                        v.content,
                    )
                })
            })
            .collect();
        if !stale.is_empty() {
            drop(state);
            let mut failures = Vec::new();
            for (id, snapshot, media, composition, revision, content) in stale {
                if let Some(r) = runtime.get_mut(&id) {
                    if let Err(e) = r.transport.update(&snapshot, media, &composition) {
                        failures.push((id, e.to_string()));
                    }
                    r.revision = revision;
                    r.content = content;
                }
            }
            // Fall through (no `continue`): a continuous stream of publications
            // must not starve rendering. A revision that arrived meanwhile is
            // still caught by the in-loop check below, as before.
            state = shared.state.lock().unwrap();
            if state.stop {
                break;
            }
            for (id, error) in failures {
                if let Some(v) = state.views.get_mut(&id) {
                    v.status.error = Some(error);
                }
            }
        }
        let now = trace::now();
        let landed = engine.media_landed();
        let mut work = None;
        let mut changed = false;
        let mut ids: Vec<_> = state.views.keys().copied().collect();
        ids.sort_by_key(|id| (*id <= cursor, *id));
        for id in ids {
            let v = state.views.get_mut(&id).unwrap();
            let r = runtime.entry(id).or_insert_with(|| {
                let mut transport =
                    transport::Transport::new(v.config.rate.exact_frame_ticks().unwrap());
                transport.clock.set_end(v.config.end);
                if v.config.audio {
                    let levels_shared = shared.clone();
                    let levels = Box::new(move |l| {
                        if let Some(v) = levels_shared.state.lock().unwrap().views.get_mut(&id) {
                            v.status.levels = Some(l);
                        }
                    });
                    // No device or decoder is opened while holding state. The
                    // AudioLink constructor only spawns its owner thread.
                    if let Err(e) = transport.enable_audio(
                        &v.config.snapshot.document,
                        v.config.composition.clone(),
                        v.config.media.audio_paths(v.media_mode),
                        levels,
                    ) {
                        v.status.error = Some(e.to_string());
                    }
                }
                Runtime {
                    transport,
                    revision: v.stamp.revision,
                    content: v.content,
                    interval: 1. / 60.,
                    next: now,
                    lag: present::PresentLag::default(),
                    grid: None,
                    last: None,
                    represented: None,
                    ahead: 1.,
                    scrub: false,
                    stand_in: None,
                    refine_at: None,
                    full_owed: None,
                    full_due: false,
                    full_for: None,
                    paused_us: None,
                    refine_costs: None,
                    was_playing: false,
                    auto: Default::default(),
                    not_before: 0.,
                    drawable_misses: 0,
                    refusals: 0,
                    waiting_since: None,
                    capture_waiting: None,
                    prepared: 0.,
                    lookahead: Lookahead::default(),
                }
            });
            if r.revision != v.stamp.revision || r.content != v.content {
                if let Err(e) = r.transport.update(
                    &v.config.snapshot,
                    v.config.media.audio_paths(v.media_mode),
                    &v.config.composition,
                ) {
                    v.status.error = Some(e.to_string());
                }
                r.revision = v.stamp.revision;
                r.content = v.content;
            }
            changed |= !v.commands.is_empty() || v.status.clock.is_none();
            if let Some((target, interval)) = v.phase {
                r.grid = Some((target, interval));
            }
            // Audio starts are placed on this viewer's last known vsync line.
            r.transport.grid = r.grid.filter(|_| !legacy);
            if let Some(sent) = v.commands_sent.take()
                && let Some(recorder) = &recorder
            {
                recorder.record(serde_json::json!({
                    "commands": v.commands.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>(),
                    "sent": sent, "drained": now, "viewer_id": id,
                }));
            }
            while let Some(cmd) = v.commands.pop_front() {
                // Any command (a seek, play) drops a pending refinement.
                (r.full_owed, r.full_due, r.full_for) = (None, false, None);
                if let TransportCommand::Seek { mode, .. } = cmd {
                    r.scrub = mode == pm_protocol::clock::SeekMode::Scrub;
                    r.refine_at = None;
                }
                r.transport.command(cmd, (now * 1e6) as i64);
            }
            if let Some(solo) = v.monitor.take() {
                r.transport.monitor(solo);
            }
            if std::mem::take(&mut v.skim_moved) {
                // A skim draws on the drag path; leaving returns exact.
                r.scrub = v.skim.is_some();
                r.refine_at = None;
                (r.full_owed, r.full_due, r.full_for) = (None, false, None);
                let frame_ms = v.config.rate.exact_frame_ticks().map_or(0, |t| {
                    (t as f64 * 1000. / pm_protocol::common::TICKS_PER_SECOND as f64).ceil() as u64
                });
                if let Some(a) = &r.transport.audio {
                    a.send(audio::AudioCtl::Skim {
                        at: v.skim.map(Ticks::get),
                        rate: v.skim_rate,
                        grain_ms: frame_ms.max(pm_audio::transport::GRAIN_MIN_MS as u64),
                    });
                }
            }
            if let (Some(on), Some(a)) = (v.skim_audio.take(), &r.transport.audio) {
                a.send(audio::AudioCtl::SkimAudio(on));
            }
            let sample = r.transport.poll((now * 1e6) as i64);
            changed |= v
                .status
                .clock
                .as_ref()
                .is_none_or(|old| old.rate != sample.rate || old.state != sample.state);
            v.status.clock = Some(sample);
            v.desired_rate = r.transport.rate();
            v.status.audio_fallback = r.transport.fallback;
            changed |= v.status.wants_display != r.transport.running();
            v.status.wants_display = r.transport.running();
            if let Some((_, interval)) = v.phase {
                r.interval = interval;
            }
            let interval = r.interval;
            if let Some((phase, _)) = v.phase.take()
                && phase > now
            {
                // A late/repeated link tick must never rewind a submitted target.
                r.next = r.next.max(phase);
            }
            let due = if r.transport.clock.is_playing() {
                let mut due = r.next;
                if due < now {
                    let skipped = ((now - due) / interval).floor() as u64 + 1;
                    v.status.skipped_targets += skipped;
                    due += skipped as f64 * interval;
                }
                Some(due)
            } else {
                None
            };
            if flights.len() >= 3 || flights.iter().filter(|f| f.id == id).count() >= 3 {
                continue;
            }
            // Encode two refreshes ahead while playing: WindowServer misses
            // the vsync when GPU completion lands within ~2 ms of target.
            // Only when content changes at most every other refresh: each
            // drawable is then held about one refresh longer, which three
            // drawables absorb at half rate but not at the full refresh rate.
            let speed = sample.rate.num.unsigned_abs() as f64 / sample.rate.den.max(1) as f64;
            let frame = v.config.rate.exact_frame_ticks().unwrap_or(1) as f64
                / pm_protocol::common::TICKS_PER_SECOND as f64
                / speed.max(1e-6);
            r.ahead = if !legacy && frame >= 1.99 * interval {
                2.
            } else {
                1.
            };
            let playing = r.transport.clock.is_playing();
            // The exact frame of a stop, and of a stand-in whose decode landed.
            if std::mem::replace(&mut r.was_playing, playing) && !playing {
                r.scrub = false;
                v.dirty = true;
            }
            if r.stand_in.is_some_and(|at| at != landed) {
                r.stand_in = None;
                v.dirty = !playing || v.dirty;
            }
            // The playhead stopped on a frame dragged at reduced resolution.
            // A new picture of the same time: its own `frame:rendered`.
            if !playing && r.refine_at.is_some_and(|at| now >= at) {
                r.refine_at = None;
                r.scrub = false;
                v.stamp.sequence += 1;
                v.dirty = true;
            }
            // The paused picture landed at the drawable's size: draw it again
            // at the composition's size, a new picture of the same time.
            if !r.transport.running() && std::mem::take(&mut r.full_due) {
                v.stamp.sequence += 1;
                v.dirty = true;
                r.full_for = Some(v.stamp);
            }
            if !v.dirty && (!playing || now < r.next - r.ahead * interval) {
                continue;
            }
            if now < r.not_before {
                continue;
            }
            // A timed present reaches glass `lag` after its request (measured
            // from presentedTime). Pick the picture for that instant, and for
            // forward play show each frame from the vsync nearest its onset,
            // so picture onsets straddle their audio instead of trailing it.
            // The request itself never depends on the estimate (see present).
            let plan = due.map(|due| {
                if legacy {
                    present::Plan {
                        requested: due,
                        shown: due,
                    }
                } else {
                    present::plan(due, interval, &r.lag, now)
                }
            });
            let shown = plan.map(|p| p.shown);
            let media = if let Some(shown) = shown {
                let at = |s: f64| {
                    sample.media_time_at(
                        pm_protocol::common::HostMicros::new((s.max(0.) * 1e6) as u64).unwrap(),
                    )
                };
                let plain = at(shown);
                let lead = at(shown + interval / 2.);
                // A lead that wrapped to the loop start is the correct frame.
                let forward = sample.rate.num > 0;
                let inside = lead.get() < plain.get()
                    || r.transport.clock.end().is_none_or(|end| lead.get() < end);
                if forward && inside && !legacy {
                    lead
                } else {
                    plain
                }
            } else {
                // Paused under the skimmer: its time, not the playhead's.
                v.skim.filter(|_| !playing).unwrap_or(sample.media_time)
            };
            let time =
                Ticks::new(r.transport.clock.snap(media.get()).max(0)).unwrap_or(Ticks::ZERO);
            r.next = due.unwrap_or(now) + interval;
            let capture = v.capture;
            if !legacy && due.is_some() && !v.dirty && !capture && r.last == Some((v.stamp, time)) {
                // Held content frame: the layer already shows these pixels.
                v.status.held_frames += 1;
                continue;
            }
            v.dirty = false;
            let moment = if playing {
                Moment::Playing
            } else if r.scrub {
                Moment::Scrub
            } else {
                Moment::Paused
            };
            let scene = Scene {
                doc: v.config.snapshot.document.clone(),
                comp: v.config.composition.clone(),
                media: if capture {
                    Arc::new(v.config.media.export_paths(false))
                } else if moment == Moment::Paused {
                    v.paused_paths.clone()
                } else {
                    v.paths.clone()
                },
                content: v.content,
            };
            cursor = id;
            let budget = frame.max(interval);
            let full = FrameSpec::paused_full_scale(&scene, &v.config.output);
            let refining = r.full_for == Some(v.stamp);
            work = Some(Work {
                id,
                stamp: v.stamp,
                scene,
                rendered_media: v.paused_paths.clone(),
                spec: FrameSpec {
                    time,
                    output: v.config.output,
                    // A capture reads the exact picture: a paused frame at full
                    // resolution whatever the moment and quality, drawn only
                    // from exact answers (no stand-in, cache or disk frame of
                    // another moment).
                    moment: if capture { Moment::Paused } else { moment },
                    scale: {
                        let scale = if capture {
                            1.
                        } else {
                            v.quality.scale(
                                &r.auto,
                                if moment == Moment::Scrub && !v.adapt_drags {
                                    Moment::Paused
                                } else {
                                    moment
                                },
                            )
                        };
                        // A paused frame at Full is drawn at the drawable's size
                        // at once, then refined at the composition's size (a
                        // capture reads the refined picture directly); playing
                        // and dragged frames keep their policy.
                        // (A play waiting for its audio start draws as it is.)
                        if scale == 1.
                            && (capture || (moment == Moment::Paused && !r.transport.running()))
                        {
                            if capture || refining || full <= 1. {
                                full
                            } else {
                                r.full_owed = Some(v.stamp);
                                1.
                            }
                        } else {
                            scale
                        }
                    },
                    decode_deadline: (playing && !capture).then(|| {
                        std::time::Instant::now() + Duration::from_secs_f64(budget * DECODE_SHARE)
                    }),
                    exact: capture,
                },
                layer: v.config.layer.clone(),
                overlays: if v.overlay_time.is_none_or(|t| t == time) {
                    v.overlays.clone()
                } else {
                    OverlayGeometry::default()
                },
                provider: v.locked_provider.clone(),
                capture: {
                    v.capture_owed |= v.capture;
                    std::mem::take(&mut v.capture)
                },
                due,
                mastering: v.mastering,
                resources: v.resources.clone(),
                input: v.input.take(),
                pipeline: v.pipeline,
                audio_host: r
                    .transport
                    .audio_host_for(time, (shown.unwrap_or(now) * 1e6) as i64)
                    .map(|t| t / 1e6),
                plan,
                frame_ticks: v.config.rate.exact_frame_ticks().unwrap_or(0),
            });
            break;
        }
        // A view waiting out a missing drawable can't draw: exports go ahead.
        let now = trace::now();
        shared.interactive.set(
            !flights.is_empty()
                || work.is_some()
                || state.views.iter().any(|(id, v)| {
                    (v.dirty && runtime.get(id).is_none_or(|r| r.not_before <= now))
                        || !v.commands.is_empty()
                })
                || runtime.values().any(|r| r.transport.running()),
        );
        if changed {
            state.notify();
        }
        if let Some(Work {
            id,
            stamp,
            scene,
            rendered_media,
            spec,
            layer,
            overlays,
            provider,
            capture,
            due,
            mastering,
            resources,
            input,
            pipeline,
            audio_host,
            plan,
            frame_ticks,
        }) = work
        {
            drop(state);
            let trace = Arc::new(Mutex::new(trace::Trace {
                worker_start: now,
                target: due.unwrap_or(0.),
                requested: plan.filter(|_| !legacy).map_or(0., |p| p.requested),
                predicted: plan.map_or(0., |p| p.shown),
                count_work: recorder.is_some(),
                ..Default::default()
            }));
            engine.mastering = mastering;
            engine.set_resources(resources);
            engine.use_compat_document(stamp.doc_id);
            let overrides = engine
                .compat()
                .filter(|store| store.producing_now())
                .map(|store| store.cache_identity());
            shared.disk.viewing(spec.render_size());
            // Fonts sync before the cache observes: a font change is seen
            // before this frame looks for a disk frame.
            let fonts = engine.font_generation();
            // Rendered frames are a paused frame's picture: observed (and
            // so fingerprinted and filled) with its media even while a
            // playing frame reads an HD proxy, or they would be drawn from
            // the proxy and orphaned at every pause and play.
            let rendered = Scene {
                media: rendered_media,
                ..scene.clone()
            };
            // Partial definitions while extensions load are never a fill's
            // source: its frames would be drawn without their effects.
            if !engine.definitions_pending()
                && shared.disk.observe(
                    &rendered,
                    engine.definitions(),
                    overrides,
                    engine.tone_identity(),
                    fonts,
                    stamp.doc_id,
                    spec.time,
                )
            {
                // A frame uploaded ahead is of the state before.
                engine.drop_upload_ahead();
                if let Some(recorder) = &recorder {
                    recorder.record(serde_json::json!({"render_cache_state": true, "at": now}));
                }
            }
            let landed_before = engine.media_landed();
            let tap = shared
                .scopes
                .lock()
                .unwrap()
                .as_ref()
                .filter(|(viewer, _)| *viewer == id)
                .map(|(_, tap)| tap.clone());
            if tap.is_some() && engine.scope_frames.is_none() {
                engine.scopes_changed();
            }
            engine.scope_frames = tap;
            #[cfg(test)]
            if shared
                .panic_frames
                .fetch_update(
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                    |n| n.checked_sub(1),
                )
                .is_ok()
            {
                panic!("injected frame failure");
            }
            // A refinement draws in bands into the frame cache first, giving
            // way to anything newer between bands; the frame then shows it.
            let refined = match runtime.get(&id) {
                Some(r) if !capture && spec.scale > 1. && r.full_for == Some(stamp) => {
                    // What bands cost is learned per document state, as a
                    // fill learns it per job: once bands cost a whole frame,
                    // later refinements draw whole without probing.
                    let state = (stamp.doc_id, stamp.revision, scene.content);
                    let mut costs = match r.refine_costs {
                        Some((at, costs)) if at == state => costs,
                        _ => crate::jobs::Costs::default(),
                    };
                    let cheap = r.paused_us.is_some_and(|us| {
                        us * spec.scale * spec.scale <= crate::jobs::BAND_BUDGET_US
                    });
                    let stop = || {
                        shared
                            .state
                            .lock()
                            .unwrap()
                            .views
                            .get(&id)
                            .is_none_or(|v| v.stamp != stamp)
                    };
                    let started = trace::now();
                    let refined = if cheap {
                        engine.refine_bands = 0;
                        Ok(true)
                    } else {
                        engine.refine(&scene, spec, &mut costs, &stop)
                    };
                    if let Some(r) = runtime.get_mut(&id) {
                        r.refine_costs = Some((state, costs));
                    }
                    if let Some(recorder) = &recorder {
                        recorder.record(serde_json::json!({
                            "refine_bands": engine.refine_bands,
                            "given_way": matches!(refined, Ok(false)),
                            "started": started, "ended": trace::now(), "viewer_id": id,
                        }));
                    }
                    refined
                }
                _ => Ok(true),
            };
            let result = match refined {
                Ok(true) => provider
                    .map_or_else(|| Ok(overlays), |p| p(&scene.doc, &scene.comp, spec.time))
                    .and_then(|overlays| {
                        engine.frame(&scene, spec, layer.as_ref(), &overlays, capture, trace)
                    }),
                // Given way: the newer picture is drawn next.
                Ok(false) => Ok(None),
                Err(e) => Err(e),
            };
            // Media ahead of the playhead is prepared after this frame is
            // submitted: building its plan must not delay this present.
            let mut prepare_at = None;
            let mut refine_at = None;
            if spec.moment == Moment::Playing
                && let Some(r) = runtime.get_mut(&id)
            {
                if now - r.prepared >= PREPARE_EVERY_SECONDS {
                    r.prepared = now;
                    let ahead = (PREPARE_AHEAD_SECONDS
                        * pm_protocol::common::TICKS_PER_SECOND as f64)
                        * f64::from(r.transport.rate().num.signum());
                    let at = (spec.time.get() + ahead as i64).max(0);
                    prepare_at = Some(Ticks::new(at).unwrap_or(spec.time));
                } else {
                    refine_at = r.lookahead.refine.pop_front();
                }
            } else if let Some(r) = runtime.get_mut(&id) {
                r.lookahead = Lookahead::default();
            }
            #[cfg(test)]
            if let Some(hook) = shared.before_submit.lock().unwrap().take() {
                hook();
            }
            // Publication/resize and final submission are serialized by this
            // short lock. Render and drawable acquisition never hold it.
            let mut state = shared.state.lock().unwrap();
            if let Some(v) = state.views.get_mut(&id) {
                if let Some(failure) = &engine.definition_failure {
                    v.status.error = Some(failure.clone());
                }
                if v.status.missing_effects != engine.missing_effects {
                    v.status.missing_effects = engine.missing_effects.clone();
                }
                // A capture asked for belongs to the next frame that lands.
                if v.stamp != stamp {
                    v.status.stale_rejections += 1;
                    v.capture |= capture;
                    continue;
                }
                match result {
                    Ok(Some(mut s)) => {
                        #[cfg(test)]
                        shared
                            .drawn
                            .lock()
                            .unwrap()
                            .push((stamp, spec.time, spec.scale));
                        v.status.error = None;
                        let late_layers = s.compat_late.len();
                        // A scrubbed frame's late override layers are asked
                        // now; playback asks ahead of the playhead instead.
                        let late = std::mem::take(&mut s.compat_late);
                        if spec.moment == Moment::Scrub
                            && !late.is_empty()
                            && let Some(store) = engine.compat()
                        {
                            store.request(late);
                        }
                        // The drawable was another size than configured (a
                        // resize whose publication lagged): later frames
                        // render at the size actually shown.
                        let shown = v.config.output.resized(s.width, s.height);
                        if s.drawable.is_some() && shown != v.config.output {
                            v.config.output = shown;
                            v.dirty = true;
                        }
                        v.status.stand_in_frames +=
                            u64::from(s.stand_ins && spec.moment == Moment::Playing);
                        // A share of Full: a paused frame rendered above the
                        // drawable is Full.
                        v.status.effective_quality = spec.scale.min(1.);
                        if let Some(r) = runtime.get_mut(&id) {
                            // Every dragged frame settles: once the playhead
                            // rests, the same time is drawn again exact.
                            if spec.moment == Moment::Scrub {
                                r.refine_at = Some(trace::now() + policy::SCRUB_SETTLE_SECONDS);
                            }
                            if capture {
                                r.capture_waiting = None;
                            }
                            r.stand_in = s.stand_ins.then_some(landed_before);
                            r.drawable_misses = 0;
                            r.refusals = 0;
                            // Override layers drawn from an earlier frame are
                            // still waited for: the wait goes on, and past a
                            // few seconds the viewer says so.
                            if late_layers == 0 {
                                r.waiting_since = None;
                            } else {
                                let now = trace::now();
                                let since = *r.waiting_since.get_or_insert(now);
                                if now - since >= WAITING_SHOWN_SECONDS {
                                    v.status.error = Some(late_note(late_layers));
                                }
                            }
                        }
                        let completed = Arc::downgrade(&shared);
                        s.submit_notifying(
                            plan.map(|p| p.requested),
                            Some(Arc::new(move || {
                                // Bounded tokens cannot lose a wake; callbacks
                                // never acquire the service state lock.
                                if let Some(completed) = completed.upgrade() {
                                    completed.wake.try_send(()).ok();
                                }
                            })),
                        );
                        if let Some(r) = runtime.get_mut(&id) {
                            r.last = Some((stamp, spec.time));
                        }
                        let played = (spec.moment == Moment::Playing)
                            .then(|| shared.disk.frame_id(&scene, spec.time))
                            .flatten();
                        flights.push_back(Flight {
                            id,
                            stamp,
                            spec,
                            submission: s,
                            input,
                            audio_host,
                            pipeline,
                            played,
                        });
                    }
                    Ok(None) => {
                        v.capture |= capture;
                        v.status.no_drawable += 1;
                        v.dirty = true;
                        v.input = input;
                        if let Some(r) = runtime.get_mut(&id) {
                            r.drawable_misses = r.drawable_misses.saturating_add(1);
                            r.not_before = trace::now() + drawable_retry(r.drawable_misses);
                        }
                    }
                    Err(e) if render::transient(&e) => {
                        v.capture |= capture;
                        v.dirty = true;
                        v.input = input;
                        // A raster override's layers: asked of the realm; its
                        // answers redraw (`CompatStore::answer_all`).
                        let pending = match e.downcast_ref::<pm_render::RenderError>() {
                            Some(pm_render::RenderError::CompatPending(needs)) => {
                                Some(needs.clone())
                            }
                            _ => None,
                        };
                        // A picture that owes its refinement asks for the
                        // refinement's answers (at the composition's scale)
                        // right behind its own: the realm draws them while this
                        // picture finishes, instead of after it lands.
                        let ahead = (pending.is_some()
                            && runtime.get(&id).is_some_and(|r| r.full_owed == Some(stamp)))
                        .then(|| {
                            let full = FrameSpec::paused_full_scale(&scene, &spec.output);
                            engine.compat_needs(
                                &scene,
                                spec.time,
                                FrameSpec {
                                    scale: full,
                                    ..spec
                                },
                            )
                        });
                        if let (Some(needs), Some(store)) = (pending.clone(), engine.compat()) {
                            store.request(needs);
                            if let Some(ahead) = ahead {
                                store.request_behind(ahead);
                            }
                        }
                        if let Some(recorder) = &recorder {
                            recorder.record(serde_json::json!({
                                "refused": refusal_kind(&e),
                                "time": spec.time.get(),
                                "at": trace::now(),
                            }));
                        }
                        if let Some(r) = runtime.get_mut(&id) {
                            let now = trace::now();
                            r.refusals = r.refusals.saturating_add(1);
                            // A playing frame retries at once each tick: a
                            // backoff doubled up to 1 s held playback for a
                            // second after a few refusals (a shader compiling).
                            r.not_before = now
                                + if spec.moment == policy::Moment::Playing {
                                    RETRY_SECONDS
                                } else {
                                    drawable_retry(r.refusals - 1)
                                };
                            // Retried for good, but never silently: past a few
                            // seconds the viewer says what it waits for.
                            let since = *r.waiting_since.get_or_insert(now);
                            if now - since >= WAITING_SHOWN_SECONDS {
                                v.status.error = Some(format!("Still waiting: {e}"));
                            }
                            // A capture waits for its exact frame, but not
                            // forever: past its bound it fails, saying why.
                            if capture {
                                let since = *r.capture_waiting.get_or_insert(now);
                                if now - since >= CAPTURE_WAIT_SECONDS {
                                    r.capture_waiting = None;
                                    v.capture = false;
                                    v.capture_owed = false;
                                    v.status.capture_failed = Some(format!(
                                        "The exact frame could not be captured: still waiting after {CAPTURE_WAIT_SECONDS} s ({e})."
                                    ));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        v.status.error = Some(e.to_string());
                        if capture {
                            v.capture_owed = false;
                            v.status.capture_failed =
                                Some(format!("The exact frame could not be captured: {e}"));
                        }
                        if let Some(r) = runtime.get_mut(&id) {
                            r.waiting_since = None;
                            r.capture_waiting = None;
                        }
                    }
                }
            }
            state.notify();
            drop(state);
            if spec.moment == Moment::Playing {
                let rate = runtime.get(&id).map(|r| r.transport.rate());
                ask_ahead(&mut engine, &shared.disk, &scene, spec, rate);
                // The next frame, if it is rendered, is uploaded in this
                // frame's slack instead of at its own turn.
                let step = frame_ticks * i64::from(rate.map_or(1, |r| r.num.signum()));
                if let Ok(next) = Ticks::new(spec.time.get() + step) {
                    engine.upload_ahead(&scene, next);
                }
            }
            // A frame rendered on disk is read, not drawn: planning it and
            // opening its media ahead would only hold this thread (a quarter
            // of a second on a heavy project) while frames are due.
            if let Some(at) = prepare_at.filter(|at| !shared.disk.rendered(&scene, *at)) {
                let videos = engine.prepare(&scene, at, spec);
                if let Some(r) = runtime.get_mut(&id) {
                    r.lookahead.planned(at, videos, frame_ticks);
                }
            } else if let Some(at) = refine_at.filter(|at| !shared.disk.rendered(&scene, *at)) {
                engine.prepare(&scene, at, spec);
            }
        } else {
            let waiting = runtime
                .values()
                .any(|r| r.stand_in.is_some() || r.not_before > now);
            // A dragged frame waiting to be drawn exact wakes the loop then.
            let settle = runtime
                .values()
                .filter_map(|r| r.refine_at)
                .map(|at| (at - now).max(0.001))
                .fold(f64::INFINITY, f64::min);
            let busy =
                waiting || !flights.is_empty() || runtime.values().any(|r| r.transport.running());
            // Only back-offs pending, nothing playing: sleep until the first is due.
            let backoff = (flights.is_empty()
                && runtime
                    .values()
                    .all(|r| r.stand_in.is_none() && !r.transport.running()))
            .then(|| {
                runtime
                    .values()
                    .map(|r| r.not_before - now)
                    .filter(|wait| *wait > 0.)
                    .fold(f64::INFINITY, f64::min)
            })
            .filter(|wait| wait.is_finite());
            let timeout = if let Some(wait) = backoff {
                Duration::from_secs_f64(wait.max(0.001))
            } else if waiting {
                Duration::from_millis(2)
            } else if busy {
                let now = trace::now();
                let next = runtime
                    .iter()
                    .filter_map(|(id, r)| {
                        let room =
                            flights.len() < 3 && flights.iter().filter(|f| f.id == *id).count() < 3;
                        if r.transport.clock.is_playing() && room {
                            Some((r.next - r.ahead * r.interval - now).max(0.0001))
                        } else if r.transport.running() && !r.transport.clock.is_playing() {
                            Some(0.001) // waiting for the first audio epoch
                        } else {
                            None
                        }
                    })
                    .fold(0.25_f64, f64::min);
                Duration::from_secs_f64(next)
            } else {
                Duration::from_secs(3600)
            };
            drop(state);
            let tap = shared.scopes.lock().unwrap().clone();
            let timeout = engine
                .flush_scopes(tap.as_ref().map(|(_, tap)| tap))
                .map_or(timeout, |retry| timeout.min(retry));
            let timeout = if settle.is_finite() {
                timeout.min(Duration::from_secs_f64(settle))
            } else {
                timeout
            };
            if engine.maintain_memory() {
                shared.disk.release_idle_reads();
                for r in runtime.values() {
                    if let Some(audio) = &r.transport.audio {
                        audio.send(audio::AudioCtl::MemoryPressure);
                    }
                }
            }
            let _ = wakes.recv_timeout(timeout.min(Duration::from_millis(250)));
        }
    }
    drop(flights); // Command completion handlers hold their source leases.
    drop(runtime); // Shutdown/join audio away from the UI thread and state lock.
}

/// Windowless renderer for CLI/MCP and independent pixel comparisons.
pub struct Headless {
    engine: render::Engine,
}
impl Headless {
    pub fn memory_usage(&self) -> memory::CacheUsage {
        self.engine.memory_usage()
    }

    /// Submit a preview frame without allocating a CPU pixel readback.
    pub fn preview(&mut self, scene: &Scene, spec: FrameSpec) -> Result<()> {
        self.preview_inner(scene, spec)
    }

    /// Pump the main-thread font broker while the render thread waits for
    /// replies, including during a frame's first text layout.
    pub fn preview_with_pump(
        &mut self,
        scene: &Scene,
        spec: FrameSpec,
        pump: impl FnMut(),
    ) -> Result<()> {
        with_pump(|| self.preview_inner(scene, spec), pump)
    }

    /// Keep one render thread for a sequence while the caller pumps its
    /// main-thread services. Recreating it per frame changes allocator
    /// ownership and does not model the app's persistent render worker.
    pub fn run_with_pump<T: Send>(
        &mut self,
        run: impl FnOnce(&mut Self) -> T + Send,
        pump: impl FnMut(),
    ) -> T {
        with_pump(|| run(self), pump)
    }

    fn preview_inner(&mut self, scene: &Scene, spec: FrameSpec) -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let rendered = objc2::rc::autoreleasepool(|_| -> Result<()> {
                let Some(s) = self.engine.frame(
                    scene,
                    spec,
                    None,
                    &OverlayGeometry::default(),
                    false,
                    Arc::default(),
                )?
                else {
                    anyhow::bail!("no headless surface");
                };
                s.commit();
                s.complete()?;
                anyhow::ensure!(
                    s.failures.is_empty(),
                    "memory benchmark frame omitted {} layers: {}",
                    s.failures.len(),
                    s.failures
                        .iter()
                        .map(|f| f.message.as_str())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join("; ")
                );
                Ok(())
            });
            match rendered {
                Ok(_) => {
                    self.engine.poll();
                    return Ok(());
                }
                Err(e) if render::transient(&e) && std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_secs_f64(RETRY_SECONDS))
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn release_idle_memory(&mut self) {
        self.engine.release_idle_memory();
    }
    /// Releases every cache owned by the previous project. The renderer and
    /// device stay available for the next project in the same window.
    pub fn release_project_memory(&mut self) {
        self.engine.release_project_memory();
    }
    pub fn memory_pressure(&mut self, pressure: memory::Pressure) {
        memory::simulate_pressure(pressure);
        self.engine.govern_memory();
        self.engine.poll();
    }
    pub fn new() -> Result<Self> {
        objc2::rc::autoreleasepool(|_| {
            Ok(Self {
                engine: render::Engine::new(pm_gpu::Gpu::new("pm-headless")?)?,
            })
        })
    }
    pub fn with_fonts(fonts: pm_render::text::host::RenderFonts) -> Result<Self> {
        let mut h = Self::new()?;
        h.engine.set_fonts(fonts);
        Ok(h)
    }
    pub fn mastering(&mut self, mode: color::Mastering) -> Result<()> {
        mode.params()?;
        self.engine.mastering = mode;
        Ok(())
    }
    pub fn frame(
        &mut self,
        scene: &Scene,
        time: Ticks,
        size: Size,
        overlays: &OverlayGeometry,
    ) -> Result<Vec<[f64; 3]>> {
        self.frame_spec_settled(
            scene,
            FrameSpec::exact(
                time,
                Output {
                    width: size.width,
                    height: size.height,
                    display: 0,
                    headroom: 1.,
                    crop: None,
                },
            ),
            overlays,
        )
    }
    /// One viewer frame exactly as the worker requests it, drawn again after
    /// the same transient refusals the worker retries.
    pub fn frame_spec_settled(
        &mut self,
        scene: &Scene,
        spec: FrameSpec,
        overlays: &OverlayGeometry,
    ) -> Result<Vec<[f64; 3]>> {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match self
                .engine
                .frame(scene, spec, None, overlays, true, Arc::default())
            {
                Ok(s) => {
                    let s = s.ok_or_else(|| anyhow::anyhow!("no offscreen target"))?;
                    s.commit();
                    s.complete()?;
                    return Ok(s.pixels());
                }
                Err(e) if render::transient(&e) && std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_secs_f64(RETRY_SECONDS));
                }
                Err(e) => return Err(e),
            }
        }
    }
    pub fn export_reference(&mut self, scene: &Scene, time: Ticks, size: Size) -> Result<Vec<f32>> {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match self.engine.export(scene, time, size) {
                Err(e) if render::transient(&e) && std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_secs_f64(RETRY_SECONDS));
                }
                other => return other,
            }
        }
    }
}

/// Font registries belong to the main thread; a render can synchronously
/// request more fonts at any point. Pumping only between renders deadlocks.
fn with_pump<T: Send>(work: impl FnOnce() -> T + Send, mut pump: impl FnMut()) -> T {
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| objc2::rc::autoreleasepool(|_| work()));
        while !worker.is_finished() {
            pump();
            std::thread::sleep(Duration::from_millis(1));
        }
        worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    use foreign_types::ForeignTypeRef;

    #[test]
    fn headless_pump_answers_fonts_during_a_blocking_render() {
        let broker = pm_text::host::FontBroker::new();
        broker.set_wake(Arc::new(|| {}));
        let worker = broker.clone();
        with_pump(
            move || worker.ask(Vec::new(), Vec::new(), Duration::from_millis(500)),
            || {
                for ask in broker.take_asks() {
                    ask.answer(Ok(pm_text::host::FontReply::default()));
                }
            },
        )
        .unwrap();
        assert_eq!(broker.counts(), (1, 1, 0));
    }
    #[test]
    fn revision_and_output_change_while_encoding_reject_the_submission() {
        let project = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap();
        #[cfg(feature = "kernel")]
        let snapshot = pm_kernel::document::DocumentStore::new_untitled(project).snapshot();
        #[cfg(not(feature = "kernel"))]
        let snapshot = Snapshot {
            doc_id: 99,
            revision: 1,
            document: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(project)),
                1,
            )),
        };
        let service = EngineService::start(pm_gpu::Gpu::new("stale-worker-test").unwrap()).unwrap();
        let (entered, signal) = std::sync::mpsc::channel();
        let (release, barrier) = std::sync::mpsc::channel();
        *service.shared.before_submit.lock().unwrap() = Some(Box::new(move || {
            entered.send(()).unwrap();
            barrier.recv_timeout(Duration::from_secs(10)).unwrap();
        }));
        let v = service
            .viewer(ViewerConfig {
                composition: snapshot.open_comp().unwrap(),
                snapshot: snapshot.clone(),
                media: Default::default(),
                output: Output {
                    width: 160,
                    height: 90,
                    display: 1,
                    headroom: 1.,
                    crop: None,
                },
                layer: None,
                rate: pm_time::Rate::FPS_30,
                end: None,
                audio: false,
            })
            .unwrap();
        signal.recv_timeout(Duration::from_secs(10)).unwrap();
        let old = v.stamp().unwrap();
        v.configure(Output {
            width: 320,
            height: 180,
            display: 2,
            headroom: 2.,
            crop: None,
        })
        .unwrap();
        v.snapshot(Snapshot {
            revision: 2,
            document: Arc::new(pm_model::Document::new(snapshot.document.to_value(), 2)),
            ..snapshot
        })
        .unwrap();
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let status = v.status();
            assert!(status.error.is_none(), "{:?}", status.error);
            if let Some(frame) = status.frame {
                assert_ne!(frame.stamp, old);
                // The new revision's picture: the current one, or the fast
                // picture whose refinement (the next stamp) is being drawn.
                let now = v.stamp().unwrap();
                assert_eq!(
                    (frame.stamp.generation, frame.stamp.revision),
                    (now.generation, now.revision)
                );
                assert!(
                    now.sequence - frame.stamp.sequence <= 1,
                    "{frame:?} {now:?}"
                );
                assert!(status.stale_rejections >= 1);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn shapes_viewer(service: &EngineService, layer: Option<metal::MetalLayer>) -> Viewer {
        service.viewer(shapes_config(layer)).unwrap()
    }
    fn shapes_config(layer: Option<metal::MetalLayer>) -> ViewerConfig {
        let project = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap();
        #[cfg(feature = "kernel")]
        let snapshot = pm_kernel::document::DocumentStore::new_untitled(project).snapshot();
        #[cfg(not(feature = "kernel"))]
        let snapshot = Snapshot {
            doc_id: 98,
            revision: 1,
            document: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(project)),
                1,
            )),
        };
        ViewerConfig {
            composition: snapshot.open_comp().unwrap(),
            snapshot,
            media: Default::default(),
            output: Output {
                width: 160,
                height: 90,
                display: 1,
                headroom: 1.,
                crop: None,
            },
            layer,
            rate: pm_time::Rate::FPS_30,
            end: None,
            audio: false,
        }
    }
    fn drawn(v: &Viewer) -> ViewerStatus {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let status = v.status();
            if let Some(frame) = &status.frame
                && frame.gpu_ok
                && frame.pixels.is_some()
                && Some(frame.stamp) == v.stamp()
            {
                return status;
            }
            assert!(std::time::Instant::now() < deadline, "{status:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A layer resized without a configure (its size publication lagged):
    /// the frame is drawn at the drawable's size and later frames follow it,
    /// instead of the drawable being dropped and the frame retried forever.
    #[test]
    fn a_drawable_of_another_size_is_drawn_not_retried() {
        let service =
            EngineService::start(pm_gpu::Gpu::new("drawable-size-test").unwrap()).unwrap();
        let layer = metal::MetalLayer::new();
        layer.set_device(&service.metal_device());
        layer.set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
        layer.set_framebuffer_only(false);
        layer.set_drawable_size(core_graphics_types::geometry::CGSize::new(200., 100.));
        let v = shapes_viewer(&service, Some(layer));
        // The first frame shows that the drawable differs; the next renders
        // at its size.
        std::thread::sleep(Duration::from_millis(300));
        v.capture_next().unwrap();
        let status = drawn(&v);
        assert_eq!(status.error, None);
        assert_eq!(status.no_drawable, 0);
        let pixels = status.frame.unwrap().pixels.unwrap();
        assert_eq!(pixels.len(), 200 * 100);
        // Settled: no frame keeps being asked for.
        std::thread::sleep(Duration::from_millis(300));
        let completed = || v.status().frame.and_then(|f| f.completion_host_seconds);
        let settled = completed();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(completed(), settled, "a paused viewer stays idle");
        assert_eq!(v.status().no_drawable, 0);
    }

    /// A closed project's viewer layer is freed with its drawables: nothing
    /// the engine thread autoreleased while returning drawables keeps it.
    #[test]
    fn a_retired_viewers_layer_is_released() {
        let service =
            EngineService::start(pm_gpu::Gpu::new("layer-release-test").unwrap()).unwrap();
        let weak = objc2::rc::autoreleasepool(|_| {
            let layer = metal::MetalLayer::new();
            layer.set_device(&service.metal_device());
            layer.set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
            layer.set_framebuffer_only(false);
            layer.set_drawable_size(core_graphics_types::geometry::CGSize::new(160., 90.));
            // This thread has no run loop to commit the implicit transaction
            // those property changes opened (it retains the layer).
            // SAFETY: a class method with no arguments.
            unsafe {
                let _: () = objc2::msg_send![objc2::class!(CATransaction), flush];
            }
            // SAFETY: a live CAMetalLayer; the Retained takes its own reference.
            let object = unsafe {
                objc2::rc::Retained::retain(
                    foreign_types::ForeignType::as_ptr(&layer).cast::<objc2::runtime::AnyObject>(),
                )
            }
            .unwrap();
            let weak = objc2::rc::Weak::from_retained(&object);
            let v = shapes_viewer(&service, Some(layer));
            v.capture_next().unwrap();
            drawn(&v);
            v.capture_next().unwrap();
            drawn(&v);
            v.retire();
            weak
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while objc2::rc::autoreleasepool(|_| weak.load().is_some()) {
            assert!(
                std::time::Instant::now() < deadline,
                "the retired viewer's layer is still alive"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_notice_names_the_commonest_reason_a_layer_is_not_drawn() {
        let failure = |layer: &str, message: &str| pm_render::LayerFailure {
            composition: "c".into(),
            layer: layer.into(),
            message: message.into(),
        };
        let mut status = ViewerStatus::default();
        assert_eq!(status.notice(), None);
        status.layer_failures = vec![
            failure("a", "no definition for effect deep-glow.effect"),
            failure("b", "no definition for effect deep-glow.effect"),
            failure("b", "no definition for effect deep-glow.effect"),
            failure("c", "no definition for effect progressive-blur"),
        ];
        assert_eq!(
            status.notice().as_deref(),
            Some("Not drawn, 2 layers: no definition for effect deep-glow.effect (and 1 more)")
        );
    }

    #[test]
    fn the_notice_names_an_effect_that_is_bypassed_because_it_is_not_installed() {
        let missing = |effect: &str, layers| pm_render::effects::versions::MissingEffect {
            effect: effect.into(),
            revision: None,
            layers,
        };
        let mut status = ViewerStatus {
            missing_effects: vec![missing("acme.glow", 4), missing("acme.blur", 1)],
            ..ViewerStatus::default()
        };
        assert_eq!(
            status.notice().as_deref(),
            Some("Effect not installed: acme.glow, bypassed on 4 layers (and 1 more)")
        );
        let newer = ViewerStatus {
            missing_effects: vec![pm_render::effects::versions::MissingEffect {
                revision: Some("99".into()),
                ..missing("blur", 1)
            }],
            ..ViewerStatus::default()
        };
        assert_eq!(
            newer.notice().as_deref(),
            Some("Effect saved by a newer Powermove: blur version 99, bypassed on 1 layer")
        );
        status.layer_failures = vec![pm_render::LayerFailure {
            composition: "c".into(),
            layer: "a".into(),
            message: "media for asset x is offline".into(),
        }];
        assert_eq!(
            status.notice().as_deref(),
            Some("Not drawn, 1 layer: media for asset x is offline (and 2 more)")
        );
    }

    #[test]
    fn a_resized_crop_stays_inside_its_canvas() {
        let output = Output {
            width: 100,
            height: 50,
            display: 1,
            headroom: 1.,
            crop: Some(Crop {
                x: 30,
                y: 10,
                canvas_width: 130,
                canvas_height: 60,
            }),
        };
        let resized = output.resized(103, 51);
        assert_eq!((resized.width, resized.height), (103, 51));
        resized.validate().unwrap();
        assert_eq!(output.resized(100, 50), output);
    }

    /// Tests that build a whole second engine (every built-in translated)
    /// run one at a time, so their limits measure the engine, not contention.
    static REBUILDS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A panic in the worker (a bug a frame hit) restarts it: the viewer
    /// reports it and draws the same picture again, and the UI thread's
    /// reads of its status keep working.
    #[test]
    fn a_worker_panic_restarts_the_worker_and_the_viewer_draws_again() {
        let _whole_engine = REBUILDS.lock().unwrap_or_else(|e| e.into_inner());
        let service = EngineService::start(pm_gpu::Gpu::new("panic-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        v.capture_next().unwrap();
        let before = drawn(&v).frame.unwrap().pixels.unwrap();
        service
            .shared
            .panic_once
            .store(true, std::sync::atomic::Ordering::Release);
        service.shared.wake.try_send(()).ok();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while v.status().restarts == 0 {
            assert!(std::time::Instant::now() < deadline, "{:?}", v.status());
            std::thread::sleep(Duration::from_millis(2));
        }
        // Reported until the restarted worker draws a good frame.
        let error = v.status().error;
        assert!(
            error.is_none()
                || error.as_deref().is_some_and(|e| e.starts_with(
                    "The viewer restarted after an internal error.\ninjected viewer failure"
                )),
            "{error:?}"
        );
        v.capture_next().unwrap();
        let after = drawn(&v);
        assert_eq!(after.error, None, "a good frame clears the report");
        assert_eq!(after.frame.unwrap().pixels.unwrap(), before);
    }

    fn wait_for(v: &Viewer, what: &str, ok: impl Fn(&ViewerStatus) -> bool) -> ViewerStatus {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let status = v.status();
            if ok(&status) {
                return status;
            }
            assert!(std::time::Instant::now() < deadline, "{what}: {status:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn seconds(t: f64) -> Ticks {
        Ticks::new((t * pm_protocol::common::TICKS_PER_SECOND as f64) as i64).unwrap()
    }

    /// A restart keeps each viewer's transport: a paused viewer stays where
    /// it was, a playing one keeps playing from where it was (not stopped at
    /// zero), with its loop.
    #[test]
    fn a_worker_restart_keeps_time_play_state_and_loop() {
        use pm_protocol::clock::{SeekMode, TransportState};
        let _whole_engine = REBUILDS.lock().unwrap_or_else(|e| e.into_inner());
        let service = EngineService::start(pm_gpu::Gpu::new("restart-transport").unwrap()).unwrap();
        let paused = shapes_viewer(&service, None);
        let playing = shapes_viewer(&service, None);
        paused
            .command(TransportCommand::Seek {
                time: seconds(1.5),
                mode: SeekMode::Exact,
            })
            .unwrap();
        let range = pm_protocol::common::TickRange {
            start: seconds(1.),
            end: seconds(2.5),
        };
        for cmd in [
            TransportCommand::SetLoop { range: Some(range) },
            TransportCommand::Seek {
                time: seconds(1.),
                mode: SeekMode::Exact,
            },
            TransportCommand::Play {
                rate: pm_protocol::common::PlaybackRate::NORMAL,
            },
        ] {
            playing.command(cmd).unwrap();
        }
        wait_for(&paused, "paused at 1.5 s", |s| {
            s.clock.is_some_and(|c| c.media_time == seconds(1.5))
        });
        wait_for(&playing, "playing in the loop", |s| {
            s.clock
                .is_some_and(|c| c.state != TransportState::Stopped && c.loop_range == Some(range))
        });
        service
            .shared
            .panic_once
            .store(true, std::sync::atomic::Ordering::Release);
        service.shared.wake.try_send(()).ok();
        wait_for(&paused, "restart", |s| s.restarts == 1);
        // The new transport's first samples.
        // A frame of the restart's stamp: drawn by the new worker, whose
        // transport took the restored commands first.
        let p = wait_for(&paused, "paused after restart", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.stamp == paused.stamp().unwrap())
        });
        let clock = p.clock.unwrap();
        assert_eq!(clock.state, TransportState::Stopped);
        assert_eq!(clock.media_time, seconds(1.5), "still at 1.5 s, not zero");
        let s = wait_for(&playing, "playing after restart", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.stamp == playing.stamp().unwrap())
                && s.clock.is_some_and(|c| c.state != TransportState::Stopped)
        });
        let clock = s.clock.unwrap();
        assert_eq!(clock.rate, pm_protocol::common::PlaybackRate::NORMAL);
        let heard = clock.media_time_at(
            pm_protocol::common::HostMicros::new((trace::now() * 1e6) as u64).unwrap(),
        );
        assert!(
            heard >= range.start && heard < range.end,
            "plays on inside its loop: {heard:?}"
        );
        wait_for(&playing, "loop kept", |s| {
            s.clock.is_some_and(|c| c.loop_range == Some(range))
        });
    }

    /// A capture whose frame was rendering when the worker panicked is
    /// drawn by the restarted worker; nobody asks again.
    #[test]
    fn a_capture_in_flight_at_a_panic_is_delivered_after_restart() {
        let _whole_engine = REBUILDS.lock().unwrap_or_else(|e| e.into_inner());
        let service = EngineService::start(pm_gpu::Gpu::new("restart-capture").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        v.capture_next().unwrap();
        let before = drawn(&v).frame.unwrap().pixels.unwrap();
        service
            .shared
            .panic_frames
            .store(1, std::sync::atomic::Ordering::Release);
        v.capture_next().unwrap();
        wait_for(&v, "restart", |s| s.restarts == 1);
        let after = drawn(&v);
        assert_eq!(after.frame.unwrap().pixels.unwrap(), before);
        assert_eq!(after.capture_failed, None);
    }

    /// When the worker stops for good (three panics in a minute), a pending
    /// capture fails with the reason and later captures are refused: no
    /// waiter waits forever.
    #[test]
    fn a_capture_pending_when_the_viewer_stops_fails() {
        let _whole_engine = REBUILDS.lock().unwrap_or_else(|e| e.into_inner());
        let service = EngineService::start(pm_gpu::Gpu::new("stop-capture").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        v.capture_next().unwrap();
        drawn(&v);
        service
            .shared
            .panic_frames
            .store(3, std::sync::atomic::Ordering::Release);
        v.capture_next().unwrap();
        let status = wait_for(&v, "capture failed", |s| s.capture_failed.is_some());
        assert_eq!(status.restarts, 2);
        assert!(
            status
                .capture_failed
                .unwrap()
                .contains("injected frame failure")
        );
        assert!(v.capture_next().is_err());
    }

    /// A removed or reset GPU: the worker rebuilds its device, points the
    /// viewer's layer and the service's device at it, and the viewer draws
    /// again without anyone reopening it.
    #[test]
    fn a_lost_device_is_rebuilt_and_the_viewer_draws_again() {
        let _whole_engine = REBUILDS.lock().unwrap_or_else(|e| e.into_inner());
        let service = EngineService::start(pm_gpu::Gpu::new("lost-device-test").unwrap()).unwrap();
        let layer = metal::MetalLayer::new();
        layer.set_device(&service.metal_device());
        layer.set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
        layer.set_framebuffer_only(false);
        layer.set_drawable_size(core_graphics_types::geometry::CGSize::new(160., 90.));
        let v = shapes_viewer(&service, Some(layer.clone()));
        let old = service.metal_device();
        v.capture_next().unwrap();
        let before = drawn(&v).frame.unwrap().pixels.unwrap();
        service
            .shared
            .lose_device
            .store(true, std::sync::atomic::Ordering::Release);
        service.shared.wake.try_send(()).ok();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while v.status().device_losses == 0 {
            assert!(std::time::Instant::now() < deadline, "{:?}", v.status());
            std::thread::sleep(Duration::from_millis(2));
        }
        v.capture_next().unwrap();
        let after = drawn(&v);
        assert_eq!(after.error, None);
        assert_eq!(after.device_losses, 1);
        assert_eq!(
            after.frame.unwrap().pixels.unwrap(),
            before,
            "same picture on the new device"
        );
        // Metal hands back the same system device object for the same GPU,
        // so here the rebuilt engine's device equals the old one; a removed
        // GPU gives a new one, which the layer and the service then report.
        let new = service.metal_device();
        assert_eq!(new.as_ptr(), old.as_ptr());
        assert_eq!(
            layer.device().as_ptr(),
            new.as_ptr(),
            "the layer follows the rebuild"
        );
    }

    /// Metal reports a command buffer complete before its completion handler
    /// runs; a frame retired in between must not read as failed. Under load
    /// the dragged frame then never showed (the bench's "scrub stuck").
    #[test]
    fn a_late_completion_handler_never_loses_a_dragged_frame() {
        let service =
            EngineService::start(pm_gpu::Gpu::new("late-completion-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        crate::render::LATE_COMPLETION_MS.store(30, std::sync::atomic::Ordering::Relaxed);
        for (i, at) in (1..=8).map(|i| {
            (
                i,
                Ticks::new(i * pm_protocol::common::TICKS_PER_SECOND / 6).unwrap(),
            )
        }) {
            v.scrub(at).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                // Display ticks keep the worker polling, as a display link and
                // the bench do: it looks at its flights while the handler is late.
                v.display_tick(trace::now() + 1. / 60., 1. / 60.).unwrap();
                let status = v.status();
                if status.frame.as_ref().is_some_and(|f| {
                    f.time == at && f.gpu_ok && f.completion_host_seconds.is_some()
                }) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "drag step {i} never showed: {status:?}"
                );
                std::thread::sleep(Duration::from_micros(200));
            }
        }
        crate::render::LATE_COMPLETION_MS.store(0, std::sync::atomic::Ordering::Relaxed);
        v.retire();
    }

    /// The skimmer shows its time without moving the playhead, and the
    /// viewer returns to the playhead's frame when the pointer leaves.
    #[test]
    fn the_skimmer_shows_its_frame_and_leaves_the_playhead() {
        let service = EngineService::start(pm_gpu::Gpu::new("skim-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        let frame = |v: &Viewer, at: Ticks| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                v.display_tick(trace::now() + 1. / 60., 1. / 60.).unwrap();
                let status = v.status();
                if status
                    .frame
                    .as_ref()
                    .is_some_and(|f| f.time == at && f.completion_host_seconds.is_some())
                {
                    return status;
                }
                assert!(std::time::Instant::now() < deadline, "{at:?}: {status:?}");
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let playhead = Ticks::new(pm_protocol::common::TICKS_PER_SECOND / 3).unwrap();
        v.scrub(playhead).unwrap();
        frame(&v, playhead);
        for (i, at) in [1, 2, 3, 2]
            .map(|s| Ticks::new(s * pm_protocol::common::TICKS_PER_SECOND / 2).unwrap())
            .into_iter()
            .enumerate()
        {
            v.skim(Some(at), trace::now() + i as f64 * 0.016).unwrap();
            let status = frame(&v, at);
            assert_eq!(
                status.clock.unwrap().media_time,
                playhead,
                "skimming never moves the playhead"
            );
        }
        v.skim(None, trace::now()).unwrap();
        frame(&v, playhead);
        v.retire();
    }

    #[test]
    fn skim_rates_follow_the_pointer() {
        assert_eq!(skim_rate(0.05, 0.1).num, 1);
        assert_eq!(skim_rate(-0.05, 0.1).num, -1);
        assert_eq!(skim_rate(0.4, 0.1).num, 2);
        assert_eq!(skim_rate(-0.4, 0.1).num, -2);
    }

    /// The fixture is 640 × 360 in a 160 × 90 drawable (Fit at a quarter):
    /// a paused frame renders the composition at its own size, as an export
    /// does, so pixel-sized effects look as delivered, and the final pass
    /// shows it at the drawable's.
    #[test]
    fn a_paused_full_frame_renders_at_the_composition_size_and_shows_at_the_drawable() {
        let service = EngineService::start(pm_gpu::Gpu::new("paused-full-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        v.capture_next().unwrap();
        let status = drawn(&v);
        let frame = status.frame.as_ref().unwrap();
        assert_eq!(frame.pixels.as_ref().unwrap().len(), 160 * 90);
        assert_eq!((frame.moment, frame.scale), (Moment::Paused, 4.));
        assert_eq!(status.effective_quality, 1., "Full");
    }

    /// A paused frame shows at once at the drawable's size, then a refined
    /// picture of the same time at the composition's; a seek starts over,
    /// and playback never waits for (or draws) a refinement.
    #[test]
    fn a_paused_frame_is_shown_at_once_then_refined_and_play_skips_the_refinement() {
        use pm_protocol::clock::SeekMode;
        let service =
            EngineService::start(pm_gpu::Gpu::new("paused-refine-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        let drawn = || service.shared.drawn.lock().unwrap().clone();
        // The frames drawn of `time` from `from` on: their scales in order.
        let scales = |from: usize, time: Ticks| -> Vec<f64> {
            drawn()[from..]
                .iter()
                .filter(|(_, t, _)| *t == time)
                .map(|(_, _, scale)| *scale)
                .collect()
        };
        let refined = |from: usize, time: Ticks| {
            wait_for(&v, "the refinement", |_| scales(from, time).contains(&4.));
            scales(from, time)
        };
        let first = wait_for(&v, "a first frame", |s| s.frame.is_some())
            .frame
            .unwrap()
            .time;
        assert_eq!(refined(0, first)[..2], [1., 4.], "at once, then refined");
        let (fast, fine) = {
            let d = drawn();
            let fast = d
                .iter()
                .find(|(_, t, s)| *t == first && *s == 1.)
                .unwrap()
                .0;
            let fine = d
                .iter()
                .find(|(_, t, s)| *t == first && *s == 4.)
                .unwrap()
                .0;
            (fast, fine)
        };
        assert!(
            fine.sequence > fast.sequence,
            "a new picture of the same time"
        );
        // A seek: the fast picture of the new time, then its refinement.
        let from = drawn().len();
        v.command(TransportCommand::Seek {
            time: seconds(1.),
            mode: SeekMode::Exact,
        })
        .unwrap();
        assert_eq!(refined(from, seconds(1.))[..2], [1., 4.]);
        // Play right after a pause's fast picture: nothing more is drawn at
        // the composition's size while the transport runs.
        let from = drawn().len();
        v.command(TransportCommand::Seek {
            time: seconds(0.5),
            mode: SeekMode::Exact,
        })
        .unwrap();
        v.command(TransportCommand::Play {
            rate: pm_protocol::common::PlaybackRate::NORMAL,
        })
        .unwrap();
        wait_for(&v, "playing", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.moment == Moment::Playing)
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            drawn()[from..].iter().all(|(_, _, scale)| *scale <= 1.),
            "{:?}",
            &drawn()[from..]
        );
    }

    /// A crop that is the whole canvas (Fit, whose window the shell can round
    /// a pixel narrower than its canvas) renders at the composition's size
    /// like no crop; a zoomed-in window keeps its scale.
    #[test]
    fn a_whole_canvas_crop_is_fit_and_a_zoomed_one_keeps_its_scale() {
        let project = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap();
        #[cfg(feature = "kernel")]
        let snapshot = pm_kernel::document::DocumentStore::new_untitled(project).snapshot();
        #[cfg(not(feature = "kernel"))]
        let snapshot = Snapshot {
            doc_id: 98,
            revision: 1,
            document: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(project)),
                1,
            )),
        };
        let scene = Scene {
            doc: snapshot.document.clone(),
            comp: snapshot.open_comp().unwrap(),
            media: Default::default(),
            content: 0,
        };
        let output = |crop| Output {
            width: 160,
            height: 90,
            display: 1,
            headroom: 1.,
            crop,
        };
        let whole = Some(crate::policy::Crop {
            x: 0,
            y: 0,
            canvas_width: 160,
            canvas_height: 90,
        });
        let zoomed = Some(crate::policy::Crop {
            x: 40,
            y: 20,
            canvas_width: 640,
            canvas_height: 360,
        });
        assert_eq!(FrameSpec::paused_full_scale(&scene, &output(None)), 4.);
        assert_eq!(FrameSpec::paused_full_scale(&scene, &output(whole)), 4.);
        // Fit rounded a pixel narrow (the shell's 1265 on 1266).
        let narrow = Some(crate::policy::Crop {
            x: 0,
            y: 0,
            canvas_width: 161,
            canvas_height: 90,
        });
        assert_eq!(FrameSpec::paused_full_scale(&scene, &output(narrow)), 4.);
        assert_eq!(FrameSpec::paused_full_scale(&scene, &output(zoomed)), 1.);
    }

    /// The composition-size surface is admitted whole within a byte budget,
    /// or not at all: an 8K composition in a 4K or 1440p drawable draws at
    /// the drawable's size, a UHD one in a 1080p drawable at its own.
    #[test]
    fn paused_supersampling_fits_a_byte_budget() {
        let scene = |w: u32, h: u32| {
            let mut project: serde_json::Value =
                serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
            (project["w"], project["h"]) = (w.into(), h.into());
            let doc = pm_model::json::parse(&project.to_string()).unwrap();
            Scene {
                doc: Arc::new(pm_model::Document::new(
                    Some(pm_model::canonical::canonical_project(doc)),
                    1,
                )),
                comp: "Pshapes-primitives-root".into(),
                media: Default::default(),
                content: 0,
            }
        };
        let output = |width, height| Output {
            width,
            height,
            display: 1,
            headroom: 1.,
            crop: None,
        };
        let fill =
            |w: u32, h: u32| FrameSpec::paused_full_scale(&scene(w, h), &output(w / 2, h / 2));
        assert_eq!(
            FrameSpec::paused_full_scale(&scene(7680, 4320), &output(3840, 2160)),
            1.
        );
        assert_eq!(
            FrameSpec::paused_full_scale(&scene(7680, 4320), &output(2560, 1440)),
            1.
        );
        assert_eq!(fill(3840, 2160), 2.);
        let bytes = 3841. * 2161. * 8.;
        assert!(bytes <= render::PAUSED_FULL_BYTES as f64);
    }

    /// Automatic's HD proxy plays a large clip, but a paused frame reads
    /// its original, as an export does; a proxy chosen in View › Media
    /// stays when paused.
    #[test]
    fn an_automatic_proxy_stands_in_only_while_playing() {
        let fixture = |name: &str| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../engine/fixtures")
                .join(name)
        };
        let (original, proxy) = (fixture("h264-aac.mp4"), fixture("hevc.mp4"));
        let project = pm_model::json::parse(
            r##"{"id":"P","name":"P","compId":"C","compName":"C","w":160,"h":90,"fps":30,"dur":3,"bg":"#000000",
            "layers":[{"id":"v","type":"video","name":"large clip","from":0,"dur":3,"on":true,"solo":false,"mblur":false,
              "p":{"position.x":{"v":80,"kf":[],"expr":null},"position.y":{"v":45,"kf":[],"expr":null},"anchor.x":{"v":0,"kf":[],"expr":null},"anchor.y":{"v":0,"kf":[],"expr":null},"scale.x":{"v":100,"kf":[],"expr":null},"scale.y":{"v":100,"kf":[],"expr":null},"rotation":{"v":0,"kf":[],"expr":null},"opacity":{"v":100,"kf":[],"expr":null}},
              "fx":[],"masks":[],"d":{"asset":"large-clip","trim":0,"speed":1,"w":160,"h":90}}],
            "comps":{},"assets":{"large-clip":{"id":"large-clip","type":"video","name":"large clip","w":160,"h":90,"dur":3}},
            "markers":[],"work":[0,3],"params":{}}"##,
        )
        .unwrap();
        #[cfg(feature = "kernel")]
        let snapshot = pm_kernel::document::DocumentStore::new_untitled(project).snapshot();
        #[cfg(not(feature = "kernel"))]
        let snapshot = Snapshot {
            doc_id: 97,
            revision: 1,
            document: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(project)),
                1,
            )),
        };
        let mut media = MediaCatalog::default();
        media
            .originals
            .insert("large-clip".into(), original.clone());
        media.proxies.insert("large-clip".into(), proxy.clone());
        media.automatic_proxies.insert("large-clip".into());
        let service =
            EngineService::start(pm_gpu::Gpu::new("automatic-proxy-test").unwrap()).unwrap();
        let v = service
            .viewer(ViewerConfig {
                composition: snapshot.open_comp().unwrap(),
                snapshot,
                media,
                output: Output {
                    width: 160,
                    height: 90,
                    display: 1,
                    headroom: 1.,
                    crop: None,
                },
                layer: None,
                rate: pm_time::Rate::FPS_30,
                end: Some(3 * pm_protocol::common::TICKS_PER_SECOND),
                audio: false,
            })
            .unwrap();
        // The clip's file in this test's viewer requests of `moment`.
        let read = |moment: Moment| {
            crate::render::MEDIA_SAMPLED
                .lock()
                .unwrap()
                .iter()
                .filter(|(exact, m, _)| !exact && *m == moment)
                .filter_map(|(_, _, paths)| paths.paths.get("large-clip").cloned())
                .filter(|p| *p == original || *p == proxy)
                .collect::<Vec<_>>()
        };
        wait_for(&v, "a paused frame", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.gpu_ok && f.moment == Moment::Paused)
        });
        assert_eq!(v.status().proxied, 1, "Automatic plays the proxy");
        let paused = read(Moment::Paused);
        assert!(
            !paused.is_empty() && paused.iter().all(|p| *p == original),
            "{paused:?}"
        );
        v.command(TransportCommand::Play {
            rate: pm_protocol::common::PlaybackRate::NORMAL,
        })
        .unwrap();
        wait_for(&v, "a playing frame", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.moment == Moment::Playing)
        });
        let playing = read(Moment::Playing);
        assert!(
            !playing.is_empty() && playing.iter().all(|p| *p == proxy),
            "{playing:?}"
        );
        v.command(TransportCommand::Pause).unwrap();
        // A proxy a person chose stays when paused.
        v.media_mode(MediaMode::ProxyOnly).unwrap();
        let stamp = v.stamp().unwrap();
        wait_for(&v, "a paused frame in Proxy Only", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.stamp == stamp && f.moment == Moment::Paused)
        });
        assert_eq!(read(Moment::Paused).last(), Some(&proxy));
        v.media_mode(MediaMode::ProxyPreferred).unwrap();
        let stamp = v.stamp().unwrap();
        wait_for(&v, "a paused frame in Proxy Preferred", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.stamp == stamp && f.moment == Moment::Paused)
        });
        assert_eq!(read(Moment::Paused).last(), Some(&proxy));
        // Optimized media outranks the original in Automatic, paused too.
        let optimized = fixture("h264-aac.mov");
        let mut media = MediaCatalog::default();
        media
            .originals
            .insert("large-clip".into(), original.clone());
        media.proxies.insert("large-clip".into(), proxy.clone());
        media
            .optimized
            .insert("large-clip".into(), optimized.clone());
        media.automatic_proxies.insert("large-clip".into());
        v.media_mode(MediaMode::Automatic).unwrap();
        v.media(media).unwrap();
        let stamp = v.stamp().unwrap();
        wait_for(&v, "a paused frame with optimized media", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.stamp == stamp && f.moment == Moment::Paused)
        });
        let last = crate::render::MEDIA_SAMPLED
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(exact, m, _)| !exact && *m == Moment::Paused)
            .and_then(|(_, _, paths)| paths.paths.get("large-clip").cloned());
        assert_eq!(last, Some(optimized));
        v.retire();
    }

    /// A capture taken during a reduced drag (explicit Half quality, a
    /// scrub) reads an exact frame: paused, at least full resolution (the
    /// composition's size, as an export), final resampler. (Other tests'
    /// captures share the record.)
    #[test]
    fn a_capture_during_a_reduced_drag_reads_the_exact_frame() {
        use pm_render::sampling::Sampling;
        let service = EngineService::start(pm_gpu::Gpu::new("capture-drag-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        v.quality(crate::policy::Quality::Half).unwrap();
        wait_for(&v, "a first frame", |s| {
            s.frame.as_ref().is_some_and(|f| f.gpu_ok)
        });
        let captured = || {
            crate::render::SAMPLED
                .lock()
                .unwrap()
                .iter()
                .filter(|(exact, ..)| *exact)
                .map(|(_, moment, scale, sampling)| (*moment, *scale, *sampling))
                .collect::<Vec<_>>()
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut second = 1;
        while captured().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "no capture landed in a drag"
            );
            v.scrub(Ticks::from_whole_seconds(second % 3)).unwrap();
            v.capture_next().unwrap();
            drawn(&v);
            second += 1;
        }
        assert!(
            captured()
                .iter()
                .all(|&(moment, scale, sampling)| moment == Moment::Paused
                    && scale >= 1.
                    && sampling == Sampling::Final),
            "{:?}",
            captured()
        );
        v.retire();
    }

    /// Overlay geometry from a zero-size composition (a division by zero)
    /// or a transform too large for a float is left out line by line; what
    /// can be drawn is kept, and nothing fails.
    #[test]
    fn overlay_lines_that_cannot_be_drawn_are_left_out() {
        let line = |x: f32| OverlayLine {
            ends: [x, 0., 1., 1.],
            color: [1., 1., 1., 1.],
        };
        let zero_width = (1.0f64 / 0.0) as f32;
        let beyond_float = (1e42f64 / 1920.) as f32;
        assert!(zero_width.is_infinite() && beyond_float.is_infinite());
        let bad = [line(zero_width), line(f32::NAN), line(beyond_float)];
        assert!(OverlayGeometry::new(bad.to_vec()).is_err());
        let kept = OverlayGeometry::lossy(bad.into_iter().chain([line(0.25), line(0.5)]));
        let firsts: Vec<f32> = kept.lines.iter().map(|l| l.ends[0]).collect();
        assert_eq!(firsts, [0.25, 0.5]);
        let many = OverlayGeometry::lossy((0..500).map(|_| line(0.5)));
        assert_eq!(many.lines.len(), OverlayGeometry::MAX_LINES);
    }

    /// A viewer whose document has a raster override answering through the
    /// returned log: every shape asks the realm, which answers only what the
    /// test answers.
    fn overridden(
        service: &EngineService,
        v: &Viewer,
    ) -> (
        Arc<compat::CompatStore>,
        Arc<Mutex<Vec<pm_protocol::snapshot::CompatNeed>>>,
    ) {
        let store = service.compat(v.stamp().unwrap().doc_id);
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        store.set_sink(Some(Arc::new(
            move |_, _, needs: Vec<pm_protocol::snapshot::CompatNeed>| {
                log.lock().unwrap().extend(needs)
            },
        )));
        store.set_state(true, 1, 1);
        (store, asked)
    }

    /// Showing another composition of an open document (the editor's
    /// composition switch, paused) never closes the document, whatever the
    /// worker is doing at that moment (here: playing another window's
    /// project, so it looks at its views all the time). Nothing of the
    /// project is released, and its override layers still ask, and are drawn
    /// from, the store the window's extensions hold from before the switch.
    #[test]
    fn a_composition_switch_keeps_the_document_and_its_override_answers() {
        let service = EngineService::start(pm_gpu::Gpu::new("comp-switch-test").unwrap()).unwrap();
        let other = shapes_viewer(&service, None);
        other
            .command(TransportCommand::SetLoop {
                range: Some(pm_protocol::common::TickRange {
                    start: seconds(0.),
                    end: seconds(2.),
                }),
            })
            .unwrap();
        other
            .command(TransportCommand::Play {
                rate: pm_protocol::common::PlaybackRate::NORMAL,
            })
            .unwrap();
        let config = shapes_config(None);
        let mut v = service.viewer(config.clone()).unwrap();
        let first = wait_for(&v, "a first frame", |s| {
            s.frame.as_ref().is_some_and(|f| f.gpu_ok)
        });
        let doc = first.frame.unwrap().stamp.doc_id;
        assert_ne!(other.stamp().unwrap().doc_id, doc);
        let (store, asked) = overridden(&service, &v);
        for switch in 1..=40i64 {
            v = service.replace_viewer(&v, config.clone()).unwrap();
            // The frame at a new time lands only once every override layer
            // in it was asked for and answered through the window's store.
            let at = Ticks::new(switch * pm_protocol::common::TICKS_PER_SECOND / 30).unwrap();
            v.command(TransportCommand::Seek {
                time: at,
                mode: pm_protocol::clock::SeekMode::Exact,
            })
            .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            let mut answered = 0;
            loop {
                let needs = std::mem::take(&mut *asked.lock().unwrap());
                answered += needs.len();
                if !needs.is_empty() {
                    store.answer_all(
                        1,
                        1,
                        needs
                            .into_iter()
                            .map(|n| (n, compat::Reply::Draw(compat::hidden_raster())))
                            .collect(),
                    );
                }
                let status = v.status();
                if answered > 0
                    && status
                        .frame
                        .as_ref()
                        .is_some_and(|f| f.gpu_ok && f.time == at && !f.stand_ins)
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "switch {switch}: {answered} override layers asked the window's store: {status:?}"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(
                Arc::ptr_eq(&service.compat(doc), &store),
                "switch {switch} gave the document another answer store"
            );
            assert_eq!(
                service.memory_usage().project_releases,
                0,
                "switch {switch} released the open project's memory"
            );
        }
        v.retire();
        // A document whose last view closed is released, once, and its
        // answers go with it: a later view of it would ask another store,
        // which the window's extensions never hear from. That is why a
        // switch replaces the view instead of retiring it first.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while service.memory_usage().project_releases != 1 {
            assert!(std::time::Instant::now() < deadline, "never released");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!Arc::ptr_eq(&service.compat(doc), &store));
        other.retire();
    }

    /// A capture is an exact read: full resolution and paused whatever the
    /// quality and moment, so Half never reaches the pixels delivered.
    #[test]
    fn a_capture_reads_the_exact_full_resolution_frame_at_half_quality() {
        let service = EngineService::start(pm_gpu::Gpu::new("capture-half-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        v.quality(crate::policy::Quality::Half).unwrap();
        wait_for(&v, "a first frame", |s| {
            s.frame.as_ref().is_some_and(|f| f.gpu_ok)
        });
        v.scrub(seconds(1.)).unwrap();
        v.capture_next().unwrap();
        let s = wait_for(&v, "captured pixels", |s| {
            s.frame.as_ref().is_some_and(|f| f.pixels.is_some())
        });
        let f = s.frame.unwrap();
        // Exact is Full: the composition's size (640 × 360 in 160 × 90).
        assert_eq!(
            (f.scale, f.moment, f.stand_ins),
            (4., Moment::Paused, false)
        );
        v.retire();
    }

    /// A capture whose override layers are never answered is not delivered
    /// from a stand-in: it waits its bound, then fails saying why.
    #[test]
    fn a_capture_waiting_on_unanswered_layers_fails_after_its_bound() {
        let service = EngineService::start(pm_gpu::Gpu::new("capture-wait-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        wait_for(&v, "a first frame", |s| {
            s.frame.as_ref().is_some_and(|f| f.gpu_ok)
        });
        let (_store, asked) = overridden(&service, &v);
        // A time not drawn yet (no cached frame from before the override).
        v.scrub(seconds(2.)).unwrap();
        v.capture_next().unwrap();
        let start = std::time::Instant::now();
        let deadline = start + Duration::from_secs_f64(CAPTURE_WAIT_SECONDS + 10.);
        let failed = loop {
            let s = v.status();
            assert!(
                s.frame.as_ref().is_none_or(|f| f.pixels.is_none()),
                "pixels delivered without the layers"
            );
            if let Some(why) = s.capture_failed {
                break why;
            }
            assert!(std::time::Instant::now() < deadline, "{s:?}");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(start.elapsed().as_secs_f64() >= CAPTURE_WAIT_SECONDS - 0.5);
        assert!(failed.contains("could not be captured"), "{failed}");
        assert!(
            !asked.lock().unwrap().is_empty(),
            "the layers were asked for"
        );
        v.retire();
    }

    /// A paused picture waiting for its override answers asks for its
    /// refinement's (at the composition's scale, 4× here) right behind its
    /// own, so they are drawn while it finishes: the moment its batch is
    /// answered, the refinement's goes, before the refinement is tried.
    #[test]
    fn a_paused_picture_asks_for_its_refinements_answers_behind_its_own() {
        let service = EngineService::start(pm_gpu::Gpu::new("prefetch-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        wait_for(&v, "a first frame", |s| {
            s.frame.as_ref().is_some_and(|f| f.gpu_ok)
        });
        let (store, asked) = overridden(&service, &v);
        v.command(TransportCommand::Seek {
            time: seconds(2.),
            mode: pm_protocol::clock::SeekMode::Exact,
        })
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while asked.lock().unwrap().is_empty() {
            assert!(std::time::Instant::now() < deadline, "never asked");
            std::thread::sleep(Duration::from_millis(2));
        }
        let first: Vec<_> = std::mem::take(&mut *asked.lock().unwrap());
        let scale = first[0].scale;
        assert!(
            first
                .iter()
                .all(|n| n.time == seconds(2.) && n.scale == scale)
        );
        // Answering the picture's batch sends the queued refinement batch.
        store.answer_all(
            1,
            1,
            first
                .into_iter()
                .map(|n| (n, compat::Reply::Draw(compat::hidden_raster())))
                .collect(),
        );
        let ahead = asked.lock().unwrap().clone();
        assert!(
            !ahead.is_empty(),
            "the refinement's answers were not asked ahead"
        );
        assert!(
            ahead
                .iter()
                .all(|n| n.time == seconds(2.) && (n.scale - 4. * scale).abs() < 1e-9),
            "{ahead:?}"
        );
        v.retire();
    }

    /// Override layers that keep showing an earlier frame while playing are
    /// still waited for: the viewer says so past a few seconds, and frames
    /// with stand-ins don't silence it.
    #[test]
    fn layers_left_on_an_earlier_frame_keep_the_wait_note() {
        let service = EngineService::start(pm_gpu::Gpu::new("late-note-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        wait_for(&v, "a first frame", |s| {
            s.frame.as_ref().is_some_and(|f| f.gpu_ok)
        });
        let (store, asked) = overridden(&service, &v);
        v.scrub(Ticks::ZERO).unwrap();
        // The first frame's layers are answered (passed through); later ones
        // never are.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while asked.lock().unwrap().is_empty() {
            assert!(std::time::Instant::now() < deadline, "never asked");
            std::thread::sleep(Duration::from_millis(5));
        }
        let first: Vec<_> = std::mem::take(&mut *asked.lock().unwrap());
        store.answer_all(
            1,
            1,
            first
                .into_iter()
                .map(|n| (n, compat::Reply::Draw(compat::hidden_raster())))
                .collect(),
        );
        v.command(TransportCommand::SetLoop {
            // No frame in the loop is answered: every one shows the 0 s answers.
            range: Some(pm_protocol::common::TickRange {
                start: seconds(0.5),
                end: seconds(2.5),
            }),
        })
        .unwrap();
        v.command(TransportCommand::Play {
            rate: pm_protocol::common::PlaybackRate::NORMAL,
        })
        .unwrap();
        let s = wait_for(&v, "the wait note", |s| {
            s.error
                .as_deref()
                .is_some_and(|e| e.contains("earlier frame"))
        });
        assert!(s.frame.is_some_and(|f| f.stand_ins));
        v.retire();
    }

    /// A drag at full resolution settles too: once the playhead rests, the
    /// same time is drawn again exact (paused), so no drag stand-in stays.
    #[test]
    fn a_full_resolution_drag_settles_to_an_exact_frame() {
        let service = EngineService::start(pm_gpu::Gpu::new("full-drag-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        v.quality(crate::policy::Quality::Full).unwrap();
        wait_for(&v, "a first frame", |s| {
            s.frame.as_ref().is_some_and(|f| f.gpu_ok)
        });
        let at = seconds(1.);
        v.scrub(at).unwrap();
        wait_for(&v, "the dragged frame", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.time == at && f.moment == Moment::Scrub)
        });
        wait_for(&v, "the settled frame", |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.time == at && f.moment == Moment::Paused && f.gpu_ok)
        });
        v.retire();
    }

    /// Auto never plays softer than the paused picture: however slow the
    /// frames, playback renders at full resolution (late frames are dropped).
    #[test]
    fn auto_plays_slow_frames_at_full_resolution() {
        let service = EngineService::start(pm_gpu::Gpu::new("auto-play-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        service
            .shared
            .slow_frames
            .store(true, std::sync::atomic::Ordering::Release);
        wait_for(&v, "a first frame", |s| {
            s.frame.as_ref().is_some_and(|f| f.gpu_ok)
        });
        v.command(TransportCommand::SetLoop {
            range: Some(pm_protocol::common::TickRange {
                start: Ticks::ZERO,
                end: Ticks::from_whole_seconds(2),
            }),
        })
        .unwrap();
        v.command(TransportCommand::Play {
            rate: pm_protocol::common::PlaybackRate::NORMAL,
        })
        .unwrap();
        let mut seen = std::collections::BTreeSet::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while seen.len() < 12 {
            assert!(std::time::Instant::now() < deadline, "{:?}", v.status());
            let status = v.status();
            if let Some(f) = status
                .frame
                .filter(|f| f.gpu_ok && f.moment == Moment::Playing)
            {
                assert_eq!(f.scale, 1., "played frame {} reduced", seen.len());
                assert_eq!(status.effective_quality, 1.);
                seen.insert(f.time);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        v.retire();
    }

    #[test]
    fn a_slow_drag_draws_reduced_then_its_stopped_frame_exact_as_a_new_picture() {
        let service = EngineService::start(pm_gpu::Gpu::new("drag-test").unwrap()).unwrap();
        let shown = |v: &Viewer| {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while !v.status().frame.is_some_and(|f| f.gpu_ok) {
                assert!(std::time::Instant::now() < deadline, "{:?}", v.status());
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let v = shapes_viewer(&service, None);
        v.adapt_drags(true).unwrap();
        service
            .shared
            .slow_frames
            .store(true, std::sync::atomic::Ordering::Release);
        shown(&v);
        let at = Ticks::from_whole_seconds(1);
        v.scrub(at).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let reduced = loop {
            let status = v.status();
            if let Some(f) = status.frame.filter(|f| f.time == at && f.gpu_ok) {
                break f;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(
            reduced.scale,
            crate::policy::AUTO_FLOOR,
            "a slow drag is reduced"
        );
        // Settled: the same time again, exact, as a picture of its own (a
        // new stamp: the window tells extensions `frame:rendered` again).
        let exact = loop {
            let status = v.status();
            // Exact: Full, the composition's size (scale 4 here).
            if let Some(f) = status.frame.filter(|f| f.time == at && f.scale >= 1.) {
                break f;
            }
            assert!(std::time::Instant::now() < deadline, "never drawn exact");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(exact.stamp.sequence > reduced.stamp.sequence);
        // A viewer that does not adapt keeps every paused frame exact.
        let plain = shapes_viewer(&service, None);
        shown(&plain);
        plain.scrub(at).unwrap();
        let f = loop {
            if let Some(f) = plain.status().frame.filter(|f| f.time == at) {
                break f;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(f.scale, 1.);
        v.retire();
        plain.retire();
    }

    /// A validation error fails the frame that raised it, visibly, without a
    /// rebuild, and that frame is never cached or reused.
    #[test]
    fn a_gpu_validation_error_fails_its_frame_and_never_caches_it() {
        let service = EngineService::start(pm_gpu::Gpu::new("validation-test").unwrap()).unwrap();
        let v = shapes_viewer(&service, None);
        v.capture_next().unwrap();
        let good = drawn(&v).frame.unwrap().pixels.unwrap();
        service
            .shared
            .invalid_call
            .store(true, std::sync::atomic::Ordering::Release);
        service.shared.wake.try_send(()).ok();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while service
            .shared
            .invalid_call
            .load(std::sync::atomic::Ordering::Acquire)
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        v.scrub(Ticks::from_whole_seconds(1)).unwrap();
        let refused = v.stamp().unwrap();
        let error = loop {
            let status = v.status();
            if let Some(error) = status.error {
                break error;
            }
            assert!(
                status.frame.as_ref().is_none_or(|f| f.stamp != refused),
                "the refused frame was shown"
            );
            assert!(std::time::Instant::now() < deadline, "{status:?}");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(error.contains("refused this frame"), "{error}");
        assert_eq!(
            v.status().device_losses,
            0,
            "a validation error is not a loss"
        );
        v.scrub(Ticks::ZERO).unwrap();
        v.scrub(Ticks::from_whole_seconds(1)).unwrap();
        v.capture_next().unwrap();
        let again = drawn(&v);
        assert_eq!(again.error, None);
        assert!(
            !again.frame.as_ref().unwrap().cache_hit,
            "the refused frame was cached"
        );
        v.scrub(Ticks::ZERO).unwrap();
        v.capture_next().unwrap();
        assert_eq!(drawn(&v).frame.unwrap().pixels.unwrap(), good);
    }
}

/// What kind of transient refusal a frame met, for the diagnostic trace
/// (never its message: messages can name documents, media or paths).
fn refusal_kind(error: &anyhow::Error) -> &'static str {
    match error.downcast_ref::<pm_render::RenderError>() {
        Some(pm_render::RenderError::CompatPending(_)) => "raster-override",
        Some(pm_render::RenderError::WatchdogPending) => "watchdog",
        Some(pm_render::RenderError::ShaderFallbackPending(_)) => "shader-fallback",
        _ if error.is::<render::CompilerBusy>() => "shader-compiler",
        _ if error.is::<render::DiskFramePending>() => "disk-frame",
        _ if error.is::<render::DefinitionsPending>() => "definitions",
        _ => "fonts",
    }
}
