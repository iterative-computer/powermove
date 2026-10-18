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

use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
        /// `Some(Some)`: a new depth and object-id pool; `Some(None)`:
        /// retire it; `None`: unchanged.
        depth: Option<Option<PoolSpec>>,
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
    /// The viewer's frame-cache share of the lease (bytes): the composer's
    /// decode service evicts down to it at once.
    FrameCache {
        bytes: usize,
    },
    /// A host snapshot of a session's document (the Phase 3c render
    /// transport, [`super::snapshot`]).
    Snapshot(Box<super::snapshot::SnapshotJob>),
    /// Compatibility pixels the session's renderer uploaded
    /// ([`super::compat`]).
    Compat {
        key: Key,
        message: pm_protocol::snapshot::CompatMessage,
    },
    /// Destroys the GPU device as a real loss would (the recovery test).
    LoseDevice,
    /// The font service's generation moved (project fonts): views draw
    /// again with fresh fonts.
    FontsChanged,
    /// The paused viewer's Blender previews (sent once, at start).
    UseBlender(Arc<super::blender::PreviewBlender>),
    /// Preview surfaces of the view arrived: draw it again when paused.
    BlenderReady {
        key: Key,
    },
    /// A snapshot's Blender frames are prepared, or why not
    /// ([`super::blender::PreviewBlender::capture`]).
    BlenderCaptured {
        token: u64,
        result: Result<Arc<Vec<super::snapshot::PreparedFrame>>, pm_blender::BlenderError>,
    },
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
    /// Keep [`Stats::stand_in_log`] (measurements only).
    pub stand_in_log: bool,
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
    /// The engine's render fonts (the host's font broker): text and
    /// captions layers are drawn natively. `None`: no fonts, those layers
    /// are refused.
    pub fonts: Option<crate::fonts::render::RenderFonts>,
}

/// Today's capture error for a Blender composition whose frame was not
/// rendered by Blender first (`legacy/gl/compositor.ts`), word for word.
pub const BLENDER_NOT_PREPARED: &str =
    "The Blender frame is not prepared. Render this frame before capturing it.";

/// The device's composer, keeping translated programs in the configured
/// cache folder.
fn composer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    config: &ThreadConfig,
    frame_cache: usize,
) -> Composer {
    let mut composer = Composer::new(device, queue);
    composer
        .renderer()
        .set_program_cache_dir(config.shader_cache.clone());
    // Viewer sessions come from a renderer with the decision 19 fallback
    // runner (`engine-connection/shader-fallback.ts`).
    composer.renderer().set_shader_fallback_runner(true);
    composer
        .renderer()
        .apply_media_lease(media_lease(frame_cache));
    composer
}

/// The composer's decode-service shares: the frame cache follows the
/// presenter's lease share; decoders are not leased yet
/// (docs/engine/scrub.md, "Memory").
fn media_lease(frame_cache: usize) -> pm_media::decode::service::MediaLease {
    pm_media::decode::service::MediaLease {
        frame_cache,
        decoders: usize::MAX,
    }
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
    /// Frames rendered ahead that an edit made stale, rendered again at the
    /// new revision (docs/engine/perf.md).
    pub rerendered: u64,
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
    /// Runs of a composition with nothing to hear, on the host clock from
    /// Play (today's playhead does not wait for the audio device either).
    pub silent_runs: u64,
    /// Audio playback, per view.
    pub audio: BTreeMap<String, AudioStatus>,
    /// The paused viewer's Blender previews.
    pub blender_preview: super::blender::BlenderPreviewStats,
    /// Frames drawn with stand-ins while scrubbing, and the frames drawn
    /// again once the exact video arrived (docs/engine/scrub.md).
    pub stand_in_frames: u64,
    pub stand_in_redraws: u64,
    /// Stand-in frames sent: (seq, the source frame the first stand-in
    /// layer shows), for measurements that count the distinct frames a
    /// drag showed. Kept only with [`ThreadConfig::stand_in_log`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stand_in_log: Vec<(u64, usize)>,
    /// The viewer's frame cache in bytes (its lease share,
    /// docs/engine/scrub.md); 0 until the presenter sized it.
    pub frame_cache: u64,
    /// The display decode workers (scrubbing, look-ahead, cancellation).
    pub scrub: Option<pm_media::decode::scrub::ScrubStats>,
    /// Snapshots (the Phase 3c render transport): answered with pixels,
    /// answered `needsRenderer`, failed (including timeouts and refusals),
    /// the slowest render, and how many went through shared memory.
    pub snapshots: u64,
    pub snapshot_needs: u64,
    pub snapshot_failures: u64,
    /// Snapshots whose Blender frames were sent to the Blender worker
    /// first (today's `prepareFrame` before an async capture).
    pub snapshot_blender: u64,
    pub snapshot_us_max: i64,
    pub snapshot_shared: u64,
    /// Compatibility rasters (the `raster` override, compatibility
    /// contexts): complete ones received, held per view, and bytes held.
    pub compat_rasters_received: u64,
    /// Depth and object-id pools made, and frames that carried a surface.
    pub depth_pools: u64,
    pub depth_frames: u64,
    pub compat_rasters: BTreeMap<String, usize>,
    pub compat_bytes: u64,
    #[serde(skip)]
    pub trace: Vec<FrameTrace>,
}

const TRACE_CAP: usize = 200_000;

/// Entries kept in [`Stats::stand_in_log`].
const STAND_IN_LOG_CAP: usize = 20_000;

/// How often a frame drawn with stand-ins checks whether its video decoded.
const STAND_IN_POLL_US: i64 = 1_000;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    display: DisplayTransform,
    /// Draws documents (made with the first document frame).
    composer: Option<Composer>,
    /// The font generation the composer's renderer lays text out with.
    fonts_installed: Option<u64>,
    /// Set by the device-lost callback; the thread rebuilds the device.
    lost: Arc<AtomicBool>,
}

