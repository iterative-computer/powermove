//! The frame-bridge presenter of the interactive engine (plan section 18;
//! amendments 4 and 5).
//!
//! [`BridgePresenter`] is the [`FramePresenter`] the engine routes viewer
//! traffic to. On the engine's JavaScript thread it sizes each viewer's
//! surface pool (viewer device pixels; ring + jitter buffer + 2 surfaces;
//! `rgbaf16` on EDR displays when the lease allows, `bgra` otherwise) and
//! charges it to the lease; everything else happens on its render thread
//! ([`render`]): the check-in with main, pool creation and hand-off,
//! rendering, the transport clock, and delivery of descriptors to the
//! viewer's port.
//!
//! Frames come from a frame source: the session's document drawn by
//! pm-render (`POWERMOVE_ENGINE_FRAME_SOURCE=document`, what main starts the
//! interactive engine with; the Phase 2 tracer bullet), or the bridge test
//! source (`test-pattern`, the bridge tests and the S1 harness). Without a
//! source the engine produces no viewer frames and the viewer keeps the
//! current engine's.

pub mod audio;
pub mod blender;
pub mod clock;
pub mod compat;
pub mod compose;
pub mod document;
mod layer_reports;
pub mod peaks;
pub mod render;
pub mod schedule;
pub mod snapshot;
pub mod stats;
pub mod view;

use std::collections::BTreeMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use pm_engine::LogLevel;
use pm_engine::present::{FramePresenter, PresentContext};
use pm_gpu::bridge_out::pool::{choose_format, estimated_bytes, pool_size};
use pm_protocol::clock::TransportCommand;
use pm_protocol::common::{RequestId, Revision, SessionId, Size};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::frame::{FrameRelease, FrameRequest, ViewerState};
use pm_protocol::host::PortId;
use pm_protocol::interop::SurfaceFormat;

use render::{Command, Key};
pub use render::{FrameTrace, Generations, PoolSpec, Sink, Stats, ThreadConfig};

/// Environment main sets for the interactive engine: the per-launch
/// bootstrap name.
pub const BRIDGE_ENV: &str = "POWERMOVE_FRAME_BRIDGE";
/// Selects the frame source (`test-pattern`).
pub const SOURCE_ENV: &str = "POWERMOVE_ENGINE_FRAME_SOURCE";
/// S1 harness knobs: render-ahead in frames, test source frame rate, extra
/// GPU passes, and the per-frame trace.
pub const LEAD_ENV: &str = "POWERMOVE_BRIDGE_LEAD";
pub const FPS_ENV: &str = "POWERMOVE_BRIDGE_TEST_FPS";
pub const LOAD_ENV: &str = "POWERMOVE_BRIDGE_TEST_LOAD";
pub const TRACE_ENV: &str = "POWERMOVE_BRIDGE_TRACE";
/// `1`: keep the stand-in log in the presenter's status (scrub
/// measurements, docs/engine/scrub.md).
pub const STAND_IN_LOG_ENV: &str = "POWERMOVE_ENGINE_STAND_IN_LOG";
/// Where timed frames start on the viewer's refresh grid, in thousandths of
/// a refresh after it (`off` renders at the due time, as before the hint).
pub const PHASE_ENV: &str = "POWERMOVE_BRIDGE_PHASE";
/// The C′ prototype: `1` presents playback through a remote Core
/// Animation layer main hosts in the viewer's window (measurement builds
/// with the `remote-layer` feature only).
pub const REMOTE_LAYER_ENV: &str = "POWERMOVE_BRIDGE_REMOTE_LAYER";
/// Where the C′ prototype appends its presentation records (JSON lines).
pub const REMOTE_TRACE_ENV: &str = "POWERMOVE_BRIDGE_REMOTE_TRACE";
/// `1`: while the C′ layer plays, ask the display for a refresh rate that
/// is a multiple of the content rate (plan section 15).
pub const REMOTE_RATE_ENV: &str = "POWERMOVE_BRIDGE_REMOTE_RATE";

/// Default render phase: thousandths of a refresh after the BeginFrame
/// (docs/engine/bridge.md, "Vsync-phase hint").
pub const DEFAULT_PHASE_PERMILLE: u32 = 500;

