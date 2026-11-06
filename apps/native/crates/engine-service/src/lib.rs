//! In-process native rendering. No Node, napi entrypoints, bridge or GPUI.
//! Callers own below-slot placement and pump native layers on the main thread.
pub mod color;
pub mod jobs;
pub mod layers;
pub mod policy;
mod present;
mod render;
pub mod resources;
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
pub use pm_node::present::document::MediaMap;
use pm_protocol::{
    clock::{AudioLevels, ClockSample, TransportCommand},
    common::{Size, Ticks},
};
pub use policy::{Crop, Moment, Output, Quality, Stamp};
pub use render::{FrameSpec, Scene, metal_device};
use std::{
    collections::{BTreeMap, VecDeque},
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
    pad: [u32; 2],
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
            pad: [0; 2],
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
        let (first, second) = match mode {
            MediaMode::Automatic => (&self.originals, Some(&self.proxies)),
            MediaMode::Originals => (&self.originals, None),
            MediaMode::ProxyPreferred => (&self.proxies, Some(&self.originals)),
            MediaMode::ProxyOnly => (&self.proxies, None),
        };
        let mut paths = second.cloned().unwrap_or_default();
        paths.extend(first.iter().map(|(k, v)| (k.clone(), v.clone())));
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
    /// Layers missing from the frame on screen, with why. The rest is drawn.
    pub layer_failures: Vec<pm_render::LayerFailure>,
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
struct Shared {
    state: Mutex<State>,
    wake: std::sync::mpsc::SyncSender<()>,
    interactive: Arc<std::sync::atomic::AtomicBool>,
    /// The worker engine's current Metal device; a GPU loss replaces it.
    device: Mutex<metal::Device>,
    #[cfg(test)]
    before_submit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    lose_device: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    invalid_call: std::sync::atomic::AtomicBool,
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
        self.shared
            .interactive
            .store(true, std::sync::atomic::Ordering::Release);
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
            self.shared
                .interactive
                .store(true, std::sync::atomic::Ordering::Release);
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
}
impl EngineService {
    /// Start off the UI thread: renderer/shader initialization may take time.
    pub fn start(gpu: pm_gpu::Gpu) -> Result<Self> {
        let device = metal_device(&gpu);
        let engine = render::Engine::new(gpu)?;
        Self::start_engine(device, engine)
    }
    pub fn start_with_fonts(
        gpu: pm_gpu::Gpu,
        fonts: pm_node::fonts::render::RenderFonts,
    ) -> Result<Self> {
        let device = metal_device(&gpu);
        let mut engine = render::Engine::new(gpu)?;
        engine.set_fonts(fonts);
        Self::start_engine(device, engine)
    }
    fn start_engine(device: metal::Device, engine: render::Engine) -> Result<Self> {
        let (wake, wakes) = std::sync::mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake,
            interactive: Arc::default(),
            device: Mutex::new(device),
            #[cfg(test)]
            before_submit: Mutex::new(None),
            #[cfg(test)]
            lose_device: Default::default(),
            #[cfg(test)]
            invalid_call: Default::default(),
        });
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("pm-engine-service".into())
            .spawn(move || run(engine, worker, wakes))?;
        Ok(Self {
            shared,
            thread: Some(thread),
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
    /// Share with ExportHandle::start_scheduled; true defers new export bands.
    pub fn export_activity(&self) -> Arc<std::sync::atomic::AtomicBool> {
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
        self.shared
            .interactive
            .store(false, std::sync::atomic::Ordering::Release);
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
    /// A clock that stops on its own still owes the exact stopped frame.
    was_playing: bool,
    auto: policy::AutoQuality,
    /// A transient refusal: draw again no earlier than this.
    not_before: f64,
    prepared: f64,
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
}
/// How much of a frame's budget a video may wait for its exact frame.
const DECODE_SHARE: f64 = 0.35;
/// Clips this far ahead of the playhead open on background workers.
const PREPARE_AHEAD_SECONDS: f64 = 0.75;
const PREPARE_EVERY_SECONDS: f64 = 0.25;
/// A transient refusal (fonts, GPU watchdog) waits this long to try again.
const RETRY_SECONDS: f64 = 0.016;
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
    let mut runtime = BTreeMap::<u64, Runtime>::new();
    let mut flights = VecDeque::<Flight>::new();
    let recorder =
        std::env::var_os("PM_ENGINE_TRACE").and_then(|path| match trace::Recorder::open(path) {
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
            }
            if let Some(v) = state.views.get_mut(&f.id) {
                let t = f.submission.trace.lock().unwrap();
                if let Some(recorder) = &recorder && !recorder.record(serde_json::json!({"display":t.display,"worker":t.worker_start,"completed":t.completed,"commit":t.commit,
                        "input":f.input,"audio_host":f.audio_host,"time":f.spec.time.get(),"cache_hit":t.cache_hit,
                        "stale":v.stamp != f.stamp,"viewer_id":f.id,"generation":f.stamp.generation,"revision":f.stamp.revision,"sequence":f.stamp.sequence,"gpu_ok":t.gpu_ok,
                        "request":f.pipeline.request,"actor_ms":f.pipeline.actor_ms,"publication_ms":f.pipeline.publication_ms,
                        "render_start":t.render_start,"render_submitted":t.render_submitted,"drawable_asked":t.drawable_asked,"drawable_acquired":t.drawable_acquired,
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
                        .map(|_| f.submission.pixels()),
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
            for (id, snapshot, media, composition, revision, content) in stale {
                if let Some(r) = runtime.get_mut(&id) {
                    r.transport.update(&snapshot, media, &composition);
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
                    was_playing: false,
                    auto: Default::default(),
                    not_before: 0.,
                    prepared: 0.,
                }
            });
            if r.revision != v.stamp.revision || r.content != v.content {
                r.transport.update(
                    &v.config.snapshot,
                    v.config.media.audio_paths(v.media_mode),
                    &v.config.composition,
                );
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
                    scale: v.quality.scale(&r.auto, moment),
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
            });
            break;
        }
        shared.interactive.store(
            !flights.is_empty()
                || work.is_some()
                || state
                    .views
                    .values()
                    .any(|v| v.dirty || !v.commands.is_empty())
                || runtime.values().any(|r| r.transport.running()),
            std::sync::atomic::Ordering::Release,
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
        }) = work
        {
            drop(state);
            let trace = Arc::new(Mutex::new(trace::Trace {
                worker_start: now,
                target: due.unwrap_or(0.),
                requested: plan.filter(|_| !legacy).map_or(0., |p| p.requested),
                predicted: plan.map_or(0., |p| p.shown),
                ..Default::default()
            }));
            engine.mastering = mastering;
            engine.set_resources(resources);
            let landed_before = engine.media_landed();
            let result = provider
                .map_or_else(|| Ok(overlays), |p| p(&scene.doc, &scene.comp, spec.time))
                .and_then(|overlays| {
                    engine.frame(&scene, spec, layer.as_ref(), &overlays, capture, trace)
                });
            if spec.moment == Moment::Playing
                && let Some(r) = runtime.get_mut(&id)
                && now - r.prepared >= PREPARE_EVERY_SECONDS
            {
                r.prepared = now;
                let ahead = (PREPARE_AHEAD_SECONDS * pm_protocol::common::TICKS_PER_SECOND as f64)
                    * f64::from(r.transport.rate().num.signum());
                let at = (spec.time.get() + ahead as i64).max(0);
                engine.prepare(&scene, Ticks::new(at).unwrap_or(spec.time), spec);
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
                // A capture asked for belongs to the next frame that lands.
                if v.stamp != stamp {
                    v.status.stale_rejections += 1;
                    v.capture |= capture;
                    continue;
                }
                match result {
                    Ok(Some(s)) => {
                        v.status.error = None;
                        v.status.stand_in_frames +=
                            u64::from(s.stand_ins && spec.moment == Moment::Playing);
                        v.status.effective_quality = spec.scale;
                        if let Some(r) = runtime.get_mut(&id) {
                            r.stand_in = s.stand_ins.then_some(landed_before);
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
                    }
                    Err(e) if render::transient(&e) => {
                        v.capture |= capture;
                        v.dirty = true;
                        v.input = input;
                        if let Some(r) = runtime.get_mut(&id) {
                            r.not_before = trace::now() + RETRY_SECONDS;
                        }
                    }
                    Err(e) => {
                        v.status.error = Some(e.to_string());
                    }
                }
            }
            state.notify();
        } else {
            let waiting = runtime
                .values()
                .any(|r| r.stand_in.is_some() || r.not_before > now);
            let busy =
                waiting || !flights.is_empty() || runtime.values().any(|r| r.transport.running());
            let timeout = if waiting {
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
    pub fn with_fonts(fonts: pm_node::fonts::render::RenderFonts) -> Result<Self> {
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
