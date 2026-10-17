//! The presenter's render thread: GPU device, surface pools, the bridge
//! client, the transport clock, and frame delivery.
//!
//! With a document source (the Phase 2 tracer bullet) the thread keeps a
//! mirror of each session's document ([`super::document`]), routes its
//! composition to the engine or the legacy viewer, draws engine frames with
//! `pm-render` ([`super::compose`]), follows the audio device clock
//! ([`super::audio`]), and rebuilds its GPU device after a loss.
//!
//! Commands arrive from the engine's JavaScript thread; descriptors, clock
//! samples, surface pool announcements, and frame responses leave through
//! the sink straight to the viewer's port. A descriptor is sent only after
//! the command buffer that wrote its surface completed, and a surface is
//! never written while the viewer holds it (contract 2's `PoolLedger`).

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pm_bridge::rendezvous::{Client, SendRight};
use pm_bridge::wire::{PixelFormat, SurfaceInfo};
use pm_engine::document::DocumentUpdate;
use pm_engine::{Action, LogLevel};
use pm_gpu::bridge_out::display::{DisplayParams, DisplayTransform, WorkingTexture};
use pm_gpu::bridge_out::pool::SurfacePool;
#[cfg(feature = "remote-layer")]
use pm_gpu::bridge_out::remote_layer::{
    Presented, RatePreference, RemoteLayer, preferred_refresh, raw_metal_device, take_display_ticks,
};
use pm_gpu::bridge_out::test_source::{TestFrame, TestSource};
use pm_protocol::clock::{AudioMessage, SeekMode, TransportCommand, TransportState};
use pm_protocol::codec;
use pm_protocol::color::WorkingEncoding;
use pm_protocol::common::{RequestId, Revision, Seq, SessionId, Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::envelope::{Envelope, Event, Message, Response};
use pm_protocol::error::{EngineError, ErrorCode, ErrorScope, FailureReport, Retry};
use pm_protocol::frame::{
    FrameDescriptor, FrameQuality, FrameRelease, FrameRequest, PoolLedger, PreviewResolution,
    Representation, ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::interop::SurfaceFormat;
use serde::Serialize;

use super::audio::{AudioCtl, AudioLink, AudioStatus};
use super::clock::{SAMPLE_INTERVAL_US, TransportClock};
use super::compose::Composer;
use super::document::{Route, ViewDocument};
use super::schedule::{Next, Phase, Schedule};
use super::view;

/// Stand-in for the C′ prototype's layer in builds without it (feature
/// `remote-layer`): uninhabited, so a pool never has a remote layer.
#[cfg(not(feature = "remote-layer"))]
enum RemoteLayer {}

/// Where the engine's actions go: posted on the utility process's ports by
/// its JavaScript (pm-node's threadsafe function), or collected by tests.
pub type Sink = Arc<dyn Fn(Vec<Action>) + Send + Sync>;

pub type Key = (PortId, SessionId);

/// Pool generation numbers, shared by the JavaScript side (new pools) and
/// the render thread (a pool made again after a failed hand-off).
#[derive(Debug, Clone)]
pub struct Generations(Arc<AtomicU32>);

impl Default for Generations {
    fn default() -> Self {
        Self(Arc::new(AtomicU32::new(1)))
    }
}

impl Generations {
    /// The next generation: never 0, never repeated before the counter wraps.
    pub fn next(&self) -> u32 {
        loop {
            let g = self.0.fetch_add(1, Ordering::Relaxed);
            if g != 0 {
                return g;
            }
        }
    }
}

/// Delays before making a pool again after its hand-off to main failed
/// part-way (a send timed out, main's queue was full). After the last one
/// the view waits for its next viewer state.
pub const POOL_RETRY_US: [i64; 4] = [50_000, 250_000, 1_000_000, 4_000_000];

/// A pool to make again: the spec the JavaScript side charged, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolRetry {
    pub spec: PoolSpec,
    /// Failed attempts so far.
    pub failures: u32,
    /// Host µs of the next attempt; `None` once the retries ran out.
    pub at: Option<i64>,
}

impl PoolRetry {
    /// The retry after the `failures`-th failed attempt at `now`.
    pub fn after_failure(spec: PoolSpec, failures: u32, now: i64) -> Self {
        let at = POOL_RETRY_US
            .get(failures.saturating_sub(1) as usize)
            .map(|delay| now + delay);
        Self { spec, failures, at }
    }

    pub fn due(&self, now: i64) -> bool {
        self.at.is_some_and(|at| at <= now)
    }

    /// A new viewer state for the view: an exhausted retry tries again now.
    pub fn rearm(&mut self, now: i64) {
        if self.at.is_none() {
            self.at = Some(now);
        }
    }
}

/// A pool the JavaScript side sized and charged to the lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSpec {
    pub generation: u32,
    pub size: Size,
    pub format: SurfaceFormat,
    pub count: u32,
}

pub enum Command {
    Configure {
        key: Key,
        state: Box<ViewerState>,
        revision: Option<Revision>,
        pool: Option<PoolSpec>,
    },
    Release {
        key: Key,
        release: FrameRelease,
    },
    Transport {
        key: Key,
        command: TransportCommand,
        at: i64,
    },
    Render {
        key: Key,
        id: RequestId,
        request: Box<FrameRequest>,
        revision: Option<Revision>,
    },
    Close {
        key: Key,
    },
    ClosePort {
        port: PortId,
    },
    /// The session's document changed (document source).
    Document {
        key: Key,
        update: DocumentUpdate,
    },
    /// The session's definitions (contract 10), and those a delta
    /// registered again.
    Definitions {
        key: Key,
        set: Arc<pm_protocol::definitions::DefinitionSetSnapshot>,
        registered: Vec<(
            pm_protocol::definitions::DefinitionKind,
            pm_protocol::common::DefinitionId,
        )>,
    },
    /// An asset's file, or `None` when it is offline or gone.
    Media {
        key: Key,
        asset: String,
        path: Option<PathBuf>,
    },
    /// The mix's monitor and data for the renderer (contract 6).
    Audio {
        key: Key,
        message: AudioMessage,
    },
    /// The renderer's shader fallback runner answered (decision 19).
    ShaderFallback {
        key: Key,
        message: pm_protocol::shader_fallback::ShaderFallbackMessage,
    },
    /// Destroys the GPU device as a real loss would (the recovery test).
    LoseDevice,
    Shutdown,
}

/// How the thread runs.
#[derive(Debug, Clone)]
pub struct ThreadConfig {
    /// The per-launch bootstrap name; `None` when main gave none.
    pub bridge_name: Option<String>,
    /// Frames of render-ahead during playback.
    pub lead: u32,
    /// Test source frame rate, frames per second.
    pub fps: u32,
    /// Extra GPU passes per frame (the S1 harness's engine load).
    pub extra_passes: u32,
    /// Keep a per-frame trace for the S1 harness.
    pub trace: bool,
    /// Fraction of a refresh after the viewer's BeginFrame at which timed
    /// frames start rendering, once the viewer sent its vsync hint; `None`
    /// renders at the due time.
    pub phase: Option<f64>,
    /// The C′ prototype: timed playback frames go to a remote Core
    /// Animation layer instead of the viewer (docs/engine/bridge.md).
    pub remote_layer: bool,
    /// File the C′ prototype appends presentation records to.
    pub remote_trace: Option<String>,
    /// While the C′ layer plays, ask the display for a multiple of the
    /// content rate (plan section 15, ProMotion).
    pub remote_rate: bool,
    /// Frames draw the session's document with pm-render (otherwise the
    /// bridge test source).
    pub document: bool,
    /// Play without the audio device: the line runs on the host clock
    /// (tests on machines without an output, and the S1 harness).
    pub host_clock: bool,
    /// The program cache folder (`SHADER_CACHE_ENV`).
    pub shader_cache: Option<PathBuf>,
}

/// The device's composer, keeping translated programs in the configured
/// cache folder.
fn composer(device: &wgpu::Device, queue: &wgpu::Queue, config: &ThreadConfig) -> Composer {
    let mut composer = Composer::new(device, queue);
    composer
        .renderer()
        .set_program_cache_dir(config.shader_cache.clone());
    // Viewer sessions come from a renderer with the decision 19 fallback
    // runner (`engine-connection/shader-fallback.ts`).
    composer.renderer().set_shader_fallback_runner(true);
    composer
}

/// One frame's record for the S1 harness.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FrameTrace {
    pub seq: u64,
    pub k: i64,
    pub slot: u32,
    pub generation: u32,
    /// Host µs the frame is shown from (timed frames) or 0.
    pub shown_from: i64,
    /// Host µs it is shown for (timed frames) or 0.
    pub shown_for: i64,
    pub view_seq: u64,
    pub revision: u64,
    pub start: i64,
    pub gpu_done: i64,
    pub sent: i64,
    pub timed: bool,
}