/// Frames being rendered or waiting for their command buffer.
pub const RING: u32 = 2;
/// Default render-ahead: S1 found a lead of 4 enough once main is off the
/// per-frame path (121 ms of slack at lead 10).
pub const DEFAULT_LEAD: u32 = 4;

/// What produces viewer frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSource {
    TestPattern,
    /// The session's document, drawn by pm-render.
    Document,
}

/// `1`: play on the host clock rather than the audio device (machines
/// without an output, and runs that must not open one).
pub const HOST_CLOCK_ENV: &str = "POWERMOVE_ENGINE_HOST_CLOCK";

/// The folder translated GLSL programs are kept in across launches (plan
/// section 11, "cache by source hash"); main passes its engine cache's
/// `Shader Cache` folder. Unset: programs are kept in memory only.
pub const SHADER_CACHE_ENV: &str = "POWERMOVE_ENGINE_SHADER_CACHE";

#[derive(Debug, Clone)]
pub struct PresenterConfig {
    pub thread: ThreadConfig,
    pub source: Option<FrameSource>,
}

impl PresenterConfig {
    /// Reads the environment the utility process was started with. `None`
    /// when this engine does not present (the job engine, or no bridge name
    /// from main).
    pub fn from_env(process: EngineProcess, env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if process != EngineProcess::Interactive {
            return None;
        }
        let bridge_name = env(BRIDGE_ENV).filter(|n| !n.is_empty())?;
        let number = |key: &str, default: u32, max: u32| {
            env(key)
                .and_then(|v| v.parse::<u32>().ok())
                .map(|v| v.min(max))
                .unwrap_or(default)
        };
        Some(Self {
            thread: ThreadConfig {
                bridge_name: Some(bridge_name),
                lead: number(LEAD_ENV, DEFAULT_LEAD, 28).max(1),
                fps: number(FPS_ENV, 60, 240).max(1),
                extra_passes: number(LOAD_ENV, 0, 8),
                trace: env(TRACE_ENV).as_deref() == Some("1"),
                stand_in_log: env(STAND_IN_LOG_ENV).as_deref() == Some("1"),
                phase: match env(PHASE_ENV).as_deref() {
                    Some("off") => None,
                    Some(v) => v
                        .parse::<u32>()
                        .ok()
                        .filter(|p| *p < 1000)
                        .map(|p| f64::from(p) / 1000.0)
                        .or(Some(f64::from(DEFAULT_PHASE_PERMILLE) / 1000.0)),
                    None => Some(f64::from(DEFAULT_PHASE_PERMILLE) / 1000.0),
                },
                remote_layer: env(REMOTE_LAYER_ENV).as_deref() == Some("1"),
                remote_trace: env(REMOTE_TRACE_ENV).filter(|p| !p.is_empty()),
                remote_rate: env(REMOTE_RATE_ENV).as_deref() == Some("1"),
                document: env(SOURCE_ENV).as_deref() == Some("document"),
                host_clock: env(HOST_CLOCK_ENV).as_deref() == Some("1"),
                shader_cache: env(SHADER_CACHE_ENV)
                    .filter(|p| !p.is_empty())
                    .map(std::path::PathBuf::from),
            },
            source: match env(SOURCE_ENV).as_deref() {
                Some("test-pattern") => Some(FrameSource::TestPattern),
                Some("document") => Some(FrameSource::Document),
                _ => None,
            },
        })
    }
}

/// Reaches the render thread from the engine's host.
#[derive(Clone)]
pub struct PresenterControl(Sender<Command>);

impl PresenterControl {
    /// Destroys the GPU device as a real loss would; the render thread
    /// rebuilds it, makes every pool again, and redraws.
    pub fn lose_device(&self) {
        let _ = self.0.send(Command::LoseDevice);
    }
}

/// What the JavaScript side remembers per viewer.
struct Meta {
    pool: Option<(PoolSpec, u64)>,
}

pub struct BridgePresenter {
    config: PresenterConfig,
    tx: Sender<Command>,
    thread: Option<JoinHandle<()>>,
    stats: Arc<Mutex<Stats>>,
    views: BTreeMap<Key, Meta>,
    generations: Generations,
    told_no_source: bool,
    /// The paused viewer's Blender jobs: host calls made from its worker
    /// (`blender`), and the previews (document source only).
    link: Option<Arc<crate::jobs::blender::HostLink>>,
    blender: Option<Arc<blender::PreviewBlender>>,
    /// The viewer's frame cache: the share sent to the render thread and
    /// the bytes reserved for it in the lease.
    frame_cache: Option<usize>,
    frame_cache_reserved: u64,
}

