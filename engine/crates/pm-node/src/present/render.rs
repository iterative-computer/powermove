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
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pm_bridge::rendezvous::{Client, SendRight};
use pm_bridge::wire::{PixelFormat, SurfaceInfo};
use pm_engine::document::DocumentUpdate;
use pm_engine::{Action, LogLevel};
use pm_gpu::bridge_out::display::{
    DisplayParams, DisplayTransform, WORKING_FORMAT, WorkingTexture,
};
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
use pm_protocol::definitions::{DefinitionKind, DefinitionSetSnapshot};
use pm_protocol::envelope::{Envelope, Event, Message, Response};
use pm_protocol::error::{EngineError, ErrorCode, ErrorDetail, ErrorScope, FailureReport, Retry};
use pm_protocol::frame::{
    FrameDescriptor, FrameQuality, FrameRelease, FrameRequest, PoolLedger, PreviewResolution,
    Representation, Shading3d, ViewOverride, ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::interop::SurfaceFormat;
use pm_protocol::preview::{PreviewAction, PreviewControl, PreviewPhase, PreviewStatus};
use pm_render::effects::Effects;
use serde::Serialize;

use super::audio::{AudioCtl, AudioIngress, AudioLink, AudioStatus, ForwardedAudio};
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

#[derive(Clone)]
struct PreviewCandidate {
    generation: Option<Seq>,
    auto_generation: Option<Seq>,
    revision: Revision,
    composition: String,
    time: Ticks,
    shading: Shading3d,
    complete: bool,
    failure: Option<String>,
}

#[derive(Clone)]
struct AutoPreview {
    generation: Seq,
    revision: Revision,
    composition: String,
    time: Ticks,
    shading: Shading3d,
    phase: PreviewPhase,
}

#[derive(Clone, Copy)]
struct CachePreview {
    key: Key,
    revision: Revision,
    generation: Seq,
}

impl AutoPreview {
    fn matches(&self, candidate: &PreviewCandidate) -> bool {
        self.revision == candidate.revision
            && self.composition == candidate.composition
            && self.time == candidate.time
            && self.shading == candidate.shading
    }

    /// Even a cached, complete candidate starts a new status generation:
    /// its Ready event follows the descriptor actually sent for that frame.
    fn next(old: Option<&Self>, last: Option<Seq>, candidate: &PreviewCandidate) -> Option<Self> {
        if candidate.generation.is_some() || old.is_some_and(|old| old.matches(candidate)) {
            return None;
        }
        Some(Self {
            generation: last
                .and_then(|g| g.next())
                .unwrap_or(Seq::new(1).expect("1 is safe")),
            revision: candidate.revision,
            composition: candidate.composition.clone(),
            time: candidate.time,
            shading: candidate.shading,
            phase: PreviewPhase::Queued,
        })
    }
}

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

/// When a paused frame that failed with an error drawing it again may fix
/// (the GPU, a readback, media, the shader fallback runner, an SVG raster)
/// is drawn again on its own, after the failure: then it waits for the next
/// edit, view or transport change, as a deterministic failure does at once.
pub const RENDER_RETRY_US: [i64; 3] = [100_000, 500_000, 2_000_000];

/// Whether drawing the same frame again can succeed: errors of the GPU,
/// its readbacks, media, the fallback runner and SVG rasters are of the
/// moment; an invalid request, a missing composition and a stopped job
/// watchdog are not (they wait for a change).
pub fn transient_render_error(error: &pm_render::RenderError) -> bool {
    use pm_render::RenderError as E;
    match error {
        E::Device(_)
        | E::Readback(_)
        | E::Gpu(_)
        | E::Media(_)
        | E::Fallback(_)
        | E::SvgRaster(_) => true,
        E::Unsupported(_)
        | E::Composition(_)
        | E::Request(_)
        | E::Watchdog(_)
        | E::ShaderFallbackPending(_)
        | E::WatchdogPending
        | E::SvgRastersPending(_)
        | E::CompatPending(_) => false,
    }
}

/// A paused frame's redraw after a transient failure (`RENDER_RETRY_US`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRetry {
    /// When it is drawn again; `None` once the retries ran out.
    pub at: Option<i64>,
    /// Retries scheduled for this frame so far.
    pub attempts: usize,
    /// The frame: document revision and view sequence. A failure of
    /// another frame starts over.
    pub frame: (u64, u64),
}

impl RenderRetry {
    /// The retry after a transient failure of `frame` at `now`, following
    /// `previous` (a retry of the same frame counts on).
    pub fn after(previous: Option<RenderRetry>, frame: (u64, u64), now: i64) -> RenderRetry {
        let attempts = previous
            .filter(|r| r.frame == frame)
            .map_or(0, |r| r.attempts);
        RenderRetry {
            at: RENDER_RETRY_US.get(attempts).map(|delay| now + delay),
            attempts: attempts + 1,
            frame,
        }
    }
}

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
        audio_forwarded: Option<ForwardedAudio>,
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
    /// A frame shown at the next refresh finished on the GPU and its
    /// descriptor went to the viewer (`super::complete`).
    FrameSent {
        key: Key,
        meta: Box<super::complete::FrameMeta>,
        gpu_done: i64,
        sent: i64,
    },
    /// A submitted frame did not finish on the GPU: its slot is free again.
    FrameFailed {
        key: Key,
        meta: Box<super::complete::FrameMeta>,
        error: String,
    },
    /// A mapped snapshot slot completed on the GPU completion worker.
    SnapshotMapReady,
    SnapshotPollFailed {
        lost: Arc<AtomicBool>,
        error: String,
    },
    /// The session's document changed (document source). `posted`: host
    /// µs the engine handed it to the presenter (the per-frame trace).
    Document {
        audio_forwarded: bool,
        key: Key,
        update: DocumentUpdate,
        posted: i64,
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
    /// The engine's memory pressure grant reached the viewer: audio drops
    /// its cold decoded blocks ([`super::audio::AudioCtl::MemoryPressure`]).
    AudioMemoryPressure,
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
    PreviewControl {
        key: Key,
        control: PreviewControl,
    },
    BlenderStarted {
        key: Key,
        revision: Revision,
        generation: Option<Seq>,
        shading: Shading3d,
    },
    /// Preview surfaces of the view arrived: draw it again when paused.
    BlenderReady {
        key: Key,
        revision: Revision,
        generation: Option<Seq>,
        shading: Shading3d,
    },
    BlenderFailed {
        key: Key,
        revision: Revision,
        generation: Option<Seq>,
        shading: Shading3d,
        error: String,
    },
    /// A snapshot's Blender frames are prepared, or why not
    /// ([`super::blender::PreviewBlender::capture`]).
    BlenderCaptured {
        token: u64,
        result: Result<Arc<Vec<super::snapshot::PreparedFrame>>, pm_blender::BlenderError>,
    },
    /// The layer raster worker answered a layer snapshot
    /// ([`super::rasters`]).
    RasterDone(Box<super::rasters::RasterDone>),
    /// A layer raster the worker could not draw from its mirror (none, or
    /// at another revision): seed it again ([`super::rasters`]).
    RasterReseed(Box<super::rasters::RasterJob>),
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
    /// Sample live resource sizes for allocation probes (off in normal use).
    pub allocation_inventory: bool,
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
    /// Caps the viewer's waits for its window's pixels (`EXTERNAL_WAIT_ENV`;
    /// [`super::wait::Bounds`]).
    pub external_wait_cap_us: Option<i64>,
    /// Tests only ([`super::SEND_DELAY_ENV`]): how long the completion
    /// thread holds a finished paused frame before sending it.
    pub send_delay: Option<Duration>,
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
#[derive(Debug, Clone, Serialize)]
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
    /// Host µs the pass that rendered this frame began (`work_view`):
    /// from there to `start` is the pass's document step and requests.
    pub work_start: i64,
    /// CPU µs of the compose (planning and encoding).
    pub cpu_us: i64,
    /// The document revisions this frame is the first to show: revision,
    /// host µs the engine handed it to the presenter, host µs the render
    /// thread applied it.
    pub revisions: Vec<[i64; 3]>,
}

/// Work counters as `{ name: count }` (the non-zero ones).
fn work_counters<S: serde::Serializer>(c: &pm_perf::Counters, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(None)?;
    for (counter, n) in c.nonzero() {
        m.serialize_entry(counter.name(), &n)?;
    }
    m.end()
}

/// One render-thread event of the per-frame trace ([`Stats::turn_trace`]).
#[derive(Debug, Clone, Serialize)]
pub struct TurnTrace {
    /// Host µs it began, and how long it took (µs).
    pub at: i64,
    pub us: i64,
    pub what: &'static str,
    /// The playback frame it concerns (`k`), or the composition frame of a
    /// layer raster; -1 when none.
    pub k: i64,
}

/// Events kept in [`Stats::turn_trace`].
pub const TURN_TRACE_CAP: usize = 8_192;

/// Counters the engine reports in its status.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Render-thread turns (one command batch, or one pass over the views)
    /// longer than [`STALL_US`]: the clock, frames, and audio status wait
    /// behind them. The longest, and what the last one was doing.
    pub stalls: u64,
    pub stall_us_max: i64,
    pub last_stall: Option<String>,
    /// Every render-thread turn of at least 50, 100, 250 ms and 1 s, by
    /// band ([`SLOW_TURN_BANDS_US`]), and the longest of them: hitches
    /// shorter than a stall show here.
    pub slow_turns: [u64; 4],
    pub turn_us_max: i64,
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
    /// The render thread's work counters (`pm_perf`: passes, draws, GPU
    /// allocations, uploads, readbacks, heap allocations when counted,
    /// cache hits) over every document frame's compose (planning, encoding,
    /// submission), and over the last one.
    #[serde(serialize_with = "work_counters")]
    pub work: pm_perf::Counters,
    #[serde(serialize_with = "work_counters")]
    pub work_last: pm_perf::Counters,
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
    /// Viewer frames drawn without a level 1 SVG bucket the window could
    /// not draw (or did not answer for in time), and waits for the window's
    /// pixels that timed out (docs/engine/perf.md).
    pub svg_failed_frames: u64,
    pub external_wait_timeouts: u64,
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
    /// Rasters the store evicted for room (least recently used first).
    pub compat_evicted: u64,
    /// Answers that replaced a different one at the same revision (a
    /// producer's output changed without an edit), each redrawing the view.
    pub compat_replaced: u64,
    /// Producer answers held and asks outstanding (the `raster`
    /// override's pass-throughs, reuses, and rasters).
    pub compat_answers: u64,
    pub compat_asked: u64,
    /// Viewer frames that waited for producer answers.
    pub compat_waits: u64,
    /// Layer rasters the worker drew off this thread ([`super::rasters`]).
    pub layer_rasters_off_thread: u64,
    /// Times the worker's document mirror was seeded (a session's first
    /// raster, or after its mirror was dropped), and jobs it sent back for
    /// a seed.
    pub layer_raster_seeds: u64,
    pub layer_raster_reseeds: u64,
    /// Document updates on this thread that copied the document because
    /// the raster worker had not yet taken its own copy of a seed.
    pub document_copies: u64,
    /// Plays that waited in the native preroll for the window's answers,
    /// those that started at the bound instead, and the longest wait (µs).
    pub prerolls: u64,
    pub preroll_timeouts: u64,
    pub preroll_us_max: i64,
    /// With the per-frame trace on ([`ThreadConfig::trace`]): the latest
    /// render-thread events (turns of a millisecond or more, schedule
    /// skips, compatibility waits, layer rasters, prerolls), oldest first.
    #[serde(skip_serializing_if = "VecDeque::is_empty")]
    pub turn_trace: VecDeque<TurnTrace>,
    /// A render-thread snapshot of live allocations. Surface bytes are the
    /// IOSurface allocation sizes; target and cache bytes are nominal texel
    /// counts. Lease reservations are not included.
    pub allocations: AllocationInventory,
    /// Rests at which idle GPU memory was released, and nominal bytes freed
    /// from tracked idle targets. Full device retirement is not byte-counted.
    pub rest_releases: u64,
    pub rest_released_bytes: u64,
    #[serde(skip)]
    pub trace: Vec<FrameTrace>,
}

/// Live resource inventory copied into status without touching render-thread
/// state from the utility process's JavaScript thread.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllocationInventory {
    pub sampled_at_us: i64,
    pub gpu_created: bool,
    pub composer_created: bool,
    pub views: Vec<ViewAllocation>,
    pub color_surface_bytes: u64,
    pub depth_surface_bytes: u64,
    pub working_target_bytes: u64,
    pub snapshot_target_bytes: u64,
    pub snapshot_readback_bytes: u64,
    /// `MTLDevice.currentAllocatedSize` of the viewer's device: every
    /// buffer, texture and heap the driver holds for it, measured rather
    /// than nominal (`pm_gpu::device::allocated_bytes`).
    pub metal_allocated_bytes: Option<u64>,
    /// Public Metal device counters; capacity and transfer rate are adapter
    /// properties, not resident-memory measurements.
    pub metal_device: Option<MetalDeviceInventory>,
    /// The cached snapshot attachment's own Metal resource allocation.
    /// This can overlap heap or IOSurface allocations and is not additive
    /// with the device-wide counter.
    pub metal_snapshot_texture: Option<MetalTextureInventory>,
    pub renderer: Option<RendererAllocation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetalDeviceInventory {
    pub current_allocated_bytes: u64,
    pub recommended_max_working_set_bytes: Option<u64>,
    pub unified_memory: Option<bool>,
    pub max_transfer_bytes_per_second: Option<u64>,
}

