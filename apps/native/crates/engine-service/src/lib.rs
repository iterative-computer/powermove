//! In-process native rendering. No Node, napi entrypoints, bridge or GPUI.
//! Callers own below-slot placement and pump native layers on the main thread.
pub mod color;
pub mod jobs;
pub mod layers;
pub mod policy;
mod render;
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
    sync::{Arc, Condvar, Mutex},
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
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    #[cfg(test)]
    before_submit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
#[derive(Clone)]
pub struct Viewer {
    id: u64,
    shared: Arc<Shared>,
}
impl Viewer {
    fn edit(&self, f: impl FnOnce(&mut View) -> Result<()>) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        let v = state
            .views
            .get_mut(&self.id)
            .ok_or_else(|| anyhow::anyhow!("viewer retired"))?;
        f(v)?;
        self.shared.wake.notify_one();
        Ok(())
    }
    /// Immutable publication input, cheap COW tree clone. The snapshot is the
    /// authoritative fallback if the consumer missed any change events.
    #[cfg(feature = "kernel")]
    pub fn publish(&self, p: &Published) -> Result<()> {
        self.snapshot(p.snapshot.clone())
    }
    pub fn snapshot(&self, snapshot: Snapshot) -> Result<()> {
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
        self.shared.wake.notify_one();
        Ok(())
    }
    /// Interactive scrubbing pauses and immediately requests the exact frame.
    pub fn scrub(&self, time: Ticks) -> Result<()> {
        self.edit(|v| {
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
            if output != v.config.output {
                v.config.output = output;
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
        self.shared.state.lock().unwrap().views.remove(&self.id);
        self.shared.wake.notify_one();
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
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            #[cfg(test)]
            before_submit: Mutex::new(None),
        });
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("pm-engine-service".into())
            .spawn(move || run(engine, worker))?;
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
        self.shared.wake.notify_one();
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
            },
        );
        self.shared.wake.notify_one();
        Ok(Viewer {
            id,
            shared: self.shared.clone(),
        })
    }
}
impl Drop for EngineService {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().stop = true;
        self.shared.wake.notify_one();
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
    last_time: Option<Ticks>,
}
struct Flight {
    id: u64,
    stamp: Stamp,
    time: Ticks,
    submission: render::Submission,
}
fn run(mut engine: render::Engine, shared: Arc<Shared>) {
    let mut runtime = BTreeMap::<u64, Runtime>::new();
    let mut flights = VecDeque::<Flight>::new();
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
            if let Some(v) = state.views.get_mut(&f.id) {
                if v.stamp != f.stamp {
                    v.status.stale_rejections += 1;
                    continue;
                }
                let t = f.submission.trace.lock().unwrap();
                v.status.frame = Some(FrameReport {
                    stamp: f.stamp,
                    time: f.time,
                    cache_hit: t.cache_hit,
                    gpu_ok: f.submission.command.status()
                        == metal::MTLCommandBufferStatus::Completed,
                    display_host_seconds: t.display.filter(|t| *t > 0.),
                    completion_host_seconds: (t.completed > 0.).then_some(t.completed),
                    pixels: f
                        .submission
                        .readback
                        .as_ref()
                        .map(|_| f.submission.pixels()),
                });
            }
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
                    last_time: None,
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
            while let Some(cmd) = v.commands.pop_front() {
                r.transport.command(cmd, (now * 1e6) as i64);
                r.last_time = None;
            }
            let sample = r.transport.poll((now * 1e6) as i64);
            v.status.clock = Some(sample);
            v.desired_rate = r.transport.rate();
            v.status.audio_fallback = r.transport.fallback;
            if let Some((_, interval)) = v.phase {
                r.interval = interval;
            }
            let interval = r.interval;
            if let Some((phase, _)) = v.phase.take()
                && phase > now
            {
                r.next = phase;
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
            if flights.len() >= 3 || flights.iter().any(|f| f.id == id) {
                continue;
            }
            if !v.dirty && (!r.transport.clock.is_playing() || now < r.next - interval) {
                continue;
            }
            let media = if let Some(due) = due {
                sample.media_time_at(
                    pm_protocol::common::HostMicros::new((due * 1e6) as u64).unwrap(),
                )
            } else {
                sample.media_time
            };
            let time =
                Ticks::new(r.transport.clock.snap(media.get()).max(0)).unwrap_or(Ticks::ZERO);
            r.next = due.unwrap_or(now) + interval;
            if !v.dirty && r.last_time == Some(time) {
                continue;
            }
            r.last_time = Some(time);
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
            ));
            break;
        }
        if let Some((id, stamp, scene, output, layer, overlays, provider, capture, time, due)) =
            work
        {
            drop(state);
            let trace = Arc::new(Mutex::new(trace::Trace {
                worker_start: now,
                ..Default::default()
            }));
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
                        s.submit(due);
                        flights.push_back(Flight {
                            id,
                            stamp,
                            time,
                            submission: s,
                        });
                    }
                    Ok(None) => {
                        v.status.no_drawable += 1;
                        v.dirty = true;
                    }
                    Err(e) => {
                        v.status.error = Some(e.to_string());
                    }
                }
            }
        } else {
            let busy = !flights.is_empty() || runtime.values().any(|r| r.transport.running());
            let _ = shared
                .wake
                .wait_timeout(
                    state,
                    if busy {
                        Duration::from_millis(1)
                    } else {
                        Duration::from_secs(3600)
                    },
                )
                .unwrap();
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