/// The largest frame cache the viewer takes: the GOP neighbourhoods a UHD
/// scrub decodes, about 40 UHD 4:2:0 frames (docs/engine/scrub.md).
pub const VIEWER_FRAME_CACHE_MAX: u64 = 512 << 20;

/// The viewer's frame-cache share of a lease of `granted` bytes, with
/// `room` bytes of it not held by anything else: a quarter of the lease
/// between the default frame cache and [`VIEWER_FRAME_CACHE_MAX`], never
/// more than the room left. Without a lease (`granted == 0`), the default
/// frame cache every renderer starts with.
pub fn viewer_frame_cache(granted: u64, room: u64) -> u64 {
    let default = pm_render::media::DEFAULT_FRAME_CACHE as u64;
    if granted == 0 {
        return default;
    }
    (granted / 4)
        .clamp(default, VIEWER_FRAME_CACHE_MAX)
        .min(room)
}

impl BridgePresenter {
    /// Starts the render thread, which checks in with main at once.
    pub fn start(config: PresenterConfig, sink: Sink) -> Result<Self, String> {
        let (tx, rx) = channel();
        let stats = Arc::new(Mutex::new(Stats::default()));
        let thread_stats = stats.clone();
        let thread_config = config.thread.clone();
        let generations = Generations::default();
        let thread_generations = generations.clone();
        let thread_sink = sink.clone();
        let thread = pm_engine::qos::spawn_engine_thread(
            EngineProcess::Interactive,
            "pm-viewer-present",
            move || {
                render::run(
                    thread_config,
                    rx,
                    thread_sink,
                    thread_stats,
                    thread_generations,
                )
            },
        )
        .map_err(|e| format!("could not start the viewer presenter: {e}"))?;
        // Documents get Blender shading while paused: its host calls wake
        // the utility process through the sink (it then calls `pump`).
        let (link, blender) = if config.source == Some(FrameSource::Document) {
            let wake_sink = sink.clone();
            let link = crate::jobs::blender::HostLink::new(Arc::new(move || wake_sink(Vec::new())));
            let ready = tx.clone();
            let blender = blender::PreviewBlender::start(
                link.clone(),
                sink,
                stats.clone(),
                Box::new(move |key| {
                    let _ = ready.send(Command::BlenderReady { key });
                }),
            )?;
            let _ = tx.send(Command::UseBlender(blender.clone()));
            (Some(link), Some(blender))
        } else {
            (None, None)
        };
        Ok(Self {
            config,
            tx,
            thread: Some(thread),
            stats,
            views: BTreeMap::new(),
            generations,
            told_no_source: false,
            link,
            blender,
            frame_cache: None,
            frame_cache_reserved: 0,
        })
    }

    /// The counters, shared with the render thread (the engine's status
    /// reads them after the presenter moved into the engine).
    pub fn stats_handle(&self) -> Arc<Mutex<Stats>> {
        self.stats.clone()
    }