impl Gpu {
    /// The device's composer (made on first use), its renderer holding
    /// fonts of the broker's current generation (`crate::fonts::render`).
    fn composer_with(&mut self, config: &ThreadConfig, frame_cache: usize) -> &mut Composer {
        if self.composer.is_none() {
            self.composer = Some(composer(&self.device, &self.queue, config, frame_cache));
            self.fonts_installed = None;
        }
        let composer = self.composer.as_mut().expect("made above");
        if let Some(fonts) = &config.fonts {
            fonts.sync(composer.renderer(), &mut self.fonts_installed);
        }
        composer
    }
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

/// A view's depth and object-id surfaces (plan section 8). Frame `seq`
/// writes slot `seq % count`, so a slot is reused only after `count`
/// newer frames: more than the frames in flight (ring + render-ahead), so
/// the surface of the frame on screen is never overwritten.
struct DepthPool {
    spec: PoolSpec,
    pool: SurfacePool,
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
    /// What the shader UI was last told about this view's layers.
    layer_reports: super::layer_reports::LayerReports,
    /// A Play waits for the audio clock's first playing sample: the host
    /// µs after which it starts on the host clock instead, and the audio
    /// epoch that was current when it was asked (not the answer).
    awaiting: Option<(i64, Option<u32>, PlaybackRateValue)>,
    /// The audio clock version last followed.
    audio_version: u64,
    /// Paused after a `seek {scrub}`: frames may show stand-ins for video
    /// still decoding (docs/engine/scrub.md). Exact seeks, steps, and
    /// playback clear it.
    scrub: bool,
    /// The frame on screen showed stand-ins: the media "landed" counter when
    /// it was drawn. A change draws the frame again.
    stand_in: Option<u64>,
    /// What presenting this session costs (`RenderStats`).
    costs: super::stats::ViewStats,
    /// The depth and object-id pool, while the viewer asks for it.
    depth: Option<DepthPool>,
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
    /// The paused viewer's Blender previews (document source).
    blender: Option<Arc<super::blender::PreviewBlender>>,
    /// The last capture token handed to the Blender worker.
    blender_token: u64,
    /// Shader fallback requests sent to each view's renderer and not yet
    /// answered (by request key), and the bands received so far.
    fallback: Fallback,
    /// The viewer's frame-cache share ([`Command::FrameCache`]).
    frame_cache: usize,
    /// Host snapshots waiting for their revision or their turn.
    snapshots: super::snapshot::SnapshotQueue,
    /// Shared-memory segments published for main and not reaped yet.
    segments: pm_bridge::shm::Segments,
    /// Host µs of the newest frame drawn for a gesture, scrub, or edit
    /// (idle snapshots wait for these to stop).
    last_interactive: Option<i64>,
    /// Snapshots served per session, and the slowest (µs).
    snapshot_costs: BTreeMap<Key, (u64, i64)>,
    /// Compatibility rasters the renderers uploaded.
    compat: super::compat::CompatStore,
    /// Waveform requests waiting for the host's media answer.
    peaks_waiting: BTreeSet<(Key, String)>,
}

/// Most waveform requests that wait for media answers at once.
const MAX_PEAKS_WAITING: usize = 4096;

/// The decision 19 fallback runner's state on this thread.
#[derive(Default)]
struct Fallback {
    asked: BTreeMap<String, Key>,
    bands: pm_render::shader_layers::Bands,
    /// Layers each view was told draw through the runner (logged once per
    /// layer, not once per animated frame's request).
    logged: BTreeSet<(Key, String)>,
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
        frame_cache: pm_render::media::DEFAULT_FRAME_CACHE,
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
        blender: None,
        blender_token: 0,
        fallback: Fallback::default(),
        snapshots: super::snapshot::SnapshotQueue::default(),
        segments: pm_bridge::shm::Segments::default(),
        last_interactive: None,
        snapshot_costs: BTreeMap::new(),
        compat: super::compat::CompatStore::default(),
        peaks_waiting: BTreeSet::new(),
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
                        fonts_installed: None,
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
        let mut remake_depth = Vec::new();
        for key in keys {
            if let Some(depth) = self.views.get_mut(&key).and_then(|v| v.depth.take()) {
                self.retire_generation(key, depth.spec.generation);
                remake_depth.push((key, depth.spec));
            }
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
        for (key, spec) in remake_depth {
            let spec = PoolSpec {
                generation: self.generations.next(),
                ..spec
            };
            self.configure_depth(key, Some(spec));
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
                depth,
            } => {
                self.configure(key, *state, revision, pool);
                if let Some(depth) = depth {
                    self.configure_depth(key, depth);
                }
            }
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
                    let composer = gpu.composer_with(&self.config, self.frame_cache);
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
                if self.peaks_waiting.remove(&(key, asset.clone())) {
                    self.peaks.request(key, asset.clone(), path.clone());
                }
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
            Command::FrameCache { bytes } => {
                self.frame_cache = bytes;
                self.stats().frame_cache = bytes as u64;
                if let Some(composer) = self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
                    composer.renderer().apply_media_lease(media_lease(bytes));
                }
            }
            Command::Snapshot(job) => {
                if let Err(refused) = self.snapshots.push(*job) {
                    let (job, refused) = *refused;
                    self.stats().snapshot_failures += 1;
                    self.snapshot_error(&job, ErrorCode::BudgetExceeded, &refused.message());
                }
            }
            Command::Compat { key, message } => {
                let applied = self.compat.apply(key, message);
                let (rasters, bytes) = self.compat.usage();
                let mut stats = self.stats();
                if applied == super::compat::Applied::Complete {
                    stats.compat_rasters_received += 1;
                }
                stats.compat_rasters = rasters;
                stats.compat_bytes = bytes;
            }
            Command::LoseDevice => {
                if let Some(gpu) = &self.gpu {
                    gpu.lost.store(true, Ordering::Release);
                    gpu.device.destroy();
                }
            }
            Command::FontsChanged => self.sync_font_generation(),
            Command::UseBlender(blender) => self.blender = Some(blender),
            Command::BlenderReady { key } => {
                if let Some(view) = self.views.get_mut(&key)
                    && !view.clock.is_playing()
                {
                    view.dirty = true;
                }
            }
            Command::BlenderCaptured { token, result } => self.blender_captured(token, result),
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
        if let Some(blender) = &self.blender {
            blender.forget(key);
        }
        self.snapshot_costs.remove(&key);
        self.compat.forget_session(key);
        self.peaks_waiting.retain(|(k, _)| *k != key);
        for job in self.snapshots.take_session(key) {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::UnknownSession,
                "The window's engine session closed.",
            );
        }
        // Requests the session was asked for, with the bands that arrived.
        let fallback = &mut self.fallback;
        fallback.asked.retain(|request, k| {
            if *k == key {
                fallback.bands.forget(request);
            }
            *k != key
        });
        fallback.logged.retain(|(k, _)| *k != key);
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
        } else {
            self.warm_audio(key);
        }
        self.refresh_timing(key);
        if let Some(view) = self.views.get_mut(&key) {
            view.dirty = true;
            // Frames already rendered ahead show the older revision.
            if view.clock.is_playing() {
                view.schedule.invalidate();
            }
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
        if matches!(
            command,
            TransportCommand::Play { .. }
                | TransportCommand::PlayAround { .. }
                | TransportCommand::Pause
        ) {
            // Playback and the frame a pause lands on are exact.
            view.scrub = false;
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
                // Nothing to hear: the line starts on the host clock at once,
                // as today's playhead does, instead of waiting for the device
                // to present its first (silent) buffer.
                if wants_audio && !self.audible(key) {
                    self.stats().silent_runs += 1;
                    let view = self.views.get_mut(&key).expect("checked");
                    view.clock.apply(TransportCommand::Play { rate }, at);
                    view.schedule.reset();
                    view.sample_due = true;
                } else if wants_audio && self.audio_for(key).is_some() {
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
                view.scrub = mode == SeekMode::Scrub && !playing && awaiting.is_none();
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
                // A step shows its exact frame.
                view.scrub = false;
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
                // Asked before the host answered for the asset (a file just
                // imported): wait for the answer rather than call it offline.
                match self.docs.get(&key).and_then(|d| d.media().answer(&asset)) {
                    Some(path) => self.peaks.request(key, asset, path),
                    None => {
                        if self.peaks_waiting.len() < MAX_PEAKS_WAITING {
                            self.peaks_waiting.insert((key, asset));
                        }
                    }
                }
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

    /// Whether the composition a view shows has anything to hear
    /// (`pm_audio::graph::audible`, without opening media).
    fn audible(&self, key: Key) -> bool {
        let (Some(doc), Some(view)) = (self.docs.get(&key), self.views.get(&key)) else {
            return false;
        };
        if !doc.has_document() {
            return false;
        }
        let mut ev = pm_eval::Evaluator::new();
        pm_audio::graph::audible(doc.document(), view.state.composition.as_str(), &mut ev)
            .unwrap_or(false)
    }

    /// Starts a document view's audio thread before its first Play, so it
    /// opens the output device once the document is audible and the editor
    /// is idle (today's `requestContextWarmup`) and that Play does not wait
    /// for the device to open.
    fn warm_audio(&mut self, key: Key) {
        if self.config.host_clock || !self.config.document || self.audio.contains_key(&key) {
            return;
        }
        let ready =
            self.views.contains_key(&key) && self.docs.get(&key).is_some_and(|d| d.has_document());
        if ready && let Some(link) = self.audio_for(key) {
            link.send(AudioCtl::Warm);
        }
    }

    fn retire(&mut self, mut view: View) {
        if let Some(live) = view.pool.take() {
            self.retire_pool(view.key, live);
        }
        if let Some(depth) = view.depth.take() {
            self.retire_generation(view.key, depth.spec.generation);
        }
    }

    /// Tells main to let go of a generation's surfaces.
    fn retire_generation(&self, key: Key, generation: u32) {
        if let Some(bridge) = &self.bridge
            && let Err(error) = bridge.retire(key.0.0, generation)
        {
            self.log(
                LogLevel::Warn,
                format!("frame bridge: could not retire generation {generation}: {error}"),
            );
        }
    }

    /// Makes, hands off, and announces the view's depth pool, or retires it
    /// (`spec` `None`). A pool that cannot be made is logged: frames go on
    /// without depth (the chrome draws without occlusion), never stop.
    fn configure_depth(&mut self, key: Key, spec: Option<PoolSpec>) {
        if let Some(old) = self.views.get_mut(&key).and_then(|v| v.depth.take()) {
            self.retire_generation(key, old.spec.generation);
        }
        let Some(spec) = spec else {
            return;
        };
        match self.create_surfaces(key, spec) {
            Ok(pool) => {
                let mut descriptor = pool.descriptor();
                descriptor.role = Some(pm_protocol::interop::PoolRole::DepthId);
                if let Some(view) = self.views.get_mut(&key) {
                    view.depth = Some(DepthPool { spec, pool });
                    view.dirty = true;
                }
                self.stats().depth_pools += 1;
                self.event(key, Event::SurfacePool(descriptor), None);
            }
            Err(error) => {
                self.stats().pool_failures += 1;
                self.log(
                    LogLevel::Error,
                    format!(
                        "frame bridge: depth pool generation {} ({}x{}) failed: {error}",
                        spec.generation, spec.size.width, spec.size.height
                    ),
                );
            }
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
            layer_reports: Default::default(),
            awaiting: None,
            audio_version: 0,
            scrub: false,
            stand_in: None,
            costs: Default::default(),
            depth: None,
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
        self.warm_audio(key);
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

    /// Makes a pool's surfaces and sends each to main once (the depth pool;
    /// the color pool adds its working texture and ledger).
    fn create_surfaces(&mut self, key: Key, spec: PoolSpec) -> Result<SurfacePool, String> {
        if self.bridge.is_none() {
            return Err("the frame bridge did not check in with main".into());
        }
        let gpu = self.gpu().ok_or("no GPU device")?;
        let pool = SurfacePool::new(
            &gpu.device,
            spec.generation,
            spec.size,
            spec.format,
            spec.count,
        )
        .map_err(|e| e.to_string())?;
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
        Ok(pool)
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
        if self.snapshots.waiting() {
            // Jobs wait for patches (their commands wake the thread), for
            // room between frames, or for a gesture to end: poll, and never
            // past a deadline.
            wake = wake.min(now + 4_000);
            if let Some(deadline) = self.snapshots.earliest_deadline() {
                wake = wake.min(deadline);
            }
        }
        for view in self.views.values() {
            if let Some(at) = view.retry.and_then(|r| r.at) {
                wake = wake.min(at);
            }
            if view.dirty || !view.requests.is_empty() || view.sample_due {
                return Duration::ZERO;
            }
            if view.stand_in.is_some() {
                // A frame with stand-ins waits for its video to decode.
                wake = wake.min(now + STAND_IN_POLL_US);
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

    /// Views draw again when the font generation moved (project fonts):
    /// the composer installs fresh fonts at its next use.
    fn sync_font_generation(&mut self) {
        let Some(fonts) = &self.config.fonts else {
            return;
        };
        let generation = fonts.broker.generation();
        let installed = self.gpu.as_ref().and_then(|g| g.fonts_installed);
        if installed.is_some_and(|g| g != generation) {
            for view in self.views.values_mut() {
                view.dirty = true;
            }
        }
    }

    fn work(&mut self) {
        self.sync_font_generation();
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
        self.work_snapshots();
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
        let fonts = self.config.fonts.is_some();
        let doc = self.docs.entry(key).or_default();
        doc.set_fonts(fonts);
        let route = doc.route(&composition);
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
        // A frame drawn with stand-ins is drawn again once video it asked
        // for has decoded.
        let landed = self
            .gpu
            .as_mut()
            .and_then(|g| g.composer.as_mut())
            .map(|c| c.renderer().media_landed());
        if let Some(view) = self.views.get_mut(&key)
            && let (Some(at), Some(landed)) = (view.stand_in, landed)
            && at != landed
        {
            if view.clock.is_playing() {
                view.stand_in = None;
            } else {
                view.dirty = true;
                self.stats().stand_in_redraws += 1;
            }
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
                    if view.schedule.redoing(k) {
                        self.stats().rerendered += 1;
                    }
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
                    if let Some(view) = self.views.get_mut(&key) {
                        view.costs.late();
                    }
                    self.stats().late_skipped += 1;
                }
                Next::WaitUntil(_) | Next::Idle => break,
            }
        }
        self.send_stats(key);
    }

    /// The session's presentation stats, when a report is due.
    fn send_stats(&mut self, key: Key) {
        let now = now();
        if !self.views.get(&key).is_some_and(|v| v.costs.due(now)) {
            return;
        }
        let (programs, rasters) = self
            .gpu
            .as_mut()
            .and_then(|g| g.composer.as_mut())
            .map(|c| {
                let r = c.renderer();
                (r.shader_state().programs(), r.vector_cache_len())
            })
            .unwrap_or((0, 0));
        let (snapshots, snapshot_us_max) = self.snapshot_costs.get(&key).copied().unwrap_or((0, 0));
        let Some(view) = self.views.get_mut(&key) else {
            return;
        };
        let report = view
            .costs
            .report(now, programs, rasters, snapshots, snapshot_us_max);
        self.event(key, Event::RenderStats(report), None);
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
        let mut depth_surface: Option<(u32, u32)> = None;
        let mut cpu_us = 0i64;
        let mut failures_n = 0usize;
        let mut stand_in = false;
        let mut stand_in_shown = None;
        if document {
            let doc = self.docs.get_mut(&key)?;
            let composer = gpu.composer_with(&self.config, self.frame_cache);
            let composition = view.state.composition.as_str().to_owned();
            // The renderer draws with this view's definitions (one renderer
            // serves every view on the device).
            let registered = doc.take_registered();
            let _ = composer.use_definitions(key, doc.definitions(), &registered);
            composer.set_playing(timing.is_some());
            // Paused in Material Preview or Rendered: Blender surfaces
            // (`super::blender`); playback and draft shadings draw natively.
            let paused = timing.is_none() && !view.clock.is_playing();
            let shading = super::blender::blender_shading(view.state.view.as_ref());
            let preview = match (&self.blender, paused, shading, view.state.view.as_ref()) {
                (Some(blender), true, Some(_), Some(v)) => Some(blender.host(v)),
                _ => None,
            };
            composer.set_blender(match &preview {
                Some(host) => pm_render::blender::BlenderRender::Preview(host.clone()),
                None => pm_render::blender::BlenderRender::Native,
            });
            // Stand-ins only for paused frames after a scrub seek; read the
            // counter first so a frame landing during the render is not
            // missed.
            let scrub = view.scrub && timing.is_none() && !view.clock.is_playing();
            composer.renderer().set_scrub(scrub);
            let landed_before = composer.renderer().media_landed();
            // The depth and object-id surface of this frame, when the
            // viewer asks for it: slot `seq % count` of the depth pool.
            let depth_slot = view.depth.as_ref().and_then(|d| {
                let index = (seq % u64::from(d.spec.count.max(1))) as usize;
                d.pool.surfaces.get(index).map(|s| {
                    (
                        index as u32,
                        d.spec.generation,
                        s.texture.create_view(&Default::default()),
                    )
                })
            });
            let composed = composer.compose_with_depth(
                doc.document(),
                &composition,
                doc.media(),
                Ticks::new(media.max(0)).unwrap_or(Ticks::ZERO),
                &view.state,
                &live.working,
                depth_slot.as_ref().map(|(_, _, v)| v),
            );
            depth_surface = depth_slot.map(|(index, generation, _)| (index, generation));
            composer.set_blender(pm_render::blender::BlenderRender::Native);
            // What the paused frame lacked (nothing in a draft shading); a
            // frame that failed to render changes nothing.
            if paused
                && composed.is_ok()
                && let Some(blender) = &self.blender
            {
                let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
                blender.submit(key, shading, revision, preview.as_deref());
            }
            composer.renderer().set_scrub(false);
            let scrub_stats = composer.renderer().scrub_stats();
            // Text laid out while the font service did not answer must not
            // be shown: keep the frame on screen and draw again.
            if let Some(why) = self.config.fonts.as_ref().and_then(|f| f.after_render()) {
                view.dirty = true;
                (self.sink)(vec![Action::Log {
                    level: LogLevel::Warn,
                    message: format!("viewer: a frame waited for fonts and is drawn again ({why})"),
                }]);
                return None;
            }
            match composed {
                Ok(composed) => {
                    view.revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
                    overlay = composed.overlay;
                    cpu_us = composed.cpu_us;
                    failures_n = composed.failures.len();
                    stand_in = !composed.stand_ins.is_empty();
                    view.stand_in = stand_in.then_some(landed_before);
                    stand_in_shown = composed.stand_ins.first().copied();
                    let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                    stats.document_frames += 1;
                    stats.compose_us_max = stats.compose_us_max.max(composed.cpu_us);
                    stats.compose_us_total += composed.cpu_us;
                    stats.layer_failures += composed.failures.len() as u64;
                    stats.stand_in_frames += u64::from(stand_in);
                    stats.scrub = Some(scrub_stats);
                    drop(stats);
                    // Shader compile errors and missing extension layers at
                    // this frame's revision, before the frame (section 11).
                    let renderer = composer.renderer();
                    let mut reports = view.layer_reports.due(
                        view.revision,
                        renderer.effects().generation(),
                        &composition,
                        || renderer.layer_reports(doc.document(), &composition),
                    );
                    reports.extend(
                        view.layer_reports
                            .runtime_due(view.revision, &composed.failures),
                    );
                    for error in reports {
                        let report = FailureReport {
                            revision: view.revision,
                            error,
                        };
                        let mut envelope = Envelope::new(Message::Event {
                            body: Event::Failure(report),
                        })
                        .with_session(key.1);
                        envelope.revision = Some(view.revision);
                        if let Ok(bytes) = codec::encode(&envelope) {
                            (self.sink)(vec![Action::Send { port: key.0, bytes }]);
                        }
                    }
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
                            // The Shader panel hears it as the layer's
                            // `NotHeadlessRenderable` report (reports.rs).
                            if !self.fallback.logged.insert((key, r.layer.clone())) {
                                continue;
                            }
                            (self.sink)(vec![Action::Log {
                                level: LogLevel::Info,
                                message: format!(
                                    "layer {} draws through the shader fallback runner \
                                     ({}x{}; native pipeline stopped at {}: {}); \
                                     not headless-renderable",
                                    r.layer,
                                    r.width,
                                    r.height,
                                    r.stage,
                                    r.message.lines().next().unwrap_or_default()
                                ),
                            }]);
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
                    // The legacy compositor fills the shader meta while it
                    // draws; frames the engine presents again repeat it all.
                    view.layer_reports.reset();
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
        let overlay_layers = overlay.layers.len();
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
                stand_in,
            },
            overlay: Some(overlay),
            depth_id: depth_surface.map(|(index, generation)| pm_protocol::frame::DepthIdSurface {
                pool_index: index,
                seq: seq_value,
                pool_generation: Seq::new(u64::from(generation)).unwrap_or(Seq::MAX),
            }),
            view_seq: view.state.view_seq,
            duration,
        };
        if depth_surface.is_some() {
            self.stats
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .depth_frames += 1;
        }
        view.costs.frame(
            timing.is_some(),
            cpu_us,
            gpu_done - start - cpu_us.min(gpu_done - start),
            overlay_layers,
            failures_n,
            view.revision.get(),
        );
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
        drop(stats);
        if timing.is_none() {
            self.last_interactive = Some(gpu_done);
        }
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(shown) = stand_in_shown
            && self.config.stand_in_log
            && stats.stand_in_log.len() < STAND_IN_LOG_CAP
        {
            stats.stand_in_log.push((seq, shown));
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

    /// Answers a snapshot with an error on main's host port.
    fn snapshot_error(&self, job: &super::snapshot::SnapshotJob, code: ErrorCode, message: &str) {
        let error = EngineError::new(code, ErrorScope::Request { id: job.id }, message);
        self.send(
            job.host,
            Envelope::new(Message::Error { id: job.id, error }),
        );
    }

    /// The Blender worker answered a snapshot's capture: the job renders
    /// with the prepared surfaces, or fails with Blender's reason (never
    /// with native shading in their place).
    fn blender_captured(
        &mut self,
        token: u64,
        result: Result<Arc<Vec<super::snapshot::PreparedFrame>>, pm_blender::BlenderError>,
    ) {
        match result {
            Ok(frames) => {
                if let Some(job) = self.snapshots.preparing_mut(token) {
                    job.blender_done(frames, now());
                }
            }
            Err(error) => {
                let Some(job) = self.snapshots.take_preparing(token) else {
                    return;
                };
                let (code, message) = match error {
                    pm_blender::BlenderError::Missing(m) => (ErrorCode::BlenderMissing, m),
                    pm_blender::BlenderError::Failed(m) => (ErrorCode::BlenderFailed, m),
                    pm_blender::BlenderError::Cancelled => (
                        ErrorCode::Cancelled,
                        "The Blender frame was cancelled.".to_owned(),
                    ),
                };
                self.stats().snapshot_failures += 1;
                self.snapshot_error(&job, code, &message);
            }
        }
    }

    /// What the views are doing, for the snapshot queue's scheduling.
    fn pressure(&self) -> super::snapshot::Pressure {
        let now = now();
        let mut pressure = super::snapshot::Pressure {
            last_interactive: self.last_interactive,
            ..Default::default()
        };
        for view in self.views.values() {
            if !(view.clock.is_playing() || view.awaiting.is_some()) {
                continue;
            }
            pressure.playing = true;
            let mut schedule = view.schedule.clone();
            let phase = Self::phase(&self.config, &view.state);
            let due = match schedule.next_in_phase(&view.clock, now, self.config.lead, phase) {
                Next::WaitUntil(at) => Some(at.ceil() as i64 - now),
                Next::Render { .. } | Next::Skip { .. } => Some(0),
                Next::Idle => None,
            };
            if let Some(due) = due {
                pressure.due_in_us = Some(pressure.due_in_us.map_or(due, |d| d.min(due)));
            }
        }
        pressure
    }

    /// Runs the snapshots that may run now: every `sync` and `async` job
    /// that is ready, then at most one `idle` job (so a gesture that starts
    /// is served after one).
    fn work_snapshots(&mut self) {
        if self.snapshots.is_empty() {
            return;
        }
        let now = now();
        for job in self.snapshots.take_expired(now) {
            self.stats().snapshot_failures += 1;
            if let Some(token) = job.preparing() {
                // Blender did not finish within the snapshot's bound: stop it.
                if let Some(blender) = &self.blender {
                    blender.cancel_capture(token);
                }
                let message = format!(
                    "Blender did not prepare the frame within {} s.",
                    job.request.blender_wait_ms / 1_000
                );
                self.snapshot_error(&job, ErrorCode::Timeout, &message);
                continue;
            }
            let have = self.docs.get(&job.key).map(|d| d.revision());
            let message = match (job.min_revision, have) {
                (Some(want), Some(have)) if have < want.get() => format!(
                    "The engine did not reach revision {} (it has {have}) within {} ms.",
                    want.get(),
                    job.request.wait_ms
                ),
                _ => format!(
                    "The snapshot did not render within {} ms.",
                    job.request.wait_ms
                ),
            };
            self.snapshot_error(&job, ErrorCode::Timeout, &message);
        }
        self.segments.reap(pm_bridge::shm::ORPHAN_AFTER);
        let mut idle_done = false;
        loop {
            let pressure = self.pressure();
            let docs = &self.docs;
            // A document whose media the host has not answered yet is not
            // ready: the snapshot waits (within its wait) rather than draw
            // media missing that is on its way.
            let revision = |key: Key| {
                docs.get(&key)
                    .filter(|d| d.has_document() && d.media_ready())
                    .map(|d| d.revision())
            };
            let asked = &self.fallback.asked;
            let outstanding = |k: &str| asked.contains_key(k);
            let Some(job) = self.snapshots.next(revision, outstanding, pressure, now) else {
                return;
            };
            let idle = job.request.class == pm_protocol::snapshot::SnapshotClass::Idle;
            if idle && idle_done {
                self.snapshots.requeue(job);
                return;
            }
            idle_done |= idle;
            self.run_snapshot(job);
        }
    }

    /// Renders one snapshot and answers it.
    fn run_snapshot(&mut self, job: super::snapshot::SnapshotJob) {
        use pm_protocol::snapshot::{
            SnapshotClass, SnapshotOutcome, SnapshotResult, SnapshotTarget,
        };
        let started = now();
        if self.gpu().is_none() {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(&job, ErrorCode::DeviceLost, "The engine has no GPU device.");
            return;
        }
        let (layer_target, motion_blur, background, draft_3d) = match &job.request.target {
            SnapshotTarget::Frame {
                motion_blur_samples,
                background,
                draft_3d,
            } => (false, *motion_blur_samples, *background, *draft_3d),
            SnapshotTarget::Layer { .. } => (true, 0, true, false),
        };
        if layer_target {
            self.run_layer_snapshot(job, started);
            return;
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let Some(doc) = self.docs.get(&job.key) else {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::UnknownSession,
                "The window's engine session has no document.",
            );
            return;
        };
        // Today's capture of a Blender composition draws Blender's surfaces
        // for its frame, never native shading labelled final; native
        // shading is only for `draft3d` captures (thumbnails, posters).
        let needs_blender = !draft_3d
            && pm_render::unsupported(doc.document(), job.request.composition.as_str())
                .iter()
                .any(|u| u.feature == pm_render::Feature::Blender);
        let composer = gpu.composer_with(&self.config, self.frame_cache);
        // The session's definitions, without consuming the re-registrations
        // its viewer's next frame applies.
        let _ = composer.use_definitions(job.key, doc.definitions(), &[]);
        composer.set_blender(pm_render::blender::BlenderRender::Native);
        composer.set_playing(false);
        let renderer = composer.renderer();
        renderer.set_scrub(false);
        let purpose = match job.request.class {
            SnapshotClass::Idle => pm_protocol::frame::FramePurpose::Thumbnail,
            SnapshotClass::Sync | SnapshotClass::Async => pm_protocol::frame::FramePurpose::Export,
        };
        let mut request = pm_render::RenderRequest {
            output_size: job.request.output_size,
            // Today's capture derives the scale from the composition size
            // (`W / comp.w`, `H / comp.h`).
            scale: None,
            region: None,
            precision: pm_protocol::color::Precision::Float16,
            depth_id: false,
            background,
            export: purpose == pm_protocol::frame::FramePurpose::Export,
            view: None,
            blender: pm_render::blender::BlenderRender::Native,
            motion_blur: if motion_blur == 0 {
                pm_render::motion::MotionBlur::Off
            } else {
                pm_render::motion::MotionBlur::Samples(motion_blur)
            },
            sampling: pm_render::sampling::Sampling::Final,
            video_end: pm_render::VideoEnd::of(purpose),
        };
        if needs_blender {
            match blender_frames(
                renderer,
                doc,
                &job,
                &request,
                self.blender.as_deref(),
            ) {
                BlenderFrames::Draw(surfaces) => {
                    request.blender = pm_render::blender::BlenderRender::Final(surfaces);
                }
                BlenderFrames::Prepare(snapshots) => {
                    let Some(blender) = self.blender.clone() else {
                        return;
                    };
                    self.blender_token += 1;
                    let token = self.blender_token;
                    let mut job = job;
                    job.start_blender(token, now());
                    blender.capture(token, job.key, snapshots);
                    self.stats().snapshot_blender += 1;
                    self.snapshots.requeue(job);
                    return;
                }
                BlenderFrames::Fail(code, message) => {
                    self.stats().snapshot_failures += 1;
                    self.snapshot_error(&job, code, &message);
                    return;
                }
            }
        }
        let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
        let rendered = renderer
            .render(
                doc.document(),
                job.request.composition.as_str(),
                job.request.time,
                &request,
                doc.media(),
            )
            .and_then(|frame| {
                let raw = pm_gpu::readback::read_raw(
                    renderer.device(),
                    renderer.queue(),
                    &frame.color.texture,
                )?;
                Ok((frame.color.width(), frame.color.height(), raw))
            });
        if let Some(why) = self.config.fonts.as_ref().and_then(|f| f.after_render()) {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::Timeout,
                &format!("The engine's fonts did not answer for this frame ({why})."),
            );
            return;
        }
        let (width, height, raw) = match rendered {
            Ok(done) => done,
            Err(pm_render::RenderError::ShaderFallbackPending(requests)) => {
                // Pixels the caller says it already posted are on their way
                // (the session's port, ahead of this request): wait for
                // them. Name the rest so the renderer draws them and asks
                // again; their bands are accepted as for the viewer's asks.
                let (posted, missing): (Vec<_>, Vec<_>) = requests
                    .into_iter()
                    .partition(|r| job.request.uploaded.contains(&r.key));
                if missing.is_empty() {
                    let mut job = job;
                    job.waiting_for = posted.into_iter().map(|r| r.key).collect();
                    for key in &job.waiting_for {
                        self.fallback.asked.entry(key.clone()).or_insert(job.key);
                    }
                    self.snapshots.requeue(job);
                    return;
                }
                let fallback: Vec<_> = missing
                    .into_iter()
                    .take(pm_protocol::shader_fallback::MAX_REQUESTS)
                    .collect();
                for r in &fallback {
                    self.fallback.asked.insert(r.key.clone(), job.key);
                }
                self.stats().snapshot_needs += 1;
                let outcome = SnapshotOutcome::NeedsRenderer {
                    revision,
                    fallback,
                    compat: Vec::new(),
                };
                let mut envelope = Envelope::new(Message::Response {
                    id: job.id,
                    body: Response::Snapshot(Box::new(outcome)),
                });
                envelope.revision = Some(revision);
                self.send(job.host, envelope);
                return;
            }
            Err(pm_render::RenderError::Unsupported(list)) => {
                // A visible, named failure: never another engine's pixels.
                let reasons: Vec<String> = list.iter().map(super::document::describe).collect();
                self.stats().snapshot_failures += 1;
                let mut message = format!(
                    "The new engine cannot draw this frame: {}.",
                    reasons.join("; ")
                );
                message.truncate(2_000);
                self.snapshot_error(&job, ErrorCode::Unsupported, &message);
                return;
            }
            Err(pm_render::RenderError::Composition(id)) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    &job,
                    ErrorCode::InvalidMessage,
                    &format!("The document has no composition {id:?}."),
                );
                return;
            }
            Err(error) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    &job,
                    ErrorCode::Internal,
                    &format!("The snapshot could not be rendered: {error}"),
                );
                return;
            }
        };
        let pixels = super::snapshot::encode_rgba8(&raw, job.request.pixels);
        drop(raw);
        let Some(payload) = self.share_pixels(&job, pixels) else {
            return;
        };
        let done = now();
        let render_us = (done - started).max(0);
        self.snapshots.observe(render_us as f64);
        {
            let mut stats = self.stats();
            stats.snapshots += 1;
            stats.snapshot_us_max = stats.snapshot_us_max.max(render_us);
        }
        let costs = self.snapshot_costs.entry(job.key).or_default();
        costs.0 += 1;
        costs.1 = costs.1.max(render_us);
        let result = SnapshotResult {
            revision,
            view_seq: job.request.view_seq,
            width,
            height,
            pixels: job.request.pixels,
            payload,
            origin: [0.0, 0.0],
            layer_box: None,
            raster: None,
            quality: FrameQuality {
                resolution: PreviewResolution::Full,
                representation: Representation::Original,
                draft_3d,
                cached: false,
                stand_in: false,
            },
            waited_us: (started - job.received).clamp(0, i64::from(u32::MAX)) as u32,
            render_us: render_us.clamp(0, i64::from(u32::MAX)) as u32,
        };
        let mut envelope = Envelope::new(Message::Response {
            id: job.id,
            body: Response::Snapshot(Box::new(SnapshotOutcome::Rendered(result))),
        });
        envelope.revision = Some(revision);
        self.send(job.host, envelope);
    }

    /// A layer's raster (the base pixels of today's `render.raster()`,
    /// `Renderer::layer_raster`): text, captions, editable paths, and shape
    /// content, with today's anchor, box, selection, and crop rules. The
    /// layer is the document's, or the object the caller passed.
    fn run_layer_snapshot(&mut self, job: super::snapshot::SnapshotJob, started: i64) {
        use pm_protocol::snapshot::{
            RasterGeometry, SnapshotOutcome, SnapshotResult, SnapshotTarget,
        };
        let SnapshotTarget::Layer {
            layer,
            scale,
            crop,
            content,
        } = &job.request.target
        else {
            return;
        };
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let Some(doc) = self.docs.get(&job.key) else {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::UnknownSession,
                "The window's engine session has no document.",
            );
            return;
        };
        // The caller's layer object, when it is not the document's.
        let content = match content {
            Some(bytes) => match pm_model::json::parse_bytes(&bytes.0) {
                Ok(value) if value.as_object().is_some() => {
                    Some(pm_model::Document::new(Some(value), doc.revision()))
                }
                _ => {
                    self.stats().snapshot_failures += 1;
                    self.snapshot_error(
                        &job,
                        ErrorCode::InvalidMessage,
                        "The layer object of a raster is not a JSON object.",
                    );
                    return;
                }
            },
            None => None,
        };
        let composer = gpu.composer_with(&self.config, self.frame_cache);
        let _ = composer.use_definitions(job.key, doc.definitions(), &[]);
        let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
        let composition = job.request.composition.as_str();
        let seconds = match pm_eval::Comp::resolve(doc.document(), composition) {
            Ok(comp) => comp.seconds_at(job.request.time.get()),
            Err(_) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    &job,
                    ErrorCode::InvalidMessage,
                    &format!("The document has no composition {composition:?}."),
                );
                return;
            }
        };
        let painted = composer.renderer().layer_raster(
            doc.document(),
            composition,
            layer.as_str(),
            content.as_ref().and_then(|c| c.root()),
            seconds,
            *scale,
            pm_protocol::color::Precision::Float16,
        );
        if let Some(why) = self.config.fonts.as_ref().and_then(|f| f.after_render()) {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::Timeout,
                &format!("The engine's fonts did not answer for this layer ({why})."),
            );
            return;
        }
        let reply = |this: &Self, outcome: SnapshotOutcome| {
            let mut envelope = Envelope::new(Message::Response {
                id: job.id,
                body: Response::Snapshot(Box::new(outcome)),
            });
            envelope.revision = Some(revision);
            this.send(job.host, envelope);
        };
        let raster = match painted {
            Ok(Some(raster)) => raster,
            Ok(None) => {
                self.stats().snapshots += 1;
                reply(self, SnapshotOutcome::Empty { revision });
                return;
            }
            Err(pm_render::RenderError::Request(why)) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    &job,
                    ErrorCode::InvalidMessage,
                    &format!("The layer raster could not be made: {why}."),
                );
                return;
            }
            Err(pm_render::RenderError::Unsupported(list)) => {
                let reasons: Vec<String> = list.iter().map(super::document::describe).collect();
                self.stats().snapshot_failures += 1;
                let mut message = format!(
                    "The new engine cannot draw this layer: {}.",
                    reasons.join("; ")
                );
                message.truncate(2_000);
                self.snapshot_error(&job, ErrorCode::Unsupported, &message);
                return;
            }
            Err(error) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    &job,
                    ErrorCode::Internal,
                    &format!("The layer raster could not be painted: {error}"),
                );
                return;
            }
        };
        let (painted, painted_crop) = match &raster.pixels {
            pm_render::LayerPixels::Float { pixels, crop } => (
                super::snapshot::encode_rgba8_f32(pixels, job.request.pixels),
                *crop,
            ),
            pm_render::LayerPixels::Rgba8(rgba) => (
                super::snapshot::encode_rgba8_unorm(rgba, job.request.pixels),
                [0, 0, raster.canvas.0, raster.canvas.1],
            ),
        };
        // Today's raster crops plain text and shapes only; the others come
        // back whole whatever the crop asked.
        let window = if raster.crops { *crop } else { None };
        let Some((pixels, width, height)) =
            super::snapshot::place_raster(&painted, painted_crop, raster.canvas, window)
        else {
            self.stats().snapshots += 1;
            reply(self, SnapshotOutcome::Empty { revision });
            return;
        };
        if u64::from(width) * u64::from(height) > job.request.max_bytes() / 4 {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::BudgetExceeded,
                &format!(
                    "The layer raster is {width}×{height}, over the {}×{} asked for.",
                    job.request.output_size.width, job.request.output_size.height
                ),
            );
            return;
        }
        let Some(payload) = self.share_pixels(&job, pixels) else {
            return;
        };
        let render_us = (now() - started).max(0);
        {
            let mut stats = self.stats();
            stats.snapshots += 1;
            stats.snapshot_us_max = stats.snapshot_us_max.max(render_us);
        }
        let origin_in_canvas = window.map_or([0, 0], |w| {
            [w.x.min(raster.canvas.0), w.y.min(raster.canvas.1)]
        });
        let result = SnapshotResult {
            revision,
            view_seq: job.request.view_seq,
            width,
            height,
            pixels: job.request.pixels,
            payload,
            origin: [raster.anchor.0, raster.anchor.1],
            layer_box: Some([raster.box_size.0, raster.box_size.1]),
            raster: Some(RasterGeometry {
                canvas: [raster.canvas.0.max(1), raster.canvas.1.max(1)],
                window: origin_in_canvas,
                cropped: window.is_some(),
                selection: raster.selection,
                font_offset: raster.font_offset.map(|(x, y)| [x, y]),
            }),
            quality: FrameQuality {
                resolution: PreviewResolution::Full,
                representation: Representation::Original,
                draft_3d: false,
                cached: false,
                stand_in: false,
            },
            waited_us: (started - job.received).clamp(0, i64::from(u32::MAX)) as u32,
            render_us: render_us.clamp(0, i64::from(u32::MAX)) as u32,
        };
        reply(self, SnapshotOutcome::Rendered(result));
    }

    /// A snapshot's pixels as a payload: inline when small, otherwise a
    /// shared-memory segment the engine unlinks if main never takes it.
    /// `None` after answering the job with the error.
    fn share_pixels(
        &mut self,
        job: &super::snapshot::SnapshotJob,
        pixels: Vec<u8>,
    ) -> Option<pm_protocol::snapshot::PixelPayload> {
        use pm_protocol::snapshot::PixelPayload;
        let length = pixels.len();
        if length <= pm_protocol::snapshot::INLINE_PIXEL_BYTES {
            return Some(PixelPayload::Inline {
                bytes: pm_protocol::common::Bytes(pixels),
            });
        }
        match pm_bridge::shm::publish(&pixels) {
            Ok(name) => {
                self.segments.add(name.clone());
                self.snapshots.published(
                    now(),
                    length as u64,
                    pm_bridge::shm::ORPHAN_AFTER.as_micros() as i64,
                );
                self.stats().snapshot_shared += 1;
                Some(PixelPayload::SharedMemory {
                    name,
                    length: pm_protocol::common::ByteSize::new(length as u64)
                        .unwrap_or(pm_protocol::common::ByteSize::MAX),
                })
            }
            Err(error) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    job,
                    ErrorCode::BudgetExceeded,
                    &format!("The snapshot's pixels could not be shared: {error}"),
                );
                None
            }
        }
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

