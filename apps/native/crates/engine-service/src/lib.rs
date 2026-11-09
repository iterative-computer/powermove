//! In-process native rendering. No Node, napi entrypoints, bridge or GPUI.
//! Callers own below-slot placement and pump native layers on the main thread.
pub mod audio;
pub mod clock;
pub mod color;
pub mod compat;
pub mod frame_codec;
pub mod jobs;
pub mod layers;
pub mod media;
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
    /// Always Play Original: these assets never play a proxy or optimized
    /// media, in any mode.
    pub play_original: std::collections::BTreeSet<String>,
}
/// What the viewer plays for each clip (View › Media). Exports read
/// originals whatever this says; only an explicit draft export uses proxies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaMode {
    /// Originals, and a proxy for a clip whose original is missing.
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
        pm_export::render::MediaPaths { paths }
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
        Some(format!(
            "Effect not installed: {}, bypassed on {}{}",
            missing.effect,
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
    overlays: OverlayGeometry,
    overlay_time: Option<Ticks>,
    locked_provider: Option<LockedOverlayProvider>,
    commands: VecDeque<TransportCommand>,
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
    media_mode: MediaMode,
    paths: Arc<pm_export::render::MediaPaths>,
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
            _ => {}
        }
        self.commands.push_back(cmd);
        self.stamp.sequence += 1;
        self.dirty = true;
        Ok(())
    }
}
#[derive(Default)]
struct State {
    views: BTreeMap<u64, View>,
    next: u64,
    content: u64,
    stop: bool,
    subscribers: Vec<async_channel::Sender<()>>,
}
type ScopeTap = (
    pm_render::scopes::ScopeConfig,
    async_channel::Sender<ScopeResult>,
);
struct Shared {
    state: Mutex<State>,
    wake: std::sync::mpsc::SyncSender<()>,
    interactive: jobs::Activity,
    /// The worker engine's current Metal device; a GPU loss replaces it.
    device: Mutex<metal::Device>,
    /// The viewer the Scopes panel analyzes, and where results go.
    scopes: Mutex<Option<(u64, ScopeTap)>>,
    disk: render_cache::DiskFrames,
    #[cfg(test)]
    before_submit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    lose_device: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    invalid_call: std::sync::atomic::AtomicBool,
    /// Every still costs more than a refresh (drag adaptation tests).
    #[cfg(test)]
    slow_stills: std::sync::atomic::AtomicBool,
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
    /// must not consume the edit's measurement token.
    pub fn input_event(&self, input: f64) {
        if let Some(v) = self.shared.state.lock().unwrap().views.get_mut(&self.id) {
            v.pending_input = Some(input);
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
        v.paths = Arc::new(pm_export::render::MediaPaths {
            paths: media.viewer_paths(v.media_mode),
        });
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
        v.paths = Arc::new(pm_export::render::MediaPaths {
            paths: v.config.media.viewer_paths(mode),
        });
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
        state.views.remove(&self.id);
        state.notify();
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
        let layer_rasters = compat::LayerRasters::start(
            gpu.device.clone(),
            gpu.queue.clone(),
            Some(fonts.clone()),
        )?;
        let mut engine = render::Engine::with_shader_cache(gpu, shader_cache)?;
        engine.set_fonts(fonts);
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
    fn start_engine(device: metal::Device, mut engine: render::Engine) -> Result<Self> {
        let (wake, wakes) = std::sync::mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake,
            interactive: jobs::Activity::default(),
            device: Mutex::new(device),
            scopes: Mutex::new(None),
            disk: render_cache::DiskFrames::default(),
            #[cfg(test)]
            before_submit: Mutex::new(None),
            #[cfg(test)]
            lose_device: Default::default(),
            #[cfg(test)]
            invalid_call: Default::default(),
            #[cfg(test)]
            slow_stills: Default::default(),
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
            .spawn(move || run(engine, worker, wakes))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            compat,
            layer_rasters: None,
        })
    }
    /// Call after the main-thread font broker's generation changes.
    pub fn fonts_changed(&self) {
        let mut state = self.shared.state.lock().unwrap();
        for v in state.views.values_mut() {
            v.stamp.sequence += 1;
            v.dirty = true;
        }
        self.shared.wake.try_send(()).ok();
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
        ensure!(state.views.len() < 8, "at most eight concurrent viewers");
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
        let paths = Arc::new(pm_export::render::MediaPaths {
            paths: config.media.viewer_paths(MediaMode::default()),
        });
        let proxied = config.media.proxied(MediaMode::default());
        state.views.insert(
            id,
            View {
                config,
                stamp,
                content,
                dirty: true,
                capture: false,
                overlays: OverlayGeometry::default(),
                overlay_time: None,
                locked_provider: None,
                commands: VecDeque::new(),
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
                media_mode: MediaMode::default(),
                paths,
            },
        );
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
    /// When the frames started being refused as transient, while they are.
    waiting_since: Option<f64>,
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
    budget_ms: f64,
    submission: render::Submission,
    input: Option<f64>,
    audio_host: Option<f64>,
    pipeline: trace::Pipeline,
}
/// One frame taken from the shared state, drawn without its lock.
struct Work {
    id: u64,
    stamp: Stamp,
    scene: Scene,
    spec: FrameSpec,
    budget_ms: f64,
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
/// every few frames; the frames between draw the nearest answer.
fn ask_ahead(
    engine: &mut render::Engine,
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
/// A GPU lost again within this long of a rebuild backs off between these.
const LOSS_WINDOW: Duration = Duration::from_secs(10);
const LOSS_BACKOFF: (Duration, Duration) = (Duration::from_millis(500), Duration::from_secs(4));
fn run(mut engine: render::Engine, shared: Arc<Shared>, wakes: std::sync::mpsc::Receiver<()>) {
    // Same interactive class as pm-node/present; export remains separately
    // governed. A default-class worker can miss display deadlines under load.
    objc2::rc::autoreleasepool(|_| {
        objc2_foundation::NSThread::currentThread()
            .setQualityOfService(objc2_foundation::NSQualityOfService::UserInteractive);
    });
    engine.set_waker(Some(shared.wake.clone()));
    engine.disk = Some(shared.disk.clone());
    shared.disk.set_wake(shared.wake.clone());
    let mut runtime = BTreeMap::<u64, Runtime>::new();
    let mut flights = VecDeque::<Flight>::new();
    // `PM_ENGINE_TRACE`, or for a development bundle launched without a
    // custom environment (`com.powermove.native.dev.*`) its own
    // `PMEngineTrace` default; a shipped app never reads the default.
    let trace_path = std::env::var_os("PM_ENGINE_TRACE").or_else(|| {
        let bundle = objc2_foundation::NSBundle::mainBundle().bundleIdentifier()?;
        if !bundle.to_string().starts_with("com.powermove.native.dev.") {
            return None;
        }
        let key = objc2_foundation::NSString::from_str("PMEngineTrace");
        // The app's own domain only, never an inherited (global) default.
        objc2_foundation::NSUserDefaults::standardUserDefaults()
            .persistentDomainForName(&bundle)?
            .objectForKey(&key)?
            .downcast::<objc2_foundation::NSString>()
            .ok()
            .map(|path| path.to_string().into())
    });
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
        // Delivers a pending device-lost callback even while idle.
        engine.poll();
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
            if !matches!(
                flights[i].submission.command.status(),
                metal::MTLCommandBufferStatus::Completed | metal::MTLCommandBufferStatus::Error
            ) {
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
                if t.gpu_ok && !t.cache_hit && f.spec.moment == Moment::Playing {
                    r.auto
                        .record((t.completed - t.worker_start) * 1000., f.budget_ms);
                }
                if t.gpu_ok && !t.cache_hit && f.spec.moment != Moment::Playing {
                    let cost = (t.completed - t.worker_start) * 1000.;
                    #[cfg(test)]
                    let cost = if shared
                        .slow_stills
                        .load(std::sync::atomic::Ordering::Acquire)
                    {
                        f64::MAX
                    } else {
                        cost
                    };
                    r.auto
                        .record_still(cost, r.interval * 1000., f.spec.scale);
                }
            }
            if let Some(v) = state.views.get_mut(&f.id) {
                let t = f.submission.trace.lock().unwrap();
                if let Some(recorder) = &recorder && !recorder.record(serde_json::json!({"display":t.display,"worker":t.worker_start,"completed":t.completed,"commit":t.commit,
                        "input":f.input,"audio_host":f.audio_host,"time":f.spec.time.get(),"disk":t.disk,"cache_hit":t.cache_hit,
                        "stale":v.stamp != f.stamp,"viewer_id":f.id,"generation":f.stamp.generation,"revision":f.stamp.revision,"sequence":f.stamp.sequence,"gpu_ok":t.gpu_ok,
                        "request":f.pipeline.request,"actor_ms":f.pipeline.actor_ms,"publication_ms":f.pipeline.publication_ms,
                        "render_start":t.render_start,"render_submitted":t.render_submitted,"render_cpu_ms":t.render_cpu_ms,"work":serde_json::from_str::<serde_json::Value>(&t.work.to_json()).unwrap_or_default(),"drawable_asked":t.drawable_asked,"drawable_acquired":t.drawable_acquired,
                        "encode_end":t.encode_end,"scheduled":t.scheduled,"gpu_start":t.gpu_start,"gpu_done":t.gpu_done,"callback":t.display_callback,"target":t.target,"requested":t.requested,"predicted":t.predicted})) {
                    v.status.error = Some("viewer diagnostic trace overflow/disconnected".into());
                }
                if v.stamp != f.stamp {
                    v.status.stale_rejections += 1;
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
                    pixels: f
                        .submission
                        .readback
                        .as_ref()
                        .map(|_| f.submission.pixels())
                        .or(kept),
                });
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
        if !retired.is_empty() {
            drop(state);
            for id in retired {
                runtime.remove(&id);
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
                    was_playing: false,
                    auto: Default::default(),
                    not_before: 0.,
                    drawable_misses: 0,
                    refusals: 0,
                    waiting_since: None,
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
            while let Some(cmd) = v.commands.pop_front() {
                if let TransportCommand::Seek { mode, .. } = cmd {
                    r.scrub = mode == pm_protocol::clock::SeekMode::Scrub;
                    r.refine_at = None;
                }
                r.transport.command(cmd, (now * 1e6) as i64);
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
                sample.media_time
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
            let scene = Scene {
                doc: v.config.snapshot.document.clone(),
                comp: v.config.composition.clone(),
                media: v.paths.clone(),
                content: v.content,
            };
            cursor = id;
            let moment = if playing {
                Moment::Playing
            } else if r.scrub {
                Moment::Scrub
            } else {
                Moment::Paused
            };
            let budget = frame.max(interval);
            work = Some(Work {
                id,
                stamp: v.stamp,
                scene,
                spec: FrameSpec {
                    time,
                    output: v.config.output,
                    moment,
                    // A capture reads the exact picture: Auto never adapts it.
                    scale: v.quality.scale(
                        &r.auto,
                        if capture || (moment == Moment::Scrub && !v.adapt_drags) {
                            Moment::Paused
                        } else {
                            moment
                        },
                    ),
                    decode_deadline: playing.then(|| {
                        std::time::Instant::now() + Duration::from_secs_f64(budget * DECODE_SHARE)
                    }),
                },
                budget_ms: budget * 1000.,
                layer: v.config.layer.clone(),
                overlays: if v.overlay_time.is_none_or(|t| t == time) {
                    v.overlays.clone()
                } else {
                    OverlayGeometry::default()
                },
                provider: v.locked_provider.clone(),
                capture: std::mem::take(&mut v.capture),
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
            spec,
            budget_ms,
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
            if shared.disk.observe(
                &scene,
                engine.definitions(),
                overrides,
                engine.tone_identity(),
                stamp.doc_id,
            ) && let Some(recorder) = &recorder
            {
                recorder.record(serde_json::json!({"render_cache_state": true, "at": now}));
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
            let result = provider
                .map_or_else(|| Ok(overlays), |p| p(&scene.doc, &scene.comp, spec.time))
                .and_then(|overlays| {
                    engine.frame(&scene, spec, layer.as_ref(), &overlays, capture, trace)
                });
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
                        v.status.error = None;
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
                        v.status.effective_quality = spec.scale;
                        if let Some(r) = runtime.get_mut(&id) {
                            if spec.moment == Moment::Scrub && spec.scale < 1. {
                                r.refine_at = Some(trace::now() + policy::SCRUB_SETTLE_SECONDS);
                            }
                            r.stand_in = s.stand_ins.then_some(landed_before);
                            r.drawable_misses = 0;
                            r.refusals = 0;
                            r.waiting_since = None;
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
                        flights.push_back(Flight {
                            id,
                            stamp,
                            spec,
                            budget_ms,
                            submission: s,
                            input,
                            audio_host,
                            pipeline,
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
                        if let (Some(needs), Some(store)) = (pending.clone(), engine.compat()) {
                            store.request(needs);
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
                        }
                    }
                    Err(e) => {
                        v.status.error = Some(e.to_string());
                        if let Some(r) = runtime.get_mut(&id) {
                            r.waiting_since = None;
                        }
                    }
                }
            }
            state.notify();
            drop(state);
            if spec.moment == Moment::Playing {
                let rate = runtime.get(&id).map(|r| r.transport.rate());
                ask_ahead(&mut engine, &scene, spec, rate);
            }
            if let Some(at) = prepare_at {
                let videos = engine.prepare(&scene, at, spec);
                if let Some(r) = runtime.get_mut(&id) {
                    r.lookahead.planned(at, videos, frame_ticks);
                }
            } else if let Some(at) = refine_at {
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
            let _ = wakes.recv_timeout(timeout);
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
    pub fn new() -> Result<Self> {
        Ok(Self {
            engine: render::Engine::new(pm_gpu::Gpu::new("pm-headless")?)?,
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

#[cfg(test)]
mod worker_tests {
    use super::*;
    use foreign_types::ForeignTypeRef;
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
                assert_eq!(frame.stamp, v.stamp().unwrap());
                assert!(status.stale_rejections >= 1);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn shapes_viewer(service: &EngineService, layer: Option<metal::MetalLayer>) -> Viewer {
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
        service
            .viewer(ViewerConfig {
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
            })
            .unwrap()
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

    /// A removed or reset GPU: the worker rebuilds its device, points the
    /// viewer's layer and the service's device at it, and the viewer draws
    /// again without anyone reopening it.
    #[test]
    fn a_lost_device_is_rebuilt_and_the_viewer_draws_again() {
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

    /// A slow project's drag draws reduced; the stopped frame is drawn again
    /// exact, as a picture of its own.
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
            .slow_stills
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
        assert_eq!(reduced.scale, crate::policy::AUTO_FLOOR, "a slow drag is reduced");
        // Settled: the same time again, exact, as a picture of its own (a
        // new stamp: the window tells extensions `frame:rendered` again).
        let exact = loop {
            let status = v.status();
            if let Some(f) = status.frame.filter(|f| f.time == at && f.scale == 1.) {
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
        _ => "fonts",
    }
}