    /// Counters for the engine's status.
    pub fn stats(&self) -> Stats {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A handle that reaches the render thread after the presenter moved
    /// into the engine (the GPU-loss recovery test).
    pub fn control(&self) -> PresenterControl {
        PresenterControl(self.tx.clone())
    }

    /// Takes the per-frame trace (S1 harness).
    pub fn take_trace(&self) -> Vec<FrameTrace> {
        std::mem::take(&mut self.stats.lock().unwrap_or_else(|e| e.into_inner()).trace)
    }

    fn send(&self, command: Command) {
        // The thread only stops on shutdown; a send after it is dropped.
        let _ = self.tx.send(command);
    }

    /// Sizes, formats, and charges a pool for `state`, replacing the
    /// viewer's current pool when anything differs.
    fn plan_pool(
        &mut self,
        ctx: &mut PresentContext<'_>,
        key: Key,
        state: &ViewerState,
    ) -> Option<PoolSpec> {
        let count = pool_size(RING, self.config.thread.lead);
        let size = Size::new(state.size.width, state.size.height);
        let want_f16 = choose_format(state.edr, true) == SurfaceFormat::Rgba16Float;
        let current = self.views.get(&key).and_then(|m| m.pool);
        if let Some((spec, _)) = current {
            let same_format = (spec.format == SurfaceFormat::Rgba16Float) == want_f16;
            if spec.size == size && spec.count == count && same_format {
                return None;
            }
        }
        // Release the old pool before charging the new one.
        if let Some((_, bytes)) = current {
            ctx.release_pool(bytes);
        }
        let f16_bytes = estimated_bytes(size, SurfaceFormat::Rgba16Float, count);
        let bgra_bytes = estimated_bytes(size, SurfaceFormat::Bgra8Unorm, count);
        let (format, bytes) = if want_f16 && ctx.charge_pool(f16_bytes) {
            (SurfaceFormat::Rgba16Float, f16_bytes)
        } else if ctx.charge_pool(bgra_bytes) {
            if want_f16 {
                ctx.log(
                    LogLevel::Info,
                    "viewer pool: rgbaf16 does not fit the lease; using bgra with dither",
                );
            }
            (SurfaceFormat::Bgra8Unorm, bgra_bytes)
        } else {
            ctx.charge_over(bgra_bytes);
            ctx.log(
                LogLevel::Warn,
                format!("viewer pool: {bgra_bytes} bytes charged past the lease"),
            );
            (SurfaceFormat::Bgra8Unorm, bgra_bytes)
        };
        let spec = PoolSpec {
            generation: self.generations.next(),
            size,
            format,
            count,
        };
        self.views.insert(
            key,
            Meta {
                pool: Some((spec, bytes)),
            },
        );
        Some(spec)
    }

    /// Sizes the viewer's frame cache from the lease and reserves it there
    /// (contract 8 `frameCache`). Called on every lease change and with
    /// each viewer state; a share that did not change sends nothing.
    fn sync_frame_cache(&mut self, ctx: &mut PresentContext<'_>) {
        if self.config.source.is_none() {
            return;
        }
        ctx.release_frame_cache(self.frame_cache_reserved);
        self.frame_cache_reserved = 0;
        let granted = ctx.granted();
        let room = granted.saturating_sub(ctx.used());
        let bytes = viewer_frame_cache(granted, room);
        if granted > 0 && ctx.reserve_frame_cache(bytes) {
            self.frame_cache_reserved = bytes;
        }
        let bytes = bytes as usize;
        if self.frame_cache != Some(bytes) {
            self.frame_cache = Some(bytes);
            self.send(Command::FrameCache { bytes });
        }
    }

    fn forget(&mut self, ctx: &mut PresentContext<'_>, key: Key) {
        if let Some(Meta {
            pool: Some((_, bytes)),
        }) = self.views.remove(&key)
        {
            ctx.release_pool(bytes);
        }
    }
}

impl FramePresenter for BridgePresenter {
    fn viewer(
        &mut self,
        ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        state: ViewerState,
        revision: Option<Revision>,
    ) {
        if self.config.source.is_none() {
            if !self.told_no_source {
                self.told_no_source = true;
                ctx.log(
                    LogLevel::Info,
                    "viewer state received; this engine has no frame source yet, so the current engine keeps drawing the viewer",
                );
            }
            return;
        }
        let key = (port, session);
        // The pool comes first (the viewer must show something); the frame
        // cache takes what the lease has left after it.
        ctx.release_frame_cache(self.frame_cache_reserved);
        self.frame_cache_reserved = 0;
        let pool = self.plan_pool(ctx, key, &state);
        self.sync_frame_cache(ctx);
        self.send(Command::Configure {
            key,
            state: Box::new(state),
            revision,
            pool,
        });
    }

    fn release(&mut self, port: PortId, session: SessionId, release: FrameRelease) {
        self.send(Command::Release {
            key: (port, session),
            release,
        });
    }

    fn transport(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        command: TransportCommand,
    ) {
        self.send(Command::Transport {
            key: (port, session),
            command,
            at: pm_bridge::now_us(),
        });
    }

    fn audio(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        message: pm_protocol::clock::AudioMessage,
    ) {
        self.send(Command::Audio {
            key: (port, session),
            message,
        });
    }