impl From<pm_gpu::device::MetalDeviceAllocation> for MetalDeviceInventory {
    fn from(value: pm_gpu::device::MetalDeviceAllocation) -> Self {
        Self {
            current_allocated_bytes: value.current_allocated_bytes,
            recommended_max_working_set_bytes: value.recommended_max_working_set_bytes,
            unified_memory: value.unified_memory,
            max_transfer_bytes_per_second: value.max_transfer_bytes_per_second,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetalTextureInventory {
    pub allocated_bytes: u64,
    pub heap_backed: bool,
}

impl From<pm_gpu::device::MetalTextureAllocation> for MetalTextureInventory {
    fn from(value: pm_gpu::device::MetalTextureAllocation) -> Self {
        Self {
            allocated_bytes: value.allocated_bytes,
            heap_backed: value.heap_backed,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewAllocation {
    pub port: u32,
    pub session: u64,
    pub color: Option<SurfaceAllocation>,
    pub depth: Option<SurfaceAllocation>,
    pub working_target_bytes: u64,
    /// This view's live scratch attachment, if one exists.
    pub metal_working_texture: Option<MetalTextureInventory>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceAllocation {
    pub generation: u32,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub count: usize,
    pub iosurface_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererAllocation {
    pub target_pool_bytes: u64,
    pub target_pool_idle: usize,
    pub target_pool_created: u64,
    pub pyramid_bytes: u64,
    pub media_texture_bytes: u64,
    pub media: pm_media::decode::service::MediaUsage,
    pub text_cache_bytes: u64,
    pub vector_cache_bytes: u64,
    pub scene3d_meshes: usize,
    pub scene3d_mesh_bytes: u64,
    pub scene3d_maps: usize,
    pub scene3d_map_bytes: u64,
}

impl SurfaceAllocation {
    fn of(spec: PoolSpec, pool: &SurfacePool) -> Self {
        Self {
            generation: spec.generation,
            width: spec.size.width,
            height: spec.size.height,
            format: format!("{:?}", spec.format),
            count: pool.len(),
            iosurface_bytes: pool.bytes(),
        }
    }
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
    /// Reused by serial snapshot copies on this device.
    snapshot_readback: pm_gpu::readback::ReadbackRing,
    /// One reusable snapshot color attachment. Every capture still redraws it.
    snapshot_target: Option<pm_gpu::Target>,
    /// Draws documents (made with the first document frame).
    composer: Option<Composer>,
    /// The font generation the composer's renderer lays text out with.
    fonts_installed: Option<u64>,
    /// Set by the device-lost callback; the thread rebuilds the device.
    lost: Arc<AtomicBool>,
    /// Which device this is (each rebuild after a loss is a new one): the
    /// layer raster worker makes its renderer again on a new device.
    epoch: u64,
}

impl Gpu {
    /// The device's composer (made on first use), its renderer holding
    /// fonts of the broker's current generation (`crate::fonts::render`).
    fn composer_with(
        &mut self,
        config: &ThreadConfig,
        frame_cache: usize,
        prepared: &mut PreparedDefinitions,
    ) -> &mut Composer {
        if self.composer.is_none() {
            let mut composer = composer(&self.device, &self.queue, config, frame_cache);
            prepared.install(&mut composer);
            self.composer = Some(composer);
            self.fonts_installed = None;
        }
        let composer = self.composer.as_mut().expect("made above");
        if let Some(fonts) = &config.fonts {
            fonts.sync(composer.renderer(), &mut self.fonts_installed);
        }
        composer
    }

    fn composer_and_snapshot_resources(
        &mut self,
        config: &ThreadConfig,
        frame_cache: usize,
        prepared: &mut PreparedDefinitions,
    ) -> (
        &mut Composer,
        &mut pm_gpu::readback::ReadbackRing,
        &mut Option<pm_gpu::Target>,
    ) {
        self.composer_with(config, frame_cache, prepared);
        (
            self.composer.as_mut().expect("made above"),
            &mut self.snapshot_readback,
            &mut self.snapshot_target,
        )
    }
}

/// Keep at most one moderate-sized snapshot attachment on a device. Large
/// captures are still rendered normally, then their attachment is released.
const MAX_CACHED_SNAPSHOT_TARGET_BYTES: u64 = 64 * 1024 * 1024;

fn cache_snapshot_target(width: u32, height: u32, format: wgpu::TextureFormat) -> bool {
    let bpp = u64::from(format.block_copy_size(None).unwrap_or(16));
    u64::from(width) * u64::from(height) * bpp <= MAX_CACHED_SNAPSHOT_TARGET_BYTES
}

fn snapshot_output_target(
    slot: &mut Option<pm_gpu::Target>,
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> pm_gpu::Target {
    if let Some(target) = slot.as_ref()
        && target.width() == width
        && target.height() == height
        && target.format() == format
    {
        return target.clone();
    }
    // Release the old attachment before a resize or format change, including
    // an oversized one-off capture, so it cannot overlap the new allocation.
    *slot = None;
    let target = pm_gpu::Target::new(
        device,
        "pm snapshot output",
        width,
        height,
        format,
        pm_gpu::target::working_usage(),
    );
    if cache_snapshot_target(width, height, format) {
        *slot = Some(target.clone());
    }
    target
}

struct Live {
    spec: PoolSpec,
    pool: SurfacePool,
    ledger: PoolLedger,
    /// Scratch target; released after a quiet rest and recreated before the
    /// next frame. The exported surfaces and their generation stay live.
    working: Option<WorkingTexture>,
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
    /// A frame waiting for Chromium's exact SVG bucket or shader pixels
    /// (`super::wait`). Other views and the audio clock keep running while
    /// its renderer answers on the port.
    external_wait: Option<super::wait::ExternalWait>,
    /// Level 1 SVG buckets the latest frame drew without (the window could
    /// not draw them): asked for again while it shows them.
    svg_without: Vec<pm_render::vector::svg_rasters::SvgRasterNeed>,
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
    /// Latest Play intent while the document or media is not ready.
    deferred_transport: Option<TransportCommand>,
    /// The deferred Play already reached the audio thread through the
    /// ingress (PLAY-1): its run is going, so the replay once the route is
    /// decided follows that run instead of starting the audio again.
    deferred_forwarded: Option<ForwardedAudio>,
    /// The audio clock version last followed.
    audio_version: u64,
    /// Paused after a `seek {scrub}`: frames may show stand-ins for video
    /// still decoding (docs/engine/scrub.md). Exact seeks, steps, and
    /// playback clear it.
    scrub: bool,
    /// The frame on screen showed stand-ins: the media "landed" counter when
    /// it was drawn. A change draws the frame again.
    stand_in: Option<u64>,
    /// The GPU watchdog held the last frame (`RenderError::WatchdogPending`):
    /// it is drawn again once the watchdog has judged the suspect frame.
    watchdog_held: bool,
    /// The last frame failed. As today's viewer does, the failure is
    /// logged once and the paused frame is drawn again at the next edit,
    /// view or transport change, not at every turn; a failure drawing it
    /// again may fix (`transient_render_error`) is also retried on its own
    /// a few times first (`render_retry`).
    render_failed: bool,
    /// After a transient failure (`transient_render_error`), when the
    /// paused frame is drawn again on its own (`RENDER_RETRY_US`).
    render_retry: Option<RenderRetry>,
    /// What presenting this session costs (`RenderStats`).
    costs: super::stats::ViewStats,
    /// The depth and object-id pool, while the viewer asks for it.
    depth: Option<DepthPool>,
    /// A Play holding the clock until the window's compatibility answers
    /// for the run's first frames arrived (the native preroll).
    preroll: Option<Preroll>,
    /// The playback frame waiting for its compatibility answers: it keeps
    /// its slot and is drawn when they arrive, unless the schedule passes
    /// it over first (it could no longer be shown in time).
    compat_blocked: Option<i64>,
    /// The paused frame is waiting for its compatibility answers: drawn
    /// again when one arrives, or at this time (host µs) to ask again for
    /// those a renderer missed ([`super::compat::ASK_AGAIN`]). Answers for
    /// other frames (the lookahead, a preroll) never redraw a paused frame
    /// that is already complete: that frame would be presented again, and
    /// extensions would hear `frame:rendered` for it again, which today's
    /// compositor never sends.
    compat_paused_wait: Option<i64>,
    /// The compatibility needs the last render consulted, and the media
    /// time it drew (unwrapped ticks).
    compat_needs: Option<(i64, Vec<pm_protocol::snapshot::CompatNeed>)>,
    /// The C′ prototype's refresh-rate request while it plays, and whether
    /// this playback already tried (a failed request is not repeated).
    #[cfg(feature = "remote-layer")]
    rate: Option<RatePreference>,
    #[cfg(feature = "remote-layer")]
    rate_tried: bool,
}

// Presenter transport decisions happen on its ordinary thread. Keep this
// diagnostic out of audio's real-time producer and disabled in normal runs.
fn trace_audio_transport(
    event: &'static str,
    key: Key,
    view: &View,
    command: Option<TransportCommand>,
) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("PM_AUDIO_PROFILE").is_some()) {
        return;
    }
    let host_us = now();
    eprintln!(
        "pm-audio-presenter event={event} host_us={host_us} key={}:{} composition={:?} route={:?} media_ticks={} playing={} awaiting={:?} deferred={:?} command={command:?}",
        key.0.0,
        key.1,
        view.state.composition,
        view.reported,
        view.clock.media_at(host_us),
        view.clock.is_playing(),
        view.awaiting,
        view.deferred_transport,
    );
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

/// The native preroll (a session with compatibility producers): a Play
/// holds the clock until the window answered for the first
/// [`PREROLL_FRAMES`] frames of the run, at most [`PREROLL_MAX_US`].
/// After a seek the window's warm-up has not filled yet, and a run that
/// started at once would skip the frames whose answers were still on their
/// way. Today's compositor shows its first frame 130 to 230 ms after Play;
/// the bound keeps the engine's first frame within that.
pub const PREROLL_FRAMES: i64 = 6;
pub const PREROLL_MAX_US: i64 = 80_000;

/// A Play waiting in the native preroll.
#[derive(Debug, Clone, Copy)]
struct Preroll {
    rate: PlaybackRateValue,
    /// Host µs of the Play, and the latest the run starts.
    asked: i64,
    until: i64,
}

struct PendingSnapshot {
    job: super::snapshot::SnapshotJob,
    ticket: pm_gpu::readback::ReadbackTicket,
    started: i64,
    map_deadline: i64,
    revision: Revision,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    rows: pm_render::SnapshotRows,
    draft_3d: bool,
}

/// Drives map callbacks off the presentation thread. Mapping still follows
/// every fresh render+copy submission; this worker never reads the pixels.
struct SnapshotPoller {
    tx: Option<Sender<(wgpu::Device, Arc<AtomicBool>)>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SnapshotPoller {
    fn start(back: Sender<Command>) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel::<(wgpu::Device, Arc<AtomicBool>)>();
        let thread = pm_engine::qos::spawn_engine_thread(
            pm_protocol::error::EngineProcess::Interactive,
            "pm-snapshot-complete",
            move || {
                // A synchronous capture's answer waits on this wakeup: it
                // runs at the presenter's class, as the frame completer
                // does (a default-class thread can wait behind background
                // work for milliseconds on a loaded machine).
                pm_bridge::set_thread_user_interactive();
                for (device, lost) in rx {
                    if let Err(error) = objc2::rc::autoreleasepool(|_| {
                        device.poll(wgpu::PollType::Wait {
                            submission_index: None,
                            timeout: Some(Duration::from_secs(2)),
                        })
                    }) {
                        let _ = back.send(Command::SnapshotPollFailed {
                            lost,
                            error: error.to_string(),
                        });
                    }
                }
            },
        )
        .map_err(|e| format!("could not start snapshot completion thread: {e}"))?;
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
        })
    }

    fn submit(&self, device: wgpu::Device, lost: Arc<AtomicBool>) -> bool {
        self.tx
            .as_ref()
            .is_some_and(|tx| tx.send((device, lost)).is_ok())
    }
}

impl Drop for SnapshotPoller {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Thread {
    config: ThreadConfig,
    sink: Sink,
    stats: Arc<Mutex<Stats>>,
    bridge: Option<Client>,
    gpu: Option<Gpu>,
    /// GLSL translation and validation completed before the first GPU frame.
    prepared: PreparedDefinitions,
    views: BTreeMap<Key, View>,
    /// Each session's document (document source), kept from its first
    /// update, which may come before the viewer state.
    docs: BTreeMap<Key, ViewDocument>,
    /// Each session's audio playback, from its first Play.
    audio: BTreeMap<Key, AudioLink>,
    audio_ingress: AudioIngress,
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
    preview_controls: super::preview_control::PreviewControls,
    cache_requested: BTreeMap<Key, PreviewControl>,
    cache_previews: BTreeMap<u64, CachePreview>,
    preview_candidates: BTreeMap<(Key, u64), PreviewCandidate>,
    auto_preview: BTreeMap<Key, AutoPreview>,
    auto_generation: BTreeMap<Key, Seq>,
    /// Shader fallback requests sent to each view's renderer and not yet
    /// answered (by request key), and the bands received so far.
    fallback: Fallback,
    /// The viewer's frame-cache share ([`Command::FrameCache`]).
    frame_cache: usize,
    /// Host snapshots waiting for their revision or their turn.
    snapshots: super::snapshot::SnapshotQueue,
    /// Frame copies waiting for their own staging slot's map callback.
    pending_snapshots: Vec<PendingSnapshot>,
    snapshot_back: Sender<Command>,
    snapshot_poller: Option<SnapshotPoller>,
    /// Shared-memory segments published for main and not reaped yet.
    segments: pm_bridge::shm::Segments,
    /// Host µs of the newest frame drawn for a gesture, scrub, or edit
    /// (idle snapshots wait for these to stop).
    last_interactive: Option<i64>,
    /// Snapshots served per session, and the slowest (µs).
    snapshot_costs: BTreeMap<Key, (u64, i64)>,
    /// Compatibility rasters the renderers uploaded, and their producers'
    /// answers (shared with the renderer for the frame being drawn).
    compat: std::sync::Arc<std::sync::Mutex<super::compat::CompatStore>>,
    /// Waveform requests waiting for the host's media answer.
    peaks_waiting: BTreeMap<(Key, String), Option<Seq>>,
    /// With the per-frame trace on: each session's revisions not shown yet
    /// (revision, posted, applied; host µs).
    doc_trace: BTreeMap<Key, Vec<[i64; 3]>>,
    /// Host µs the current `work_view` pass began.
    work_start: i64,
    /// Sends frames shown at the next refresh once their GPU work is done.
    completer: Option<super::complete::Completer>,
    /// Level 1 SVG rasters asked of the renderers (decision S1).
    svg: super::svg::SvgState,
    /// GPU work seen last (frames and snapshots) and when; the idle
    /// targets and staging go once the viewer has rested ([`REST_RELEASE_US`]).
    rest: Rest,
    /// Draws layer rasters (`render.raster()` bases) off this thread
    /// ([`super::rasters`]); `None` when it could not start (they are drawn
    /// here then).
    rasters: Option<super::rasters::RasterWorker>,
    /// Sessions whose document the raster worker mirrors: their updates
    /// go to it too ([`super::rasters`]).
    raster_mirrors: BTreeSet<Key>,
    /// Devices made so far ([`Gpu::epoch`]).
    gpu_epoch: u64,
}

/// The presenter's last GPU work, for releasing idle GPU memory at rest.
#[derive(Debug, Default)]
struct Rest {
    /// (frames, snapshots) when last looked at.
    seen: (u64, u64),
    /// Host µs that count last changed.
    since: i64,
    /// Idle memory was released since then.
    released: bool,
}

/// Definition sets prepared on the presenter thread before a GPU device exists.
/// Each owner retains its watchdog and unchanged translated programs; the
/// translation cache is shared across owners just as it is in `Composer`.
#[derive(Default)]
struct PreparedDefinitions {
    base: Option<Effects>,
    owners: BTreeMap<Key, (Effects, (usize, u64))>,
}

impl PreparedDefinitions {
    fn prepare(
        &mut self,
        key: Key,
        definitions: &(Arc<DefinitionSetSnapshot>, u64),
        registered: &[(DefinitionKind, pm_protocol::common::DefinitionId)],
        shader_cache: Option<&PathBuf>,
    ) -> Vec<EngineError> {
        let base = self.base.get_or_insert_with(|| {
            let mut effects = Effects::new(format!("{}; wgpu 30", std::env::consts::OS));
            effects.set_cache_dir(shader_cache.map(|dir| dir.join("glsl")));
            effects
        });
        let (set, version) = definitions;
        let loaded = (Arc::as_ptr(set) as usize, *version);
        let (mut effects, previous) = self
            .owners
            .remove(&key)
            .unwrap_or_else(|| (base.sibling(), (0, 0)));
        for (kind, id) in registered {
            effects.watchdog.forget(*kind, id);
        }
        let failures = if previous == loaded {
            Vec::new()
        } else {
            effects.set_snapshot(set).unwrap_or_default()
        };
        self.owners.insert(key, (effects, loaded));
        failures
    }

    fn install(&mut self, composer: &mut Composer) {
        for (key, (effects, loaded)) in std::mem::take(&mut self.owners) {
            composer.park_prepared_definitions(key, effects, loaded);
        }
        // The installed owners hold the shared cache. Do not keep it alive
        // independently after the final session and its composer close.
        self.base = None;
    }

    fn forget(&mut self, key: Key) {
        self.owners.remove(&key);
        if self.owners.is_empty() {
            self.base = None;
        }
    }

    fn stage(&mut self, key: Key, effects: Effects, loaded: (usize, u64)) {
        if self.base.is_none() {
            self.base = Some(effects.sibling());
        }
        self.owners.insert(key, (effects, loaded));
    }
}

/// How long the presenter must have drawn nothing (no frame, snapshot, or
/// playback) before it frees idle intermediate targets, viewer scratch
/// textures, the cached snapshot attachment and staging buffers. They are
/// recreated by the next frame (docs/engine/memory.md, "At rest").
pub const REST_RELEASE_US: i64 = 2_000_000;

/// Most waveform requests that wait for media answers at once.
const MAX_PEAKS_WAITING: usize = 4096;
const SNAPSHOT_GPU_TIMEOUT_US: i64 = 2_000_000;

/// The decision 19 fallback runner's state on this thread.
#[derive(Default)]
struct Fallback {
    /// Requests asked for and not answered: who asked, and when (host µs).
    asked: BTreeMap<String, (Key, i64)>,
    bands: pm_render::shader_layers::Bands,
    /// Layers each view was told draw through the runner (logged once per
    /// layer, not once per animated frame's request).
    logged: BTreeSet<(Key, String)>,
    /// Views to draw again when a program whose request failed transiently
    /// (the runner's context was lost) is asked for again (host µs).
    retry_at: BTreeMap<Key, i64>,
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
    back: Sender<Command>,
    sink: Sink,
    stats: Arc<Mutex<Stats>>,
    generations: Generations,
    audio_ingress: AudioIngress,
) {
    pm_bridge::set_thread_user_interactive();
    let fps = i64::from(config.fps.max(1));
    let peaks = super::peaks::PeakWorker::new(sink.clone());
    // Layer rasters (`render.raster()` bases) are drawn off this thread.
    let rasters = match super::rasters::RasterWorker::start(
        sink.clone(),
        back.clone(),
        stats.clone(),
        config.fonts.clone(),
    ) {
        Ok(w) => Some(w),
        Err(error) => {
            sink(vec![Action::Log {
                level: LogLevel::Warn,
                message: error,
            }]);
            None
        }
    };
    let snapshot_poller = match SnapshotPoller::start(back.clone()) {
        Ok(worker) => Some(worker),
        Err(error) => {
            sink(vec![Action::Log {
                level: LogLevel::Warn,
                message: error,
            }]);
            None
        }
    };
    // Without the completion thread every frame waits for the GPU here.
    let completer =
        match super::complete::Completer::start(sink.clone(), back.clone(), config.send_delay) {
            Ok(c) => Some(c),
            Err(error) => {
                sink(vec![Action::Log {
                    level: LogLevel::Warn,
                    message: error,
                }]);
                None
            }
        };
    let mut thread = Thread {
        frame_ticks: TICKS_PER_SECOND / fps,
        frame_cache: pm_render::media::DEFAULT_FRAME_CACHE,
        config,
        sink,
        stats,
        bridge: None,
        gpu: None,
        prepared: PreparedDefinitions::default(),
        views: BTreeMap::new(),
        docs: BTreeMap::new(),
        audio: BTreeMap::new(),
        audio_ingress,
        audio_prefs: BTreeMap::new(),
        peaks,
        generations,
        blender: None,
        blender_token: 0,
        preview_controls: Default::default(),
        cache_requested: BTreeMap::new(),
        cache_previews: BTreeMap::new(),
        preview_candidates: BTreeMap::new(),
        auto_preview: BTreeMap::new(),
        auto_generation: BTreeMap::new(),
        fallback: Fallback::default(),
        snapshots: super::snapshot::SnapshotQueue::default(),
        pending_snapshots: Vec::new(),
        snapshot_back: back,
        snapshot_poller,
        segments: pm_bridge::shm::Segments::default(),
        last_interactive: None,
        snapshot_costs: BTreeMap::new(),
        compat: Default::default(),
        peaks_waiting: BTreeMap::new(),
        doc_trace: BTreeMap::new(),
        work_start: 0,
        completer,
        svg: super::svg::SvgState::default(),
        rest: Rest::default(),
        rasters,
        raster_mirrors: BTreeSet::new(),
        gpu_epoch: 0,
    };
    objc2::rc::autoreleasepool(|_| thread.check_in());
    let mut last_inventory_us = 0;
    loop {
        let timeout = thread.next_wake();
        match rx.recv_timeout(timeout) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                let started = now();
                let what = command_name(&command);
                if !objc2::rc::autoreleasepool(|_| thread.apply(command)) {
                    break;
                }
                thread.note_turn(what, started);
                while let Ok(command) = rx.try_recv() {
                    let started = now();
                    let what = command_name(&command);
                    if matches!(command, Command::Shutdown)
                        || !objc2::rc::autoreleasepool(|_| thread.apply(command))
                    {
                        objc2::rc::autoreleasepool(|_| thread.shutdown());
                        return;
                    }
                    thread.note_turn(what, started);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let started = now();
        objc2::rc::autoreleasepool(|_| thread.work());
        thread.note_turn("work", started);
        let sampled_at_us = now();
        objc2::rc::autoreleasepool(|_| thread.release_at_rest(sampled_at_us));
        if thread.config.allocation_inventory
            && sampled_at_us.saturating_sub(last_inventory_us) >= 50_000
        {
            objc2::rc::autoreleasepool(|_| thread.snapshot_allocations(sampled_at_us));
            last_inventory_us = sampled_at_us;
        }
    }
    objc2::rc::autoreleasepool(|_| thread.shutdown());
}

/// A render-thread turn longer than this is a stall (diagnostics: the
/// status report and a warning in the engine log).
pub const STALL_US: i64 = 250_000;

/// The lower bounds of the slow-turn bands in [`Stats::slow_turns`].
pub const SLOW_TURN_BANDS_US: [i64; 4] = [50_000, 100_000, STALL_US, 1_000_000];

/// The band of [`SLOW_TURN_BANDS_US`] a turn of `took` µs falls in.
pub fn slow_turn_band(took: i64) -> Option<usize> {
    SLOW_TURN_BANDS_US.iter().rposition(|&low| took >= low)
}

/// What a command is, for stall diagnostics.
fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Shutdown => "shutdown",
        Command::Configure { .. } => "viewer state",
        Command::Release { .. } => "release",
        Command::Transport { .. } => "transport",
        Command::Audio { .. } => "audio",
        Command::Render { .. } => "frame request",
        Command::Close { .. } => "close",
        Command::ClosePort { .. } => "close port",
        Command::Document { .. } => "document",
        Command::Definitions { .. } => "definitions",
        Command::Media { .. } => "media",
        Command::ShaderFallback { .. } => "shader fallback",
        Command::AudioMemoryPressure => "audio memory pressure",
        Command::FrameCache { .. } => "frame cache",
        Command::Snapshot(_) => "snapshot",
        Command::SnapshotMapReady => "snapshot map ready",
        Command::SnapshotPollFailed { .. } => "snapshot poll failed",
        Command::Compat { .. } => "compat",
        Command::RasterDone(_) => "layer raster done",
        Command::RasterReseed(_) => "layer raster reseed",
        Command::LoseDevice => "lose device",
        Command::FontsChanged => "fonts",
        #[allow(unreachable_patterns)]
        _ => "command",
    }
}

impl Thread {
    /// Frees idle GPU memory once nothing was drawn for [`REST_RELEASE_US`]:
    /// the renderer's idle intermediate targets, viewer scratch textures,
    /// the cached snapshot attachment and staging buffers. Exported surface
    /// pools remain live. With no remaining session, the device and its
    /// composer go too. Done once per rest; any frame or snapshot starts a
    /// new one.
    fn release_at_rest(&mut self, now_us: i64) {
        let seen = {
            let stats = self.stats();
            (stats.frames, stats.snapshots)
        };
        if seen != self.rest.seen || self.rest.since == 0 {
            self.rest = Rest {
                seen,
                since: now_us,
                released: false,
            };
            return;
        }
        if self.rest.released || now_us - self.rest.since < REST_RELEASE_US {
            return;
        }
        let busy = !self.pending_snapshots.is_empty()
            || self.snapshots.waiting()
            || self.completer.as_ref().is_some_and(|c| c.in_flight() > 0)
            || self.views.values().any(|v| {
                v.clock.is_playing()
                    || (v.dirty && v.state.headless != Some(true))
                    || v.awaiting.is_some()
                    || !v.requests.is_empty()
            });
        let Some(gpu) = self.gpu.as_mut() else {
            self.rest.released = true;
            return;
        };
        if busy || gpu.snapshot_readback.in_flight() {
            return;
        }
        let no_presenter = (self.views.is_empty() && self.docs.is_empty())
            || (!self.views.is_empty()
                && self.views.values().all(|view| {
                    view.state.headless == Some(true)
                        && view.pool.is_none()
                        && view.depth.is_none()
                        && !view.clock.is_playing()
                        && view.awaiting.is_none()
                        && view.retry.is_none()
                        && view.external_wait.is_none()
                }));
        if no_presenter {
            // A queued headless capture may need this device on its next
            // turn. Keep the rest pending rather than marking it released.
            if !self.cache_requested.is_empty() || !self.cache_previews.is_empty() {
                return;
            }
            // Headless sessions keep their document and translated sources,
            // but no presentation surface needs this device. All submitted
            // frames and readbacks have finished at this point.
            if let Some(composer) = gpu.composer.as_mut() {
                if !composer.definitions_quiescent() {
                    return;
                }
                for key in self.docs.keys().copied() {
                    if let Some((effects, loaded)) = composer.take_prepared_definitions(key) {
                        self.prepared.stage(key, effects, loaded);
                    }
                }
            }
            self.gpu = None;
            self.rest.released = true;
            self.stats().rest_releases += 1;
            return;
        }
        let mut freed = gpu.snapshot_target.take().map_or(0, |t| t.bytes());
        freed += gpu.snapshot_readback.capacity_bytes();
        gpu.snapshot_readback.release_idle();
        freed -= gpu.snapshot_readback.capacity_bytes();
        if let Some(composer) = gpu.composer.as_mut() {
            freed += composer.renderer().release_idle_targets();
        }
        for view in self.views.values_mut() {
            if let Some(live) = view.pool.as_mut()
                && let Some(working) = live.working.take()
            {
                freed += u64::from(working.size.width)
                    * u64::from(working.size.height)
                    * u64::from(WORKING_FORMAT.block_copy_size(None).unwrap_or(8));
            }
        }
        self.rest.released = true;
        let mut stats = self.stats();
        stats.rest_releases += 1;
        stats.rest_released_bytes += freed;
    }

    /// Copies the render thread's live resource sizes into shared status.
    /// Sampling follows completed work so the first frame's caches and a
    /// retired pool are reflected without reading mutable state off-thread.
    fn snapshot_allocations(&mut self, sampled_at_us: i64) {
        let metal_device = self
            .gpu
            .as_ref()
            .and_then(|g| pm_gpu::device::metal_device_allocation(&g.device));
        let mut inventory = AllocationInventory {
            sampled_at_us,
            gpu_created: self.gpu.is_some(),
            composer_created: self.gpu.as_ref().is_some_and(|g| g.composer.is_some()),
            snapshot_target_bytes: self
                .gpu
                .as_ref()
                .and_then(|g| g.snapshot_target.as_ref())
                .map_or(0, pm_gpu::Target::bytes),
            snapshot_readback_bytes: self
                .gpu
                .as_ref()
                .map_or(0, |g| g.snapshot_readback.capacity_bytes()),
            metal_allocated_bytes: metal_device.map(|d| d.current_allocated_bytes),
            metal_device: metal_device.map(Into::into),
            metal_snapshot_texture: self
                .gpu
                .as_ref()
                .and_then(|g| g.snapshot_target.as_ref())
                .and_then(|target| pm_gpu::device::metal_texture_allocation(&target.texture))
                .map(Into::into),
            ..AllocationInventory::default()
        };
        for (&(port, session), view) in &self.views {
            let color = view
                .pool
                .as_ref()
                .map(|live| SurfaceAllocation::of(live.spec, &live.pool));
            let depth = view
                .depth
                .as_ref()
                .map(|live| SurfaceAllocation::of(live.spec, &live.pool));
            let working_target_bytes = view
                .pool
                .as_ref()
                .and_then(|live| live.working.as_ref())
                .map_or(0, |working| {
                    u64::from(working.size.width)
                        * u64::from(working.size.height)
                        * u64::from(WORKING_FORMAT.block_copy_size(None).unwrap_or(8))
                });
            let metal_working_texture = view
                .pool
                .as_ref()
                .and_then(|live| live.working.as_ref())
                .and_then(|working| pm_gpu::device::metal_texture_allocation(&working.texture))
                .map(Into::into);
            inventory.color_surface_bytes += color.as_ref().map_or(0, |pool| pool.iosurface_bytes);
            inventory.depth_surface_bytes += depth.as_ref().map_or(0, |pool| pool.iosurface_bytes);
            inventory.working_target_bytes += working_target_bytes;
            inventory.views.push(ViewAllocation {
                port: port.0,
                session: session.get(),
                color,
                depth,
                working_target_bytes,
                metal_working_texture,
            });
        }
        if let Some(composer) = self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
            let renderer = composer.renderer();
            let scene3d = renderer.scene3d_resident();
            inventory.renderer = Some(RendererAllocation {
                target_pool_bytes: renderer.target_pool_bytes(),
                target_pool_idle: renderer.target_pool_idle(),
                target_pool_created: renderer.target_pool_created(),
                pyramid_bytes: renderer.pyramid_bytes(),
                media_texture_bytes: renderer.media_texture_bytes(),
                media: renderer.media_usage(),
                text_cache_bytes: renderer.text_cache_bytes(),
                vector_cache_bytes: renderer.vector_cache_bytes(),
                scene3d_meshes: scene3d.meshes,
                scene3d_mesh_bytes: scene3d.mesh_bytes,
                scene3d_maps: scene3d.maps,
                scene3d_map_bytes: scene3d.map_bytes,
            });
        }
        self.stats().allocations = inventory;
    }
    fn log(&self, level: LogLevel, message: String) {
        (self.sink)(vec![Action::Log { level, message }]);
    }

    /// Records a render-thread event in the trace (trace builds only).
    fn trace_event(&self, what: &'static str, at: i64, us: i64, k: i64) {
        if !self.config.trace {
            return;
        }
        let mut stats = self.stats();
        // A repeated instant event (a wait polled every millisecond) once.
        if us == 0
            && stats
                .turn_trace
                .back()
                .is_some_and(|e| e.us == 0 && e.what == what && e.k == k)
        {
            return;
        }
        if stats.turn_trace.len() >= TURN_TRACE_CAP {
            stats.turn_trace.pop_front();
        }
        stats.turn_trace.push_back(TurnTrace { at, us, what, k });
    }

    fn compat_store(&self) -> std::sync::MutexGuard<'_, super::compat::CompatStore> {
        self.compat.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn stats(&self) -> std::sync::MutexGuard<'_, Stats> {
        self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a turn that stalled the thread (see [`STALL_US`]).
    fn note_turn(&self, what: &'static str, started: i64) {
        let took = now() - started;
        if took >= 1_000 {
            self.trace_event(what, started, took, -1);
        }
        let band = slow_turn_band(took);
        if band.is_none() {
            return;
        }
        {
            let mut stats = self.stats();
            stats.turn_us_max = stats.turn_us_max.max(took);
            if let Some(band) = band {
                stats.slow_turns[band] += 1;
            }
            if took < STALL_US {
                return;
            }
            stats.stalls += 1;
            stats.stall_us_max = stats.stall_us_max.max(took);
            stats.last_stall = Some(format!("{what}: {} ms", took / 1000));
        }
        self.log(
            LogLevel::Warn,
            format!(
                "presenter: the render thread spent {} ms on {what}",
                took / 1000
            ),
        );
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
                    self.gpu_epoch += 1;
                    if self.config.document
                        && let Some(rasters) = &self.rasters
                    {
                        rasters.warm(&device, &queue, self.gpu_epoch);
                    }
                    self.gpu = Some(Gpu {
                        device,
                        queue,
                        display,
                        snapshot_readback: pm_gpu::readback::ReadbackRing::default(),
                        snapshot_target: None,
                        composer: None,
                        fonts_installed: None,
                        lost,
                        epoch: self.gpu_epoch,
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
        // A lost device fails its submissions at once.
        self.drain_frames();
        self.fail_pending_snapshot_maps(
            ErrorCode::DeviceLost,
            "The GPU was lost before the snapshot finished.",
        );
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

    fn preview_status(&self, key: Key, status: PreviewStatus) {
        let revision = status.revision;
        self.event(key, Event::PreviewStatus(status), Some(revision));
    }

    fn fail_preview_snapshot(
        &mut self,
        key: Key,
        revision: Revision,
        generation: Option<Seq>,
        error: Option<String>,
    ) {
        if let Some(error) = error
            && let Some(status) = self
                .preview_controls
                .failed(key, revision, generation, error)
        {
            self.preview_status(key, status);
        }
    }

    /// A build that produced no frame cannot enter `queue_auto_preview` or
    /// `preview_frame_sent`; end its pending preview generation here.
    #[allow(clippy::too_many_arguments)]
    fn fail_preview_without_frame(
        &mut self,
        key: Key,
        revision: Revision,
        generation: Option<Seq>,
        shading: Option<Shading3d>,
        composition: &str,
        time: Ticks,
        viewer_frame: bool,
        error: String,
    ) {
        if let Some(generation) = generation {
            if let Some(status) =
                self.preview_controls
                    .failed(key, revision, Some(generation), error)
            {
                self.preview_status(key, status);
            }
            return;
        }
        if !viewer_frame {
            return;
        }
        let Some(shading) = shading else { return };
        let candidate = PreviewCandidate {
            generation: None,
            auto_generation: None,
            revision,
            composition: composition.to_owned(),
            time,
            shading,
            complete: false,
            failure: Some(error.clone()),
        };
        if let Some(auto) = AutoPreview::next(
            self.auto_preview.get(&key),
            self.auto_generation.get(&key).copied(),
            &candidate,
        ) {
            self.auto_generation.insert(key, auto.generation);
            self.auto_preview.insert(key, auto);
        }
        if let Some(auto) = self.auto_preview.get_mut(&key)
            && auto.matches(&candidate)
            && auto.phase != PreviewPhase::Failed
        {
            auto.phase = PreviewPhase::Failed;
            let status = Self::auto_status(auto, PreviewPhase::Failed, None, Some(error));
            self.preview_status(key, status);
        }
    }

    /// A frame that cannot be delivered must settle the accepted preview.
    /// The candidate carries the identity sampled when this render began.
    fn fail_preview_delivery(
        &mut self,
        key: Key,
        candidate: Option<&PreviewCandidate>,
        error: String,
    ) {
        if let Some(candidate) = candidate {
            self.fail_preview_without_frame(
                key,
                candidate.revision,
                candidate.generation,
                Some(candidate.shading),
                &candidate.composition,
                candidate.time,
                true,
                error,
            );
        } else if let Some(control) = self.preview_controls.current(key).cloned() {
            self.fail_preview_snapshot(
                key,
                control.revision,
                Some(control.generation),
                Some(error),
            );
        }
    }

    /// The GPU completion thread can fail after the automatic status was
    /// queued. Only that candidate's generation may receive its failure.
    fn fail_preview_sent(&mut self, key: Key, candidate: PreviewCandidate, error: String) {
        if let Some(generation) = candidate.generation {
            self.fail_preview_snapshot(key, candidate.revision, Some(generation), Some(error));
        } else if let Some(auto) = self.auto_preview.get_mut(&key)
            && candidate.auto_generation == Some(auto.generation)
            && auto.matches(&candidate)
            && auto.phase != PreviewPhase::Failed
        {
            auto.phase = PreviewPhase::Failed;
            let status = Self::auto_status(auto, PreviewPhase::Failed, None, Some(error));
            self.preview_status(key, status);
        }
    }

    fn auto_status(
        auto: &AutoPreview,
        phase: PreviewPhase,
        frame_seq: Option<Seq>,
        error: Option<String>,
    ) -> PreviewStatus {
        PreviewStatus {
            generation: auto.generation,
            automatic: true,
            revision: auto.revision,
            composition: auto.composition.as_str().into(),
            time: auto.time,
            phase,
            frame_seq,
            error,
            cache_only: false,
        }
    }

    fn queue_auto_preview(&mut self, key: Key, seq: Seq) {
        let Some(candidate) = self.preview_candidates.get(&(key, seq.get())).cloned() else {
            return;
        };
        if candidate.generation.is_some() {
            return;
        }
        if let Some(auto) = AutoPreview::next(
            self.auto_preview.get(&key),
            self.auto_generation.get(&key).copied(),
            &candidate,
        ) {
            self.auto_generation.insert(key, auto.generation);
            let status = Self::auto_status(&auto, PreviewPhase::Queued, None, None);
            self.auto_preview.insert(key, auto);
            self.preview_status(key, status);
        }
        let generation = self
            .auto_preview
            .get(&key)
            .filter(|auto| auto.matches(&candidate))
            .map(|auto| auto.generation);
        if let Some(candidate) = self.preview_candidates.get_mut(&(key, seq.get())) {
            candidate.auto_generation = generation;
        }
        if let Some(error) = candidate.failure.as_ref()
            && let Some(auto) = self.auto_preview.get_mut(&key)
            && auto.matches(&candidate)
            && auto.phase != PreviewPhase::Failed
        {
            auto.phase = PreviewPhase::Failed;
            let status = Self::auto_status(auto, PreviewPhase::Failed, None, Some(error.clone()));
            self.preview_status(key, status);
        }
    }

    /// The completion thread already sent this descriptor after GPU success.
    /// A generation captured while encoding prevents an older frame at the
    /// same revision/time from completing a restarted request.
    fn preview_frame_sent(&mut self, key: Key, seq: Seq) {
        let Some(candidate) = self.preview_candidates.remove(&(key, seq.get())) else {
            return;
        };
        if !candidate.complete {
            return;
        }
        if let Some(generation) = candidate.generation {
            if let Some(status) = self.preview_controls.presented(
                key,
                super::preview_control::PresentedFrame {
                    revision: candidate.revision,
                    composition: &candidate.composition,
                    time: candidate.time,
                    seq,
                    all_surfaces: true,
                    generation,
                },
            ) {
                self.preview_status(key, status);
            }
        } else if let Some(auto) = self.auto_preview.get(&key)
            && candidate.auto_generation == Some(auto.generation)
            && auto.matches(&candidate)
        {
            let status = Self::auto_status(auto, PreviewPhase::Ready, Some(seq), None);
            self.auto_preview.remove(&key);
            self.preview_status(key, status);
        }
    }

    fn cancel_preview(&mut self, key: Key) {
        self.cancel_cache_preview(key);
        if let Some(status) = self.preview_controls.cancelled(key) {
            self.preview_status(key, status);
        }
        self.cancel_auto_preview(key);
        if let Some(blender) = &self.blender {
            let revision = self.views.get(&key).map_or(Revision::ZERO, |v| v.revision);
            blender.submit(key, None, revision, None, None);
        }
    }

    fn cancel_auto_preview(&mut self, key: Key) {
        if let Some(auto) = self.auto_preview.remove(&key) {
            self.preview_status(
                key,
                Self::auto_status(&auto, PreviewPhase::Cancelled, None, None),
            );
        }
        self.preview_candidates.retain(|(k, _), _| *k != key);
    }

    fn cancel_cache_preview(&mut self, key: Key) {
        self.cache_requested.remove(&key);
        let tokens: Vec<u64> = self
            .cache_previews
            .iter()
            .filter_map(|(token, job)| (job.key == key).then_some(*token))
            .collect();
        for token in tokens {
            self.cache_previews.remove(&token);
            if let Some(blender) = &self.blender {
                blender.cancel_capture(token);
            }
        }
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
                if self
                    .preview_controls
                    .current(key)
                    .is_some_and(|pending| pending.composition != state.composition)
                    || (state.headless == Some(true)
                        && self
                            .preview_controls
                            .current(key)
                            .is_some_and(|pending| !pending.cache_only))
                {
                    self.cancel_preview(key);
                }
                self.configure(key, *state, revision, pool);
                if self.views.get(&key).is_some_and(|view| {
                    super::blender::blender_shading(view.state.view.as_ref()).is_none()
                }) && let Some(auto) = self.auto_preview.remove(&key)
                {
                    self.preview_status(
                        key,
                        Self::auto_status(&auto, PreviewPhase::Cancelled, None, None),
                    );
                }
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
            Command::Transport {
                key,
                command,
                at,
                audio_forwarded,
            } => {
                let interrupts = match command {
                    TransportCommand::Play { .. }
                    | TransportCommand::Step { .. }
                    | TransportCommand::PlayAround { .. } => true,
                    TransportCommand::Pause => {
                        self.views.get(&key).is_some_and(|v| v.clock.is_playing())
                    }
                    TransportCommand::Seek { time, .. } => {
                        self.preview_controls
                            .current(key)
                            .is_some_and(|pending| pending.time != time)
                            || self
                                .auto_preview
                                .get(&key)
                                .is_some_and(|auto| auto.time != time)
                    }
                    _ => false,
                };
                if interrupts {
                    if self
                        .preview_controls
                        .current(key)
                        .is_some_and(|c| c.cache_only)
                    {
                        self.cancel_auto_preview(key);
                    } else {
                        self.cancel_preview(key);
                    }
                }
                self.transport(key, command, at, audio_forwarded);
                self.refresh_audio_ingress(key);
            }
            Command::PreviewControl { key, control } => {
                let Some(status) = self.preview_controls.control(key, control.clone()) else {
                    return true;
                };
                self.cancel_cache_preview(key);
                if control.action == PreviewAction::Cancel {
                    if let Some(blender) = &self.blender {
                        blender.submit(key, None, control.revision, None, None);
                    }
                    self.preview_candidates.retain(|(k, _), _| *k != key);
                    self.auto_preview.remove(&key);
                    self.preview_status(key, status);
                } else {
                    self.preview_status(key, status);
                    if control.cache_only {
                        self.cache_requested.insert(key, control);
                        return true;
                    }
                    let valid = self.config.document
                        && self.blender.is_some()
                        && self.views.get(&key).is_some_and(|v| {
                            v.pool.is_some()
                                && v.state.headless != Some(true)
                                && v.state.composition == control.composition
                        });
                    if !valid {
                        if let Some(failed) = self.preview_controls.failed(
                            key,
                            control.revision,
                            Some(control.generation),
                            "The native viewer cannot render this composition yet.".into(),
                        ) {
                            self.preview_status(key, failed);
                        }
                    } else {
                        if let Some(view) = self.views.get_mut(&key) {
                            view.dirty = true;
                        }
                    }
                }
            }
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
            Command::Document {
                audio_forwarded,
                key,
                update,
                posted,
            } => {
                let mirrored = self.raster_mirrors.contains(&key).then(|| update.clone());
                self.document(key, update, posted, audio_forwarded);
                self.mirror_update(key, mirrored);
            }
            Command::FrameSent {
                key,
                meta,
                gpu_done,
                sent,
            } => {
                self.record_frame(key, &meta, gpu_done, sent);
                if let Ok(seq) = Seq::new(meta.seq) {
                    self.preview_frame_sent(key, seq);
                }
            }
            Command::FrameFailed { key, meta, error } => {
                if let Some(candidate) = self.preview_candidates.remove(&(key, meta.seq)) {
                    self.fail_preview_sent(
                        key,
                        candidate,
                        format!("The native viewer could not present the preview: {error}"),
                    );
                }
                if let Some(view) = self.views.get_mut(&key) {
                    if let (Some(live), Ok(seq)) = (view.pool.as_mut(), Seq::new(meta.seq))
                        && live.spec.generation == meta.generation
                    {
                        let _ = live.ledger.release(FrameRelease {
                            pool_index: meta.index,
                            seq,
                        });
                    }
                    view.dirty = true;
                }
                self.log(
                    LogLevel::Error,
                    format!("frame bridge: the GPU did not finish a frame: {error}"),
                );
            }
            Command::SnapshotMapReady => {}
            Command::SnapshotPollFailed { lost, error } => {
                if !self
                    .gpu
                    .as_ref()
                    .is_some_and(|gpu| Arc::ptr_eq(&gpu.lost, &lost))
                {
                    return true;
                }
                lost.store(true, Ordering::Release);
                self.fail_pending_snapshot_maps(
                    ErrorCode::DeviceLost,
                    &format!("The GPU could not finish the snapshot: {error}"),
                );
            }
            Command::Definitions {
                key,
                set,
                registered,
            } => {
                let doc = self.docs.entry(key).or_default();
                doc.set_definitions(set, registered);
                let definitions = doc.definitions().cloned();
                let registered = doc.take_registered();
                // A definition failure is reported at the definitions
                // generation it was translated at (not a document
                // revision): the kernel drops one older than its latest
                // push of that definition, the translation of a set already
                // replaced (kernel/engine-definitions.ts).
                let revision = definitions
                    .as_ref()
                    .and_then(|(set, _)| Revision::new(set.generation.get()).ok())
                    .unwrap_or(Revision::ZERO);
                // Translate now (not at the next frame) and report each
                // definition that fails (contract 10; the kernel fills the
                // shader UI from these).
                let mut failures = Vec::new();
                if self.config.document {
                    if let Some(composer) = self.gpu.as_mut().and_then(|gpu| gpu.composer.as_mut())
                    {
                        failures = composer
                            .use_definitions(key, definitions.as_ref(), &registered)
                            .unwrap_or_default();
                    } else if let Some(definitions) = definitions.as_ref() {
                        failures = self.prepared.prepare(
                            key,
                            definitions,
                            &registered,
                            self.config.shader_cache.as_ref(),
                        );
                    }
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
                if let Some(request) = self.peaks_waiting.remove(&(key, asset.clone())) {
                    self.peaks
                        .request(key, asset.clone(), path.clone(), request);
                }
                let doc = self.docs.entry(key).or_default();
                doc.set_media(&asset, path);
                let media = doc.media().clone();
                if let Some(audio) = self.audio.get(&key) {
                    audio.send(AudioCtl::Media(media));
                }
                self.refresh_audio_ingress(key);
                if let Some(view) = self.views.get_mut(&key) {
                    view.dirty = true;
                }
            }
            Command::ShaderFallback { key, message } => self.shader_fallback(key, message),
            Command::AudioMemoryPressure => {
                for audio in self.audio.values() {
                    audio.send(AudioCtl::MemoryPressure);
                }
            }
            Command::FrameCache { bytes } => {
                self.frame_cache = bytes;
                self.stats().frame_cache = bytes as u64;
                if let Some(composer) = self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
                    composer.renderer().apply_media_lease(media_lease(bytes));
                }
            }
            Command::RasterDone(done) => self.raster_done(*done),
            Command::RasterReseed(job) => self.raster_reseed(*job),
            Command::Snapshot(job) => {
                if let Err(refused) = self.snapshots.push(*job) {
                    let (job, refused) = *refused;
                    self.stats().snapshot_failures += 1;
                    self.snapshot_error(&job, ErrorCode::BudgetExceeded, &refused.message());
                }
            }
            Command::Compat {
                key,
                message:
                    message @ (pm_protocol::snapshot::CompatMessage::SvgPixels(_)
                    | pm_protocol::snapshot::CompatMessage::SvgFailed { .. }
                    | pm_protocol::snapshot::CompatMessage::SvgRequests { .. }),
            } => self.svg_message(key, message),
            Command::Compat { key, message } => {
                let (applied, rasters, bytes, answers, evicted) = {
                    let mut store = self.compat_store();
                    let applied = store.apply(key, message);
                    let (rasters, bytes) = store.usage();
                    (
                        applied,
                        rasters,
                        bytes,
                        store.answer_counts(),
                        store.evicted(),
                    )
                };
                {
                    let mut stats = self.stats();
                    if matches!(
                        applied,
                        super::compat::Applied::Complete
                            | super::compat::Applied::Replaced { pixels: true }
                    ) {
                        stats.compat_rasters_received += 1;
                    }
                    if matches!(applied, super::compat::Applied::Replaced { .. }) {
                        stats.compat_replaced += 1;
                    }
                    stats.compat_rasters = rasters;
                    stats.compat_bytes = bytes;
                    stats.compat_evicted = evicted;
                    stats.compat_answers = answers.0 as u64;
                    stats.compat_asked = answers.1 as u64;
                }
                // A frame waiting on the producer's answers draws now; a
                // changed producer list, or answers the producer took back
                // (an invalidation), redraw what is shown.
                if let Some(view) = self.views.get_mut(&key)
                    && applied
                        .redraws(view.compat_blocked.is_some() || view.compat_paused_wait.is_some())
                {
                    view.dirty = true;
                    view.compat_blocked = None;
                    view.compat_paused_wait = None;
                }
                self.refresh_audio_ingress(key);
            }
            Command::LoseDevice => {
                if let Some(gpu) = &self.gpu {
                    gpu.lost.store(true, Ordering::Release);
                    gpu.device.destroy();
                }
            }
            Command::FontsChanged => self.sync_font_generation(),
            Command::UseBlender(blender) => self.blender = Some(blender),
            Command::BlenderStarted {
                key,
                revision,
                generation,
                shading,
            } => {
                if let Some(status) = self.preview_controls.running(key, revision, generation) {
                    self.preview_status(key, status);
                } else if generation.is_none()
                    && let Some(auto) = self.auto_preview.get_mut(&key)
                    && auto.revision == revision
                    && auto.shading == shading
                    && auto.phase == PreviewPhase::Queued
                {
                    auto.phase = PreviewPhase::Running;
                    let status = Self::auto_status(auto, PreviewPhase::Running, None, None);
                    self.preview_status(key, status);
                }
            }
            Command::BlenderReady { key, .. } => {
                if let Some(view) = self.views.get_mut(&key)
                    && !view.clock.is_playing()
                {
                    view.dirty = true;
                }
            }
            Command::BlenderFailed {
                key,
                revision,
                generation,
                shading,
                error,
            } => {
                if let Some(status) =
                    self.preview_controls
                        .failed(key, revision, generation, error.clone())
                {
                    self.preview_status(key, status);
                } else if generation.is_none()
                    && let Some(auto) = self.auto_preview.get_mut(&key)
                    && auto.revision == revision
                    && auto.shading == shading
                {
                    auto.phase = PreviewPhase::Failed;
                    let status = Self::auto_status(auto, PreviewPhase::Failed, None, Some(error));
                    self.preview_status(key, status);
                }
            }
            Command::BlenderCaptured { token, result } => self.blender_captured(token, result),
        }
        true
    }

    /// A renderer's level 1 SVG raster band or failure (decision S1). A
    /// complete raster goes into the renderer every session on this thread
    /// shares, and every view draws again (any of them may show it).
    fn svg_message(&mut self, key: Key, message: pm_protocol::snapshot::CompatMessage) {
        use pm_protocol::snapshot::CompatMessage as M;
        match message {
            M::SvgPixels(band) => match self.svg.band(key, band, now()) {
                super::svg::Band::Partial => {}
                super::svg::Band::Complete(raster, data) => {
                    let (w, h) = (raster.width, raster.height);
                    let stored = match self.gpu.as_mut() {
                        Some(gpu) => gpu
                            .composer_with(&self.config, self.frame_cache, &mut self.prepared)
                            .renderer()
                            .put_svg_raster(raster, data),
                        None => Err("the engine has no GPU device".into()),
                    };
                    if let Err(why) = stored {
                        (self.sink)(vec![Action::Log {
                            level: LogLevel::Warn,
                            message: format!(
                                "viewer: an SVG raster ({w}x{h}) was not stored: {why}"
                            ),
                        }]);
                    }
                    for view in self.views.values_mut() {
                        view.external_wait = None;
                        view.dirty = true;
                    }
                }
                super::svg::Band::Refused(raster) => {
                    (self.sink)(vec![Action::Log {
                        level: LogLevel::Warn,
                        message:
                            "viewer: too many SVG rasters are arriving at once; a band was dropped"
                                .into(),
                    }]);
                    self.svg_failed(
                        raster,
                        "refused: too many SVG rasters were arriving at once",
                    );
                }
            },
            M::SvgFailed {
                key: raster,
                reason,
                message,
            } => self.svg_failed(raster, &format!("{reason}: {message}")),
            // Logged failures, and requests (they travel the other way).
            _ => {}
        }
    }

    /// Asks `key`'s renderer for the level 1 SVG rasters a frame drew
    /// without; returns whether one of them arrived meanwhile (the view
    /// draws again).
    fn ask_svg(
        svg: &mut super::svg::SvgState,
        sink: &Sink,
        renderer: &pm_render::Renderer,
        key: Key,
        revision: Option<Revision>,
        misses: &[pm_render::vector::svg_rasters::SvgRasterNeed],
    ) -> bool {
        let arrived = misses
            .iter()
            .any(|m| renderer.has_svg_raster(&m.request.key));
        let requests = svg.wanted(key, misses, now(), |k| renderer.has_svg_raster(k));
        for chunk in requests.chunks(pm_protocol::svg_raster::MAX_SVG_REQUESTS) {
            let body = Event::Compat(pm_protocol::snapshot::CompatMessage::SvgRequests {
                requests: chunk.to_vec(),
            });
            let mut envelope = Envelope::new(Message::Event { body }).with_session(key.1);
            envelope.revision = revision;
            if let Ok(bytes) = codec::encode(&envelope) {
                sink(vec![Action::Send { port: key.0, bytes }]);
            }
        }
        arrived
    }

    /// The runner's answer: a band of pixels (the frame renders again once
    /// every band of a request arrived), or a failure: the program draws
    /// nothing or is skipped, as WebGL does with a program it refuses, and
    /// is reported (a draw past the runner's deadline is turned off by the
    /// watchdog). Nothing is routed to the legacy viewer (amendment 21).
    fn shader_fallback(
        &mut self,
        key: Key,
        message: pm_protocol::shader_fallback::ShaderFallbackMessage,
    ) {
        use pm_protocol::shader_fallback::ShaderFallbackMessage as M;
        match message {
            M::Pixels(band) => {
                if self.fallback.asked.get(&band.key).map(|a| a.0) != Some(key) {
                    return;
                }
                let Some((request, pixels)) = self.fallback.bands.add(&band) else {
                    return;
                };
                self.fallback.asked.remove(&request);
                if let Some(composer) = self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
                    // Effects' requests/results belong to their session's
                    // definition set. Another viewer/snapshot may have
                    // parked it while ANGLE drew this request.
                    if let Some(doc) = self.docs.get(&key) {
                        let _ = composer.use_definitions(key, doc.definitions(), &[]);
                    }
                    composer.renderer().put_fallback_pixels(request, pixels);
                }
                if let Some(view) = self.views.get_mut(&key) {
                    view.external_wait = None;
                    view.dirty = true;
                }
            }
            M::Failed {
                key: request,
                reason,
                message,
            } => {
                if self.fallback.asked.get(&request).map(|a| a.0) != Some(key) {
                    return;
                }
                self.fallback_failed(key, &request, reason, message);
            }
            // Requests and their textures travel the other way.
            M::Requests { .. } | M::Input(_) => {}
        }
    }

    /// A runner request `key`'s view asked for will not be answered with
    /// pixels (the runner failed, or did not answer in time): the program
    /// draws nothing or is skipped (a transient failure is asked for again
    /// after the renderer's backoff), and the view draws at once instead of
    /// waiting out its deadline (`super::wait`).
    fn fallback_failed(&mut self, key: Key, request: &str, reason: String, message: String) {
        self.fallback.asked.remove(request);
        self.fallback.bands.forget(request);
        (self.sink)(vec![Action::Log {
            level: LogLevel::Warn,
            message: format!(
                "viewer: the shader fallback runner could not draw a program ({reason}: {message})"
            ),
        }]);
        // A transient failure (a lost context) is asked for again
        // after a backoff: the view is drawn again then.
        let retry = self
            .gpu
            .as_mut()
            .and_then(|g| g.composer.as_mut())
            .and_then(|composer| {
                // The request belongs to its session's definition set
                // (another viewer or snapshot may have parked it).
                if let Some(doc) = self.docs.get(&key) {
                    let _ = composer.use_definitions(key, doc.definitions(), &[]);
                }
                composer.renderer().put_fallback_failure(
                    request,
                    pm_render::shader_layers::FallbackFailure { reason, message },
                )
            });
        if let Some(delay) = retry.filter(|d| !d.is_zero()) {
            let at = now() + delay.as_micros() as i64;
            let slot = self.fallback.retry_at.entry(key).or_insert(at);
            *slot = (*slot).min(at);
        }
        if let Some(view) = self.views.get_mut(&key) {
            if view.external_wait.as_ref().is_some_and(|w| w.is_fallback()) {
                view.external_wait = None;
            }
            view.dirty = true;
        }
    }

    /// The window could not draw level 1 SVG bucket `raster` (`why`), a
    /// band of it was refused, or no answer came in time: viewer frames
    /// draw without it from now on (`Renderer::fail_svg_raster`) until its
    /// raster arrives, views waiting for it draw at once, and it is asked
    /// for again after `super::svg::RETRY_US` while a view draws without
    /// it. Logged once per key.
    fn svg_failed(&mut self, raster: pm_protocol::svg_raster::SvgRasterKey, why: &str) {
        let log = self.svg.failed(&raster, now());
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer_with(&self.config, self.frame_cache, &mut self.prepared)
                .renderer()
                .fail_svg_raster(raster.clone());
        }
        if log {
            (self.sink)(vec![Action::Log {
                level: LogLevel::Warn,
                message: format!(
                    "viewer: the renderer could not draw an SVG raster {}x{} ({why}); \
                     the frame is drawn without it",
                    raster.width, raster.height
                ),
            }]);
        }
        for view in self.views.values_mut() {
            if view
                .external_wait
                .as_ref()
                .is_some_and(|w| w.waits_for_svg(&raster))
            {
                view.external_wait = None;
                view.dirty = true;
            }
        }
    }

    /// A view's wait for its window's pixels timed out (`super::wait`):
    /// what it waited for is treated as a failure, and the frame is drawn
    /// without it. Its answer arriving later still draws it.
    fn external_wait_timed_out(&mut self, key: Key, on: super::wait::Awaited) {
        if let Some(view) = self.views.get_mut(&key) {
            view.dirty = true;
        }
        // A frame to draw again at once (nothing left to wait for).
        if matches!(&on, super::wait::Awaited::Svg(k) if k.is_empty()) {
            return;
        }
        self.stats().external_wait_timeouts += 1;
        match on {
            super::wait::Awaited::Svg(keys) => {
                // Those neither stored nor already failed meanwhile.
                let late: Vec<_> = match self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
                    Some(composer) => {
                        let renderer = composer.renderer();
                        keys.into_iter()
                            .filter(|k| {
                                !renderer.has_svg_raster(k) && !renderer.svg_raster_failed(k)
                            })
                            .collect()
                    }
                    None => keys,
                };
                let secs = self.wait_bounds().svg as f64 / 1e6;
                for raster in late {
                    self.svg_failed(raster, &format!("no answer within {secs} s"));
                }
            }
            super::wait::Awaited::Fallback(requests) => {
                let secs = self.wait_bounds().fallback as f64 / 1e6;
                for request in requests {
                    if self.fallback.asked.get(&request).map(|a| a.0) == Some(key) {
                        self.fallback_failed(
                            key,
                            &request,
                            "unanswered".into(),
                            format!("the runner did not answer within {secs} s"),
                        );
                    }
                }
            }
        }
    }

    /// The viewer's longest waits for its window's pixels.
    fn wait_bounds(&self) -> super::wait::Bounds {
        super::wait::Bounds::new(self.config.external_wait_cap_us)
    }

    fn forget_session(&mut self, key: Key) {
        self.prepared.forget(key);
        self.cancel_cache_preview(key);
        self.preview_controls.forget(key);
        self.preview_candidates.retain(|(k, _), _| *k != key);
        self.auto_preview.remove(&key);
        self.auto_generation.remove(&key);
        if let Some(blender) = &self.blender {
            blender.forget(key);
        }
        self.snapshot_costs.remove(&key);
        self.compat_store().forget_session(key);
        self.peaks_waiting.retain(|(k, _), _| *k != key);
        // A frame already submitted for readback is no longer in the
        // scheduler. Cancel its mapping too, before the session disappears.
        let (ended, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_snapshots)
            .into_iter()
            .partition(|pending| pending.job.key == key);
        self.pending_snapshots = live;
        if let Some(gpu) = self.gpu.as_mut() {
            for pending in &ended {
                gpu.snapshot_readback
                    .cancel(pending.ticket, MAX_CACHED_SNAPSHOT_TARGET_BYTES);
            }
        }
        for pending in ended {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &pending.job,
                ErrorCode::UnknownSession,
                "The window's engine session closed.",
            );
        }
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
        fallback.asked.retain(|request, (k, _)| {
            if *k == key {
                fallback.bands.forget(request);
            }
            *k != key
        });
        fallback.logged.retain(|(k, _)| *k != key);
        fallback.retry_at.remove(&key);
        self.svg.forget_session(key);
        self.docs.remove(&key);
        if self.raster_mirrors.remove(&key)
            && let Some(rasters) = &self.rasters
        {
            rasters.forget(key);
        }
        self.audio_ingress.forget(key);
        self.audio.remove(&key);
        self.audio_prefs.remove(&key);
        if self
            .views
            .values()
            .all(|view| view.state.headless == Some(true))
        {
            // A still-open session may already have completed its idle
            // cleanup. Start a new rest when the final attached viewer closes.
            self.rest = Rest::default();
        }
        if let Some(composer) = self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
            composer.forget_definitions(key);
        }
        let mut stats = self.stats();
        stats.routes.remove(&view_name(key));
        stats.audio.remove(&view_name(key));
    }

    /// A document update: the mirror, the audio thread's mirror, the frame
    /// grid, and a redraw.
    fn document(&mut self, key: Key, update: DocumentUpdate, posted: i64, audio_forwarded: bool) {
        let traced = i64::try_from(update.revision().get()).unwrap_or(i64::MAX);
        let doc = self.docs.entry(key).or_default();
        let copies = doc.copies();
        let updated = doc.update(&update);
        if doc.copies() > copies {
            self.stats().document_copies += 1;
        }
        let doc = self.docs.entry(key).or_default();
        if let Err(error) = updated {
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
        if !audio_forwarded {
            if let Some(audio) = self.audio.get(&key) {
                let revision = update.revision().get();
                audio.send(AudioCtl::Document(update));
                self.audio_ingress.document_sent(key, revision);
            } else {
                self.warm_audio(key);
            }
        }
        if self.config.trace {
            let pending = self.doc_trace.entry(key).or_default();
            if pending.len() < 4_096 {
                pending.push([traced, posted, now()]);
            }
        }
        self.refresh_timing(key);
        self.refresh_audio_ingress(key);
        if let Some(view) = self.views.get_mut(&key) {
            view.dirty = true;
            // A paused headless session has no frame to announce this
            // revision. Refresh its readiness clock after every update.
            if view.state.headless == Some(true) {
                view.sample_due = true;
            }
            // Frames already rendered ahead show the older revision.
            if view.clock.is_playing() {
                view.schedule.invalidate();
            }
        }
        if self.preview_controls.current(key).is_some_and(|control| {
            control.cache_only
                && self
                    .docs
                    .get(&key)
                    .is_some_and(|doc| doc.revision() > control.revision.get())
        }) {
            self.cancel_preview(key);
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
    /// Starts the line at `rate` (a Play, or the end of its preroll).
    /// `audio_forwarded`: the presenter already handed this Play to the
    /// audio thread (PLAY-1), so the line only waits for its first sample.
    fn start_play(
        &mut self,
        key: Key,
        rate: PlaybackRateValue,
        at: i64,
        audio_forwarded: Option<ForwardedAudio>,
    ) {
        let host_clock = self.config.host_clock;
        let Some(view) = self.views.get_mut(&key) else {
            return;
        };
        view.preroll = None;
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
            let before = audio_forwarded.map_or_else(
                || audio.clock().and_then(|c| c.latest()).map(|s| s.epoch),
                |f| f.before,
            );
            trace_audio_transport(
                "play-send",
                key,
                &self.views[&key],
                Some(TransportCommand::Play { rate }),
            );
            if audio_forwarded.is_none() {
                audio.send(AudioCtl::Play { from, rate });
            }
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

    /// Whether the window answered for the first [`PREROLL_FRAMES`] frames
    /// a run at `rate` from the stopped line would show: every need the last
    /// render consulted, at each of those frames' times (a layer's time
    /// moves with the frame's). A reverse run (J) shows the frames before
    /// the playhead. Frames past the composition's end, or before its start,
    /// are not asked for.
    fn preroll_ready(&self, key: Key, rate: PlaybackRateValue) -> bool {
        let Some(view) = self.views.get(&key) else {
            return true;
        };
        let Some((drawn, needs)) = &view.compat_needs else {
            return false;
        };
        if needs.is_empty() {
            return true;
        }
        let Some(doc) = self.docs.get(&key) else {
            return true;
        };
        let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
        let f = view.clock.frame_ticks();
        let start = view.clock.snap(view.clock.anchor_media());
        let end = view.clock.end();
        // Media per shown frame at this rate (at least one frame, in the
        // run's direction).
        let den = i128::from(rate.den.max(1));
        let step = (i128::from(f) * i128::from(rate.num) / den) as i64;
        let step = if step == 0 {
            if rate.num < 0 { -f } else { f }
        } else {
            step
        };
        let mut store = self.compat_store();
        for i in 0..PREROLL_FRAMES {
            let media = view.clock.snap(start + i * step);
            if end.is_some_and(|e| media >= e) || media < 0 {
                break;
            }
            let shift = media - drawn;
            for need in needs {
                let time = (need.time.get() + shift) as f64 / TICKS_PER_SECOND as f64;
                if time < 0.0 {
                    continue;
                }
                let answer = store.answer(
                    key,
                    need.composition.as_str(),
                    need.layer.as_str(),
                    time,
                    need.scale,
                    need.source,
                    need.crop,
                    revision,
                );
                if matches!(answer, pm_render::compat::CompatAnswer::Missing) {
                    return false;
                }
            }
        }
        true
    }

    /// Ends a preroll whose answers arrived or whose bound passed.
    fn finish_preroll(&mut self, key: Key) {
        let Some(preroll) = self.views.get(&key).and_then(|v| v.preroll) else {
            return;
        };
        let t = now();
        let ready = self.preroll_ready(key, preroll.rate);
        if !ready && t < preroll.until {
            return;
        }
        self.trace_event(
            if ready {
                "preroll ready"
            } else {
                "preroll bound"
            },
            preroll.asked,
            t - preroll.asked,
            -1,
        );
        {
            let mut stats = self.stats();
            stats.prerolls += 1;
            if !ready {
                stats.preroll_timeouts += 1;
            }
            stats.preroll_us_max = stats.preroll_us_max.max(t - preroll.asked);
        }
        self.start_play(key, preroll.rate, t, None);
    }

    fn transport(
        &mut self,
        key: Key,
        command: TransportCommand,
        at: i64,
        audio_forwarded: Option<ForwardedAudio>,
    ) {
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
                if !takes_transport(view.reported.as_ref()) {
                    // Not decided yet (document or media on its way): the
                    // latest Play waits for the route.
                    view.deferred_transport = Some(TransportCommand::Play { rate });
                    view.deferred_forwarded = audio_forwarded;
                    trace_audio_transport("play-deferred", key, view, view.deferred_transport);
                    return;
                }
                view.deferred_transport = None;
                view.deferred_forwarded = None;
                if view.awaiting.is_some_and(|a| a.2 == rate)
                    || (view.clock.is_playing() && view.clock.rate() == rate)
                    || view.preroll.is_some_and(|p| p.rate == rate)
                {
                    return;
                }
                // The native preroll: a stopped line with compatibility
                // producers starts once the window answered for the run's
                // first frames (or at the bound).
                // A Play the presenter already forwarded to the audio thread
                // (PLAY-1) has started; the ingress stays off for sessions
                // with producers, so only a producer that arrived after the
                // Play skips the preroll.
                if audio_forwarded.is_none()
                    && !view.clock.is_playing()
                    && view.pool.is_some()
                    && view.awaiting.is_none()
                {
                    let producers = self.compat_store().has_producers(key);
                    // Answered already (the paused warm-up filled them):
                    // the run starts at once.
                    if producers && !self.preroll_ready(key, rate) {
                        let view = self.views.get_mut(&key).expect("checked");
                        view.preroll = Some(Preroll {
                            rate,
                            asked: at,
                            until: at + PREROLL_MAX_US,
                        });
                        self.trace_event("preroll", at, 0, -1);
                        return;
                    }
                }
                self.start_play(key, rate, at, audio_forwarded);
            }
            TransportCommand::PlayAround {
                pre_roll,
                post_roll,
            } => {
                if !takes_transport(view.reported.as_ref()) {
                    view.deferred_transport = Some(TransportCommand::PlayAround {
                        pre_roll,
                        post_roll,
                    });
                    view.deferred_forwarded = audio_forwarded;
                    trace_audio_transport(
                        "play-around-deferred",
                        key,
                        view,
                        view.deferred_transport,
                    );
                    return;
                }
                view.deferred_transport = None;
                view.preroll = None;
                view.deferred_forwarded = None;
                let here = view.clock.media_at(at);
                if !host_clock && self.audio_for(key).is_some() {
                    let audio = &self.audio[&key];
                    let before = audio_forwarded.map_or_else(
                        || audio.clock().and_then(|c| c.latest()).map(|s| s.epoch),
                        |f| f.before,
                    );
                    if audio_forwarded.is_none() {
                        audio.send(AudioCtl::PlayAround {
                            at: here,
                            pre: pre_roll,
                            post: post_roll,
                        });
                    }
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
                trace_audio_transport("pause-explicit", key, view, None);
                view.deferred_transport = None;
                view.deferred_forwarded = None;
                view.awaiting = None;
                view.preroll = None;
                if view.clock.apply(TransportCommand::Pause, at) {
                    view.schedule.reset();
                    view.sample_due = true;
                    view.dirty = true;
                }
                if let Some(audio) = self.audio.get(&key)
                    && audio_forwarded.is_none()
                {
                    audio.send(AudioCtl::Pause);
                }
            }
            TransportCommand::Seek { time, mode } => {
                let playing = view.clock.following() && view.clock.is_playing();
                let awaiting = view.awaiting;
                let engine = takes_transport(view.reported.as_ref());
                view.scrub = mode == SeekMode::Scrub && !playing && awaiting.is_none();
                view.clock.apply(TransportCommand::Seek { time, mode }, at);
                if playing || awaiting.is_some() {
                    // The audio seeks and starts a new run; the line holds the
                    // target until its first sample.
                    view.clock.hold(time.get(), at);
                    let before = audio_forwarded.map_or_else(
                        || {
                            self.audio
                                .get(&key)
                                .and_then(|a| a.clock())
                                .and_then(|c| c.latest())
                                .map(|s| s.epoch)
                        },
                        |f| f.before,
                    );
                    let rate = awaiting.map_or(PlaybackRateValue::NORMAL, |a| a.2);
                    let view = self.views.get_mut(&key).expect("checked");
                    view.awaiting = Some((at + AUDIO_START_TIMEOUT_US, before, rate));
                    if let Some(audio) = self.audio.get(&key)
                        && audio_forwarded.is_none()
                    {
                        audio.send(AudioCtl::Seek(time.get()));
                    }
                } else if mode == SeekMode::Scrub && audio_forwarded.is_none() {
                    self.scrub_grain(key, time.get());
                } else if !host_clock
                    && engine
                    && let Some(audio) = self.audio_for(key)
                {
                    // Paused exact seeks prepare the next audio start too.
                    if audio_forwarded.is_none() {
                        audio.send(AudioCtl::Seek(time.get()));
                    }
                }
                let view = self.views.get_mut(&key).expect("checked");
                view.schedule.reset();
                view.sample_due = true;
                view.dirty = true;
            }
            TransportCommand::SetLoop { range } => {
                view.clock.apply(TransportCommand::SetLoop { range }, at);
                view.sample_due = true;
                if let Some(audio) = self.audio.get(&key)
                    && audio_forwarded.is_none()
                {
                    audio.send(AudioCtl::SetLoop(range));
                }
            }
            TransportCommand::AudioScrub { enabled } => {
                self.audio_prefs.entry(key).or_default().scrub = enabled;
                if let Some(audio) = self.audio.get(&key)
                    && audio_forwarded.is_none()
                {
                    audio.send(AudioCtl::AudioScrub(enabled));
                }
            }
            TransportCommand::Step { frames } => {
                // Steps while paused: the line, and a grain where it lands.
                // A step shows its exact frame.
                view.scrub = false;
                view.preroll = None;
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

    /// Update the paused audio playhead; AudioCtl::Scrub plays a grain
    /// only when enabled, otherwise it silently prepares the next start.
    fn scrub_grain(&mut self, key: Key, media: i64) {
        if self.config.host_clock
            || !takes_transport(self.views.get(&key).and_then(|v| v.reported.as_ref()))
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
            AudioMessage::PeaksWanted { asset, request } => {
                // Asked before the host answered for the asset (a file just
                // imported): wait for the answer rather than call it offline.
                match self.docs.get(&key).and_then(|d| d.media().answer(&asset)) {
                    Some(path) => self.peaks.request(key, asset, path, request),
                    None => {
                        let pending = (key, asset);
                        if self.peaks_waiting.contains_key(&pending)
                            || self.peaks_waiting.len() < MAX_PEAKS_WAITING
                        {
                            self.peaks_waiting.insert(pending, request);
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
            let media = self.views.get(&key)?.clock.media_at(now());
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
                    link.send(AudioCtl::Seek(media));
                    let prefs = self.audio_prefs.get(&key).cloned().unwrap_or_default();
                    link.send(AudioCtl::AudioScrub(prefs.scrub));
                    link.send(AudioCtl::Solo(prefs.solo));
                    // Whichever control creates the link (a paused seek can
                    // precede the first document), it warms like
                    // `warm_audio`; a Play right after opens the device anyway.
                    if self.config.document {
                        link.send(AudioCtl::Warm);
                    }
                    let enabled = self.ingress_enabled(key, &link);
                    self.audio_ingress
                        .register(key, &link, enabled, doc.revision());
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

    fn refresh_audio_ingress(&self, key: Key) {
        if let Some(link) = self.audio.get(&key) {
            self.audio_ingress.register(
                key,
                link,
                self.ingress_enabled(key, link),
                self.docs.get(&key).map_or(0, |doc| doc.revision()),
            );
        }
    }

    /// Whether the presenter may hand `key`'s transport straight to its
    /// audio thread (PLAY-1): an audible, routed view on the device clock.
    /// A session with compatibility producers keeps its transport on this
    /// thread, whose Play may first preroll the producers' answers.
    fn ingress_enabled(&self, key: Key, link: &AudioLink) -> bool {
        !self.config.host_clock
            && !link.failed()
            && self.audible(key)
            && takes_transport(self.views.get(&key).and_then(|v| v.reported.as_ref()))
            && !self.compat_store().has_producers(key)
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
        if ready {
            // A new link is sent AudioCtl::Warm by `audio_for`.
            self.audio_for(key);
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
        // A new state or pool comes after the frames of the old one.
        self.drain_frames();
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
            external_wait: None,
            svg_without: Vec::new(),
            requests: VecDeque::new(),
            retry: None,
            reported: None,
            layer_reports: Default::default(),
            awaiting: None,
            deferred_transport: None,
            deferred_forwarded: None,
            audio_version: 0,
            scrub: false,
            stand_in: None,
            watchdog_held: false,
            render_failed: false,
            render_retry: None,
            costs: Default::default(),
            depth: None,
            preroll: None,
            compat_blocked: None,
            compat_paused_wait: None,
            compat_needs: None,
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
        let attached = view.state.headless == Some(true) && state.headless != Some(true);
        view.state = state;
        // A viewer attaching to a session whose composition the renderer
        // cannot draw is told so (the report went out while headless).
        if attached && matches!(view.reported, Some(Route::Legacy(_))) {
            view.reported = None;
        }
        if let Some(revision) = revision {
            view.revision = view.revision.max(revision);
        }
        if !hint_only {
            view.external_wait = None;
            view.dirty = true;
        }
        let refresh = composition_changed || !view.clock.is_playing();
        if composition_changed {
            self.audio_ingress.forget(key);
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
        if self.views[&key].state.headless == Some(true) {
            self.rest = Rest::default();
            if let Some(blender) = &self.blender {
                // Presentation leaves; captures still belong to this session.
                blender.submit(key, None, self.views[&key].revision, None, None);
            }
            if let Some(old) = self.views.get_mut(&key).and_then(|v| v.pool.take()) {
                self.retire_pool(key, old);
            }
            self.configure_depth(key, None);
            let view = self.views.get_mut(&key).expect("configured");
            view.retry = None;
            view.stand_in = None;
            view.schedule.reset();
            view.sample_due = true;
            return;
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
                    view.external_wait = None;
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
                if retry.is_none_or(|retry| retry.at.is_none()) {
                    let message = format!(
                        "The native viewer could not create the preview frame surface: {error}"
                    );
                    if let Some(control) = self.preview_controls.current(key).cloned() {
                        self.fail_preview_snapshot(
                            key,
                            control.revision,
                            Some(control.generation),
                            Some(message.clone()),
                        );
                    }
                    if let Some(auto) = self.auto_preview.get_mut(&key)
                        && auto.phase != PreviewPhase::Failed
                    {
                        auto.phase = PreviewPhase::Failed;
                        let status =
                            Self::auto_status(auto, PreviewPhase::Failed, None, Some(message));
                        self.preview_status(key, status);
                    }
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
        let working = Some(WorkingTexture::new(&gpu.device, spec.size));
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
        if let Some(deadline) = self.pending_snapshots.iter().map(|p| p.map_deadline).min() {
            wake = wake.min(deadline);
        }
        if let Some(at) = self.fallback.retry_at.values().min() {
            wake = wake.min(*at);
        }
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
            if view.render_failed
                && let Some(at) = view.render_retry.and_then(|r| r.at)
            {
                wake = wake.min(at);
            }
            let external_waiting = view.external_wait.as_ref().is_some_and(|w| w.until > now);
            if let Some(w) = &view.external_wait {
                wake = wake.min(w.until);
            }
            // Buckets the latest frame drew without are asked for again.
            for need in &view.svg_without {
                if let Some(at) = self.svg.retry_at(&need.request.key) {
                    wake = wake.min(at);
                }
            }
            if (view.dirty
                && !external_waiting
                && view.state.headless != Some(true)
                && view.preroll.is_none()
                && view.awaiting.is_none())
                || !view.requests.is_empty()
                || (view.sample_due
                    && (view.state.headless != Some(true)
                        || takes_transport(view.reported.as_ref())))
            {
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
            if let Some(preroll) = view.preroll {
                // Answers arriving wake the thread; the bound needs a timer.
                wake = wake.min(preroll.until.max(now));
            }
            if let Some(at) = view.compat_paused_wait {
                wake = wake.min(at.max(now));
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
                if view.state.headless == Some(true) {
                    continue;
                }
                if external_waiting {
                    continue;
                }
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
        let generation = fonts.generation();
        let installed = self.gpu.as_ref().and_then(|g| g.fonts_installed);
        if installed.is_some_and(|g| g != generation) {
            for view in self.views.values_mut() {
                view.dirty = true;
            }
        }
    }

    /// Waits until every frame waiting on the GPU is sent (or abandoned),
    /// for anything whose order against them matters. Their bookkeeping
    /// arrives as commands.
    fn drain_frames(&mut self) {
        let Some(completer) = &self.completer else {
            return;
        };
        if completer.in_flight() == 0 {
            return;
        }
        completer.drain();
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
        self.work_cache_previews();
        // The GPU watchdog sees completions promptly and frames that run
        // away while they run (a turn is at most 50 ms apart); a frame it
        // held is drawn again once the suspect frame is judged.
        let holding = match self.gpu.as_mut().and_then(|g| g.composer.as_mut()) {
            Some(composer) => {
                composer.renderer().watchdog_watch();
                composer.renderer().watchdog_holding()
            }
            None => false,
        };
        if !holding {
            for view in self.views.values_mut() {
                if std::mem::take(&mut view.watchdog_held) {
                    view.dirty = true;
                }
            }
        }
        // Programs whose runner request failed transiently are asked for
        // again once their backoff passed.
        let t = now();
        let views = &mut self.views;
        self.fallback.retry_at.retain(|key, at| {
            if *at > t {
                return true;
            }
            if let Some(view) = views.get_mut(key) {
                if view.external_wait.as_ref().is_some_and(|w| w.is_fallback()) {
                    view.external_wait = None;
                }
                view.dirty = true;
            }
            false
        });
        // Level 1 SVG buckets views draw without (the window could not
        // draw them) are asked for again once due; no frame is drawn for
        // it, their raster arriving draws them.
        let due: Vec<Key> = self
            .views
            .iter()
            .filter(|(_, v)| {
                v.svg_without
                    .iter()
                    .any(|n| self.svg.retry_at(&n.request.key).is_some_and(|at| at <= t))
            })
            .map(|(k, _)| *k)
            .collect();
        for key in due {
            let Some(view) = self.views.get(&key) else {
                continue;
            };
            let revision = Some(view.revision);
            let needs = view.svg_without.clone();
            if let Some(gpu) = self.gpu.as_mut() {
                let composer =
                    gpu.composer_with(&self.config, self.frame_cache, &mut self.prepared);
                if Self::ask_svg(
                    &mut self.svg,
                    &self.sink,
                    composer.renderer(),
                    key,
                    revision,
                    &needs,
                ) && let Some(view) = self.views.get_mut(&key)
                {
                    view.dirty = true;
                }
            }
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
                trace_audio_transport("pause-fallback", key, view, None);
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
        let audio_revision = doc.revision();
        let ingress_enabled = self
            .views
            .get(&key)
            .is_some_and(|v| v.reported.as_ref() != Some(&route))
            && !self.config.host_clock
            && takes_transport(Some(&route))
            && self.audible(key);
        let view = self.views.get_mut(&key).expect("checked");
        let changed = view.reported.as_ref() != Some(&route);
        let mut stop = false;
        if changed {
            view.reported = Some(route.clone());
            if let Some(link) = self.audio.get(&key) {
                self.audio_ingress
                    .register(key, link, ingress_enabled, audio_revision);
            }
            trace_audio_transport("route-changed", key, view, None);
            match &route {
                Route::Engine => {
                    view.dirty = true;
                    view.sample_due = true;
                }
                // Something the renderer cannot draw (plan amendment 21: no
                // other engine draws it): the stage shows the named failure,
                // and the transport and audio go on as they were.
                Route::Legacy(_) => {}
                Route::Pending(_) => {
                    stop = view.clock.is_playing() || view.awaiting.is_some();
                }
            }
        }
        if stop {
            // Not decided (a resync, media being resolved): the run stops
            // and resumes, from where it stopped, once the route is known.
            let rate = view.awaiting.map_or(view.clock.rate(), |a| a.2);
            view.deferred_transport
                .get_or_insert(TransportCommand::Play { rate });
            // The audio pauses below: the replay starts it again.
            view.deferred_forwarded = None;
            view.awaiting = None;
            view.clock.apply(TransportCommand::Pause, t);
            view.schedule.reset();
            view.sample_due = true;
            if let Some(link) = self.audio.get(&key) {
                trace_audio_transport("pause-route-pending", key, view, None);
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
        if takes_transport(Some(&route)) {
            // Paused exact seeks while the route was undecided moved only
            // the line (a load, a composition switch, media resolving): the
            // audio a paused session already has prepares its next start
            // at the line again.
            if changed
                && !self.config.host_clock
                && let Some(view) = self.views.get(&key)
                && !view.clock.is_playing()
                && view.awaiting.is_none()
                && view.deferred_transport.is_none()
                && let Some(link) = self.audio.get(&key)
            {
                link.send(AudioCtl::Seek(view.clock.media_at(t)));
            }
            let deferred = self.views.get_mut(&key).and_then(|v| {
                v.deferred_transport
                    .take()
                    .map(|c| (c, v.deferred_forwarded.take()))
            });
            if let Some((command, forwarded)) = deferred {
                if let Some(view) = self.views.get(&key) {
                    trace_audio_transport("deferred-replay", key, view, Some(command));
                }
                // A Play the ingress forwarded before the route was known
                // already started its audio run: sending it again would
                // restart what is being heard.
                self.transport(key, command, t, forwarded);
            } else if changed && let Some(view) = self.views.get(&key) {
                trace_audio_transport("decided-without-deferred", key, view, None);
            }
        }
        let status = self.audio.get(&key).map(AudioLink::status);
        if let Some(view) = self.views.get_mut(&key) {
            // The renderer's status bar and performance panel (A5).
            view.costs.audio(status.as_ref());
        }
        if let Some(status) = status {
            self.stats().audio.insert(view_name(key), status);
        }
        route == Route::Engine
    }

    /// Tells the viewer its composition is drawn by the legacy engine
    /// (capability routing, section 21): a session-scope `unsupported`
    /// failure at the revision decided.
    ///
    /// Every route to `legacy` reports here: a static reason, a draw that
    /// failed (`force_legacy` from `RenderError::Unsupported`), and the
    /// shader fallback runner's failure (also `force_legacy`). Paused
    /// frames leave from the completion thread once the GPU finishes them,
    /// so the frames still on the GPU are sent first: no frame the engine
    /// drew before the decision reaches the viewer after its report. The
    /// report names the newest frame seq the view was given
    /// (`ErrorDetail::Unsupported`), so the viewer drops any frame at or
    /// below it as well.
    fn report_legacy(&mut self, key: Key, reasons: &[String]) {
        self.drain_frames();
        let last_frame = self
            .views
            .get(&key)
            .map_or(0, |view| view.next_seq.saturating_sub(1));
        let revision = self
            .docs
            .get(&key)
            .and_then(|d| Revision::new(d.revision()).ok())
            .unwrap_or(Revision::ZERO);
        let mut message = format!(
            "The new engine cannot draw this composition yet: {}.",
            reasons.join("; ")
        );
        if message.len() > 2_000 {
            message.truncate(1_997);
            message.push_str("...");
        }
        let error = EngineError::new(
            ErrorCode::Unsupported,
            ErrorScope::Session { session: key.1 },
            message,
        )
        .with_detail(ErrorDetail::Unsupported {
            last_frame: Seq::new(last_frame).unwrap_or(Seq::ZERO),
        });
        self.event(
            key,
            Event::Failure(FailureReport { revision, error }),
            Some(revision),
        );
    }

    fn work_view(&mut self, key: Key) {
        self.work_start = now();
        let lead = self.config.lead;
        self.retry_pool(key);
        let draws = !self.config.document || self.document_step(key);
        // A Play in its preroll starts once the first frames are answered,
        // before this pass's clock sample: the viewer must hear the run
        // before its first timed frame (a timed frame of a line it believes
        // stopped is dropped).
        if draws {
            self.finish_preroll(key);
        }
        // A headless view's clock is heard once its route is decided (drawn
        // or not: the transport runs either way).
        let takes = draws
            || self
                .views
                .get(&key)
                .is_some_and(|v| takes_transport(v.reported.as_ref()));
        // Clock samples.
        if let Some(view) = self.views.get_mut(&key) {
            let t = now();
            let periodic = view.clock.is_playing()
                && view
                    .last_sample
                    .is_none_or(|last| t - last >= SAMPLE_INTERVAL_US);
            if (view.sample_due || periodic) && (!view.state.headless.is_some_and(|h| h) || takes) {
                view.sample_due = false;
                view.last_sample = Some(t);
                let sample = view.clock.sample(t);
                // A headless clock carries the document revision only while
                // the engine draws the composition: the renderer has no
                // frame to learn the route from, so a tagged clock after an
                // `unsupported` report is how it hears the composition is
                // drawable again (an edit, a composition switch).
                let revision = view
                    .state
                    .headless
                    .filter(|h| *h && draws)
                    .and_then(|_| self.docs.get(&key))
                    .and_then(|doc| Revision::new(doc.revision()).ok());
                self.event(key, Event::Clock(sample), revision);
            }
        }
        if !draws {
            if let Some(Route::Legacy(reasons)) =
                self.views.get(&key).and_then(|view| view.reported.as_ref())
            {
                let reason = format!(
                    "The native viewer cannot render this composition: {}.",
                    reasons.join("; ")
                );
                if let Some(control) = self
                    .preview_controls
                    .current(key)
                    .filter(|c| !c.cache_only)
                    .cloned()
                    && let Some(status) = self.preview_controls.failed(
                        key,
                        control.revision,
                        Some(control.generation),
                        reason,
                    )
                {
                    self.preview_status(key, status);
                }
            }
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
                    "The new engine cannot draw this composition yet.",
                );
            }
            return;
        }
        // Headless sessions follow the device and publish clock/levels and
        // stats, but allocate and render no presentation frame. Offscreen
        // callers use snapshots; a frame request needs an attached viewer.
        if self
            .views
            .get(&key)
            .is_some_and(|v| v.state.headless == Some(true))
        {
            while let Some((id, _, _)) = self
                .views
                .get_mut(&key)
                .and_then(|v| v.requests.pop_front())
            {
                self.error(
                    key,
                    id,
                    ErrorCode::Unsupported,
                    "Attach a viewer before requesting a presentation frame.",
                );
            }
            if let Some(view) = self.views.get_mut(&key) {
                view.dirty = false;
            }
            self.send_stats(key);
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
        // A newer edit, view, or transport discontinuity must never wait
        // for an obsolete bucket. Otherwise the raster's completion wakes
        // this view; no polling render, and no blocked audio/other views.
        let current = self.docs.get(&key).map_or(0, |d| d.revision());
        if let Some(view) = self.views.get_mut(&key)
            && let Some(wait) = view.external_wait.take()
        {
            use super::wait::State;
            match wait.state(
                now(),
                current,
                view.state.view_seq.get(),
                view.clock.epoch(),
            ) {
                State::Waiting => {
                    view.external_wait = Some(wait);
                    self.send_stats(key);
                    return;
                }
                State::Obsolete => {}
                State::TimedOut => self.external_wait_timed_out(key, wait.on),
            }
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
        // A paused frame still waiting for compatibility answers asks again
        // for those a renderer missed.
        if let Some(view) = self.views.get_mut(&key)
            && view.compat_paused_wait.is_some_and(|at| at <= now())
        {
            view.compat_paused_wait = None;
            view.dirty = true;
        }
        // A paused frame that failed for the moment is drawn again on its own.
        if let Some(view) = self.views.get_mut(&key)
            && view.render_failed
            && !view.clock.is_playing()
            && let Some(retry) = view.render_retry.as_mut()
            && retry.at.is_some_and(|at| at <= now())
        {
            retry.at = None;
            view.dirty = true;
        }
        // A paused frame after a view or transport change. Not while a Play
        // waits to start (its preroll, or the audio's first sample): the run
        // presents that frame itself in a moment, and drawing it now would
        // present it twice (a seek and a Play in one task: today draws once).
        let paused_dirty = self.views.get(&key).is_some_and(|v| {
            v.dirty
                && v.pool.is_some()
                && !v.clock.is_playing()
                && v.preroll.is_none()
                && v.awaiting.is_none()
        });
        if paused_dirty {
            let media = self
                .preview_controls
                .current(key)
                .filter(|c| !c.cache_only)
                .map_or_else(|| self.views[&key].clock.media_at(now()), |c| c.time.get());
            // At most `MAX_IN_FLIGHT` frames wait on the GPU; the completion
            // thread sends this one (its descriptor is not sent here).
            let deferred = self.completer.is_some();
            if let Some(view) = self.views.get_mut(&key) {
                view.render_failed = false;
            }
            if let Some(descriptor) = self.render(key, media, -1, None, deferred, true) {
                let seq = descriptor.seq;
                self.queue_auto_preview(key, seq);
                if let Some(view) = self.views.get_mut(&key) {
                    view.dirty = false;
                    view.render_retry = None;
                }
                if !deferred {
                    let revision = descriptor.revision;
                    self.event(key, Event::Frame(descriptor), Some(revision));
                    self.preview_frame_sent(key, seq);
                }
            } else if let Some(view) = self.views.get_mut(&key)
                && view.render_failed
            {
                // Waits for the next change (which sets `dirty` again).
                view.dirty = false;
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
            // Before its pool arrives a view draws nothing, but its audio
            // status still reports (below).
            if view.pool.is_none() {
                break;
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
                        self.trace_event("pool wait", t, 0, k);
                        break;
                    }
                    // Its answers are on their way and it can still be shown:
                    // it waits (an answer arriving wakes the thread).
                    if view.compat_blocked == Some(k) {
                        self.trace_event("compat wait", t, 0, k);
                        break;
                    }
                    let f = view.clock.frame_ticks();
                    let media = view.clock.wrap(k * f);
                    let shown_for = super::schedule::Schedule::frame_us(&view.clock);
                    if view.schedule.redoing(k) {
                        self.stats().rerendered += 1;
                    }
                    let started = now();
                    // Timed frames are sent in order after any frame still
                    // finishing on the GPU.
                    self.drain_frames();
                    let rendered =
                        self.render(key, media, k, Some((shown_from, shown_for)), false, false);
                    self.trace_event(
                        if rendered.is_some() {
                            "timed frame"
                        } else {
                            "timed frame not drawn"
                        },
                        started,
                        now() - started,
                        k,
                    );
                    let Some(view) = self.views.get_mut(&key) else {
                        return;
                    };
                    // Pending external pixels have not rendered this k yet.
                    // Keep its slot in the render-ahead schedule so the exact
                    // frame follows when the raster arrives (never skip it
                    // merely because its asynchronous request was sent).
                    if rendered.is_none() && view.external_wait.is_some() {
                        break;
                    }
                    // Waiting for the window's compatibility answers keeps
                    // this frame's slot too, rather than skipping a frame
                    // that could still be shown when they arrive.
                    if rendered.is_none() && view.compat_blocked == Some(k) {
                        return;
                    }
                    view.compat_blocked = None;
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
                Next::Skip { k } => {
                    if let Some(view) = self.views.get_mut(&key) {
                        view.costs.late();
                    }
                    self.stats().late_skipped += 1;
                    self.trace_event("skip late", t, 0, k);
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
        self.drain_frames();
        if let Some(descriptor) = self.render(key, media, -1, None, false, false) {
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
        deferred: bool,
        viewer_frame: bool,
    ) -> Option<FrameDescriptor> {
        let document = self.config.document;
        let explicit_control = self
            .preview_controls
            .current(key)
            .filter(|c| !c.cache_only)
            .cloned();
        let explicit_generation = explicit_control.as_ref().map(|c| c.generation);
        let gpu = self.gpu.as_mut()?;
        let view = self.views.get_mut(&key)?;
        let live = view.pool.as_mut()?;
        let index = live.ledger.free()?;
        let size = live.spec.size;
        let working = live
            .working
            .get_or_insert_with(|| WorkingTexture::new(&gpu.device, size));
        let seq = view.next_seq;
        let start = now();
        let overlay;
        let mut depth_surface: Option<(u32, u32)> = None;
        let mut scene3d_meta = Vec::new();
        let mut chrome_legend = Vec::new();
        let mut chrome_glyphs = Vec::new();
        let mut chrome_selection = None;
        let mut cpu_us = 0i64;
        let mut failures_n = 0usize;
        let mut stand_in = false;
        let mut stand_in_shown = None;
        let mut preview_candidate: Option<PreviewCandidate> = None;
        let mut preview_failure: Option<String> = None;
        // The frame's work on this thread (docs/engine/perfbench.md).
        let work = pm_perf::Scope::begin();
        if document {
            let doc = self.docs.get_mut(&key)?;
            let composer = gpu.composer_with(&self.config, self.frame_cache, &mut self.prepared);
            let composition = view.state.composition.as_str().to_owned();
            // The renderer draws with this view's definitions (one renderer
            // serves every view on the device).
            let registered = doc.take_registered();
            let _ = composer.use_definitions(key, doc.definitions(), &registered);
            // A preceding capture's hide-captions option is scoped to it.
            composer.renderer().set_captions_hidden(false);
            // Text measures with the font cache seeds of the window the view
            // belongs to (each window is its own renderer).
            composer
                .renderer()
                .set_text_seed_scope(pm_render::text::served::SeedScope {
                    port: Some(key.0.0),
                    session: Some(key.1.get()),
                });
            composer.set_playing(timing.is_some());
            // Paused in Material Preview or Rendered: Blender surfaces
            // (`super::blender`); playback and draft shadings draw natively.
            let paused = timing.is_none() && !view.clock.is_playing();
            let render_state = view.state.clone();
            let shading = super::blender::blender_shading(render_state.view.as_ref());
            let preview = match (&self.blender, paused, shading, render_state.view.as_ref()) {
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
            // viewer asks for it: the pair owned by this color slot.
            let depth_slot = view.depth.as_ref().and_then(|d| {
                // Color slot ownership also owns its depth and coverage pair.
                // Sequence modulo can overwrite an older held slot when the
                // viewer releases color frames out of order.
                let index = (index as usize) * 2;
                d.pool.surfaces.get(index).map(|s| {
                    (
                        index as u32,
                        d.spec.generation,
                        s.texture.create_view(&Default::default()),
                        d.pool.surfaces[index + 1]
                            .texture
                            .create_view(&Default::default()),
                    )
                })
            });
            // Layers the session's compatibility producers draw (the
            // `raster` override; `super::compat`).
            let doc_revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
            let consulted = Arc::new(Mutex::new(Vec::new()));
            let compat_view = compat_view(&self.compat, key, doc_revision, Some(consulted.clone()));
            composer.renderer().set_compat_rasters(compat_view);
            let png_resolver = self.blender.as_ref().map(|blender| {
                crate::jobs::blender_png::HostBlenderPng::new(
                    Arc::new(doc.media().clone()),
                    blender.host_link(),
                    key.1,
                    Revision::new(doc.revision()).unwrap_or(Revision::ZERO),
                    doc.document(),
                )
            });
            let assets: &dyn pm_render::AssetResolver = png_resolver
                .as_ref()
                .map_or(doc.media() as &dyn pm_render::AssetResolver, |resolver| {
                    resolver
                });
            let composed = composer.compose_with_depth(
                doc.document(),
                &composition,
                assets,
                Ticks::new(media.max(0)).unwrap_or(Ticks::ZERO),
                &render_state,
                working,
                depth_slot
                    .as_ref()
                    .map(|(_, _, depth, coverage)| (depth, coverage)),
            );
            composer.renderer().set_compat_rasters(None);
            // What the next frames will ask the producers for (the preroll).
            let consulted =
                std::mem::take(&mut *consulted.lock().unwrap_or_else(|e| e.into_inner()));
            view.compat_needs = Some((media.max(0), consulted));
            depth_surface = depth_slot.map(|(index, generation, _, _)| (index, generation));
            composer.set_blender(pm_render::blender::BlenderRender::Native);
            preview_failure = preview.as_ref().and_then(|host| host.failure());
            if viewer_frame
                && paused
                && composed.is_ok()
                && let Some(shading) = shading
            {
                preview_candidate = Some(PreviewCandidate {
                    generation: explicit_generation,
                    auto_generation: None,
                    revision: Revision::new(doc.revision()).unwrap_or(Revision::ZERO),
                    composition: composition.clone(),
                    time: Ticks::new(media.max(0)).unwrap_or(Ticks::ZERO),
                    shading,
                    complete: preview
                        .as_ref()
                        .is_some_and(|host| host.pending_count() == 0)
                        && preview_failure.is_none(),
                    failure: preview_failure.clone(),
                });
            }
            // What the paused frame lacked (nothing in a draft shading); a
            // frame that failed to render changes nothing.
            if paused
                && composed.is_ok()
                && let Some(blender) = &self.blender
            {
                let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
                blender.submit(
                    key,
                    shading,
                    revision,
                    preview.as_deref(),
                    explicit_generation,
                );
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
                    scene3d_meta = composed.scene3d_meta;
                    chrome_legend = composed.chrome_legend;
                    chrome_glyphs = composed.chrome_glyphs;
                    chrome_selection = composed.chrome_selection;
                    cpu_us = composed.cpu_us;
                    failures_n = composed.failures.len();
                    stand_in = !composed.stand_ins.is_empty();
                    view.stand_in = stand_in.then_some(landed_before);
                    stand_in_shown = composed.stand_ins.first().copied();
                    // Level 1 SVG buckets this frame drew without: only
                    // those the window could not draw (or did not answer
                    // for in time) reach here; the viewer waits for every
                    // other (`SvgRastersPending`). They are asked for again
                    // after a while, and the view draws again when one
                    // arrives. Not a stand-in: no other bucket is shown.
                    view.svg_without = composed.svg_misses.clone();
                    if !composed.svg_misses.is_empty() {
                        self.stats
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .svg_failed_frames += 1;
                        if Self::ask_svg(
                            &mut self.svg,
                            &self.sink,
                            composer.renderer(),
                            key,
                            Some(view.revision),
                            &composed.svg_misses,
                        ) {
                            view.dirty = true;
                        }
                    }
                    let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
                    stats.document_frames += 1;
                    stats.work_last = work.finish();
                    let last = stats.work_last;
                    stats.work.accumulate(&last);
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
                Err(pm_render::RenderError::CompatPending(needs)) => {
                    // Layers a compatibility producer draws (the `raster`
                    // override): ask the view's renderer, and render the
                    // frame again when every answer arrived.
                    let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
                    if timing.is_some() {
                        view.compat_blocked = Some(k);
                    } else if viewer_frame {
                        // Drawn again when an answer arrives, not on every
                        // turn until then.
                        view.dirty = false;
                        view.compat_paused_wait =
                            Some(now() + super::compat::ASK_AGAIN.as_micros() as i64);
                    }
                    let fresh =
                        self.compat_store()
                            .to_ask(key, revision, needs, std::time::Instant::now());
                    for chunk in fresh.chunks(pm_protocol::snapshot::MAX_COMPAT_NEEDS) {
                        let envelope = super::compat::needs_event(key.1, revision, chunk.to_vec());
                        if let Ok(bytes) = codec::encode(&envelope) {
                            (self.sink)(vec![Action::Send { port: key.0, bytes }]);
                        }
                    }
                    self.stats().compat_waits += 1;
                    return None;
                }
                Err(pm_render::RenderError::SvgRastersPending(needs)) => {
                    // Buckets this thread already knows failed but this
                    // renderer does not (a renderer made again after a
                    // device loss): drawn without at once.
                    let renderer = composer.renderer();
                    let mut waiting = Vec::new();
                    for need in &needs {
                        let k = &need.request.key;
                        if renderer.svg_raster_failed(k) {
                            continue;
                        }
                        if self.svg.has_failed(k) {
                            renderer.fail_svg_raster(k.clone());
                        } else if !waiting.contains(k) {
                            waiting.push(k.clone());
                        }
                    }
                    let arrived = Self::ask_svg(
                        &mut self.svg,
                        &self.sink,
                        renderer,
                        key,
                        Revision::new(doc.revision()).ok(),
                        &needs,
                    );
                    let bound = super::wait::Bounds::new(self.config.external_wait_cap_us).svg;
                    let t = now();
                    // Nothing left to wait for (or it arrived meanwhile):
                    // the frame is drawn again at once, keeping its slot.
                    let until = if waiting.is_empty() || arrived {
                        view.dirty = true;
                        waiting.clear();
                        t
                    } else {
                        super::wait::deadline(
                            waiting.iter().map(|k| self.svg.asked_at(k)),
                            t,
                            bound,
                        )
                    };
                    view.external_wait = Some(super::wait::ExternalWait {
                        until,
                        revision: doc.revision(),
                        seq: view.state.view_seq.get(),
                        epoch: view.clock.epoch(),
                        on: super::wait::Awaited::Svg(waiting),
                    });
                    return None;
                }
                Err(pm_render::RenderError::ShaderFallbackPending(requests)) => {
                    // Layers only the fallback runner draws (decision 19):
                    // ask the view's renderer, and render the frame again
                    // when the pixels arrive.
                    let t = now();
                    let waiting: Vec<String> = requests.iter().map(|r| r.key.clone()).collect();
                    let fresh: Vec<_> = requests
                        .into_iter()
                        .filter(|r| !self.fallback.asked.contains_key(&r.key))
                        .collect();
                    for chunk in fresh.chunks(pm_protocol::shader_fallback::MAX_REQUESTS) {
                        for r in chunk {
                            self.fallback.asked.insert(r.key.clone(), (key, t));
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
                        // The textures an effect or transition samples, in
                        // row bands after its request.
                        for r in chunk.iter().filter(|r| r.textures > 0) {
                            let textures = composer
                                .renderer()
                                .fallback_textures(&r.key)
                                .unwrap_or_default();
                            for (texture, pixels) in textures.iter().enumerate() {
                                for (row, rows, bytes) in pm_render::shader_layers::fallback::bands(
                                    pixels,
                                    pm_protocol::shader_fallback::MAX_BAND_BYTES,
                                ) {
                                    let (rgba16f, rgba32f) = if pixels.rgba32f.is_some() {
                                        (
                                            pm_protocol::common::Bytes(Vec::new()),
                                            Some(pm_protocol::common::Bytes(bytes)),
                                        )
                                    } else {
                                        (pm_protocol::common::Bytes(bytes), None)
                                    };
                                    let body = Event::ShaderFallback(
                                        pm_protocol::shader_fallback::ShaderFallbackMessage::Input(
                                            pm_protocol::shader_fallback::ShaderFallbackInput {
                                                key: r.key.clone(),
                                                texture: texture as u32,
                                                width: pixels.width,
                                                height: pixels.height,
                                                row,
                                                rows,
                                                rgba16f,
                                                rgba32f,
                                            },
                                        ),
                                    );
                                    let envelope =
                                        Envelope::new(Message::Event { body }).with_session(key.1);
                                    if let Ok(bytes) = codec::encode(&envelope) {
                                        (self.sink)(vec![Action::Send { port: key.0, bytes }]);
                                    }
                                }
                            }
                        }
                    }
                    let bound = super::wait::Bounds::new(self.config.external_wait_cap_us).fallback;
                    // Counted from this view's own asks; a request another
                    // job asked for (a snapshot) gets the full bound from
                    // now, as only the view's own asks time out here.
                    let asked = waiting.iter().map(|k| {
                        self.fallback
                            .asked
                            .get(k)
                            .filter(|a| a.0 == key)
                            .map(|a| a.1)
                    });
                    view.external_wait = Some(super::wait::ExternalWait {
                        until: super::wait::deadline(asked, t, bound),
                        revision: doc.revision(),
                        seq: view.state.view_seq.get(),
                        epoch: view.clock.epoch(),
                        on: super::wait::Awaited::Fallback(waiting),
                    });
                    return None;
                }
                Err(pm_render::RenderError::WatchdogPending) => {
                    // A frame drawing one of these programs was seen running
                    // past the hang budget: nothing is submitted with them
                    // until the watchdog judged it (`work`).
                    view.watchdog_held = true;
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
                    view.render_failed = true;
                    view.render_retry = transient_render_error(&error).then(|| {
                        RenderRetry::after(
                            view.render_retry,
                            (doc.revision(), view.state.view_seq.get()),
                            now(),
                        )
                    });
                    if preview.is_some() {
                        let why = preview_failure.take().unwrap_or_else(|| {
                            format!("The Blender preview could not be prepared: {error}")
                        });
                        let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
                        self.fail_preview_without_frame(
                            key,
                            revision,
                            explicit_generation,
                            shading,
                            &composition,
                            Ticks::new(media.max(0)).unwrap_or(Ticks::ZERO),
                            viewer_frame,
                            why,
                        );
                    }
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
                .encode(&gpu.queue, &mut encoder, working, &frame);
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
            working,
            surface,
            params,
        ) {
            let message = format!("frame bridge: display transform failed: {error}");
            (self.sink)(vec![Action::Log {
                level: LogLevel::Error,
                message: message.clone(),
            }]);
            self.fail_preview_delivery(key, preview_candidate.as_ref(), message);
            return None;
        }
        let submission = gpu.queue.submit([encoder.finish()]);
        let seq_value = match Seq::new(seq) {
            Ok(value) => value,
            Err(_) => {
                self.fail_preview_delivery(
                    key,
                    preview_candidate.as_ref(),
                    "The native viewer could not assign a frame sequence to the preview.".into(),
                );
                return None;
            }
        };
        let duration = timing.map(|_| Ticks::new(view.clock.frame_ticks()).unwrap_or(Ticks::MAX));
        let mut meta = super::complete::FrameMeta {
            seq,
            k,
            index,
            generation: live.spec.generation,
            timing,
            view_seq: view.state.view_seq.get(),
            revision: view.revision.get(),
            start,
            work_start: self.work_start,
            cpu_us,
            overlay_layers: overlay.layers.len(),
            failures: failures_n,
            stand_in_shown,
            depth: depth_surface.is_some(),
            held: 0,
        };
        let descriptor = FrameDescriptor {
            scene3d_meta,
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
            depth_id: depth_surface.map(|(index, generation)| {
                Box::new(pm_protocol::frame::DepthIdSurface {
                    encoding: Some("chromePlanesV1".into()),
                    coverage_pool_index: Some(index + 1),
                    legend: chrome_legend,
                    glyphs: chrome_glyphs,
                    selection: chrome_selection,
                    pool_index: index,
                    seq: seq_value,
                    pool_generation: Seq::new(u64::from(generation)).unwrap_or(Seq::MAX),
                })
            }),
            view_seq: view.state.view_seq,
            duration,
        };
        // A frame shown at the next refresh leaves now; the completion
        // thread sends it once the GPU is done (`super::complete`).
        if let (true, Some(completer)) = (deferred, self.completer.as_ref()) {
            let mut envelope = Envelope::new(Message::Event {
                body: Event::Frame(descriptor.clone()),
            })
            .with_session(key.1);
            envelope.revision = Some(descriptor.revision);
            let bytes = match codec::encode(&envelope) {
                Ok(bytes) => bytes,
                Err(error) => {
                    (self.sink)(vec![Action::Log {
                        level: LogLevel::Error,
                        message: format!("frame bridge: could not encode a message: {error}"),
                    }]);
                    self.fail_preview_delivery(
                        key,
                        preview_candidate.as_ref(),
                        format!("The native viewer could not encode the preview frame: {error}"),
                    );
                    return None;
                }
            };
            if !live.ledger.present(index, seq_value) {
                self.fail_preview_delivery(
                    key,
                    preview_candidate.as_ref(),
                    "The native viewer could not reserve the preview frame surface.".into(),
                );
                return None;
            }
            view.next_seq += 1;
            if let Some(mut candidate) = preview_candidate.take() {
                candidate.revision = descriptor.revision;
                self.preview_candidates.insert((key, seq), candidate);
            }
            meta.held = live.ledger.held_count() as u32;
            if timing.is_none() {
                self.last_interactive = Some(now());
            }
            completer.submit(super::complete::Pending {
                key,
                device: gpu.device.clone(),
                submission,
                bytes,
                meta,
            });
            self.fail_preview_snapshot(
                key,
                descriptor.revision,
                explicit_generation,
                preview_failure,
            );
            return Some(descriptor);
        }
        let done = gpu.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(2)),
        });
        let gpu_done = now();
        if let Err(error) = done {
            let message = format!("frame bridge: the GPU did not finish a frame: {error}");
            (self.sink)(vec![Action::Log {
                level: LogLevel::Error,
                message: message.clone(),
            }]);
            self.fail_preview_delivery(key, preview_candidate.as_ref(), message);
            return None;
        }
        if !live.ledger.present(index, seq_value) {
            self.fail_preview_delivery(
                key,
                preview_candidate.as_ref(),
                "The native viewer could not reserve the preview frame surface.".into(),
            );
            return None;
        }
        view.next_seq += 1;
        if let Some(mut candidate) = preview_candidate {
            candidate.revision = descriptor.revision;
            self.preview_candidates.insert((key, seq), candidate);
        }
        meta.held = live.ledger.held_count() as u32;
        self.record_frame(key, &meta, gpu_done, now());
        self.fail_preview_snapshot(
            key,
            descriptor.revision,
            explicit_generation,
            preview_failure,
        );
        Some(descriptor)
    }

    /// The stats, the session's costs, and the per-frame trace of a frame
    /// sent at `sent` whose GPU work finished at `gpu_done` (host µs).
    fn record_frame(
        &mut self,
        key: Key,
        meta: &super::complete::FrameMeta,
        gpu_done: i64,
        sent: i64,
    ) {
        let trace = self.config.trace;
        let elapsed = gpu_done - meta.start;
        if let Some(view) = self.views.get_mut(&key) {
            view.costs.frame(
                meta.timing.is_some(),
                meta.cpu_us,
                elapsed - meta.cpu_us.min(elapsed),
                meta.overlay_layers,
                meta.failures,
                meta.revision,
            );
        }
        if meta.timing.is_none() {
            self.last_interactive = Some(gpu_done);
        }
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        if meta.depth {
            stats.depth_frames += 1;
        }
        stats.frames += 1;
        if meta.timing.is_some() {
            stats.timed_frames += 1;
        } else {
            stats.immediate_frames += 1;
        }
        if let Some(shown) = meta.stand_in_shown
            && self.config.stand_in_log
            && stats.stand_in_log.len() < STAND_IN_LOG_CAP
        {
            stats.stand_in_log.push((meta.seq, shown));
        }
        stats.gpu_us_max = stats.gpu_us_max.max(elapsed);
        stats.gpu_us_total += elapsed;
        stats.max_held = stats.max_held.max(meta.held);
        if trace && stats.trace.len() < TRACE_CAP {
            let (shown_from, shown_for) = meta.timing.unwrap_or((0.0, 0.0));
            let shown = i64::try_from(meta.revision).unwrap_or(i64::MAX);
            stats.trace.push(FrameTrace {
                seq: meta.seq,
                k: meta.k,
                slot: meta.index,
                generation: meta.generation,
                shown_from: shown_from.round() as i64,
                shown_for: shown_for.round() as i64,
                view_seq: meta.view_seq,
                revision: meta.revision,
                start: meta.start,
                gpu_done,
                sent,
                timed: meta.timing.is_some(),
                work_start: meta.work_start,
                cpu_us: meta.cpu_us,
                revisions: self
                    .doc_trace
                    .get_mut(&key)
                    .map(|pending| {
                        let split = pending.partition_point(|r| r[0] <= shown);
                        pending.drain(..split).collect()
                    })
                    .unwrap_or_default(),
            });
        }
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
        if let Some(job) = self.cache_previews.remove(&token) {
            let current = self.preview_controls.current(job.key);
            if !current.is_some_and(|c| {
                c.cache_only && c.revision == job.revision && c.generation == job.generation
            }) {
                return;
            }
            let revision = Revision::new(self.docs.get(&job.key).map_or(0, ViewDocument::revision))
                .unwrap_or(Revision::ZERO);
            if revision != job.revision {
                if let Some(status) = self.preview_controls.cancelled(job.key) {
                    self.preview_status(job.key, status);
                }
                return;
            }
            let status = match result {
                Ok(_) => self
                    .preview_controls
                    .ready_cache(job.key, revision, job.generation),
                Err(error) => self.preview_controls.failed(
                    job.key,
                    revision,
                    Some(job.generation),
                    format!("The Blender preview could not be prepared: {error}"),
                ),
            };
            if let Some(status) = status {
                self.preview_status(job.key, status);
            }
            return;
        }
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

    /// Prepare Blender's interactive cache from the document mirror even if
    /// no viewer surface exists. The transport clock and viewport are untouched.
    fn work_cache_previews(&mut self) {
        let ready: Vec<(Key, PreviewControl)> = self
            .cache_requested
            .iter()
            .filter(|(key, control)| {
                self.docs.get(key).is_some_and(|doc| {
                    doc.has_document() && doc.revision() >= control.revision.get()
                })
            })
            .map(|(key, control)| (*key, control.clone()))
            .collect();
        for (key, control) in ready {
            self.cache_requested.remove(&key);
            if self
                .preview_controls
                .current(key)
                .is_none_or(|c| c.generation != control.generation)
            {
                continue;
            }
            let Some(blender) = self.blender.clone() else {
                if let Some(status) = self.preview_controls.failed(
                    key,
                    control.revision,
                    Some(control.generation),
                    "The Blender capture worker is unavailable.".into(),
                ) {
                    self.preview_status(key, status);
                }
                continue;
            };
            if let Some(status) =
                self.preview_controls
                    .running(key, control.revision, Some(control.generation))
            {
                self.preview_status(key, status);
            }
            let result = self.cache_preview_snapshots(key, &control, &blender);
            match result {
                Ok(snapshots) if snapshots.is_empty() || blender.prepared(&snapshots).is_some() => {
                    if let Some(status) =
                        self.preview_controls
                            .ready_cache(key, control.revision, control.generation)
                    {
                        self.preview_status(key, status);
                    }
                }
                Ok(snapshots) => {
                    self.blender_token += 1;
                    let token = self.blender_token;
                    self.cache_previews.insert(
                        token,
                        CachePreview {
                            key,
                            revision: control.revision,
                            generation: control.generation,
                        },
                    );
                    blender.capture(token, key, snapshots, true);
                }
                Err(error) => {
                    if let Some(status) = self.preview_controls.failed(
                        key,
                        control.revision,
                        Some(control.generation),
                        error,
                    ) {
                        self.preview_status(key, status);
                    }
                }
            }
        }
    }

    fn cache_preview_snapshots(
        &mut self,
        key: Key,
        control: &PreviewControl,
        blender: &super::blender::PreviewBlender,
    ) -> Result<Vec<pm_blender::snapshot::Snapshot>, String> {
        // Headless sessions have no pool to initialize the device for us.
        if self.gpu().is_none() {
            return Err("The engine has no GPU device.".into());
        }
        let gpu = self.gpu.as_mut().ok_or("The engine has no GPU device.")?;
        let doc = self
            .docs
            .get(&key)
            .ok_or("The engine session has no document.")?;
        if doc.revision() != control.revision.get() {
            return Err("The preview document changed before Blender could start.".into());
        }
        let composer = gpu.composer_with(&self.config, self.frame_cache, &mut self.prepared);
        let _ = composer.use_definitions(key, doc.definitions(), &[]);
        let renderer = composer.renderer();
        renderer.set_scrub(false);
        renderer.set_captions_hidden(false);
        renderer.set_text_seed_scope(pm_render::text::served::SeedScope {
            port: Some(key.0.0),
            session: Some(key.1.get()),
        });
        let request = pm_render::RenderRequest {
            output_size: control.size.ok_or("The preview has no composition size.")?,
            scale: None,
            preview_canvas: false,
            origin: None,
            region: None,
            precision: pm_protocol::color::Precision::Float16,
            depth_id: false,
            background: true,
            export: false,
            view: None,
            blender: pm_render::blender::BlenderRender::Native,
            motion_blur: pm_render::motion::MotionBlur::Off,
            sampling: pm_render::sampling::Sampling::Final,
            video_end: pm_render::VideoEnd::of(pm_protocol::frame::FramePurpose::Viewer),
        };
        let png = crate::jobs::blender_png::HostBlenderPng::new(
            Arc::new(doc.media().clone()),
            blender.host_link(),
            key.1,
            control.revision,
            doc.document(),
        );
        let camera = control
            .camera
            .map(|camera| {
                pm_render::scene3d::View3d::from_override(&ViewOverride {
                    camera: Some(camera),
                    shading: control.shading,
                    xray: false,
                    local_view: Vec::new(),
                })
            })
            .and_then(|view| view.camera);
        let snapshots = renderer
            .blender_snapshots(
                doc.document(),
                control.composition.as_str(),
                control.time,
                &request,
                &png,
                true,
                pm_render::blender::SnapshotOptions {
                    force_eevee: control.shading == Shading3d::Material,
                    camera,
                    settings: None,
                },
            )
            .map_err(|error| format!("The Blender preview could not be planned: {error}"))?;
        if let Some((frame, why)) = snapshots.errors.first() {
            return Err(format!(
                "The Blender frame of {} at {} s could not be prepared: {why}",
                frame.composition(),
                frame.time
            ));
        }
        Ok(snapshots.snapshots)
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
            // Headless transport has no presentation deadline to compete with
            // foreground captures. Idle captures still wait for playback.
            if view.state.headless == Some(true) {
                continue;
            }
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

    /// Runs ready snapshots. During playback, return to view scheduling
    /// after each job; otherwise drain sync/async and at most one idle job.
    fn work_snapshots(&mut self) {
        self.finish_pending_snapshots();
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
        self.dispatch_layer_rasters(now);
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
            let compat = &self.compat;
            let svg = &self.svg;
            let outstanding = |k: &str| {
                if k.starts_with("compat:") {
                    compat
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .outstanding(k)
                } else {
                    asked.contains_key(k) || svg.outstanding(k)
                }
            };
            let Some(job) = self.snapshots.next(revision, outstanding, pressure, now) else {
                return;
            };
            if matches!(
                &job.request.target,
                pm_protocol::snapshot::SnapshotTarget::Frame { .. }
            ) {
                // Float32 is the largest possible target format. Reserving
                // against this bound can delay a Float16 job, never let a
                // job escape the ring's memory limit after rendering.
                let width = u64::from(job.request.output_size.width);
                let height = u64::from(job.request.output_size.height);
                let estimated = (width * 16).div_ceil(256) * 256 * height;
                if self.gpu.as_ref().is_some_and(|gpu| {
                    !gpu.snapshot_readback
                        .can_reserve(estimated, MAX_CACHED_SNAPSHOT_TARGET_BYTES)
                }) {
                    self.snapshots.requeue(job);
                    return;
                }
            }
            let idle = job.request.class == pm_protocol::snapshot::SnapshotClass::Idle;
            if idle && idle_done {
                self.snapshots.requeue(job);
                return;
            }
            idle_done |= idle;
            self.run_snapshot(job);
            // Compatibility bases use sync priority to unblock a view that
            // needs them. Draining a burst of future bases at that priority
            // can instead run past the next frame's deadline. Keep their
            // admission priority, but give views and commands a turn between
            // snapshots. The queue's 4 ms wake keeps dependencies progressing
            // even while a view waits for its first compatibility answer.
            if pressure.playing {
                return;
            }
        }
    }

    /// Hands every ready layer raster to the worker while it has room
    /// ([`super::rasters`]): they do not wait for playback, and never run on
    /// this thread while the worker runs.
    fn dispatch_layer_rasters(&mut self, now: i64) {
        loop {
            if !self.rasters.as_ref().is_some_and(|r| r.has_room()) || self.gpu.is_none() {
                return;
            }
            let pressure = self.pressure();
            let docs = &self.docs;
            let revision = |key: Key| {
                docs.get(&key)
                    .filter(|d| d.has_document() && d.media_ready())
                    .map(|d| d.revision())
            };
            let asked = &self.fallback.asked;
            let compat = &self.compat;
            let svg = &self.svg;
            let outstanding = |k: &str| {
                if k.starts_with("compat:") {
                    compat
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .outstanding(k)
                } else {
                    asked.contains_key(k) || svg.outstanding(k)
                }
            };
            let Some(job) = self
                .snapshots
                .next_layer(revision, outstanding, pressure, now)
            else {
                return;
            };
            self.run_layer_snapshot(job, now);
        }
    }

    /// Hands the raster worker an update its mirror of session `key` needs
    /// (`update`: the one just applied, when it mirrors the session). A
    /// document that could not take it waits for a resync: the mirror is
    /// dropped and the next raster seeds a new one.
    fn mirror_update(&mut self, key: Key, update: Option<DocumentUpdate>) {
        let Some(update) = update else {
            return;
        };
        let Some(rasters) = &self.rasters else {
            return;
        };
        let applied = self
            .docs
            .get(&key)
            .is_some_and(|d| d.revision() == update.revision().get() && !d.is_broken());
        if applied {
            rasters.update(key, update);
        } else {
            rasters.forget(key);
            self.raster_mirrors.remove(&key);
        }
    }

    /// A layer raster the worker sent back to be seeded
    /// ([`Command::RasterReseed`]).
    fn raster_reseed(&mut self, job: super::rasters::RasterJob) {
        if let Some(rasters) = self.rasters.as_mut() {
            rasters.finished(job.job.bytes());
            self.snapshots.set_in_worker(rasters.in_flight_bytes());
        }
        self.raster_mirrors.remove(&job.job.key);
        self.stats().layer_raster_reseeds += 1;
        self.run_layer_snapshot(job.job, now());
    }

    /// The layer raster worker answered a job ([`Command::RasterDone`]).
    fn raster_done(&mut self, done: super::rasters::RasterDone) {
        if let Some(rasters) = self.rasters.as_mut() {
            rasters.finished(done.bytes);
            self.snapshots.set_in_worker(rasters.in_flight_bytes());
        }
        if let Some((name, length)) = done.published {
            self.segments.add(name.clone());
            self.snapshots.published(name, length);
        }
        let t = now();
        if let Some(render_us) = done.render_us {
            let costs = self.snapshot_costs.entry(done.key).or_default();
            costs.0 += 1;
            costs.1 = costs.1.max(render_us);
            self.trace_event("layer raster", t - render_us, render_us, done.time);
        } else {
            self.trace_event("layer raster failed", t, 0, done.time);
        }
    }

    /// Advances map callbacks without waiting for the GPU. A pending
    /// snapshot owns its staging ticket and result metadata; meanwhile the
    /// normal work turn may render and submit viewer frames.
    fn finish_pending_snapshots(&mut self) {
        if self.pending_snapshots.is_empty() {
            return;
        }
        // Responses stay in request order even if a later slot's map
        // callback runs first.
        while let Some(first) = self.pending_snapshots.first() {
            let result = self
                .gpu
                .as_mut()
                .expect("polled GPU")
                .snapshot_readback
                .try_with_mapped_rows(
                    first.ticket,
                    |data, row, stride, height| {
                        super::snapshot::encode_mapped_rgba8(
                            first.format,
                            data,
                            row,
                            stride,
                            height,
                            first.job.request.pixels,
                            first.rows,
                        )
                    },
                    MAX_CACHED_SNAPSHOT_TARGET_BYTES,
                );
            let Some(result) = result else { break };
            let pending = self.pending_snapshots.remove(0);
            match result.and_then(|result| result) {
                Ok(pixels) => self.complete_frame_snapshot(pending, pixels),
                Err(error) => {
                    self.stats().snapshot_failures += 1;
                    self.snapshot_error(
                        &pending.job,
                        ErrorCode::Internal,
                        &format!("The GPU snapshot readback failed: {error}"),
                    );
                }
            }
        }
        if self
            .pending_snapshots
            .first()
            .is_some_and(|pending| pending.map_deadline <= now())
        {
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.lost.store(true, Ordering::Release);
            }
            self.fail_pending_snapshot_maps(
                ErrorCode::Timeout,
                "The GPU snapshot readback did not complete within 2 seconds.",
            );
        }
    }

    fn fail_pending_snapshot_maps(&mut self, code: ErrorCode, message: &str) {
        let pending = std::mem::take(&mut self.pending_snapshots);
        if let Some(gpu) = self.gpu.as_mut() {
            for item in &pending {
                gpu.snapshot_readback
                    .cancel(item.ticket, MAX_CACHED_SNAPSHOT_TARGET_BYTES);
            }
        }
        for item in pending {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(&item.job, code, message);
        }
    }

    fn complete_frame_snapshot(&mut self, pending: PendingSnapshot, pixels: Vec<u8>) {
        use pm_protocol::snapshot::{SnapshotOutcome, SnapshotResult};
        let PendingSnapshot {
            job,
            started,
            revision,
            width,
            height,
            draft_3d,
            ..
        } = pending;
        let Some(payload) = self.share_pixels(&job, pixels) else {
            return;
        };
        let render_us = (now() - started).max(0);
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
            view_seq: job
                .request
                .viewer
                .as_ref()
                .map(|v| v.view_seq)
                .or(job.request.view_seq),
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

    /// Renders one snapshot and answers it.
    fn run_snapshot(&mut self, job: super::snapshot::SnapshotJob) {
        use pm_protocol::snapshot::{SnapshotClass, SnapshotOutcome, SnapshotTarget};
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
        if self.snapshot_poller.is_none() {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::Internal,
                "The GPU snapshot completion worker is unavailable.",
            );
            return;
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let gpu_lost = gpu.lost.clone();
        let gpu_device = gpu.device.clone();
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
        let needs_blender = job.request.viewer.is_none()
            && !draft_3d
            && pm_render::unsupported(doc.document(), job.request.composition.as_str())
                .iter()
                .any(|u| u.feature == pm_render::Feature::Blender);
        let viewer_grid = if let Some(viewer) = &job.request.viewer {
            let comp =
                match pm_eval::Comp::resolve(doc.document(), job.request.composition.as_str()) {
                    Ok(comp) => comp,
                    Err(_) => {
                        self.stats().snapshot_failures += 1;
                        self.snapshot_error(
                            &job,
                            ErrorCode::InvalidMessage,
                            "The viewer snapshot has no composition.",
                        );
                        return;
                    }
                };
            let number = |key: &str| {
                let v = pm_eval::node::to_number(comp.member(key));
                if v.is_finite() && v > 0.0 { v } else { 1.0 }
            };
            Some(super::compose::canvas_render_grid(
                &viewer.canvas,
                number("w"),
                number("h"),
            ))
        } else {
            None
        };
        let (composer, readback, snapshot_target) =
            gpu.composer_and_snapshot_resources(&self.config, self.frame_cache, &mut self.prepared);
        // The session's definitions, without consuming the re-registrations
        // its viewer's next frame applies.
        let _ = composer.use_definitions(job.key, doc.definitions(), &[]);
        composer.set_blender(pm_render::blender::BlenderRender::Native);
        composer.set_playing(false);
        let renderer = composer.renderer();
        renderer.set_scrub(false);
        renderer.set_captions_hidden(job.request.captions_hidden);
        renderer.set_text_seed_scope(pm_render::text::served::SeedScope {
            port: Some(job.key.0.0),
            session: Some(job.key.1.get()),
        });
        let purpose = match job.request.class {
            SnapshotClass::Idle => pm_protocol::frame::FramePurpose::Thumbnail,
            SnapshotClass::Sync | SnapshotClass::Async => pm_protocol::frame::FramePurpose::Export,
        };
        let mut request = pm_render::RenderRequest {
            output_size: job.request.output_size,
            // Today's capture derives the scale from the composition size
            // (`W / comp.w`, `H / comp.h`).
            scale: viewer_grid.and_then(|grid| grid.0),
            region: None,
            // A pinned canvas is the live viewer's canvas: the same
            // compatibility clip window and canvas density rules.
            preview_canvas: job.request.viewer.is_some(),
            precision: pm_protocol::color::Precision::Float16,
            depth_id: false,
            background: if job.request.viewer.is_some() {
                true
            } else {
                background
            },
            export: job.request.viewer.is_none()
                && purpose == pm_protocol::frame::FramePurpose::Export,
            view: job.request.viewer.as_ref().and_then(|v| v.view.clone()),
            blender: job
                .request
                .viewer
                .as_ref()
                .and_then(|v| {
                    let view = v.view.as_ref()?;
                    super::blender::blender_shading(Some(view))?;
                    Some(pm_render::blender::BlenderRender::Preview(
                        self.blender.as_ref()?.host(view),
                    ))
                })
                .unwrap_or(pm_render::blender::BlenderRender::Native),
            motion_blur: if job.request.viewer.is_some() {
                pm_render::motion::MotionBlur::Kind(
                    pm_protocol::frame::MotionBlurKind::PreviewPaused,
                )
            } else if motion_blur == 0 {
                pm_render::motion::MotionBlur::Off
            } else {
                pm_render::motion::MotionBlur::Samples(motion_blur)
            },
            sampling: pm_render::sampling::Sampling::Final,
            video_end: if job.request.viewer.is_some() {
                pm_render::VideoEnd::Preview
            } else {
                pm_render::VideoEnd::of(purpose)
            },
            origin: viewer_grid.and_then(|grid| grid.1),
        };
        if needs_blender {
            match blender_frames(renderer, doc, &job, &request, self.blender.as_deref()) {
                BlenderFrames::Draw(surfaces) => {
                    request.blender = pm_render::blender::BlenderRender::Final(surfaces);
                }
                BlenderFrames::Prepare(snapshots) => {
                    let Some(blender) = self.blender.clone() else {
                        self.stats().snapshot_failures += 1;
                        self.snapshot_error(
                            &job,
                            ErrorCode::BlenderFailed,
                            "The Blender capture worker is unavailable.",
                        );
                        return;
                    };
                    self.blender_token += 1;
                    let token = self.blender_token;
                    let mut job = job;
                    job.start_blender(token, now());
                    if job.expires_at() <= now() {
                        self.stats().snapshot_failures += 1;
                        self.snapshot_error(
                            &job,
                            ErrorCode::Timeout,
                            "The Blender capture exceeded its time limit.",
                        );
                        return;
                    }
                    blender.capture(token, job.key, snapshots, false);
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
        // The session's compatibility producers (the `raster` override).
        renderer.set_compat_rasters(compat_view(&self.compat, job.key, revision, None));
        let size = request.target_size();
        let format = renderer.device_target_format(doc.document(), &request);
        let target = snapshot_output_target(
            snapshot_target,
            renderer.device(),
            size.width,
            size.height,
            format,
        );
        let (ticket, copy) = match readback.reserve(
            renderer.device(),
            &target.texture,
            MAX_CACHED_SNAPSHOT_TARGET_BYTES,
        ) {
            Ok(Some(reserved)) => reserved,
            Ok(None) => {
                self.snapshots.requeue(job);
                return;
            }
            Err(error) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    &job,
                    ErrorCode::Internal,
                    &format!("The snapshot could not reserve a GPU readback: {error}"),
                );
                return;
            }
        };
        // A snapshot never draws an SVG bucket's stand-in: it waits for the
        // renderer's raster (decision S1). The renderer is the viewer's
        // too: the guard gives the viewer its policy back however this
        // returns (a viewer left waiting fails its misses instead of asking
        // the window for them).
        let rendered = {
            let mut renderer = renderer.svg_waiting();
            let rendered = if job.request.viewer.is_some() {
                renderer.render_viewer_snapshot_into_copy(
                    doc.document(),
                    job.request.composition.as_str(),
                    job.request.time,
                    &request,
                    doc.media(),
                    &target,
                    &copy,
                )
            } else {
                renderer.render_snapshot_into_copy(
                    doc.document(),
                    job.request.composition.as_str(),
                    job.request.time,
                    &request,
                    doc.media(),
                    &target,
                    &copy,
                )
            };
            rendered.map(|(_, rows)| (target.width(), target.height(), rows))
        };
        renderer.set_compat_rasters(None);
        if let Some(why) = self.config.fonts.as_ref().and_then(|f| f.after_render()) {
            readback.cancel(ticket, MAX_CACHED_SNAPSHOT_TARGET_BYTES);
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::Timeout,
                &format!("The engine's fonts did not answer for this frame ({why})."),
            );
            return;
        }
        if rendered.is_err() {
            readback.cancel(ticket, MAX_CACHED_SNAPSHOT_TARGET_BYTES);
        }
        let (width, height, rows) = match rendered {
            Ok(done) => done,
            Err(pm_render::RenderError::CompatPending(needs)) => {
                // The renderer draws them on its blocked thread and posts
                // them on the session's port, then asks again: needs it
                // was just sent are on their way (the two ports race), so
                // the job waits for them; the rest are named.
                let tags: Vec<String> = needs
                    .iter()
                    .map(|n| {
                        super::compat::CompatStore::snapshot_tag(
                            job.key,
                            n,
                            revision,
                            job.request.class,
                        )
                    })
                    .collect();
                let fresh = self.compat_store().snapshot_needs(
                    job.key,
                    revision,
                    needs,
                    &job.request.compat_uploaded,
                    std::time::Instant::now(),
                    job.request.class,
                );
                if fresh.is_empty() {
                    let mut job = job;
                    job.waiting_for = tags;
                    self.snapshots.requeue(job);
                    return;
                }
                self.stats().snapshot_needs += 1;
                let outcome = SnapshotOutcome::NeedsRenderer {
                    revision,
                    fallback: Vec::new(),
                    compat: fresh
                        .into_iter()
                        .take(pm_protocol::snapshot::MAX_COMPAT_NEEDS)
                        .collect(),
                    svg: Vec::new(),
                };
                let mut envelope = Envelope::new(Message::Response {
                    id: job.id,
                    body: Response::Snapshot(Box::new(outcome)),
                });
                envelope.revision = Some(revision);
                self.send(job.host, envelope);
                return;
            }
            Err(pm_render::RenderError::SvgRastersPending(needs)) => {
                // As for fallback pixels: what the renderer posted is on
                // its way (wait for it); the rest is named for it to draw.
                let (posted, missing): (Vec<_>, Vec<_>) = needs
                    .into_iter()
                    .partition(|n| job.request.uploaded.contains(&n.request.key.upload_key()));
                if let Some(failed) = posted.iter().find(|n| self.svg.has_failed(&n.request.key)) {
                    let k = &failed.request.key;
                    self.stats().snapshot_failures += 1;
                    self.snapshot_error(
                        &job,
                        ErrorCode::Internal,
                        &format!(
                            "The window could not draw an SVG image's {}x{} raster for this frame.",
                            k.width, k.height
                        ),
                    );
                    return;
                }
                if missing.is_empty() {
                    let mut job = job;
                    job.waiting_for = posted.iter().map(|n| n.request.key.upload_key()).collect();
                    for n in &posted {
                        self.svg.expect(job.key, &n.request.key, now());
                    }
                    self.snapshots.requeue(job);
                    return;
                }
                self.stats().snapshot_needs += 1;
                let outcome = SnapshotOutcome::NeedsRenderer {
                    revision,
                    fallback: Vec::new(),
                    compat: Vec::new(),
                    svg: missing
                        .into_iter()
                        .take(pm_protocol::svg_raster::MAX_SVG_REQUESTS)
                        .map(|n| n.request)
                        .collect(),
                };
                let mut envelope = Envelope::new(Message::Response {
                    id: job.id,
                    body: Response::Snapshot(Box::new(outcome)),
                });
                envelope.revision = Some(revision);
                self.send(job.host, envelope);
                return;
            }
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
                        self.fallback
                            .asked
                            .entry(key.clone())
                            .or_insert((job.key, now()));
                    }
                    self.snapshots.requeue(job);
                    return;
                }
                let fallback: Vec<_> = missing
                    .into_iter()
                    .take(pm_protocol::shader_fallback::MAX_REQUESTS)
                    .collect();
                // The textures an effect or a transition samples go to main
                // ahead of the answer, on the same port so they arrive
                // first, in row bands: the renderer may be the thread
                // blocked on this snapshot and cannot read its own port, so
                // main hands them back with the answer
                // (`main/engine/fallback-inputs.ts`).
                let inputs = fallback_inputs(renderer, &fallback, job.key.1);
                for r in &fallback {
                    self.fallback.asked.insert(r.key.clone(), (job.key, now()));
                }
                for input in inputs {
                    self.send(job.host, input);
                }
                self.stats().snapshot_needs += 1;
                let outcome = SnapshotOutcome::NeedsRenderer {
                    revision,
                    fallback,
                    compat: Vec::new(),
                    svg: Vec::new(),
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
        let back = self.snapshot_back.clone();
        readback.start_map(ticket, move || {
            let _ = back.send(Command::SnapshotMapReady);
        });
        if !self
            .snapshot_poller
            .as_ref()
            .expect("checked above")
            .submit(gpu_device, gpu_lost.clone())
        {
            readback.cancel(ticket, MAX_CACHED_SNAPSHOT_TARGET_BYTES);
            gpu_lost.store(true, Ordering::Release);
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::DeviceLost,
                "The GPU snapshot completion worker stopped.",
            );
            return;
        }
        self.pending_snapshots.push(PendingSnapshot {
            job,
            ticket,
            started,
            map_deadline: now() + SNAPSHOT_GPU_TIMEOUT_US,
            revision,
            width,
            height,
            format,
            rows,
            draft_3d,
        });
    }

    /// A layer's raster (the base pixels of today's `render.raster()`):
    /// drawn by the layer raster worker ([`super::rasters`]), which answers
    /// main itself; here only when the worker could not start.
    fn run_layer_snapshot(&mut self, job: super::snapshot::SnapshotJob, started: i64) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let (device, queue, device_epoch) = (gpu.device.clone(), gpu.queue.clone(), gpu.epoch);
        let Some(doc) = self.docs.get(&job.key) else {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::UnknownSession,
                "The window's engine session has no document.",
            );
            return;
        };
        let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
        let key = job.key;
        // The worker draws from its mirror of the document, which the
        // session's updates keep at this revision; the first raster seeds
        // it with this thread's document, which the worker copies itself.
        let mirrored = self.raster_mirrors.contains(&key);
        let raster_job = super::rasters::RasterJob {
            job,
            document: if mirrored {
                super::rasters::RasterDocument::Mirror
            } else {
                super::rasters::RasterDocument::Seed(doc.shared_document())
            },
            revision,
            device,
            queue,
            device_epoch,
        };
        let raster_job = match self.rasters.as_mut() {
            Some(worker) => match worker.submit(raster_job) {
                Ok(()) => {
                    self.snapshots.set_in_worker(worker.in_flight_bytes());
                    if !mirrored {
                        self.raster_mirrors.insert(key);
                        self.stats().layer_raster_seeds += 1;
                    }
                    return;
                }
                Err(job) => *job,
            },
            None => raster_job,
        };
        // Without the worker: drawn here from the document itself, as
        // before the worker.
        let super::rasters::RasterJob { job, .. } = &raster_job;
        let fonts = self.config.fonts.clone();
        let (Some(gpu), Some(doc)) = (self.gpu.as_mut(), self.docs.get(&key)) else {
            return;
        };
        let composer = gpu.composer_with(&self.config, self.frame_cache, &mut self.prepared);
        let _ = composer.use_definitions(key, doc.definitions(), &[]);
        let outcome = super::rasters::layer_snapshot(
            composer.renderer(),
            fonts.as_ref(),
            doc.document(),
            &raster_job,
            started,
        );
        match outcome {
            Ok((envelope, published)) => {
                self.send(job.host, envelope);
                let render_us = (now() - started).max(0);
                {
                    let mut stats = self.stats();
                    stats.snapshots += 1;
                    stats.snapshot_us_max = stats.snapshot_us_max.max(render_us);
                    if published.is_some() {
                        stats.snapshot_shared += 1;
                    }
                }
                if let Some((name, length)) = published {
                    self.segments.add(name.clone());
                    self.snapshots.published(name, length);
                }
            }
            Err((code, message)) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(job, code, &message);
            }
        }
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
                self.snapshots.published(name.clone(), length as u64);
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
        self.drain_frames();
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

/// Whether a view's route takes transport: the engine runs the clock and
/// audio for every decided route, drawn or not (amendment 21: nothing hands
/// playback to the legacy engine). Only an undecided route defers it.
fn takes_transport(route: Option<&Route>) -> bool {
    matches!(route, Some(Route::Engine | Route::Legacy(_)))
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
    let png_resolver = blender.map(|blender| {
        crate::jobs::blender_png::HostBlenderPng::new(
            Arc::new(doc.media().clone()),
            blender.host_link(),
            job.key.1,
            Revision::new(doc.revision()).unwrap_or(Revision::ZERO),
            doc.document(),
        )
        .cached_only_if(job.request.blender_wait_ms == 0)
    });
    let assets: &dyn pm_render::AssetResolver = png_resolver
        .as_ref()
        .map_or(doc.media() as &dyn pm_render::AssetResolver, |resolver| {
            resolver
        });
    let snaps = match renderer.blender_snapshots(
        doc.document(),
        composition,
        job.request.time,
        request,
        assets,
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
    if png_resolver
        .as_ref()
        .is_some_and(|resolver| resolver.cold_miss())
    {
        // A cold synchronous capture cannot block this IPC turn asking the
        // window for Canvas2D bytes. Today's capture returns immediately
        // until a preview/export has prepared the frame and its inputs.
        return BlenderFrames::Fail(ErrorCode::BlenderFailed, BLENDER_NOT_PREPARED.to_owned());
    }
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
            if hashes
                .iter()
                .all(|h| frames.iter().any(|(fh, _, _)| fh == h)) =>
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

/// The texture bands of the fallback requests that sample textures (effects
/// and transitions), as `ShaderFallback(Input)` events of `session`.
fn fallback_inputs(
    renderer: &pm_render::Renderer,
    requests: &[pm_protocol::shader_fallback::ShaderFallbackRequest],
    session: SessionId,
) -> Vec<Envelope> {
    use pm_protocol::shader_fallback::{ShaderFallbackInput, ShaderFallbackMessage};
    let mut out = Vec::new();
    for r in requests.iter().filter(|r| r.textures > 0) {
        let textures = renderer.fallback_textures(&r.key).unwrap_or_default();
        for (texture, pixels) in textures.iter().enumerate() {
            for (row, rows, bytes) in pm_render::shader_layers::fallback::bands(
                pixels,
                pm_protocol::shader_fallback::MAX_BAND_BYTES,
            ) {
                let (rgba16f, rgba32f) = if pixels.rgba32f.is_some() {
                    (
                        pm_protocol::common::Bytes(Vec::new()),
                        Some(pm_protocol::common::Bytes(bytes)),
                    )
                } else {
                    (pm_protocol::common::Bytes(bytes), None)
                };
                let body =
                    Event::ShaderFallback(ShaderFallbackMessage::Input(ShaderFallbackInput {
                        key: r.key.clone(),
                        texture: texture as u32,
                        width: pixels.width,
                        height: pixels.height,
                        row,
                        rows,
                        rgba16f,
                        rgba32f,
                    }));
                out.push(Envelope::new(Message::Event { body }).with_session(session));
            }
        }
    }
    out
}

/// The renderer's view of session `key`'s compatibility producers for one
/// render at `revision`; `None` without a producer.
fn compat_view(
    store: &std::sync::Arc<std::sync::Mutex<super::compat::CompatStore>>,
    key: Key,
    revision: Revision,
    consulted: Option<Arc<Mutex<Vec<pm_protocol::snapshot::CompatNeed>>>>,
) -> Option<std::sync::Arc<dyn pm_render::compat::CompatRasters>> {
    let has = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .has_producers(key);
    has.then(|| {
        std::sync::Arc::new(super::compat::CompatView {
            store: store.clone(),
            key,
            revision,
            consulted,
        }) as std::sync::Arc<dyn pm_render::compat::CompatRasters>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_target_cache_policy_accounts_for_format_and_resize() {
        use wgpu::TextureFormat::{Rgba8Unorm, Rgba16Float, Rgba32Float};

        assert!(cache_snapshot_target(320, 180, Rgba16Float));
        assert!(cache_snapshot_target(4096, 2048, Rgba16Float));
        assert!(!cache_snapshot_target(4096, 2049, Rgba16Float));
        assert!(cache_snapshot_target(4096, 4096, Rgba8Unorm));
        assert!(!cache_snapshot_target(4096, 4096, Rgba16Float));
        assert!(!cache_snapshot_target(4096, 2048, Rgba32Float));
    }

    fn preview_thread() -> (Thread, Arc<Mutex<Vec<PreviewStatus>>>) {
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let received = statuses.clone();
        let sink: Sink = Arc::new(move |actions| {
            for action in actions {
                if let Action::Send { bytes, .. } = action {
                    let envelope = codec::decode(&bytes).unwrap();
                    if let Message::Event {
                        body: Event::PreviewStatus(status),
                    } = envelope.message
                    {
                        received.lock().unwrap().push(status);
                    }
                }
            }
        });
        let peaks = super::super::peaks::PeakWorker::new(sink.clone());
        let (snapshot_back, _snapshot_rx) = std::sync::mpsc::channel();
        let thread = Thread {
            config: ThreadConfig {
                bridge_name: None,
                lead: 1,
                fps: 60,
                extra_passes: 0,
                trace: false,
                stand_in_log: false,
                allocation_inventory: false,
                phase: None,
                remote_layer: false,
                remote_trace: None,
                remote_rate: false,
                document: true,
                host_clock: true,
                shader_cache: None,
                fonts: None,
                external_wait_cap_us: None,
                send_delay: None,
            },
            sink,
            stats: Arc::new(Mutex::new(Stats::default())),
            bridge: None,
            gpu: None,
            prepared: PreparedDefinitions::default(),
            views: BTreeMap::new(),
            docs: BTreeMap::new(),
            audio: BTreeMap::new(),
            audio_ingress: AudioIngress::default(),
            audio_prefs: BTreeMap::new(),
            peaks,
            frame_ticks: TICKS_PER_SECOND / 60,
            generations: Generations::default(),
            blender: None,
            blender_token: 0,
            preview_controls: Default::default(),
            cache_requested: BTreeMap::new(),
            cache_previews: BTreeMap::new(),
            preview_candidates: BTreeMap::new(),
            auto_preview: BTreeMap::new(),
            auto_generation: BTreeMap::new(),
            fallback: Fallback::default(),
            frame_cache: pm_render::media::DEFAULT_FRAME_CACHE,
            snapshots: super::super::snapshot::SnapshotQueue::default(),
            pending_snapshots: Vec::new(),
            snapshot_back,
            snapshot_poller: None,
            segments: pm_bridge::shm::Segments::default(),
            last_interactive: None,
            snapshot_costs: BTreeMap::new(),
            compat: Default::default(),
            peaks_waiting: BTreeMap::new(),
            doc_trace: BTreeMap::new(),
            work_start: 0,
            completer: None,
            svg: super::super::svg::SvgState::default(),
            rest: Rest::default(),
            rasters: None,
            raster_mirrors: BTreeSet::new(),
            gpu_epoch: 0,
        };
        (thread, statuses)
    }

    fn preview_key() -> Key {
        (PortId(1), SessionId::new(2).unwrap())
    }

    fn headless_test_state(headless: bool) -> ViewerState {
        use pm_protocol::color::ViewerColorSpace;
        use pm_protocol::frame::MediaPreference;
        ViewerState {
            headless: Some(headless),
            composition: "main".into(),
            size: Size::new(320, 180),
            device_pixel_ratio: 1.0,
            zoom: 1.0,
            pan: [0.0, 0.0],
            resolution: PreviewResolution::Full,
            media: MediaPreference::Automatic,
            color_space: ViewerColorSpace::Srgb,
            view: None,
            play_every_frame: false,
            edr: false,
            view_seq: Seq::new(1).unwrap(),
            vsync: None,
            depth_id: None,
            chrome_selection: None,
            canvas: None,
            surface_epoch: None,
        }
    }

    /// PLAY-1: the ingress forwards an edit and the Play after it before
    /// the render thread decides the edit's route. When the route is then
    /// pending (a new asset's media answer on its way), the Play waits for
    /// it; its replay follows the audio run the forwarded Play started and
    /// does not start (restart) the audio again. A Play the ingress did not
    /// forward still starts the audio at the replay.
    #[test]
    fn deferred_forwarded_play_replays_without_restarting_the_audio() {
        use super::super::audio::{AudioCtl, AudioLink, ForwardedAudio};
        use pm_engine::document::DocumentUpdate;

        let text = r#"{"id":"P","compId":"main","name":"T","w":320,"h":180,"fps":30,"dur":4,"layers":[{"id":"l","type":"audio","name":"l","from":0,"dur":4,"on":true,"solo":false,"group":null,"parent":null,"p":{},"fx":[],"masks":[],"d":{"asset":"a","gain":1,"trim":0,"fadeIn":0,"fadeOut":0}}],"comps":{},"assets":{"a":{"id":"a","name":"tone.wav","kind":"audio","format":"wav"}},"params":{}}"#;
        let play = TransportCommand::Play {
            rate: PlaybackRateValue::NORMAL,
        };
        for forwarded in [Some(ForwardedAudio { before: None }), None] {
            let (mut thread, _) = preview_thread();
            thread.config.host_clock = false;
            let key = preview_key();
            let (link, audio) = AudioLink::test_link();
            thread.audio.insert(key, link);
            let mut doc = ViewDocument::default();
            doc.update(&DocumentUpdate::Replace {
                root: Some(Arc::new(pm_model::json::parse(text).unwrap())),
                revision: Revision::new(1).unwrap(),
            })
            .unwrap();
            thread.docs.insert(key, doc);
            thread.configure(key, headless_test_state(true), None, None);
            thread.document_step(key);
            assert!(
                matches!(thread.views[&key].reported, Some(Route::Pending(_))),
                "{:?}",
                thread.views[&key].reported
            );
            assert!(thread.audible(key));
            let _ = audio.try_iter().count();

            thread.transport(key, play, now(), forwarded);
            assert_eq!(thread.views[&key].deferred_transport, Some(play));
            assert!(audio.try_recv().is_err(), "a deferred Play sends nothing");

            thread
                .docs
                .get_mut(&key)
                .unwrap()
                .set_media("a", Some("/media/tone.wav".into()));
            thread.document_step(key);
            let view = &thread.views[&key];
            assert_eq!(view.reported, Some(Route::Engine));
            assert_eq!(view.deferred_transport, None);
            assert!(view.deferred_forwarded.is_none());
            assert!(
                view.awaiting.is_some(),
                "the line waits for the audio clock"
            );
            let sent: Vec<AudioCtl> = audio.try_iter().collect();
            let plays = sent
                .iter()
                .filter(|c| matches!(c, AudioCtl::Play { .. }))
                .count();
            assert_eq!(
                plays,
                usize::from(forwarded.is_none()),
                "forwarded {forwarded:?}: {} controls",
                sent.len()
            );
        }
    }

    #[test]
    fn compatibility_producers_keep_the_transport_off_the_audio_ingress() {
        use super::super::audio::AudioLink;
        use pm_engine::document::DocumentUpdate;
        use pm_protocol::snapshot::{CompatMessage, CompatSource};

        let text = r#"{"id":"P","compId":"main","name":"T","w":320,"h":180,"fps":30,"dur":4,"layers":[{"id":"l","type":"audio","name":"l","from":0,"dur":4,"on":true,"solo":false,"group":null,"parent":null,"p":{},"fx":[],"masks":[],"d":{"asset":"a","gain":1,"trim":0,"fadeIn":0,"fadeOut":0}}],"comps":{},"assets":{"a":{"id":"a","name":"tone.wav","kind":"audio","format":"wav"}},"params":{}}"#;
        let play = TransportCommand::Play {
            rate: PlaybackRateValue::NORMAL,
        };
        let (mut thread, _) = preview_thread();
        thread.config.host_clock = false;
        let key = preview_key();
        let (link, _audio) = AudioLink::test_link();
        thread.audio.insert(key, link);
        let mut doc = ViewDocument::default();
        doc.update(&DocumentUpdate::Replace {
            root: Some(Arc::new(pm_model::json::parse(text).unwrap())),
            revision: Revision::new(1).unwrap(),
        })
        .unwrap();
        doc.set_media("a", Some("/media/tone.wav".into()));
        thread.docs.insert(key, doc);
        thread.configure(key, headless_test_state(true), None, None);
        thread.document_step(key);
        assert_eq!(thread.views[&key].reported, Some(Route::Engine));
        let producers = |sources| Command::Compat {
            key,
            message: CompatMessage::Producers { sources },
        };

        thread.refresh_audio_ingress(key);
        assert!(
            thread.audio_ingress.transport(key, play).is_some(),
            "an audible routed view hands its Play to the audio thread"
        );
        thread.apply(producers(vec![CompatSource::RasterOverride]));
        assert!(
            thread.audio_ingress.transport(key, play).is_none(),
            "a Play that may preroll stays on the render thread"
        );
        thread.apply(producers(vec![]));
        assert!(thread.audio_ingress.transport(key, play).is_some());
    }

    #[test]
    fn headless_rest_retires_device_and_reuses_cpu_definitions() {
        let (mut thread, _) = preview_thread();
        let a = preview_key();
        let b = (PortId(2), SessionId::new(3).unwrap());
        let mut set = pm_render::effects::builtin_definitions();
        set.definitions.truncate(1);
        let def = set.definitions[0].clone();
        let set = Arc::new(set);
        for key in [a, b] {
            thread.apply(Command::Definitions {
                key,
                set: set.clone(),
                registered: vec![],
            });
            thread.configure(key, headless_test_state(false), None, None);
            thread.views.get_mut(&key).unwrap().dirty = false;
        }
        assert!(thread.gpu().is_some());
        let device = thread.gpu.as_ref().unwrap().device.clone();
        let composer = thread.gpu.as_mut().unwrap().composer_with(
            &thread.config,
            thread.frame_cache,
            &mut thread.prepared,
        );
        let a_defs = thread.docs[&a].definitions().cloned();
        let b_defs = thread.docs[&b].definitions().cloned();
        assert!(composer.use_definitions(a, a_defs.as_ref(), &[]).is_none());
        assert!(
            composer
                .renderer()
                .effects_mut()
                .build(&device, def.kind, def.id.as_str())
                .is_none()
        );
        assert!(
            composer
                .renderer()
                .effects()
                .has_program(def.kind, def.id.as_str())
        );
        composer
            .renderer()
            .effects_mut()
            .watchdog
            .set_budget(Duration::from_millis(7));
        assert!(composer.use_definitions(b, b_defs.as_ref(), &[]).is_none());
        composer
            .renderer()
            .effects_mut()
            .watchdog
            .set_budget(Duration::from_millis(9));
        drop(device);
        let metal_before = pm_gpu::device::allocated_bytes(&thread.gpu.as_ref().unwrap().device);
        let t0 = 1_000_000;
        thread.release_at_rest(t0);
        thread.release_at_rest(t0 + REST_RELEASE_US + 1);
        assert!(thread.gpu.is_some(), "an attached viewer keeps the device");

        for key in [a, b] {
            thread.configure(key, headless_test_state(true), None, None);
        }
        thread.cache_previews.insert(
            1,
            CachePreview {
                key: a,
                revision: Revision::ZERO,
                generation: Seq::new(1).unwrap(),
            },
        );
        thread.release_at_rest(t0 + 2 * REST_RELEASE_US);
        thread.release_at_rest(t0 + 4 * REST_RELEASE_US);
        assert!(thread.gpu.is_some(), "pending preview keeps the device");
        thread.cache_previews.clear();
        thread.release_at_rest(t0 + 4 * REST_RELEASE_US + 1);
        assert!(thread.gpu.is_none());
        thread.snapshot_allocations(t0 + 4 * REST_RELEASE_US + 1);
        assert!(!thread.stats().allocations.gpu_created);
        assert_eq!(thread.stats().allocations.metal_allocated_bytes, None);
        assert!(thread.stats().allocations.metal_device.is_none());
        assert!(metal_before.is_some_and(|bytes| bytes > 0));
        assert_eq!(thread.prepared.owners.len(), 2);
        for (key, budget) in [(a, 7), (b, 9)] {
            let effects = &thread.prepared.owners[&key].0;
            assert!(effects.source_key(def.kind, def.id.as_str()).is_some());
            assert!(!effects.has_program(def.kind, def.id.as_str()));
            assert_eq!(
                effects.watchdog.budget(),
                Some(Duration::from_millis(budget))
            );
        }

        thread.configure(a, headless_test_state(false), None, None);
        let started = std::time::Instant::now();
        assert!(thread.gpu().is_some());
        let composer = thread.gpu.as_mut().unwrap().composer_with(
            &thread.config,
            thread.frame_cache,
            &mut thread.prepared,
        );
        assert!(composer.use_definitions(a, a_defs.as_ref(), &[]).is_none());
        let resumed_us = started.elapsed().as_micros();
        eprintln!("headless device reattach: {resumed_us} us; prior Metal bytes: {metal_before:?}");
        assert!(resumed_us < 50_000, "headless reattach exceeded 50 ms");
        assert_eq!(
            composer.renderer().effects().watchdog.budget(),
            Some(Duration::from_millis(7))
        );
    }

    #[test]
    fn definitions_prepare_without_a_gpu_and_transfer_per_owner() {
        let (mut thread, _) = preview_thread();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let received = reports.clone();
        thread.sink = Arc::new(move |actions| {
            for action in actions {
                if let Action::Send { bytes, .. } = action {
                    let envelope = codec::decode(&bytes).unwrap();
                    if let Message::Event {
                        body: Event::Failure(report),
                    } = envelope.message
                    {
                        received.lock().unwrap().push(report);
                    }
                }
            }
        });
        let owner_a = preview_key();
        let owner_b = (PortId(2), SessionId::new(3).unwrap());
        let mut set = pm_render::effects::builtin_definitions();
        set.definitions.truncate(1);
        let valid = set.definitions[0].clone();
        let mut invalid = valid.clone();
        invalid.id = "invalid-before-gpu".into();
        invalid
            .glsl
            .as_mut()
            .expect("effect GLSL")
            .source
            .push_str("\nthis is invalid GLSL;");
        set.definitions.push(invalid.clone());
        let set = Arc::new(set);
        assert!(thread.apply(Command::Definitions {
            key: owner_a,
            set: set.clone(),
            registered: vec![(valid.kind, valid.id.clone())],
        }));
        assert!(thread.gpu.is_none());
        assert_eq!(reports.lock().unwrap().len(), 1);
        assert_eq!(
            reports.lock().unwrap()[0].revision.get(),
            set.generation.get()
        );
        let prepared = &thread.prepared.owners[&owner_a].0;
        assert!(prepared.source_key(valid.kind, valid.id.as_str()).is_some());
        assert!(prepared.status(invalid.kind, invalid.id.as_str()).is_some());
        assert!(!prepared.has_program(valid.kind, valid.id.as_str()));

        // A second registration keeps each unchanged translation and reports
        // an unchanged invalid source at its current definitions generation.
        assert!(thread.apply(Command::Definitions {
            key: owner_a,
            set: set.clone(),
            registered: vec![(invalid.kind, invalid.id.clone())],
        }));
        let (_, loaded) = &thread.prepared.owners[&owner_a];
        assert_eq!(loaded.1, 2);
        assert_eq!(reports.lock().unwrap().len(), 2);

        let mut other = (*set).clone();
        other.definitions.pop();
        let other = Arc::new(other);
        assert!(thread.apply(Command::Definitions {
            key: owner_b,
            set: other,
            registered: vec![],
        }));
        assert_eq!(thread.prepared.owners.len(), 2);
        assert_eq!(reports.lock().unwrap().len(), 2);

        let gpu = pm_gpu::Gpu::new("prepared definitions test").expect("a Metal device");
        let mut composer = composer(&gpu.device, &gpu.queue, &thread.config, thread.frame_cache);
        composer
            .renderer()
            .effects_mut()
            .parity
            .configure(Some("prepared-test".into()), None);
        thread.prepared.install(&mut composer);
        assert!(thread.prepared.owners.is_empty());
        assert!(thread.prepared.base.is_none());
        let a = thread.docs[&owner_a].definitions().cloned();
        let b = thread.docs[&owner_b].definitions().cloned();
        assert!(composer.use_definitions(owner_a, a.as_ref(), &[]).is_none());
        assert_eq!(
            composer.renderer().effects().parity.environment(),
            Some("prepared-test")
        );
        composer
            .renderer()
            .effects_mut()
            .watchdog
            .set_budget(Duration::from_millis(7));
        assert!(composer.use_definitions(owner_b, b.as_ref(), &[]).is_none());
        assert_eq!(composer.renderer().effects().watchdog.budget(), None);
        assert!(composer.use_definitions(owner_a, a.as_ref(), &[]).is_none());
        assert_eq!(
            composer.renderer().effects().watchdog.budget(),
            Some(Duration::from_millis(7))
        );
        assert!(
            composer
                .renderer()
                .effects()
                .source_key(valid.kind, valid.id.as_str())
                .is_some()
        );
    }

    #[test]
    fn idle_gpu_memory_goes_once_the_viewer_rests() {
        let (mut thread, _) = preview_thread();
        thread.docs.insert(preview_key(), ViewDocument::default());
        let gpu = thread.gpu.insert({
            let g = pm_gpu::Gpu::new("rest release test").expect("a Metal device");
            Gpu {
                display: DisplayTransform::new(&g.device, &g.queue),
                snapshot_readback: pm_gpu::readback::ReadbackRing::default(),
                snapshot_target: None,
                composer: None,
                fonts_installed: None,
                lost: Arc::new(AtomicBool::new(false)),
                epoch: 0,
                device: g.device,
                queue: g.queue,
            }
        });
        let target = snapshot_output_target(
            &mut gpu.snapshot_target,
            &gpu.device,
            320,
            180,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        drop(target);
        assert!(gpu.snapshot_target.is_some());
        let t0 = 1_000_000;
        thread.release_at_rest(t0);
        thread.release_at_rest(t0 + REST_RELEASE_US - 1);
        assert!(
            thread.gpu.as_ref().unwrap().snapshot_target.is_some(),
            "too early"
        );
        // A frame starts a new rest.
        thread.stats().frames += 1;
        thread.release_at_rest(t0 + REST_RELEASE_US + 1);
        assert!(thread.gpu.as_ref().unwrap().snapshot_target.is_some());
        thread.release_at_rest(t0 + 2 * REST_RELEASE_US);
        assert!(thread.gpu.as_ref().unwrap().snapshot_target.is_some());
        thread.release_at_rest(t0 + 2 * REST_RELEASE_US + 2);
        assert!(thread.gpu.as_ref().unwrap().snapshot_target.is_none());
        let stats = thread.stats().clone();
        assert_eq!(stats.rest_releases, 1);
        assert_eq!(stats.rest_released_bytes, 320 * 180 * 4);
        // Once per rest.
        thread.release_at_rest(t0 + 4 * REST_RELEASE_US);
        assert_eq!(thread.stats().rest_releases, 1);

        // Closing the final session drops the device after the same rest
        // bound. A new session will create it on its first render.
        assert!(thread.apply(Command::Close { key: preview_key() }));
        thread.release_at_rest(t0 + 5 * REST_RELEASE_US);
        thread.release_at_rest(t0 + 6 * REST_RELEASE_US);
        assert!(thread.gpu.is_none());
        assert_eq!(thread.stats().rest_releases, 2);
        assert!(thread.gpu().is_some());
    }

    #[test]
    fn rested_working_texture_recreates_without_changing_a_frame_or_pool() {
        use pm_protocol::color::ViewerColorSpace;
        use pm_protocol::frame::MediaPreference;

        let (mut thread, _) = preview_thread();
        thread.config.document = false;
        let key = preview_key();
        let size = Size::new(1496, 904);
        thread.configure(
            key,
            ViewerState {
                headless: None,
                composition: "main".into(),
                size,
                device_pixel_ratio: 1.0,
                zoom: 1.0,
                pan: [0.0, 0.0],
                resolution: PreviewResolution::Full,
                media: MediaPreference::Automatic,
                color_space: ViewerColorSpace::Srgb,
                view: None,
                play_every_frame: false,
                edr: false,
                view_seq: Seq::new(1).unwrap(),
                vsync: None,
                depth_id: None,
                chrome_selection: None,
                canvas: None,
                surface_epoch: None,
            },
            None,
            None,
        );
        let spec = PoolSpec {
            generation: 7,
            size,
            format: SurfaceFormat::Bgra8Unorm,
            count: 4,
        };
        let live = {
            let gpu = thread.gpu().expect("a Metal device");
            Live {
                spec,
                pool: SurfacePool::new(&gpu.device, spec.generation, size, spec.format, spec.count)
                    .unwrap(),
                ledger: PoolLedger::new(spec.count as usize),
                working: Some(WorkingTexture::new(&gpu.device, size)),
                source: Some(TestSource::new(&gpu.device, size, 0)),
                remote: None,
            }
        };
        let view = thread.views.get_mut(&key).unwrap();
        view.pool = Some(live);
        view.dirty = false;

        let first = thread.render(key, 0, -1, None, false, false).unwrap();
        let first_pixels = thread.views[&key].pool.as_ref().unwrap().pool.surfaces
            [first.pool_index as usize]
            .iosurface
            .read();
        let allocated_before =
            pm_gpu::device::allocated_bytes(&thread.gpu.as_ref().unwrap().device);
        thread.apply(Command::Release {
            key,
            release: FrameRelease {
                pool_index: first.pool_index,
                seq: first.seq,
            },
        });
        let t0 = 1_000_000;
        thread.release_at_rest(t0);
        thread.release_at_rest(t0 + REST_RELEASE_US + 1);
        let live = thread.views[&key].pool.as_ref().unwrap();
        assert!(live.working.is_none());
        assert_eq!(live.spec.generation, spec.generation);
        assert_eq!(live.pool.len(), spec.count as usize);
        assert_eq!(
            thread.stats().rest_released_bytes,
            u64::from(size.width * size.height * 8)
        );
        let allocated_at_rest =
            pm_gpu::device::allocated_bytes(&thread.gpu.as_ref().unwrap().device);

        // The same sequence drives both the pattern and display dither.
        thread.views.get_mut(&key).unwrap().next_seq = first.seq.get();
        let started = std::time::Instant::now();
        let second = thread.render(key, 0, -1, None, false, false).unwrap();
        let resumed_us = started.elapsed().as_micros();
        let live = thread.views[&key].pool.as_ref().unwrap();
        assert!(live.working.is_some());
        assert_eq!(live.spec.generation, spec.generation);
        let second_pixels = live.pool.surfaces[second.pool_index as usize]
            .iosurface
            .read();
        assert_eq!(first_pixels, second_pixels);
        let allocated_after = pm_gpu::device::allocated_bytes(&thread.gpu.as_ref().unwrap().device);
        eprintln!(
            "first frame after scratch rest: {resumed_us} us; Metal bytes before/rest/after: {allocated_before:?}/{allocated_at_rest:?}/{allocated_after:?}"
        );
        assert!(resumed_us < 50_000, "first resumed frame exceeded 50 ms");

        // Closing the viewer leaves a headless session. Its surfaces are
        // retired; attaching again must use a fresh pool generation and
        // reproduce the exact pixels on the new device.
        let mut state = thread.views[&key].state.clone();
        state.headless = Some(true);
        thread.configure(key, state.clone(), None, None);
        assert!(thread.views[&key].pool.is_none());
        thread.release_at_rest(t0 + 2 * REST_RELEASE_US);
        thread.release_at_rest(t0 + 4 * REST_RELEASE_US + 1);
        assert!(thread.gpu.is_none());
        state.headless = None;
        thread.configure(key, state, None, None);
        let new_spec = PoolSpec {
            generation: spec.generation + 1,
            ..spec
        };
        let live = {
            let gpu = thread.gpu().expect("a new Metal device");
            Live {
                spec: new_spec,
                pool: SurfacePool::new(
                    &gpu.device,
                    new_spec.generation,
                    size,
                    new_spec.format,
                    new_spec.count,
                )
                .unwrap(),
                ledger: PoolLedger::new(new_spec.count as usize),
                working: Some(WorkingTexture::new(&gpu.device, size)),
                source: Some(TestSource::new(&gpu.device, size, 0)),
                remote: None,
            }
        };
        let view = thread.views.get_mut(&key).unwrap();
        view.pool = Some(live);
        view.next_seq = first.seq.get();
        view.dirty = false;
        let third = thread.render(key, 0, -1, None, false, false).unwrap();
        let live = thread.views[&key].pool.as_ref().unwrap();
        assert_eq!(live.spec.generation, spec.generation + 1);
        assert_eq!(
            first_pixels,
            live.pool.surfaces[third.pool_index as usize]
                .iosurface
                .read()
        );
    }

    #[test]
    fn pending_waveforms_keep_latest_generation_even_at_capacity() {
        let (mut thread, _) = preview_thread();
        let key = preview_key();
        for index in 0..MAX_PEAKS_WAITING {
            thread.audio_message(
                key,
                AudioMessage::PeaksWanted {
                    asset: format!("clip-{index}"),
                    request: Some(Seq::new(1).unwrap()),
                },
            );
        }
        thread.audio_message(
            key,
            AudioMessage::PeaksWanted {
                asset: "clip-0".into(),
                request: Some(Seq::new(2).unwrap()),
            },
        );
        thread.audio_message(
            key,
            AudioMessage::PeaksWanted {
                asset: "beyond-capacity".into(),
                request: Some(Seq::new(3).unwrap()),
            },
        );
        assert_eq!(thread.peaks_waiting.len(), MAX_PEAKS_WAITING);
        assert_eq!(
            thread.peaks_waiting[&(key, "clip-0".into())],
            Some(Seq::new(2).unwrap())
        );
        assert!(
            !thread
                .peaks_waiting
                .contains_key(&(key, "beyond-capacity".into()))
        );
    }

    #[test]
    fn cache_only_control_queues_without_viewer_or_transport_change() {
        use pm_protocol::common::CompositionId;
        let (mut thread, statuses) = preview_thread();
        let key = preview_key();
        let control = PreviewControl {
            generation: Seq::new(7).unwrap(),
            revision: Revision::new(3).unwrap(),
            composition: CompositionId::new("comp"),
            time: Ticks::new(705_600_000).unwrap(),
            shading: Shading3d::Rendered,
            action: PreviewAction::Render,
            cache_only: true,
            size: Some(Size::new(640, 360)),
            camera: None,
        };
        thread.apply(Command::PreviewControl {
            key,
            control: control.clone(),
        });
        assert!(thread.views.is_empty());
        assert_eq!(thread.cache_requested.get(&key), Some(&control));
        assert_eq!(statuses.lock().unwrap()[0].phase, PreviewPhase::Queued);
        thread.apply(Command::PreviewControl {
            key,
            control: PreviewControl {
                generation: Seq::new(8).unwrap(),
                action: PreviewAction::Cancel,
                cache_only: false,
                size: None,
                ..control
            },
        });
        assert!(!thread.cache_requested.contains_key(&key));
        assert_eq!(
            statuses.lock().unwrap().last().unwrap().phase,
            PreviewPhase::Cancelled
        );
    }

    fn candidate(shading: Shading3d, time: i64, complete: bool) -> PreviewCandidate {
        PreviewCandidate {
            generation: None,
            auto_generation: None,
            revision: Revision::new(7).unwrap(),
            composition: "comp".into(),
            time: Ticks::new(time).unwrap(),
            shading,
            complete,
            failure: None,
        }
    }

    #[test]
    fn automatic_preview_shading_change_gets_a_new_generation() {
        let material = candidate(Shading3d::Material, 0, false);
        let first = AutoPreview::next(None, None, &material).unwrap();
        assert!(first.matches(&material));
        let rendered = candidate(Shading3d::Rendered, 0, false);
        assert!(!first.matches(&rendered));
        let second = AutoPreview::next(Some(&first), Some(first.generation), &rendered).unwrap();
        assert_eq!(second.generation.get(), 2);
        assert_eq!(second.shading, Shading3d::Rendered);
    }

    #[test]
    fn cached_complete_frame_replaces_stale_automatic_busy_status() {
        let first =
            AutoPreview::next(None, None, &candidate(Shading3d::Rendered, 0, false)).unwrap();
        let cached = candidate(Shading3d::Rendered, 42, true);
        let second = AutoPreview::next(Some(&first), Some(first.generation), &cached).unwrap();
        assert_eq!(second.generation.get(), 2);
        assert!(second.matches(&cached));
        assert_eq!(second.phase, PreviewPhase::Queued);
    }

    #[test]
    fn failed_snapshot_reports_automatic_failure_without_a_ready_frame() {
        let (mut thread, statuses) = preview_thread();
        let key = preview_key();
        let seq = Seq::new(11).unwrap();
        let mut failed = candidate(Shading3d::Rendered, 0, false);
        failed.failure = Some("The Blender preview could not be prepared: bad map".into());
        thread.preview_candidates.insert((key, seq.get()), failed);

        thread.queue_auto_preview(key, seq);
        thread.preview_frame_sent(key, seq);

        let statuses = statuses.lock().unwrap();
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].phase, PreviewPhase::Queued);
        assert_eq!(statuses[1].phase, PreviewPhase::Failed);
        assert_eq!(statuses[0].generation, statuses[1].generation);
        assert!(statuses[1].automatic);
        assert_eq!(
            statuses[1].error.as_deref(),
            Some("The Blender preview could not be prepared: bad map")
        );
        assert!(statuses.iter().all(|s| s.phase != PreviewPhase::Ready));
    }

    #[test]
    fn failed_snapshot_ends_explicit_request_and_cannot_become_ready() {
        use pm_protocol::common::CompositionId;
        let (mut thread, statuses) = preview_thread();
        let key = preview_key();
        let generation = Seq::new(3).unwrap();
        let seq = Seq::new(11).unwrap();
        thread
            .preview_controls
            .control(
                key,
                PreviewControl {
                    generation,
                    revision: Revision::new(7).unwrap(),
                    composition: CompositionId::new("comp"),
                    time: Ticks::ZERO,
                    shading: Shading3d::Rendered,
                    action: PreviewAction::Render,
                    cache_only: false,
                    size: None,
                    camera: None,
                },
            )
            .unwrap();
        let mut failed = candidate(Shading3d::Rendered, 0, false);
        failed.generation = Some(generation);
        failed.failure = Some("The Blender preview could not be prepared: bad map".into());
        thread.preview_candidates.insert((key, seq.get()), failed);

        thread.fail_preview_snapshot(
            key,
            Revision::new(7).unwrap(),
            Some(generation),
            Some("The Blender preview could not be prepared: bad map".into()),
        );
        thread.preview_frame_sent(key, seq);

        let statuses = statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].phase, PreviewPhase::Failed);
        assert!(!statuses[0].automatic);
        assert_eq!(statuses[0].generation, generation);
        assert_eq!(
            statuses[0].error.as_deref(),
            Some("The Blender preview could not be prepared: bad map")
        );
        assert!(thread.preview_controls.current(key).is_none());
    }

    #[test]
    fn synchronous_delivery_error_fails_explicit_preview() {
        use pm_protocol::common::CompositionId;
        let (mut thread, statuses) = preview_thread();
        let key = preview_key();
        let generation = Seq::new(4).unwrap();
        thread
            .preview_controls
            .control(
                key,
                PreviewControl {
                    generation,
                    revision: Revision::new(7).unwrap(),
                    composition: CompositionId::new("comp"),
                    time: Ticks::ZERO,
                    shading: Shading3d::Rendered,
                    action: PreviewAction::Render,
                    cache_only: false,
                    size: None,
                    camera: None,
                },
            )
            .unwrap();
        let mut pending = candidate(Shading3d::Rendered, 0, true);
        pending.generation = Some(generation);
        thread.fail_preview_delivery(key, Some(&pending), "GPU did not finish".into());

        let statuses = statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].phase, PreviewPhase::Failed);
        assert_eq!(statuses[0].error.as_deref(), Some("GPU did not finish"));
        assert!(thread.preview_controls.current(key).is_none());
    }

    #[test]
    fn deferred_frame_failure_reports_only_its_automatic_generation() {
        let (mut thread, statuses) = preview_thread();
        let key = preview_key();
        let seq = Seq::new(12).unwrap();
        thread
            .preview_candidates
            .insert((key, seq.get()), candidate(Shading3d::Rendered, 0, true));
        thread.queue_auto_preview(key, seq);
        let meta = super::super::complete::FrameMeta {
            seq: seq.get(),
            k: -1,
            index: 0,
            generation: 1,
            timing: None,
            view_seq: 1,
            revision: 7,
            start: 0,
            work_start: 0,
            cpu_us: 0,
            overlay_layers: 0,
            failures: 0,
            stand_in_shown: None,
            depth: false,
            held: 0,
        };
        thread.apply(Command::FrameFailed {
            key,
            meta: Box::new(meta),
            error: "GPU lost".into(),
        });
        thread.preview_frame_sent(key, seq);

        let statuses = statuses.lock().unwrap();
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].phase, PreviewPhase::Queued);
        assert_eq!(statuses[1].phase, PreviewPhase::Failed);
        assert_eq!(statuses[1].generation, statuses[0].generation);
        assert_eq!(
            statuses[1].error.as_deref(),
            Some("The native viewer could not present the preview: GPU lost")
        );
        assert!(statuses.iter().all(|s| s.phase != PreviewPhase::Ready));
    }

    #[test]
    fn scene_build_error_without_frame_fails_explicit_and_automatic_previews() {
        use pm_protocol::common::CompositionId;
        let (mut thread, statuses) = preview_thread();
        let key = preview_key();
        let revision = Revision::new(7).unwrap();
        let generation = Seq::new(1).unwrap();
        thread
            .preview_controls
            .control(
                key,
                PreviewControl {
                    generation,
                    revision,
                    composition: CompositionId::new("comp"),
                    time: Ticks::ZERO,
                    shading: Shading3d::Rendered,
                    action: PreviewAction::Render,
                    cache_only: false,
                    size: None,
                    camera: None,
                },
            )
            .unwrap();
        thread.fail_preview_without_frame(
            key,
            revision,
            Some(generation),
            Some(Shading3d::Rendered),
            "comp",
            Ticks::ZERO,
            true,
            "scene evaluation failed".into(),
        );
        assert!(thread.preview_controls.current(key).is_none());
        thread.fail_preview_without_frame(
            key,
            revision,
            None,
            Some(Shading3d::Rendered),
            "comp",
            Ticks::ZERO,
            true,
            "scene evaluation failed".into(),
        );

        let statuses = statuses.lock().unwrap();
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].phase, PreviewPhase::Failed);
        assert!(!statuses[0].automatic);
        assert_eq!(statuses[1].phase, PreviewPhase::Failed);
        assert!(statuses[1].automatic);
        assert_eq!(statuses[0].error, statuses[1].error);
        assert!(statuses.iter().all(|s| s.phase != PreviewPhase::Ready));
    }

    #[test]
    fn terminal_pool_failure_ends_explicit_and_automatic_previews() {
        use pm_protocol::common::CompositionId;
        let (mut thread, statuses) = preview_thread();
        let key = preview_key();
        let revision = Revision::new(7).unwrap();
        let generation = Seq::new(4).unwrap();
        thread
            .preview_controls
            .control(
                key,
                PreviewControl {
                    generation,
                    revision,
                    composition: CompositionId::new("comp"),
                    time: Ticks::ZERO,
                    shading: Shading3d::Rendered,
                    action: PreviewAction::Render,
                    cache_only: false,
                    size: None,
                    camera: None,
                },
            )
            .unwrap();
        let auto =
            AutoPreview::next(None, None, &candidate(Shading3d::Rendered, 0, false)).unwrap();
        thread.auto_preview.insert(key, auto);

        // No frame bridge makes pool creation permanently impossible.
        thread.make_pool(key, spec(), 0, None);

        let statuses = statuses.lock().unwrap();
        assert_eq!(statuses.len(), 2);
        assert!(statuses.iter().all(|s| s.phase == PreviewPhase::Failed));
        assert!(statuses.iter().all(|s| {
            s.error
                .as_deref()
                .is_some_and(|e| e.contains("could not create the preview frame surface"))
        }));
        assert!(thread.preview_controls.current(key).is_none());
    }

    #[test]
    fn slow_turns_fall_in_their_bands() {
        assert_eq!(slow_turn_band(0), None);
        assert_eq!(slow_turn_band(49_999), None);
        assert_eq!(slow_turn_band(50_000), Some(0));
        assert_eq!(slow_turn_band(120_000), Some(1));
        assert_eq!(slow_turn_band(STALL_US), Some(2));
        assert_eq!(slow_turn_band(999_999), Some(2));
        assert_eq!(slow_turn_band(4_000_000), Some(3));
    }

    fn spec() -> PoolSpec {
        PoolSpec {
            generation: 3,
            size: Size::new(1280, 720),
            format: SurfaceFormat::Bgra8Unorm,
            count: 8,
        }
    }

    /// A paused frame that failed for the moment is drawn again after
    /// 100 ms, 500 ms and 2 s, then waits for a change; a deterministic
    /// failure (an over-budget scene reports an invalid request) waits at
    /// once. A failure of another frame starts the retries over.
    #[test]
    fn transient_render_failures_retry_with_backoff_then_wait_for_a_change() {
        use pm_render::RenderError as E;
        assert!(transient_render_error(&E::Gpu("lost".into())));
        assert!(transient_render_error(&E::Media("decode".into())));
        assert!(transient_render_error(&E::Fallback("runner".into())));
        assert!(transient_render_error(&E::SvgRaster("window".into())));
        assert!(!transient_render_error(&E::Request(
            "viewport geometry: an imported scene over two million vertices".into()
        )));
        assert!(!transient_render_error(&E::Composition("main".into())));
        assert!(!transient_render_error(&E::Watchdog("runaway".into())));

        let frame = (7, 3);
        let mut retry = None;
        let mut at = Vec::new();
        for now in [0, 1_000_000, 2_000_000, 3_000_000] {
            let next = RenderRetry::after(retry, frame, now);
            at.push(next.at.map(|t| t - now));
            retry = Some(next);
        }
        assert_eq!(at, [Some(100_000), Some(500_000), Some(2_000_000), None]);
        // Another frame (an edit) starts over.
        let edited = RenderRetry::after(retry, (8, 3), 5_000_000);
        assert_eq!((edited.at, edited.attempts), (Some(5_100_000), 1));
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
