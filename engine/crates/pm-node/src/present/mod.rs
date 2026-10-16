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
//! Frames come from a frame source. Until pm-render draws real frames the
//! only source is the bridge test source, selected with
//! `POWERMOVE_ENGINE_FRAME_SOURCE=test-pattern` (tests and the S1 harness);
//! without a source the engine produces no viewer frames and the viewer
//! keeps the current engine's.

pub mod clock;
pub mod render;
pub mod schedule;
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
}

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
            },
            source: match env(SOURCE_ENV).as_deref() {
                Some("test-pattern") => Some(FrameSource::TestPattern),
                _ => None,
            },
        })
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
        let thread = pm_engine::qos::spawn_engine_thread(
            EngineProcess::Interactive,
            "pm-viewer-present",
            move || render::run(thread_config, rx, sink, thread_stats, thread_generations),
        )
        .map_err(|e| format!("could not start the viewer presenter: {e}"))?;
        Ok(Self {
            config,
            tx,
            thread: Some(thread),
            stats,
            views: BTreeMap::new(),
            generations,
            told_no_source: false,
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
        let pool = self.plan_pool(ctx, key, &state);
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
}

impl Drop for BridgePresenter {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