    fn shader_fallback(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        message: pm_protocol::shader_fallback::ShaderFallbackMessage,
    ) {
        self.send(Command::ShaderFallback {
            key: (port, session),
            message,
        });
    }

    fn render(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        id: RequestId,
        request: FrameRequest,
        revision: Option<Revision>,
    ) -> Result<(), Box<EngineError>> {
        if self.config.source.is_none() {
            return Err(Box::new(EngineError::new(
                ErrorCode::Unsupported,
                ErrorScope::Request { id },
                "The new engine cannot render yet; the current engine handles this.",
            )));
        }
        self.send(Command::Render {
            key: (port, session),
            id,
            request: Box::new(request),
            revision,
        });
        Ok(())
    }

    fn compat(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        message: pm_protocol::snapshot::CompatMessage,
    ) {
        self.send(Command::Compat {
            key: (port, session),
            message,
        });
    }

    fn snapshot(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        host: PortId,
        id: RequestId,
        renderer: PortId,
        session: SessionId,
        request: pm_protocol::snapshot::SnapshotRequest,
        revision: Option<Revision>,
    ) -> Result<(), Box<EngineError>> {
        if self.config.source != Some(FrameSource::Document) {
            return Err(Box::new(EngineError::new(
                ErrorCode::Unsupported,
                ErrorScope::Request { id },
                "This engine draws no documents (it runs the bridge test source).",
            )));
        }
        let job = snapshot::SnapshotJob::new(
            host,
            id,
            (renderer, session),
            request,
            revision,
            render::now_us(),
        );
        self.send(Command::Snapshot(Box::new(job)));
        Ok(())
    }

    fn lease_changed(&mut self, ctx: &mut PresentContext<'_>) {
        self.sync_frame_cache(ctx);
    }

    fn session_closed(&mut self, ctx: &mut PresentContext<'_>, port: PortId, session: SessionId) {
        let key = (port, session);
        self.forget(ctx, key);
        self.send(Command::Close { key });
    }

    fn port_closed(&mut self, ctx: &mut PresentContext<'_>, port: PortId) {
        let keys: Vec<Key> = self.views.keys().filter(|k| k.0 == port).copied().collect();
        for key in keys {
            self.forget(ctx, key);
        }
        self.send(Command::ClosePort { port });
    }

    fn drain_host_requests(&mut self) -> Vec<pm_engine::jobs::HostRequest> {
        self.link.as_ref().map(|l| l.drain()).unwrap_or_default()
    }

    fn host_answer(
        &mut self,
        token: u64,
        answer: Result<pm_protocol::envelope::Response, EngineError>,
    ) {
        if let Some(link) = &self.link {
            link.answer(token, answer);
        }
    }

    fn host_event(&mut self, event: &pm_protocol::envelope::Event) {
        if let Some(link) = &self.link {
            link.event(event);
        }
    }

    fn renders_documents(&self) -> bool {
        self.config.source == Some(FrameSource::Document)
    }

    fn document(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        update: pm_engine::document::DocumentUpdate,
    ) {
        self.send(Command::Document {
            key: (port, session),
            update,
        });
    }

    fn definitions(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        set: &pm_protocol::definitions::DefinitionSetSnapshot,
        registered: &[(
            pm_protocol::definitions::DefinitionKind,
            pm_protocol::common::DefinitionId,
        )],
    ) {
        self.send(Command::Definitions {
            key: (port, session),
            set: std::sync::Arc::new(set.clone()),
            registered: registered.to_vec(),
        });
    }