/// Counters the engine reports in its status.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub checked_in: bool,
    pub check_in_us: i64,
    pub bridge_error: Option<String>,
    pub adapter: Option<String>,
    pub pools: u64,
    /// Pools whose creation or hand-off to main failed (each attempt).
    pub pool_failures: u64,
    pub surfaces_sent: u64,
    pub frames: u64,
    pub timed_frames: u64,
    pub immediate_frames: u64,
    pub responses: u64,
    pub late_skipped: u64,
    pub pool_waits: u64,
    pub releases: u64,
    pub release_mismatches: u64,
    pub gpu_us_max: i64,
    pub gpu_us_total: i64,
    pub max_held: u32,
    /// The C′ prototype: frames given to the remote layer, and how many Core
    /// Animation reported presented or dropped.
    pub remote_frames: u64,
    pub remote_presented: u64,
    pub remote_dropped: u64,
    /// Frames the C′ layer skipped to bring presentation back to its asked
    /// times after a stall left it a frame late.
    pub remote_resyncs: u64,
    pub remote_error: Option<String>,
    /// The refresh rate the C′ prototype asked for while playing.
    pub remote_rate_hz: Option<f64>,
    /// Document frames drawn, and the CPU time of the slowest.
    pub document_frames: u64,
    pub compose_us_max: i64,
    pub compose_us_total: i64,
    /// Frames that failed to render (logged), and layer failures.
    pub render_errors: u64,
    pub layer_failures: u64,
    /// Times a composition was routed to the legacy viewer.
    pub routed_legacy: u64,
    /// The last routing decision per view, as text.
    pub routes: BTreeMap<String, String>,
    /// GPU device losses recovered from.
    pub device_losses: u64,
    /// Runs that fell back to the host clock (no audio output).
    pub host_clock_runs: u64,
    /// Audio playback, per view.
    pub audio: BTreeMap<String, AudioStatus>,
    #[serde(skip)]
    pub trace: Vec<FrameTrace>,
}

const TRACE_CAP: usize = 200_000;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    display: DisplayTransform,
    /// Draws documents (made with the first document frame).
    composer: Option<Composer>,
    /// Set by the device-lost callback; the thread rebuilds the device.
    lost: Arc<AtomicBool>,
}

struct Live {
    spec: PoolSpec,
    pool: SurfacePool,
    ledger: PoolLedger,
    working: WorkingTexture,
    /// The bridge test source; `None` for documents.
    source: Option<TestSource>,
    /// The C′ prototype's layer for this pool's size and format.
    remote: Option<RemoteLayer>,
}

struct View {
    key: Key,
    state: ViewerState,
    revision: Revision,
    pool: Option<Live>,
    clock: TransportClock,
    schedule: Schedule,
    last_sample: Option<i64>,
    sample_due: bool,
    next_seq: u64,
    dirty: bool,
    requests: VecDeque<(RequestId, FrameRequest, Option<Revision>)>,
    /// The pool to make again after a failed hand-off.
    retry: Option<PoolRetry>,
    /// The route last reported to the viewer (document source).
    reported: Option<Route>,
    /// A Play waits for the audio clock's first playing sample: the host
    /// µs after which it starts on the host clock instead, and the audio
    /// epoch that was current when it was asked (not the answer).
    awaiting: Option<(i64, Option<u32>, PlaybackRateValue)>,
    /// The audio clock version last followed.
    audio_version: u64,
    /// The C′ prototype's refresh-rate request while it plays, and whether
    /// this playback already tried (a failed request is not repeated).
    #[cfg(feature = "remote-layer")]
    rate: Option<RatePreference>,
    #[cfg(feature = "remote-layer")]
    rate_tried: bool,
}

type PlaybackRateValue = pm_protocol::common::PlaybackRate;

/// A session's listening choices.
#[derive(Debug, Clone, Default)]
struct AudioPrefs {
    scrub: bool,
    solo: Option<std::collections::HashSet<String>>,
}

/// How long a Play waits for the audio device before the line starts on
/// the host clock.
pub const AUDIO_START_TIMEOUT_US: i64 = 1_500_000;

struct Thread {
    config: ThreadConfig,
    sink: Sink,
    stats: Arc<Mutex<Stats>>,
    bridge: Option<Client>,
    gpu: Option<Gpu>,
    views: BTreeMap<Key, View>,
    /// Each session's document (document source), kept from its first
    /// update, which may come before the viewer state.
    docs: BTreeMap<Key, ViewDocument>,
    /// Each session's audio playback, from its first Play.
    audio: BTreeMap<Key, AudioLink>,
    /// Each session's audio scrub switch and monitor solo, kept before
    /// its playback opens.
    audio_prefs: BTreeMap<Key, AudioPrefs>,
    /// Waveform peaks, computed off this thread.
    peaks: super::peaks::PeakWorker,
    frame_ticks: i64,
    generations: Generations,
    /// Shader fallback requests sent to each view's renderer and not yet
    /// answered (by request key), and the bands received so far.
    fallback: Fallback,
}

/// The decision 19 fallback runner's state on this thread.
#[derive(Default)]
struct Fallback {
    asked: BTreeMap<String, Key>,
    bands: pm_render::shader_layers::Bands,
}

fn now() -> i64 {
    pm_bridge::now_us()
}

/// Host µs (`mach_absolute_time`), as the bridge and the clocks read it.
pub fn now_us() -> i64 {
    now()
}

/// The stats key of a view.
fn view_name(key: Key) -> String {
    format!("{}:{}", key.0.0, key.1.get())
}

