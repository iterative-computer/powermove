//! In-process native rendering. No Node, napi entrypoints, bridge or GPUI.
//! Callers own below-slot placement and pump native layers on the main thread.
pub mod color;
pub mod jobs;
pub mod layers;
pub mod policy;
mod render;
pub mod resources;
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
pub use policy::{Output, Stamp};
pub use render::{Scene, metal_device};
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
    pad: [u32; 3],
    output: [f32; 4],
    lines: [OverlayLine; 64],
}
impl OverlayGeometry {
    pub fn new(lines: Vec<OverlayLine>) -> Result<Self> {
        ensure!(lines.len() <= 64, "at most 64 locked overlay lines");
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
            pad: [0; 3],
            output: [0.; 4],
            lines: [OverlayLine::default(); 64],
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
#[derive(Clone, Default)]
pub struct MediaCatalog {
    pub originals: BTreeMap<String, std::path::PathBuf>,
    pub proxies: BTreeMap<String, std::path::PathBuf>,
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
    pub fn audio_paths(&self) -> MediaMap {
        MediaMap(Arc::new(
            self.originals
                .iter()
                .map(|(k, v)| (k.clone(), Some(v.clone())))
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
    /// Playing or waiting for the audio epoch: the host should keep its
    /// display-phase source running so the start can align to vsync.
    pub wants_display: bool,
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
    #[cfg(test)]
    before_submit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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
                v.overlays = OverlayGeometry::default();
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
        v.config.media = media;
        v.content = content;
        v.stamp.sequence += 1;
        v.dirty = true;
        self.shared.wake.try_send(()).ok();
        Ok(())
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
    device: metal::Device,
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
            #[cfg(test)]
            before_submit: Mutex::new(None),
        });
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("pm-engine-service".into())
            .spawn(move || run(engine, worker, wakes))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            device,
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
    pub fn metal_device(&self) -> &metal::DeviceRef {
        &self.device
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
                attached.cast::<metal::MTLDevice>() == self.device.as_ref().as_ptr(),
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
                status: ViewerStatus::default(),
                phase: None,
                input: None,
                pending_input: None,
                pipeline: Default::default(),
                mastering: Default::default(),
                resources: Arc::default(),
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
    /// Observed presentedTime − requested target for timed presents: the
    /// WindowServer/compositor queue the picture cannot avoid. Media time is
    /// chosen for target + this lag, so a frame lands when its audio plays.
    lag: VecDeque<f64>,
    /// Most recent known vsync (host s) and refresh interval.
    grid: Option<(f64, f64)>,
    /// Stamp/time already submitted. Repeating identical pixels at the
    /// display rate only queues more drawables behind the compositor.
    last: Option<(Stamp, Ticks)>,
}
impl Runtime {
    fn lag(&self) -> f64 {
        if self.lag.len() < 3 {
            return 0.;
        }
        let mut v: Vec<_> = self.lag.iter().copied().collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }
}
struct Flight {
    id: u64,
    stamp: Stamp,
    time: Ticks,
    submission: render::Submission,
    input: Option<f64>,
    audio_host: Option<f64>,
    pipeline: trace::Pipeline,
}
fn run(mut engine: render::Engine, shared: Arc<Shared>, wakes: std::sync::mpsc::Receiver<()>) {
    // Same interactive class as pm-node/present; export remains separately
    // governed. A default-class worker can miss display deadlines under load.
    objc2::rc::autoreleasepool(|_| {
        objc2_foundation::NSThread::currentThread()
            .setQualityOfService(objc2_foundation::NSQualityOfService::UserInteractive);
    });
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
    loop {
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
            if let Some(r) = runtime.get_mut(&f.id) {
                let t = f.submission.trace.lock().unwrap();
                match t.display.filter(|d| *d > 0.) {
                    Some(display) => {
                        r.grid = Some((display, r.interval));
                        if t.target > 0. && display >= t.target - 0.0005 {
                            if r.lag.len() == 15 {
                                r.lag.pop_front();
                            }
                            r.lag.push_back(display - t.target);
                        }
                    }
                    // Never shown on a layer: the screen may not hold it.
                    None => {
                        if f.submission.drawable.is_some() && r.last == Some((f.stamp, f.time)) {
                            r.last = None;
                        }
                    }
                }
                if !t.gpu_ok && r.last == Some((f.stamp, f.time)) {
                    r.last = None;
                }
            }
            if let Some(v) = state.views.get_mut(&f.id) {
                let t = f.submission.trace.lock().unwrap();
                if let Some(recorder) = &recorder && !recorder.record(serde_json::json!({"display":t.display,"worker":t.worker_start,"completed":t.completed,"commit":t.commit,
                        "input":f.input,"audio_host":f.audio_host,"time":f.time.get(),"cache_hit":t.cache_hit,
                        "stale":v.stamp != f.stamp,"viewer_id":f.id,"generation":f.stamp.generation,"revision":f.stamp.revision,"sequence":f.stamp.sequence,"gpu_ok":t.gpu_ok,
                        "request":f.pipeline.request,"actor_ms":f.pipeline.actor_ms,"publication_ms":f.pipeline.publication_ms,
                        "render_start":t.render_start,"render_submitted":t.render_submitted,"drawable_asked":t.drawable_asked,"drawable_acquired":t.drawable_acquired,
                        "encode_end":t.encode_end,"scheduled":t.scheduled,"gpu_start":t.gpu_start,"gpu_done":t.gpu_done,"callback":t.display_callback,"target":t.target,"predicted":t.predicted})) {
                    v.status.error = Some("viewer diagnostic trace overflow/disconnected".into());
                }
                if v.stamp != f.stamp {
                    v.status.stale_rejections += 1;
                    continue;
                }
                v.status.frame = Some(FrameReport {
                    stamp: f.stamp,
                    time: f.time,
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
        let now = trace::now();
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
                        v.config.media.audio_paths(),
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
                    lag: VecDeque::with_capacity(15),
                    grid: None,
                    last: None,
                }
            });
            if r.revision != v.stamp.revision || r.content != v.content {
                r.transport.update(
                    &v.config.snapshot,
                    v.config.media.audio_paths(),
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
            if !v.dirty && (!r.transport.clock.is_playing() || now < r.next - interval) {
                continue;
            }
            // A timed present reaches glass `lag` after its target (measured
            // from presentedTime). Pick the picture for that instant, and for
            // forward play show each frame from the vsync nearest its onset,
            // so picture onsets straddle their audio instead of trailing it.
            let shown = due.map(|due| due + if legacy { 0. } else { r.lag() });
            let media = if let Some(shown) = shown {
                let at = |s: f64| {
                    sample.media_time_at(
                        pm_protocol::common::HostMicros::new((s.max(0.) * 1e6) as u64).unwrap(),
                    )
                };
                let plain = at(shown);
                let lead = at(shown + interval / 2.);
                let forward = sample.rate.num > 0 && lead.get() >= plain.get();
                let inside = r.transport.clock.end().is_none_or(|end| lead.get() < end);
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
                media: v.config.media.export_paths(false),
                content: v.content,
            };
            cursor = id;
            work = Some((
                id,
                v.stamp,
                scene,
                v.config.output,
                v.config.layer.clone(),
                if v.overlay_time.is_none_or(|t| t == time) {
                    v.overlays.clone()
                } else {
                    OverlayGeometry::default()
                },
                v.locked_provider.clone(),
                std::mem::take(&mut v.capture),
                time,
                due,
                v.mastering,
                v.input.take(),
                v.pipeline,
                r.transport
                    .audio_host_for(time, (shown.unwrap_or(now) * 1e6) as i64)
                    .map(|t| t / 1e6),
                shown,
            ));
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
        if let Some((
            id,
            stamp,
            scene,
            output,
            layer,
            overlays,
            provider,
            capture,
            time,
            due,
            mastering,
            input,
            pipeline,
            audio_host,
            shown,
        )) = work
        {
            drop(state);
            let trace = Arc::new(Mutex::new(trace::Trace {
                worker_start: now,
                target: due.unwrap_or(0.),
                predicted: shown.unwrap_or(0.),
                ..Default::default()
            }));
            engine.mastering = mastering;
            let result = provider
                .map_or_else(|| Ok(overlays), |p| p(&scene.doc, &scene.comp, time))
                .and_then(|overlays| {
                    engine.frame(
                        &scene,
                        time,
                        output,
                        layer.as_ref(),
                        &overlays,
                        capture,
                        trace,
                    )
                });
            #[cfg(test)]
            if let Some(hook) = shared.before_submit.lock().unwrap().take() {
                hook();
            }
            // Publication/resize and final submission are serialized by this
            // short lock. Render and drawable acquisition never hold it.
            let mut state = shared.state.lock().unwrap();
            if let Some(v) = state.views.get_mut(&id) {
                if v.stamp != stamp {
                    v.status.stale_rejections += 1;
                    continue;
                }
                match result {
                    Ok(Some(s)) => {
                        let completed = Arc::downgrade(&shared);
                        s.submit_notifying(
                            due,
                            Some(Arc::new(move || {
                                // Bounded tokens cannot lose a wake; callbacks
                                // never acquire the service state lock.
                                if let Some(completed) = completed.upgrade() {
                                    completed.wake.try_send(()).ok();
                                }
                            })),
                        );
                        if let Some(r) = runtime.get_mut(&id) {
                            r.last = Some((stamp, time));
                        }
                        flights.push_back(Flight {
                            id,
                            stamp,
                            time,
                            submission: s,
                            input,
                            audio_host,
                            pipeline,
                        });
                    }
                    Ok(None) => {
                        v.status.no_drawable += 1;
                        v.dirty = true;
                        v.input = input;
                    }
                    Err(e) => {
                        v.status.error = Some(e.to_string());
                    }
                }
            }
            state.notify();
        } else {
            let busy = !flights.is_empty() || runtime.values().any(|r| r.transport.running());
            let timeout = if busy {
                let now = trace::now();
                let next = runtime
                    .iter()
                    .filter_map(|(id, r)| {
                        let room =
                            flights.len() < 3 && flights.iter().filter(|f| f.id == *id).count() < 3;
                        if r.transport.clock.is_playing() && room {
                            Some((r.next - r.interval - now).max(0.0001))
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
        let s = self
            .engine
            .frame(
                scene,
                time,
                Output {
                    width: size.width,
                    height: size.height,
                    display: 0,
                    headroom: 1.,
                },
                None,
                overlays,
                true,
                Arc::default(),
            )?
            .ok_or_else(|| anyhow::anyhow!("no offscreen target"))?;
        s.commit();
        s.complete()?;
        Ok(s.pixels())
    }
    pub fn export_reference(&mut self, scene: &Scene, time: Ticks, size: Size) -> Result<Vec<f32>> {
        self.engine.export(scene, time, size)
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
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
}
