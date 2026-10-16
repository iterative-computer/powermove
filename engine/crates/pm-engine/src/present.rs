//! Viewer presentation hooks (plan section 18; contract 2).
//!
//! The interactive engine's host installs a [`FramePresenter`] (pm-node's
//! frame-bridge presenter). The engine routes each renderer session's viewer
//! traffic to it: viewer state, frame releases, transport commands, viewer
//! and scrub frame requests, and session and port closes. The presenter owns
//! the surface pools, renders on its own thread, and sends descriptors,
//! clock samples, and frame responses straight to the renderer's port
//! through its own sink, never through this state machine: frames are not
//! paced by the JavaScript thread that feeds [`crate::Engine`].
//!
//! Pool memory is charged to the lease here, on the engine's thread, through
//! [`PresentContext`].

use pm_protocol::clock::TransportCommand;
use pm_protocol::common::{RequestId, Revision, SessionId};
use pm_protocol::error::EngineError;
use pm_protocol::frame::{FrameRelease, FrameRequest, ViewerState};
use pm_protocol::host::PortId;

use crate::document::DocumentUpdate;
use crate::engine::LogLevel;
use crate::lease::{Category, Evict, LeaseBudget};

/// What a presenter may touch synchronously while the engine handles a
/// message: the lease, and log lines.
pub struct PresentContext<'a> {
    pub(crate) lease: &'a mut LeaseBudget,
    pub(crate) evict: &'a mut dyn Evict,
    pub(crate) logs: Vec<(LogLevel, String)>,
}

impl<'a> PresentContext<'a> {
    /// A context over a lease (tests and hosts that drive a presenter
    /// directly).
    pub fn new(lease: &'a mut LeaseBudget, evict: &'a mut dyn Evict) -> Self {
        Self {
            lease,
            evict,
            logs: Vec::new(),
        }
    }

    /// Charges pool memory, evicting caches first when the lease is full.
    /// Returns false when it does not fit even then; the caller chooses a
    /// smaller pool or [`Self::charge_over`].
    pub fn charge_pool(&mut self, bytes: u64) -> bool {
        self.lease
            .charge(Category::SurfacePool, bytes, self.evict)
            .is_ok()
    }

    /// Charges pool memory past the lease: the viewer must show something,
    /// and the surface pool is the last thing evicted. The next usage report
    /// shows the overage.
    pub fn charge_over(&mut self, bytes: u64) {
        self.lease.charge_over(Category::SurfacePool, bytes);
    }

    pub fn release_pool(&mut self, bytes: u64) {
        self.lease.release(Category::SurfacePool, bytes);
    }

    pub fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.logs.push((level, message.into()));
    }

    pub fn take_logs(&mut self) -> Vec<(LogLevel, String)> {
        std::mem::take(&mut self.logs)
    }
}

/// Presents viewer frames for renderer sessions. Every call names the port
/// and the session that port owns (the engine checks ownership first).
pub trait FramePresenter: Send {
    /// The viewer's state changed. `revision` is the envelope's minimum
    /// document revision to render.
    fn viewer(
        &mut self,
        ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        state: ViewerState,
        revision: Option<Revision>,
    );

    /// The viewer finished with a frame (contract 2 `FrameRelease`).
    fn release(&mut self, port: PortId, session: SessionId, release: FrameRelease);

    /// Play, pause, seek, step, loop.
    fn transport(
        &mut self,
        ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        command: TransportCommand,
    );

    /// A viewer or scrub frame request. The presenter answers later on the
    /// port with `Response::Frame` (or an error) for `id`; an `Err` here is
    /// sent at once.
    fn render(
        &mut self,
        ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        id: RequestId,
        request: FrameRequest,
        revision: Option<Revision>,
    ) -> Result<(), Box<EngineError>>;

    /// The session closed: retire its pool.
    fn session_closed(&mut self, ctx: &mut PresentContext<'_>, port: PortId, session: SessionId);

    /// The port closed with every session on it.
    fn port_closed(&mut self, ctx: &mut PresentContext<'_>, port: PortId);

    /// Whether this presenter renders the session's document. When it
    /// does, the engine forwards every document change ([`Self::document`])
    /// and resolves the document's media with the host ([`Self::media`]).
    fn renders_documents(&self) -> bool {
        false
    }

    /// The session's document changed (a resync or a patch batch, in
    /// revision order; plan section 6, "The patch stream").
    fn document(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        _port: PortId,
        _session: SessionId,
        _update: DocumentUpdate,
    ) {
    }

    /// An asset of the session's document resolved to a file, or went
    /// offline (`None`: the host could not resolve it, or it left the
    /// document).
    fn media(
        &mut self,
        _ctx: &mut PresentContext<'_>,
        _port: PortId,
        _session: SessionId,
        _asset: &str,
        _path: Option<std::path::PathBuf>,
    ) {
    }
}