/// What a snapshot of a Blender composition draws.
enum BlenderFrames {
    /// The frame's surfaces, all prepared.
    Draw(Arc<pm_blender::job::Surfaces>),
    /// Blender renders these first (`blender_wait_ms` above 0).
    Prepare(Vec<pm_blender::snapshot::Snapshot>),
    Fail(ErrorCode, String),
}

/// Snapshot build failures the static check routes for exports
/// (`pm_export::blender`): the engine cannot build these frames.
fn blender_routed(why: &str) -> bool {
    matches!(why, "imported models" | "model-group recipe parts")
}

/// The Blender frames a snapshot of `job` needs: planned exactly as the
/// render plans them (sizes, times, shutter samples; today's
/// `blenderFrameRequests`), with the composition's own settings, and drawn
/// from this job's preparation or an earlier capture's (today's prepared
/// frame). A synchronous capture (`blender_wait_ms` 0) of a frame nobody
/// prepared fails with today's sentence.
fn blender_frames(
    renderer: &mut pm_render::Renderer,
    doc: &super::document::ViewDocument,
    job: &super::snapshot::SnapshotJob,
    request: &pm_render::RenderRequest,
    blender: Option<&super::blender::PreviewBlender>,
) -> BlenderFrames {
    use super::snapshot::BlenderPrep;
    let composition = job.request.composition.as_str();
    let snaps = match renderer.blender_snapshots(
        doc.document(),
        composition,
        job.request.time,
        request,
        doc.media(),
        false,
        pm_render::blender::SnapshotOptions::default(),
    ) {
        Ok(snaps) => snaps,
        Err(pm_render::RenderError::Unsupported(list)) => {
            let reasons: Vec<String> = list.iter().map(super::document::describe).collect();
            let mut message = format!(
                "The new engine cannot draw this frame: {}.",
                reasons.join("; ")
            );
            message.truncate(2_000);
            return BlenderFrames::Fail(ErrorCode::Unsupported, message);
        }
        Err(e) => {
            return BlenderFrames::Fail(
                ErrorCode::Internal,
                format!("The snapshot could not be rendered: {e}"),
            );
        }
    };
    if let Some((frame, why)) = snaps.errors.first() {
        return if blender_routed(why) {
            BlenderFrames::Fail(
                ErrorCode::Unsupported,
                format!("The new engine cannot draw this frame: Blender-rendered 3D: {why}."),
            )
        } else {
            BlenderFrames::Fail(
                ErrorCode::BlenderFailed,
                format!(
                    "The Blender frame of {} at {} s could not be prepared: {why}",
                    frame.composition(),
                    frame.time
                ),
            )
        };
    }
    if snaps.snapshots.is_empty() {
        // No 3D object is active then: nothing for Blender to draw.
        return BlenderFrames::Draw(Arc::new(pm_blender::job::Surfaces::new()));
    }
    let hashes: Vec<[u8; 32]> = snaps.snapshots.iter().map(|s| s.hash().0).collect();
    let own = match &job.blender {
        Some(BlenderPrep::Ready(frames))
            if hashes.iter().all(|h| frames.iter().any(|(fh, _, _)| fh == h)) =>
        {
            Some(frames.clone())
        }
        _ => None,
    };
    let prepared = own.or_else(|| {
        blender
            .and_then(|b| b.prepared(&snaps.snapshots))
            .map(Arc::new)
    });
    if let Some(frames) = prepared {
        let mut surfaces = pm_blender::job::Surfaces::new();
        for (hash, frame, layers) in frames.iter() {
            if hashes.contains(hash) {
                surfaces.insert(frame.clone(), layers.clone());
            }
        }
        return BlenderFrames::Draw(Arc::new(surfaces));
    }
    if job.request.blender_wait_ms == 0 {
        return BlenderFrames::Fail(ErrorCode::BlenderFailed, BLENDER_NOT_PREPARED.to_owned());
    }
    if blender.is_none() {
        return BlenderFrames::Fail(
            ErrorCode::BlenderFailed,
            "This engine has no Blender worker for snapshots.".to_owned(),
        );
    }
    if job.blender_attempts >= super::snapshot::MAX_BLENDER_ATTEMPTS {
        return BlenderFrames::Fail(
            ErrorCode::BlenderFailed,
            format!(
                "The Blender frame changed {} times while Blender rendered it.",
                job.blender_attempts
            ),
        );
    }
    BlenderFrames::Prepare(snaps.snapshots)
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