    fn media(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        asset: &str,
        path: Option<std::path::PathBuf>,
    ) {
        self.send(Command::Media {
            key: (port, session),
            asset: asset.to_owned(),
            path,
        });
    }
}

impl Drop for BridgePresenter {
    fn drop(&mut self) {
        // Blender first: a running preview job is cancelled.
        if let Some(blender) = self.blender.take() {
            blender.shutdown();
        }
        let _ = self.tx.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_engine::lease::{Category, LeaseBudget, NoEviction};
    use pm_protocol::common::{ByteSize, HostMicros, Seq};
    use pm_protocol::lease::{Lease, LeaseReason};
    use std::sync::mpsc::Receiver;

    const MIB: u64 = 1 << 20;
    const DEFAULT: u64 = pm_render::media::DEFAULT_FRAME_CACHE as u64;

    fn presenter() -> (BridgePresenter, Receiver<Command>) {
        let config = PresenterConfig::from_env(EngineProcess::Interactive, |key| match key {
            BRIDGE_ENV => Some("test".into()),
            SOURCE_ENV => Some("document".into()),
            _ => None,
        })
        .unwrap();
        let (tx, rx) = channel();
        let presenter = BridgePresenter {
            config,
            tx,
            thread: None,
            stats: Arc::default(),
            views: BTreeMap::new(),
            generations: Generations::default(),
            told_no_source: false,
            link: None,
            blender: None,
            frame_cache: None,
            frame_cache_reserved: 0,
        };
        (presenter, rx)
    }

    fn grant(budget: &mut LeaseBudget, epoch: u64, bytes: u64) -> bool {
        let lease = Lease {
            process: EngineProcess::Interactive,
            epoch: Seq::new(epoch).unwrap(),
            bytes: ByteSize::new(bytes).unwrap(),
            reason: LeaseReason::MemoryPressure,
            ack_deadline_ms: 500,
        };
        budget
            .grant(lease, HostMicros::new(epoch).unwrap(), &mut NoEviction)
            .unwrap()
            .ack
    }

    /// The frame-cache shares the presenter sent, oldest first.
    fn sent(rx: &Receiver<Command>) -> Vec<u64> {
        rx.try_iter()
            .filter_map(|c| match c {
                Command::FrameCache { bytes } => Some(bytes as u64),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_share_is_a_quarter_of_the_lease_within_bounds() {
        assert_eq!(viewer_frame_cache(0, 0), DEFAULT);
        assert_eq!(viewer_frame_cache(100 * MIB, 100 * MIB), DEFAULT);
        assert_eq!(viewer_frame_cache(1024 * MIB, 1024 * MIB), 256 * MIB);
        // An 8 GB Mac's idle interactive lease (60 % of 4 GiB).
        assert_eq!(
            viewer_frame_cache(2458 * MIB, 2458 * MIB),
            VIEWER_FRAME_CACHE_MAX
        );
        assert_eq!(viewer_frame_cache(1024 * MIB, 10 * MIB), 10 * MIB);
    }

    /// The viewer's frame cache follows the lease: it is reserved in the
    /// ledger (so usage reports show it), shrinks with a shrinking lease
    /// (whose acknowledgement waits for it), and grows back.
    #[test]
    fn the_viewer_frame_cache_follows_the_lease() {
        let (mut p, rx) = presenter();
        let mut budget = LeaseBudget::new(EngineProcess::Interactive);
        let mut evict = NoEviction;
        // Before a grant: the default cache, nothing reserved.
        p.lease_changed(&mut PresentContext::new(&mut budget, &mut evict));
        assert_eq!(sent(&rx), vec![DEFAULT]);
        assert_eq!(budget.used_in(Category::FrameCache), 0);

        assert!(grant(&mut budget, 1, 4096 * MIB));
        p.lease_changed(&mut PresentContext::new(&mut budget, &mut evict));
        assert_eq!(sent(&rx), vec![VIEWER_FRAME_CACHE_MAX]);
        assert_eq!(budget.used_in(Category::FrameCache), VIEWER_FRAME_CACHE_MAX);

        // Memory pressure: the shrink is not acknowledged while the cache
        // holds more than fits, and is once the presenter resized it.
        assert!(!grant(&mut budget, 2, 400 * MIB));
        p.lease_changed(&mut PresentContext::new(&mut budget, &mut evict));
        assert_eq!(sent(&rx), vec![100 * MIB]);
        assert_eq!(budget.used_in(Category::FrameCache), 100 * MIB);
        let report = budget.poll_report(HostMicros::new(3).unwrap(), false);
        assert!(report.is_some_and(|r| r.ack));

        // An unchanged share sends nothing.
        p.lease_changed(&mut PresentContext::new(&mut budget, &mut evict));
        assert!(sent(&rx).is_empty());
        assert_eq!(budget.used_in(Category::FrameCache), 100 * MIB);

        assert!(grant(&mut budget, 4, 8192 * MIB));
        p.lease_changed(&mut PresentContext::new(&mut budget, &mut evict));
        assert_eq!(sent(&rx), vec![VIEWER_FRAME_CACHE_MAX]);
    }
}