/// The render thread's body.
pub fn run(
    config: ThreadConfig,
    rx: Receiver<Command>,
    sink: Sink,
    stats: Arc<Mutex<Stats>>,
    generations: Generations,
) {
    pm_bridge::set_thread_user_interactive();
    let fps = i64::from(config.fps.max(1));
    let peaks = super::peaks::PeakWorker::new(sink.clone());
    let mut thread = Thread {
        frame_ticks: TICKS_PER_SECOND / fps,
        config,
        sink,
        stats,
        bridge: None,
        gpu: None,
        views: BTreeMap::new(),
        docs: BTreeMap::new(),
        audio: BTreeMap::new(),
        audio_prefs: BTreeMap::new(),
        peaks,
        generations,
        fallback: Fallback::default(),
    };
    thread.check_in();
    loop {
        let timeout = thread.next_wake();
        match rx.recv_timeout(timeout) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                if !thread.apply(command) {
                    break;
                }
                while let Ok(command) = rx.try_recv() {
                    if matches!(command, Command::Shutdown) || !thread.apply(command) {
                        thread.shutdown();
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        thread.work();
    }
    thread.shutdown();
}

impl Thread {
    fn log(&self, level: LogLevel, message: String) {
        (self.sink)(vec![Action::Log { level, message }]);
    }

    fn stats(&self) -> std::sync::MutexGuard<'_, Stats> {
        self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check_in(&mut self) {
        let Some(name) = self.config.bridge_name.clone() else {
            return;
        };
        let start = now();
        match Client::check_in(&name, Duration::from_secs(10)) {
            Ok(client) => {
                let mut stats = self.stats();
                stats.checked_in = true;
                stats.check_in_us = now() - start;
                drop(stats);
                self.log(
                    LogLevel::Info,
                    format!(
                        "frame bridge: checked in with main (pid {}) in {} µs",
                        client.bridge_peer().pid(),
                        now() - start
                    ),
                );
                self.bridge = Some(client);
            }
            Err(error) => {
                self.stats().bridge_error = Some(error.to_string());
                self.log(
                    LogLevel::Error,
                    format!("frame bridge: check-in failed: {error}"),
                );
            }
        }
    }

    fn gpu(&mut self) -> Option<&Gpu> {
        if self.gpu.is_none() {
            // Documents need the render graph's features (16-bit planes,
            // safe-math passthrough shaders, rgba32float when present).
            let made = if self.config.document {
                pm_gpu::Gpu::new("interactive engine viewer")
                    .map(|g| (g.device, g.queue, g.info))
                    .map_err(|e| e.to_string())
            } else {
                pm_gpu::bridge_out::metal_device("interactive engine viewer")
                    .map_err(|e| e.to_string())
            };
            match made {
                Ok((device, queue, info)) => {
                    let display = DisplayTransform::new(&device, &queue);
                    self.stats().adapter = Some(info.name.clone());
                    let lost = Arc::new(AtomicBool::new(false));
                    let flag = lost.clone();
                    let sink = self.sink.clone();
                    device.set_device_lost_callback(move |reason, message| {
                        flag.store(true, Ordering::Release);
                        sink(vec![Action::Log {
                            level: LogLevel::Warn,
                            message: format!(
                                "frame bridge: GPU device lost ({reason:?}): {message}"
                            ),
                        }]);
                    });
                    // A validation error must not take the engine down: log it.
                    // Out-of-memory and internal errors rebuild the device.
                    let flag = lost.clone();
                    let sink = self.sink.clone();
                    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                        if !matches!(error, wgpu::Error::Validation { .. }) {
                            flag.store(true, Ordering::Release);
                        }
                        sink(vec![Action::Log {
                            level: LogLevel::Error,
                            message: format!("frame bridge: GPU error: {error}"),
                        }]);
                    }));
                    self.gpu = Some(Gpu {
                        device,
                        queue,
                        display,
                        composer: None,
                        lost,
                    });
                }
                Err(error) => {
                    self.log(
                        LogLevel::Error,
                        format!("frame bridge: no GPU device: {error}"),
                    );
                    return None;
                }
            }
        }
        self.gpu.as_ref()
    }

    /// Rebuilds the GPU device after a loss: every pool is retired and made
    /// again on the new device as a new generation (the viewer imports the
    /// new surfaces), and every view redraws.
    fn recover_gpu(&mut self) {
        let Some(gpu) = self.gpu.take() else {
            return;
        };
        drop(gpu);
        self.stats().device_losses += 1;
        let keys: Vec<Key> = self.views.keys().copied().collect();
        let mut remake = Vec::new();
        for key in keys {
            if let Some(live) = self.views.get_mut(&key).and_then(|v| v.pool.take()) {
                let spec = live.spec;
                self.retire_pool(key, live);
                remake.push((key, spec));
            }
            // The viewer hears why its frames paused (it keeps the last one).
            let error = EngineError::new(
                ErrorCode::DeviceLost,
                ErrorScope::Session { session: key.1 },
                "The GPU device was lost; it is being rebuilt.",
            )
            .with_retry(Retry::After { ms: 0 });
            let revision = self
                .views
                .get(&key)
                .map(|v| v.revision)
                .unwrap_or(Revision::ZERO);
            self.event(
                key,
                Event::Failure(FailureReport { revision, error }),
                Some(revision),
            );
        }
        self.log(
            LogLevel::Warn,
            format!(
                "frame bridge: rebuilding the GPU device and {} pool(s)",
                remake.len()
            ),
        );
        for (key, spec) in remake {
            let spec = PoolSpec {
                generation: self.generations.next(),
                ..spec
            };
            self.make_pool(key, spec, 0, None);
        }
    }

    /// Sends one envelope on a port.
    fn send(&self, port: PortId, envelope: Envelope) {
        match codec::encode(&envelope) {
            Ok(bytes) => (self.sink)(vec![Action::Send { port, bytes }]),
            Err(error) => self.log(
                LogLevel::Error,
                format!("frame bridge: could not encode a message: {error}"),
            ),
        }
    }

    fn event(&self, key: Key, body: Event, revision: Option<Revision>) {
        let mut envelope = Envelope::new(Message::Event { body }).with_session(key.1);
        envelope.revision = revision;
        self.send(key.0, envelope);
    }

    fn error(&self, key: Key, id: RequestId, code: ErrorCode, message: &str) {
        let error = EngineError::new(code, ErrorScope::Request { id }, message);
        self.send(key.0, Envelope::new(Message::Error { id, error }));
    }

    /// Returns false when the thread must stop.
    fn apply(&mut self, command: Command) -> bool {
        match command {
            Command::Shutdown => return false,
            Command::Configure {
                key,
                state,
                revision,
                pool,
            } => self.configure(key, *state, revision, pool),
            Command::Release { key, release } => {
                let ok = self
                    .views
                    .get_mut(&key)
                    .and_then(|view| view.pool.as_mut())
                    .is_some_and(|live| live.ledger.release(release).is_ok());
                let mut stats = self.stats();
                if ok {
                    stats.releases += 1;
                } else {
                    // A frame of a retired pool, or a duplicate: ignored.
                    stats.release_mismatches += 1;
                }
            }
            Command::Transport { key, command, at } => self.transport(key, command, at),
            Command::Audio { key, message } => self.audio_message(key, message),
            Command::Render {
                key,
                id,
                request,
                revision,
            } => match self.views.get_mut(&key) {
                Some(view) => view.requests.push_back((id, *request, revision)),
                None => self.error(
                    key,
                    id,
                    ErrorCode::InvalidMessage,
                    "Send the viewer state before asking for viewer frames.",
                ),
            },
            Command::Close { key } => {
                if let Some(view) = self.views.remove(&key) {
                    self.retire(view);
                }
                self.forget_session(key);
            }
            Command::ClosePort { port } => {
                let keys: Vec<Key> = self.views.keys().filter(|k| k.0 == port).copied().collect();
                for key in keys {
                    if let Some(view) = self.views.remove(&key) {
                        self.retire(view);
                    }
                }
                let keys: Vec<Key> = self
                    .docs
                    .keys()
                    .chain(self.audio.keys())
                    .filter(|k| k.0 == port)
                    .copied()
                    .collect();
                for key in keys {
                    self.forget_session(key);
                }
            }
            Command::Document { key, update } => self.document(key, update),
            Command::Definitions {
                key,
                set,
                registered,
            } => {
                let doc = self.docs.entry(key).or_default();
                doc.set_definitions(set, registered);
                let definitions = doc.definitions().cloned();
                let registered = doc.take_registered();
                let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
                // Translate now (not at the next frame) and report each
                // definition that fails (contract 10; the kernel fills the
                // shader UI from these).
                let mut failures = Vec::new();
                if self.config.document
                    && let Some(gpu) = self.gpu.as_mut()
                {
                    let composer = gpu
                        .composer
                        .get_or_insert_with(|| composer(&gpu.device, &gpu.queue, &self.config));
                    failures = composer
                        .use_definitions(key, definitions.as_ref(), &registered)
                        .unwrap_or_default();
                } else if let Some(doc) = self.docs.get_mut(&key) {
                    // Applied with the view's next frame.
                    doc.keep_registered(registered);
                }
                for error in failures {
                    let report = pm_protocol::error::FailureReport { revision, error };
                    self.event(key, Event::Failure(report), None);
                }
                if let Some(view) = self.views.get_mut(&key) {
                    view.dirty = true;
                }
            }
            Command::Media { key, asset, path } => {
                let doc = self.docs.entry(key).or_default();
                doc.set_media(&asset, path);
                let media = doc.media().clone();
                if let Some(audio) = self.audio.get(&key) {
                    audio.send(AudioCtl::Media(media));
                }
                if let Some(view) = self.views.get_mut(&key) {
                    view.dirty = true;
                }
            }
            Command::ShaderFallback { key, message } => self.shader_fallback(key, message),
            Command::LoseDevice => {
                if let Some(gpu) = &self.gpu {
                    gpu.lost.store(true, Ordering::Release);
                    gpu.device.destroy();
                }
            }
        }
        true
    }

    /// The runner's answer: a band of pixels (the frame renders again once
    /// every band of a request arrived), or a failure (the composition is
    /// routed to the legacy viewer, as a program the engine cannot draw).
    fn shader_fallback(
        &mut self,
        key: Key,
        message: pm_protocol::shader_fallback::ShaderFallbackMessage,
    ) {
        use pm_protocol::shader_fallback::ShaderFallbackMessage as M;
        match message {
            M::Pixels(band) => {
                if self.fallback.asked.get(&band.key) != Some(&key) {
                    return;
                }
                let Some((request, pixels)) = self.fallback.bands.add(&band) else {
                    return;
                };
                self.fallback.asked.remove(&request);
                if let Some(composer) = self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
                    composer.renderer().put_fallback_pixels(request, pixels);
                }
                if let Some(view) = self.views.get_mut(&key) {
                    view.dirty = true;
                }
            }
            M::Failed {
                key: request,
                reason,
                message,
            } => {
                if self.fallback.asked.remove(&request) != Some(key) {
                    return;
                }
                self.fallback.bands.forget(&request);
                (self.sink)(vec![Action::Log {
                    level: LogLevel::Warn,
                    message: format!(
                        "viewer: the shader fallback runner could not draw a layer ({reason}: {message})"
                    ),
                }]);
                if let (Some(view), Some(doc)) = (self.views.get_mut(&key), self.docs.get_mut(&key))
                {
                    let composition = view.state.composition.as_str().to_owned();
                    doc.force_legacy(
                        &composition,
                        vec![format!(
                            "the shader fallback runner could not draw a layer ({reason})"
                        )],
                    );
                    view.dirty = true;
                }
            }
            // Requests travel the other way.
            M::Requests { .. } => {}
        }
    }

    fn forget_session(&mut self, key: Key) {
        self.fallback.asked.retain(|_, k| *k != key);
        self.docs.remove(&key);
        self.audio.remove(&key);
        self.audio_prefs.remove(&key);
        if let Some(composer) = self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
            composer.forget_definitions(key);
        }
        let mut stats = self.stats();
        stats.routes.remove(&view_name(key));
        stats.audio.remove(&view_name(key));
    }

    /// A document update: the mirror, the audio thread's mirror, the frame
    /// grid, and a redraw.
    fn document(&mut self, key: Key, update: DocumentUpdate) {
        let doc = self.docs.entry(key).or_default();
        if let Err(error) = doc.update(&update) {
            // The engine applied this batch to its own mirror; this one could
            // not. Ask the renderer for a resync rather than draw a wrong frame.
            let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
            let failure = EngineError::new(
                ErrorCode::StaleRevision,
                ErrorScope::Session { session: key.1 },
                format!("The viewer's document did not apply a batch ({error}); resync."),
            )
            .with_retry(Retry::AfterResync);
            self.event(
                key,
                Event::Failure(FailureReport {
                    revision,
                    error: failure,
                }),
                Some(revision),
            );
        }
        if let Some(audio) = self.audio.get(&key) {
            audio.send(AudioCtl::Document(update));
        }
        self.refresh_timing(key);
        if let Some(view) = self.views.get_mut(&key) {
            view.dirty = true;
        }
    }

    /// The frame grid and end of the composition a view shows.
    fn refresh_timing(&mut self, key: Key) {
        let (Some(view), Some(doc)) = (self.views.get_mut(&key), self.docs.get(&key)) else {
            return;
        };
        if !doc.has_document() {
            return;
        }
        let Ok(comp) = pm_eval::Comp::resolve(doc.document(), view.state.composition.as_str())
        else {
            return;
        };
        if let Some(rate) = comp.rate() {
            let num = rate.rate.num();
            let den = rate.rate.den();
            if num > 0 && den > 0 {
                let ticks = TICKS_PER_SECOND * den / num;
                if ticks != view.clock.frame_ticks() {
                    view.clock.set_frame_ticks(ticks);
                    view.schedule.reset();
                }
            }
        }
        let dur = comp.duration();
        view.clock.set_end(
            (dur.is_finite() && dur > 0.0).then(|| (dur * TICKS_PER_SECOND as f64).round() as i64),
        );
    }

    /// A transport command from the viewer. With a document, Play (J/K/L at
    /// 1× and 2×, forward and reverse) and play around go to the audio
    /// device and the line starts with its first playing sample (the audio
    /// clock is the master); without an output device, and for the test
    /// source, the line runs on the host clock. Scrub seeks while paused
    /// play an audio grain when audio scrub is on.
    fn transport(&mut self, key: Key, command: TransportCommand, at: i64) {
        let document = self.config.document;
        let host_clock = self.config.host_clock;
        let Some(view) = self.views.get_mut(&key) else {
            return;
        };
        if !document {
            if view.clock.apply(command, at) {
                view.schedule.reset();
                view.sample_due = true;
                if !view.clock.is_playing() {
                    view.dirty = true;
                }
            }
            return;
        }
        match command {
            TransportCommand::Play { rate } => {
                if view.awaiting.is_some_and(|a| a.2 == rate)
                    || (view.clock.is_playing() && view.clock.rate() == rate)
                {
                    return;
                }
                // Routed to the legacy viewer: the legacy transport plays.
                if view.reported != Some(Route::Engine) {
                    return;
                }
                let from = view.clock.media_at(at);
                let wants_audio = !host_clock && pm_protocol::clock::JKL_RATES.contains(&rate);
                if wants_audio && self.audio_for(key).is_some() {
                    let audio = &self.audio[&key];
                    let before = audio.clock().and_then(|c| c.latest()).map(|s| s.epoch);
                    audio.send(AudioCtl::Play { from, rate });
                    let view = self.views.get_mut(&key).expect("checked");
                    // A new rate while playing holds the line where it is
                    // until the new run's first sample.
                    if view.clock.is_playing() {
                        view.clock.hold(from, at);
                        view.schedule.reset();
                    }
                    view.awaiting = Some((at + AUDIO_START_TIMEOUT_US, before, rate));
                    view.sample_due = true;
                } else {
                    self.stats().host_clock_runs += 1;
                    let view = self.views.get_mut(&key).expect("checked");
                    view.clock.apply(TransportCommand::Play { rate }, at);
                    view.schedule.reset();
                    view.sample_due = true;
                }
            }
            TransportCommand::PlayAround {
                pre_roll,
                post_roll,
            } => {
                if view.reported != Some(Route::Engine) {
                    return;
                }
                let here = view.clock.media_at(at);
                if !host_clock && self.audio_for(key).is_some() {
                    let audio = &self.audio[&key];
                    let before = audio.clock().and_then(|c| c.latest()).map(|s| s.epoch);
                    audio.send(AudioCtl::PlayAround {
                        at: here,
                        pre: pre_roll,
                        post: post_roll,
                    });
                    let view = self.views.get_mut(&key).expect("checked");
                    view.clock.hold(here, at);
                    view.schedule.reset();
                    view.awaiting = Some((
                        at + AUDIO_START_TIMEOUT_US,
                        before,
                        PlaybackRateValue::NORMAL,
                    ));
                    view.sample_due = true;
                } else {
                    self.stats().host_clock_runs += 1;
                    let view = self.views.get_mut(&key).expect("checked");
                    view.clock
                        .play_around(here, pre_roll.get(), post_roll.get(), at);
                    view.schedule.reset();
                    view.sample_due = true;
                }
            }
            TransportCommand::Pause => {
                view.awaiting = None;
                if view.clock.apply(TransportCommand::Pause, at) {
                    view.schedule.reset();
                    view.sample_due = true;
                    view.dirty = true;
                }
                if let Some(audio) = self.audio.get(&key) {
                    audio.send(AudioCtl::Pause);
                }
            }
            TransportCommand::Seek { time, mode } => {
                let playing = view.clock.following() && view.clock.is_playing();
                let awaiting = view.awaiting;
                view.clock.apply(TransportCommand::Seek { time, mode }, at);
                if playing || awaiting.is_some() {
                    // The audio seeks and starts a new run; the line holds the
                    // target until its first sample.
                    view.clock.hold(time.get(), at);
                    let before = self
                        .audio
                        .get(&key)
                        .and_then(|a| a.clock())
                        .and_then(|c| c.latest())
                        .map(|s| s.epoch);
                    let rate = awaiting.map_or(PlaybackRateValue::NORMAL, |a| a.2);
                    let view = self.views.get_mut(&key).expect("checked");
                    view.awaiting = Some((at + AUDIO_START_TIMEOUT_US, before, rate));
                    if let Some(audio) = self.audio.get(&key) {
                        audio.send(AudioCtl::Seek(time.get()));
                    }
                } else if mode == SeekMode::Scrub {
                    self.scrub_grain(key, time.get());
                }
                let view = self.views.get_mut(&key).expect("checked");
                view.schedule.reset();
                view.sample_due = true;
                view.dirty = true;
            }
            TransportCommand::SetLoop { range } => {
                view.clock.apply(TransportCommand::SetLoop { range }, at);
                view.sample_due = true;
                if let Some(audio) = self.audio.get(&key) {
                    audio.send(AudioCtl::SetLoop(range));
                }
            }
            TransportCommand::AudioScrub { enabled } => {
                self.audio_prefs.entry(key).or_default().scrub = enabled;
                if let Some(audio) = self.audio.get(&key) {
                    audio.send(AudioCtl::AudioScrub(enabled));
                }
            }
            TransportCommand::Step { frames } => {
                // Steps while paused: the line, and a grain where it lands.
                let paused = !view.clock.is_playing() && view.awaiting.is_none();
                if view.clock.apply(TransportCommand::Step { frames }, at) {
                    view.schedule.reset();
                    view.sample_due = true;
                    view.dirty = true;
                }
                if paused {
                    let media = view.clock.media_at(at);
                    self.scrub_grain(key, media);
                }
            }
        }
    }

    /// An audio grain at `media` for a scrub while paused, when the
    /// session's audio scrub is on.
    fn scrub_grain(&mut self, key: Key, media: i64) {
        if self.config.host_clock
            || !self.audio_prefs.get(&key).is_some_and(|p| p.scrub)
            || self.views.get(&key).and_then(|v| v.reported.as_ref()) != Some(&Route::Engine)
        {
            return;
        }
        if let Some(audio) = self.audio_for(key) {
            audio.send(AudioCtl::Scrub(media));
        }
    }

    /// The mixer's monitor solo and waveform requests from the renderer.
    fn audio_message(&mut self, key: Key, message: AudioMessage) {
        match message {
            AudioMessage::Monitor { solo } => {
                let solo = (!solo.is_empty()).then(|| solo.into_iter().collect());
                self.audio_prefs.entry(key).or_default().solo = solo.clone();
                if let Some(audio) = self.audio.get(&key) {
                    audio.send(AudioCtl::Solo(solo));
                }
            }
            AudioMessage::PeaksWanted { asset } => {
                let path = self.docs.get(&key).and_then(|d| d.media().path(&asset));
                self.peaks.request(key, asset, path);
            }
            AudioMessage::Levels(_) | AudioMessage::Peaks(_) => self.log(
                LogLevel::Warn,
                "the renderer sent an engine-to-renderer audio message".into(),
            ),
        }
    }

    /// The view's audio playback, started at its first use; `None` when no
    /// output device opens (the run uses the host clock).
    fn audio_for(&mut self, key: Key) -> Option<&AudioLink> {
        if !self.audio.contains_key(&key) {
            let doc = self.docs.get(&key)?;
            let composition = self.views.get(&key)?.state.composition.as_str().to_owned();
            let loop_range = self.views.get(&key)?.clock.loop_range();
            let sink = self.sink.clone();
            let levels: super::audio::LevelsSink = Box::new(move |levels| {
                let envelope = Envelope::new(Message::Event {
                    body: Event::Audio(AudioMessage::Levels(levels)),
                })
                .with_session(key.1);
                if let Ok(bytes) = codec::encode(&envelope) {
                    sink(vec![Action::Send { port: key.0, bytes }]);
                }
            });
            match AudioLink::start(
                doc.document().clone(),
                composition,
                doc.media().clone(),
                levels,
            ) {
                Ok(link) => {
                    link.send(AudioCtl::SetLoop(loop_range));
                    let prefs = self.audio_prefs.get(&key).cloned().unwrap_or_default();
                    link.send(AudioCtl::AudioScrub(prefs.scrub));
                    link.send(AudioCtl::Solo(prefs.solo));
                    self.audio.insert(key, link);
                }
                Err(error) => {
                    self.log(LogLevel::Error, format!("viewer audio: {error}"));
                    return None;
                }
            }
        }
        let link = self.audio.get(&key)?;
        (!link.failed()).then_some(link)
    }

    fn retire(&mut self, mut view: View) {
        if let Some(live) = view.pool.take() {
            self.retire_pool(view.key, live);
        }
    }

    /// Tells main to stop hosting the viewer's remote layer (C′ prototype).
    fn withdraw_remote(&self, key: Key, generation: u32) {
        if let Some(bridge) = &self.bridge
            && let Err(error) = bridge.remote_layer(key.0.0, generation, 0, (0, 0), 0)
        {
            self.log(
                LogLevel::Warn,
                format!("frame bridge: could not withdraw the remote layer: {error}"),
            );
        }
    }

    fn retire_pool(&self, key: Key, live: Live) {
        if live.remote.is_some() {
            self.withdraw_remote(key, live.spec.generation);
        }
        if let Some(bridge) = &self.bridge
            && let Err(error) = bridge.retire(key.0.0, live.spec.generation)
        {
            self.log(
                LogLevel::Warn,
                format!(
                    "frame bridge: could not retire generation {}: {error}",
                    live.spec.generation
                ),
            );
        }
        // The engine's references go; main and Chromium keep theirs until
        // they release them.
        drop(live);
    }

    fn configure(
        &mut self,
        key: Key,
        state: ViewerState,
        revision: Option<Revision>,
        pool: Option<PoolSpec>,
    ) {
        let frame_ticks = self.frame_ticks;
        let view = self.views.entry(key).or_insert_with(|| View {
            key,
            state: state.clone(),
            revision: Revision::ZERO,
            pool: None,
            clock: TransportClock::new(frame_ticks),
            schedule: Schedule::default(),
            last_sample: None,
            sample_due: true,
            next_seq: 1,
            dirty: true,
            requests: VecDeque::new(),
            retry: None,
            reported: None,
            awaiting: None,
            audio_version: 0,
            #[cfg(feature = "remote-layer")]
            rate: None,
            #[cfg(feature = "remote-layer")]
            rate_tried: false,
        });
        // A new vsync hint alone is not a new view: nothing to redraw.
        let hint_only = view.state.vsync != state.vsync && {
            let mut a = view.state.clone();
            a.vsync = state.vsync;
            a == state
        };
        let composition_changed = view.state.composition != state.composition;
        view.state = state;
        if let Some(revision) = revision {
            view.revision = view.revision.max(revision);
        }
        if !hint_only {
            view.dirty = true;
        }
        let refresh = composition_changed || !view.clock.is_playing();
        if composition_changed {
            view.reported = None;
        }
        match pool {
            // The same pool as before. If making it failed and the retries
            // ran out, this new viewer state tries again.
            None => {
                if let Some(retry) = view.retry.as_mut() {
                    retry.rearm(now());
                }
            }
            Some(_) => view.retry = None,
        }
        if composition_changed && let Some(audio) = self.audio.get(&key) {
            audio.send(AudioCtl::Composition(
                self.views[&key].state.composition.as_str().to_owned(),
            ));
        }
        if refresh {
            self.refresh_timing(key);
        }
        let Some(spec) = pool else {
            return;
        };
        // The C′ prototype's layer moves to the new pool: main keeps
        // hosting its context, so a live resize never leaves it unhosted.
        let mut carried = None;
        if let Some(mut old) = self.views.get_mut(&key).and_then(|v| v.pool.take()) {
            carried = old.remote.take();
            self.retire_pool(key, old);
        }
        self.make_pool(key, spec, 0, carried);
    }

    /// Makes and hands off a pool; on failure, schedules it again with a
    /// fresh generation (`PoolRetry`). The JavaScript side charged the
    /// lease for `spec` and records it as the viewer's pool, so it would
    /// never ask again for an identical viewer state: the render thread
    /// must.
    fn make_pool(&mut self, key: Key, spec: PoolSpec, failures: u32, carried: Option<RemoteLayer>) {
        let had_remote = carried.is_some();
        match self.create_pool(key, spec, carried) {
            Ok(live) => {
                let descriptor = live.pool.descriptor();
                if let Some(view) = self.views.get_mut(&key) {
                    view.pool = Some(live);
                    view.retry = None;
                    view.dirty = true;
                }
                if failures > 0 {
                    self.log(
                        LogLevel::Info,
                        format!(
                            "frame bridge: pool generation {} made after {failures} failed attempts",
                            spec.generation
                        ),
                    );
                }
                self.event(key, Event::SurfacePool(descriptor), None);
            }
            Err(error) => {
                // The carried layer went with the failed pool: main stops
                // hosting its context.
                if had_remote {
                    self.withdraw_remote(key, spec.generation);
                }
                let failures = failures + 1;
                self.stats().pool_failures += 1;
                // Without a bridge nothing can succeed; do not retry.
                let retry = self
                    .bridge
                    .as_ref()
                    .map(|_| PoolRetry::after_failure(spec, failures, now()));
                let next = match retry.and_then(|r| r.at) {
                    Some(at) => format!("trying again in {} ms", (at - now()).max(0) / 1000),
                    None => "waiting for the next viewer state".into(),
                };
                self.log(
                    LogLevel::Error,
                    format!(
                        "frame bridge: pool generation {} ({}x{} {:?}) failed: {error}; {next}",
                        spec.generation, spec.size.width, spec.size.height, spec.format
                    ),
                );
                if let Some(view) = self.views.get_mut(&key) {
                    view.retry = retry;
                }
            }
        }
    }

    /// Makes a failed pool again once its retry is due, as a new generation.
    fn retry_pool(&mut self, key: Key) {
        let Some(retry) = self
            .views
            .get(&key)
            .and_then(|v| v.retry)
            .filter(|r| r.due(now()))
        else {
            return;
        };
        let spec = PoolSpec {
            generation: self.generations.next(),
            ..retry.spec
        };
        self.make_pool(key, spec, retry.failures, None);
    }

    fn create_pool(
        &mut self,
        key: Key,
        spec: PoolSpec,
        carried: Option<RemoteLayer>,
    ) -> Result<Live, String> {
        if self.bridge.is_none() {
            return Err("the frame bridge did not check in with main".into());
        }
        let extra_passes = self.config.extra_passes;
        let document = self.config.document;
        let gpu = self.gpu().ok_or("no GPU device")?;
        let pool = SurfacePool::new(
            &gpu.device,
            spec.generation,
            spec.size,
            spec.format,
            spec.count,
        )
        .map_err(|e| e.to_string())?;
        let working = WorkingTexture::new(&gpu.device, spec.size);
        let source = (!document).then(|| TestSource::new(&gpu.device, spec.size, extra_passes));
        let bridge = self.bridge.as_ref().ok_or("no bridge")?;
        let format = match spec.format {
            SurfaceFormat::Bgra8Unorm => PixelFormat::Bgra8,
            _ => PixelFormat::Rgba16F,
        };
        for (index, surface) in pool.surfaces.iter().enumerate() {
            let port = surface
                .iosurface
                .mach_port()
                .ok_or("IOSurfaceCreateMachPort failed")?;
            let info = SurfaceInfo {
                client_port: key.0.0,
                generation: spec.generation,
                pool_index: index as u32,
                pool_size: spec.count,
                width: spec.size.width,
                height: spec.size.height,
                format,
                bytes_per_row: surface.iosurface.bytes_per_row(),
            };
            if let Err(e) = bridge.send_surface(SendRight::from_raw(port), &info) {
                // Main may hold the surfaces sent before this one: retire the
                // generation so it lets go of them now, not at engine exit.
                if index > 0
                    && let Err(retire) = bridge.retire(key.0.0, spec.generation)
                {
                    return Err(format!(
                        "sending surface {index}: {e}; retiring the {index} sent: {retire}"
                    ));
                }
                return Err(format!("sending surface {index}: {e}"));
            }
        }
        let mut stats = self.stats();
        stats.pools += 1;
        stats.surfaces_sent += u64::from(spec.count);
        drop(stats);
        let remote = if self.config.remote_layer {
            self.make_remote(key, spec, carried)
        } else {
            None
        };
        Ok(Live {
            spec,
            ledger: PoolLedger::new(pool.len()),
            pool,
            working,
            source,
            remote,
        })
    }

    /// The C′ prototype: a remote layer for `spec`, announced to main. The
    /// previous pool's layer, when there is one, is moved to the new size
    /// and format and keeps its context.
    #[cfg(feature = "remote-layer")]
    fn make_remote(
        &mut self,
        key: Key,
        spec: PoolSpec,
        carried: Option<RemoteLayer>,
    ) -> Option<RemoteLayer> {
        let (scale, space) = self
            .views
            .get(&key)
            .map(|v| (v.state.device_pixel_ratio, v.state.color_space))?;
        let reused = carried.and_then(|mut layer| {
            match layer.reconfigure(spec.size, scale, spec.format, space) {
                Ok(()) => Some(layer),
                Err(error) => {
                    self.log(
                        LogLevel::Warn,
                        format!("frame bridge: remote layer not reused (C′ prototype): {error}"),
                    );
                    None
                }
            }
        });
        let made = match reused {
            Some(layer) => Ok(layer),
            None => {
                let device = self
                    .gpu
                    .as_ref()
                    .and_then(|g| raw_metal_device(&g.device))?;
                RemoteLayer::new(device, spec.size, scale, spec.format, space)
                    .map_err(|e| e.to_string())
            }
        };
        let made = made.and_then(|layer| {
            let bridge = self.bridge.as_ref().ok_or("no bridge")?;
            bridge
                .remote_layer(
                    key.0.0,
                    spec.generation,
                    layer.context_id(),
                    (spec.size.width, spec.size.height),
                    (scale * 1000.0).round() as u32,
                )
                .map_err(|e| e.to_string())?;
            Ok(layer)
        });
        match made {
            Ok(layer) => {
                self.log(
                    LogLevel::Info,
                    format!(
                        "frame bridge: remote layer context {} for generation {} (C′ prototype)",
                        layer.context_id(),
                        spec.generation
                    ),
                );
                Some(layer)
            }
            Err(error) => {
                self.stats().remote_error = Some(error.clone());
                self.log(
                    LogLevel::Error,
                    format!("frame bridge: no remote layer (C′ prototype): {error}"),
                );
                None
            }
        }
    }

    /// Asked for the C′ prototype in a build without it (feature
    /// `remote-layer`): says so, and the viewer presents as usual.
    #[cfg(not(feature = "remote-layer"))]
    fn make_remote(
        &mut self,
        _key: Key,
        _spec: PoolSpec,
        _carried: Option<RemoteLayer>,
    ) -> Option<RemoteLayer> {
        let error = "this engine was built without the C′ prototype (feature remote-layer)";
        self.stats().remote_error = Some(error.into());
        self.log(
            LogLevel::Error,
            format!("frame bridge: no remote layer: {error}"),
        );
        None
    }

    /// The C′ prototype: hands a rendered timed frame to the remote layer
    /// to show at host µs `shown_from`, then frees its surface.
    #[cfg(feature = "remote-layer")]
    fn present_remote(&mut self, key: Key, descriptor: &FrameDescriptor, shown_from: f64) {
        let Some(view) = self.views.get_mut(&key) else {
            return;
        };
        let Some(live) = view.pool.as_mut() else {
            return;
        };
        let index = descriptor.pool_index;
        let result = match (live.remote.as_mut(), live.pool.surfaces.get(index as usize)) {
            (Some(remote), Some(surface)) => remote
                .present(
                    &surface.iosurface,
                    descriptor.seq.get(),
                    shown_from.round() as i64,
                )
                .map(|shown| (shown, remote.resyncs()))
                .map_err(|e| e.to_string()),
            _ => Err("no remote layer or surface".into()),
        };
        let _ = live.ledger.release(FrameRelease {
            pool_index: index,
            seq: descriptor.seq,
        });
        let mut stats = self.stats();
        match result {
            Ok((true, resyncs)) => {
                stats.remote_frames += 1;
                stats.remote_resyncs = resyncs;
            }
            // Skipped to drain Core Animation's queue (Pacing).
            Ok((false, resyncs)) => stats.remote_resyncs = resyncs,
            Err(error) => stats.remote_error = Some(error),
        }
    }

    /// Without the C′ prototype no pool has a remote layer (the type is
    /// uninhabited), so this is never called.
    #[cfg(not(feature = "remote-layer"))]
    fn present_remote(&mut self, _key: Key, _descriptor: &FrameDescriptor, _shown_from: f64) {
        unreachable!("no remote layer exists without the remote-layer feature")
    }

    /// Collects the remote layers' presentation records into the stats and
    /// the trace file.
    #[cfg(feature = "remote-layer")]
    fn collect_remote(&mut self) {
        // The refreshes the rate request's display link saw, next to the
        // presentation records (`<trace>.ticks`: host µs of each refresh).
        if self.config.remote_rate
            && let Some(path) = &self.config.remote_trace
        {
            let ticks = take_display_ticks();
            if !ticks.is_empty() {
                use std::io::Write;
                let mut text = String::new();
                for (at, _) in ticks {
                    text.push_str(&format!("{}\n", (at * 1e6).round() as i64));
                }
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(format!("{path}.ticks"))
                {
                    let _ = file.write_all(text.as_bytes());
                }
            }
        }
        let mut records: Vec<Presented> = Vec::new();
        for view in self.views.values() {
            if let Some(remote) = view.pool.as_ref().and_then(|l| l.remote.as_ref()) {
                records.extend(remote.take_presented());
            }
        }
        if records.is_empty() {
            return;
        }
        let mut stats = self.stats();
        for r in &records {
            if r.presented_us.is_some() {
                stats.remote_presented += 1;
            } else {
                stats.remote_dropped += 1;
            }
        }
        drop(stats);
        if let Some(path) = &self.config.remote_trace {
            use std::io::Write;
            let mut text = String::new();
            for r in &records {
                text.push_str(&format!(
                    "[{},{},{},{},{}]\n",
                    r.seq,
                    r.asked_us,
                    r.presented_us.map_or("null".into(), |v| v.to_string()),
                    r.committed_us,
                    r.drawable_wait_us
                ));
            }
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = file.write_all(text.as_bytes());
            }
        }
    }

    /// The render phase for a view: its vsync hint and the configured
    /// fraction, when both exist.
    fn phase(config: &ThreadConfig, state: &ViewerState) -> Option<Phase> {
        Some(Phase {
            hint: state.vsync?,
            fraction: config.phase?,
        })
    }

    /// How long to wait for a command before there is work.
    fn next_wake(&self) -> Duration {
        let now = now();
        let mut wake = now + 50_000;
        for view in self.views.values() {
            if let Some(at) = view.retry.and_then(|r| r.at) {
                wake = wake.min(at);
            }
            if view.dirty || !view.requests.is_empty() || view.sample_due {
                return Duration::ZERO;
            }
            if view.awaiting.is_some() || view.clock.following() {
                // Follow the audio clock promptly (it publishes on its own).
                wake = wake.min(now + 2_000);
            }
            if let Some(entry) = view.clock.loop_entry_at() {
                let entry = entry.ceil() as i64;
                if entry > now {
                    wake = wake.min(entry);
                }
            }
            if view.clock.is_playing() {
                let sample_at = view.last_sample.unwrap_or(now) + SAMPLE_INTERVAL_US;
                wake = wake.min(sample_at);
                let mut schedule = view.schedule.clone();
                let phase = Self::phase(&self.config, &view.state);
                match schedule.next_in_phase(&view.clock, now, self.config.lead, phase) {
                    Next::WaitUntil(at) => wake = wake.min(at.ceil() as i64),
                    Next::Render { .. } | Next::Skip { .. } => {
                        // Waiting for a free slot: a release wakes us; poll
                        // briefly in case it already came.
                        wake = wake.min(now + 1_000);
                    }
                    Next::Idle => {}
                }
            }
        }
        Duration::from_micros((wake - now).max(0) as u64)
    }

    fn work(&mut self) {
        if self
            .gpu
            .as_ref()
            .is_some_and(|g| g.lost.load(Ordering::Acquire))
        {
            self.recover_gpu();
        }
        let keys: Vec<Key> = self.views.keys().copied().collect();
        for key in keys {
            self.work_view(key);
        }
        #[cfg(feature = "remote-layer")]
        if self.config.remote_layer {
            self.collect_remote();
            if self.config.remote_rate {
                self.request_rates();
            }
        }
    }

    /// Holds a refresh-rate request for each view whose C′ layer plays,
    /// and drops it when playback stops.
    #[cfg(feature = "remote-layer")]
    fn request_rates(&mut self) {
        let fps = f64::from(self.config.fps);
        let mut asked = None;
        let mut failed = None;
        for view in self.views.values_mut() {
            let playing =
                view.clock.is_playing() && view.pool.as_ref().is_some_and(|l| l.remote.is_some());
            if !playing {
                view.rate = None;
                view.rate_tried = false;
                continue;
            }
            if view.rate_tried {
                continue;
            }
            view.rate_tried = true;
            let max_hz = view
                .state
                .vsync
                .map_or(120.0, |h| (1e6 / h.period_us).round());
            match RatePreference::request(preferred_refresh(fps, max_hz)) {
                Ok(rate) => {
                    asked = Some(rate.hz);
                    view.rate = Some(rate);
                }
                Err(error) => failed = Some(error.to_string()),
            }
        }
        if asked.is_some() || failed.is_some() {
            let mut stats = self.stats();
            if let Some(hz) = asked {
                stats.remote_rate_hz = Some(hz);
            }
            if let Some(error) = failed {
                stats.remote_error = Some(format!("refresh rate: {error}"));
            }
        }
    }

    /// The audio clock, the host-clock fallback, the end of a run, and the
    /// routing decision of a document view. Returns whether the engine
    /// draws its frames now.
    fn document_step(&mut self, key: Key) -> bool {
        let t = now();
        let Some(view) = self.views.get_mut(&key) else {
            return false;
        };
        let mut fell_back = false;
        if let Some(link) = self.audio.get(&key) {
            if let Some(handle) = link.clock() {
                let version = handle.version();
                if version != view.audio_version {
                    view.audio_version = version;
                    if let Some(sample) = handle.latest() {
                        let accept = match view.awaiting {
                            Some((_, before, _)) => {
                                sample.state != TransportState::Stopped
                                    && Some(sample.epoch) != before
                            }
                            None => view.clock.following(),
                        };
                        if accept {
                            if view.clock.follow(&sample) {
                                view.schedule.reset();
                                if !view.clock.is_playing() {
                                    view.dirty = true;
                                }
                            }
                            if view.clock.is_playing() {
                                view.awaiting = None;
                            }
                            view.sample_due = true;
                        }
                    }
                }
            }
            if let Some((deadline, _, rate)) = view.awaiting
                && (link.failed() || t >= deadline)
            {
                // No playing sample from the device: start on the host clock.
                view.awaiting = None;
                view.clock.apply(TransportCommand::Play { rate }, t);
                view.schedule.reset();
                view.sample_due = true;
                link.send(AudioCtl::Pause);
                fell_back = true;
            }
        }
        if view.clock.stop_at_end(t) {
            view.schedule.reset();
            view.sample_due = true;
            view.dirty = true;
        }
        if let Some(entry) = view.clock.loop_entry_at()
            && entry <= t as f64
            && view.last_sample.is_some_and(|l| (l as f64) < entry)
        {
            view.sample_due = true;
        }
        let composition = view.state.composition.as_str().to_owned();
        let route = self.docs.entry(key).or_default().route(&composition);
        let view = self.views.get_mut(&key).expect("checked");
        let changed = view.reported.as_ref() != Some(&route);
        let mut stop = false;
        if changed {
            view.reported = Some(route.clone());
            match &route {
                Route::Engine => view.dirty = true,
                Route::Legacy(_) | Route::Pending(_) => {
                    stop = view.clock.is_playing() || view.awaiting.is_some();
                }
            }
        }
        if stop {
            view.awaiting = None;
            view.clock.apply(TransportCommand::Pause, t);
            view.schedule.reset();
            view.sample_due = true;
            if let Some(link) = self.audio.get(&key) {
                link.send(AudioCtl::Pause);
            }
        }
        if fell_back {
            self.stats().host_clock_runs += 1;
            self.log(
                LogLevel::Warn,
                "viewer audio did not start; playing on the host clock".into(),
            );
        }
        if changed {
            let text = match &route {
                Route::Engine => "engine".to_owned(),
                Route::Legacy(reasons) => format!("legacy: {}", reasons.join("; ")),
                Route::Pending(why) => format!("pending: {why}"),
            };
            {
                let mut stats = self.stats();
                stats.routes.insert(view_name(key), text);
                if matches!(route, Route::Legacy(_)) {
                    stats.routed_legacy += 1;
                }
            }
            if let Route::Legacy(reasons) = &route {
                self.report_legacy(key, reasons);
            }
        }
        if let Some(link) = self.audio.get(&key) {
            let status = link.status();
            self.stats().audio.insert(view_name(key), status);
        }
        route == Route::Engine
    }

    /// Tells the viewer its composition is drawn by the legacy engine
    /// (capability routing, section 21): a session-scope `unsupported`
    /// failure at the revision decided.
    fn report_legacy(&self, key: Key, reasons: &[String]) {
        let revision = self
            .docs
            .get(&key)
            .and_then(|d| Revision::new(d.revision()).ok())
            .unwrap_or(Revision::ZERO);
        let mut message = format!("Drawn by the current engine: {}.", reasons.join("; "));
        if message.len() > 2_000 {
            message.truncate(1_997);
            message.push_str("...");
        }
        let error = EngineError::new(
            ErrorCode::Unsupported,
            ErrorScope::Session { session: key.1 },
            message,
        );
        self.event(
            key,
            Event::Failure(FailureReport { revision, error }),
            Some(revision),
        );
    }

    fn work_view(&mut self, key: Key) {
        let lead = self.config.lead;
        self.retry_pool(key);
        let draws = !self.config.document || self.document_step(key);
        // Clock samples.
        if let Some(view) = self.views.get_mut(&key) {
            let t = now();
            let periodic = view.clock.is_playing()
                && view
                    .last_sample
                    .is_none_or(|last| t - last >= SAMPLE_INTERVAL_US);
            if view.sample_due || periodic {
                view.sample_due = false;
                view.last_sample = Some(t);
                let sample = view.clock.sample(t);
                self.event(key, Event::Clock(sample), None);
            }
        }
        if !draws {
            // Requests wait while the legacy viewer draws this composition.
            while let Some((id, _, _)) = self
                .views
                .get_mut(&key)
                .and_then(|view| view.requests.pop_front())
            {
                self.error(
                    key,
                    id,
                    ErrorCode::Unsupported,
                    "The current engine draws this composition.",
                );
            }
            return;
        }
        // Frame requests (viewer and scrub): shown at the next refresh.
        while let Some((id, request, revision)) = self
            .views
            .get_mut(&key)
            .and_then(|view| view.requests.pop_front())
        {
            self.answer(key, id, &request, revision);
        }
        // A paused frame after a view or transport change.
        let paused_dirty = self
            .views
            .get(&key)
            .is_some_and(|v| v.dirty && v.pool.is_some() && !v.clock.is_playing());
        if paused_dirty {
            let media = self.views[&key].clock.media_at(now());
            if let Some(descriptor) = self.render(key, media, -1, None) {
                if let Some(view) = self.views.get_mut(&key) {
                    view.dirty = false;
                }
                let revision = descriptor.revision;
                self.event(key, Event::Frame(descriptor), Some(revision));
            }
        } else if let Some(view) = self.views.get_mut(&key)
            && view.clock.is_playing()
        {
            view.dirty = false;
        }
        // Playback: render ahead while frames are due and slots are free.
        loop {
            let Some(view) = self.views.get_mut(&key) else {
                return;
            };
            if view.pool.is_none() {
                return;
            }
            let t = now();
            let phase = Self::phase(&self.config, &view.state);
            match view.schedule.next_in_phase(&view.clock, t, lead, phase) {
                Next::Render { k, shown_from } => {
                    if view
                        .pool
                        .as_ref()
                        .is_some_and(|l| l.ledger.free().is_none())
                    {
                        self.stats().pool_waits += 1;
                        return;
                    }
                    let f = view.clock.frame_ticks();
                    let media = view.clock.wrap(k * f);
                    let shown_for = super::schedule::Schedule::frame_us(&view.clock);
                    let started = now();
                    let rendered = self.render(key, media, k, Some((shown_from, shown_for)));
                    let Some(view) = self.views.get_mut(&key) else {
                        return;
                    };
                    view.schedule.rendered(&view.clock, k);
                    if rendered.is_some() {
                        // What the next frames will cost (deadline-aware skips).
                        view.schedule.observe((now() - started) as f64);
                    }
                    let remote = view.pool.as_ref().is_some_and(|l| l.remote.is_some());
                    if let Some(descriptor) = rendered {
                        if remote {
                            self.present_remote(key, &descriptor, shown_from);
                        } else {
                            let revision = descriptor.revision;
                            self.event(key, Event::Frame(descriptor), Some(revision));
                        }
                    }
                }
                Next::Skip { .. } => {
                    self.stats().late_skipped += 1;
                }
                Next::WaitUntil(_) | Next::Idle => return,
            }
        }
    }

    fn answer(
        &mut self,
        key: Key,
        id: RequestId,
        request: &FrameRequest,
        revision: Option<Revision>,
    ) {
        let Some(view) = self.views.get_mut(&key) else {
            return;
        };
        if let Some(revision) = revision {
            view.revision = view.revision.max(revision);
        }
        let Some(live) = view.pool.as_ref() else {
            self.error(
                key,
                id,
                ErrorCode::Unsupported,
                "The viewer has no surface pool yet.",
            );
            return;
        };
        if request.output_size != live.spec.size {
            self.error(
                key,
                id,
                ErrorCode::InvalidMessage,
                "A viewer frame request must match the viewer state's size.",
            );
            return;
        }
        if live.ledger.free().is_none() {
            self.error(
                key,
                id,
                ErrorCode::BudgetExceeded,
                "Every pool surface is held by the viewer.",
            );
            return;
        }
        let media = request.time.get();
        if let Some(descriptor) = self.render(key, media, -1, None) {
            self.stats().responses += 1;
            let mut envelope = Envelope::new(Message::Response {
                id,
                body: Response::Frame(descriptor.clone()),
            })
            .with_session(key.1);
            envelope.revision = Some(descriptor.revision);
            self.send(key.0, envelope);
        } else {
            self.error(
                key,
                id,
                ErrorCode::Internal,
                "The frame could not be rendered.",
            );
        }
    }

    /// Renders the frame showing `media` into a free surface, waits for the
    /// GPU, and returns its descriptor. `timing` is `(shown_from, shown_for)`
    /// in host µs for timed frames.
    fn render(
        &mut self,
        key: Key,
        media: i64,
        k: i64,
        timing: Option<(f64, f64)>,
    ) -> Option<FrameDescriptor> {
        let trace = self.config.trace;
        let document = self.config.document;
        let gpu = self.gpu.as_mut()?;
        let view = self.views.get_mut(&key)?;
        let live = view.pool.as_mut()?;
        let index = live.ledger.free()?;
        let seq = view.next_seq;
        let start = now();
        let overlay;
        if document {
            let doc = self.docs.get_mut(&key)?;
            let composer = gpu
                .composer
                .get_or_insert_with(|| composer(&gpu.device, &gpu.queue, &self.config));
            let composition = view.state.composition.as_str().to_owned();
            // The renderer draws with this view's definitions (one renderer
            // serves every view on the device).
            let registered = doc.take_registered();
            let _ = composer.use_definitions(key, doc.definitions(), &registered);
            match composer.compose(
                doc.document(),
                &composition,
                doc.media(),
                Ticks::new(media.max(0)).unwrap_or(Ticks::ZERO),
                &view.state,
                &live.working,
            ) {
                Ok(composed) => {
                    view.revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
                    overlay = composed.overlay;
                    let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                    stats.document_frames += 1;
                    stats.compose_us_max = stats.compose_us_max.max(composed.cpu_us);
                    stats.compose_us_total += composed.cpu_us;
                    stats.layer_failures += composed.failures.len() as u64;
                    drop(stats);
                    // Effects the GPU watchdog turned off (measured on an
                    // earlier frame): reported at this frame's revision.
                    for error in composer.renderer().watchdog_reports(false) {
                        let report = pm_protocol::error::FailureReport {
                            revision: view.revision,
                            error,
                        };
                        let envelope = Envelope::new(Message::Event {
                            body: Event::Failure(report),
                        })
                        .with_session(key.1);
                        if let Ok(bytes) = codec::encode(&envelope) {
                            (self.sink)(vec![Action::Send { port: key.0, bytes }]);
                        }
                    }
                }
                Err(pm_render::RenderError::ShaderFallbackPending(requests)) => {
                    // Layers only the fallback runner draws (decision 19):
                    // ask the view's renderer, and render the frame again
                    // when the pixels arrive.
                    let fresh: Vec<_> = requests
                        .into_iter()
                        .filter(|r| !self.fallback.asked.contains_key(&r.key))
                        .collect();
                    for chunk in fresh.chunks(pm_protocol::shader_fallback::MAX_REQUESTS) {
                        for r in chunk {
                            self.fallback.asked.insert(r.key.clone(), key);
                        }
                        let body = Event::ShaderFallback(
                            pm_protocol::shader_fallback::ShaderFallbackMessage::Requests {
                                requests: chunk.to_vec(),
                            },
                        );
                        let mut envelope =
                            Envelope::new(Message::Event { body }).with_session(key.1);
                        envelope.revision = Revision::new(doc.revision()).ok();
                        if let Ok(bytes) = codec::encode(&envelope) {
                            (self.sink)(vec![Action::Send { port: key.0, bytes }]);
                        }
                    }
                    return None;
                }
                Err(pm_render::RenderError::Unsupported(list)) => {
                    let reasons: Vec<String> = list.iter().map(super::document::describe).collect();
                    doc.force_legacy(&composition, reasons);
                    // The next turn reports the route and stops a run.
                    view.dirty = true;
                    return None;
                }
                Err(error) => {
                    self.stats
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .render_errors += 1;
                    (self.sink)(vec![Action::Log {
                        level: LogLevel::Error,
                        message: format!("viewer: the frame could not be rendered: {error}"),
                    }]);
                    return None;
                }
            }
        } else {
            let scene = view::scene(&view.state);
            overlay = scene.overlay;
            let frame = TestFrame {
                time_s: (media as f64 / TICKS_PER_SECOND as f64) as f32,
                seq,
                view_seq: view.state.view_seq.get(),
                composition: scene.composition,
                marker: scene.marker,
                camera: scene.camera,
            };
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            live.source
                .as_ref()?
                .encode(&gpu.queue, &mut encoder, &live.working, &frame);
            gpu.queue.submit([encoder.finish()]);
        }
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let params = DisplayParams {
            working: WorkingEncoding::Standard,
            canvas: view.state.color_space,
            noise_offset: DisplayParams::noise_offset_for(seq),
        };
        let surface = &live.pool.surfaces[index as usize];
        if let Err(error) = gpu.display.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &live.working,
            surface,
            params,
        ) {
            let message = format!("frame bridge: display transform failed: {error}");
            (self.sink)(vec![Action::Log {
                level: LogLevel::Error,
                message,
            }]);
            return None;
        }
        let submission = gpu.queue.submit([encoder.finish()]);
        let done = gpu.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(2)),
        });
        let gpu_done = now();
        if let Err(error) = done {
            let message = format!("frame bridge: the GPU did not finish a frame: {error}");
            (self.sink)(vec![Action::Log {
                level: LogLevel::Error,
                message,
            }]);
            return None;
        }
        let seq_value = Seq::new(seq).ok()?;
        if !live.ledger.present(index, seq_value) {
            return None;
        }
        view.next_seq += 1;
        let duration = timing.map(|_| Ticks::new(view.clock.frame_ticks()).unwrap_or(Ticks::MAX));
        let descriptor = FrameDescriptor {
            pool_index: index,
            pool_generation: Seq::new(u64::from(live.spec.generation)).unwrap_or(Seq::MAX),
            seq: seq_value,
            pts: Ticks::new(media).unwrap_or(Ticks::ZERO),
            revision: view.revision,
            color_space: view.state.color_space,
            format: live.spec.format,
            quality: FrameQuality {
                resolution: match view.state.resolution {
                    PreviewResolution::Automatic => PreviewResolution::Full,
                    other => other,
                },
                representation: Representation::Original,
                draft_3d: false,
                cached: false,
            },
            overlay: Some(overlay),
            depth_id: None,
            view_seq: view.state.view_seq,
            duration,
        };
        let held = live.ledger.held_count() as u32;
        let generation = live.spec.generation;
        let view_seq = view.state.view_seq.get();
        let revision = view.revision.get();
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        stats.frames += 1;
        if timing.is_some() {
            stats.timed_frames += 1;
        } else {
            stats.immediate_frames += 1;
        }
        stats.gpu_us_max = stats.gpu_us_max.max(gpu_done - start);
        stats.gpu_us_total += gpu_done - start;
        stats.max_held = stats.max_held.max(held);
        if trace && stats.trace.len() < TRACE_CAP {
            let (shown_from, shown_for) = timing.unwrap_or((0.0, 0.0));
            stats.trace.push(FrameTrace {
                seq,
                k,
                slot: index,
                generation,
                shown_from: shown_from.round() as i64,
                shown_for: shown_for.round() as i64,
                view_seq,
                revision,
                start,
                gpu_done,
                sent: now(),
                timed: timing.is_some(),
            });
        }
        Some(descriptor)
    }

    fn shutdown(&mut self) {
        let views: Vec<View> = std::mem::take(&mut self.views).into_values().collect();
        for view in views {
            self.retire(view);
        }
        if let Some(bridge) = self.bridge.take() {
            let _ = bridge.goodbye();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> PoolSpec {
        PoolSpec {
            generation: 3,
            size: Size::new(1280, 720),
            format: SurfaceFormat::Bgra8Unorm,
            count: 8,
        }
    }

    #[test]
    fn a_failed_pool_is_retried_with_backoff_then_waits_for_a_new_viewer_state() {
        let mut at = Vec::new();
        for failures in 1..=POOL_RETRY_US.len() as u32 + 1 {
            let retry = PoolRetry::after_failure(spec(), failures, 1_000);
            at.push(retry.at);
        }
        let expected: Vec<Option<i64>> = POOL_RETRY_US
            .iter()
            .map(|d| Some(1_000 + d))
            .chain([None])
            .collect();
        assert_eq!(at, expected);

        let mut retry = PoolRetry::after_failure(spec(), 1, 0);
        assert!(!retry.due(POOL_RETRY_US[0] - 1));
        assert!(retry.due(POOL_RETRY_US[0]));
        // A new viewer state does not hurry a pending retry...
        retry.rearm(10);
        assert_eq!(retry.at, Some(POOL_RETRY_US[0]));
        // ...but tries again at once after the retries ran out.
        let mut exhausted = PoolRetry::after_failure(spec(), POOL_RETRY_US.len() as u32 + 1, 0);
        assert!(!exhausted.due(i64::MAX));
        exhausted.rearm(77);
        assert!(exhausted.due(77));
        assert_eq!(exhausted.spec, spec());
    }

    #[test]
    fn generations_are_shared_and_never_zero() {
        let a = Generations::default();
        let b = a.clone();
        assert_eq!(a.next(), 1);
        assert_eq!(b.next(), 2);
        let wrap = Generations(Arc::new(AtomicU32::new(u32::MAX)));
        assert_eq!(wrap.next(), u32::MAX);
        assert_eq!(wrap.next(), 1);
    }
}
