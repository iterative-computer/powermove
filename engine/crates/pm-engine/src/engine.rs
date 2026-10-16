//! The engine process core: connections, version negotiation, sessions,
//! leases, slicing state, crash breadcrumbs, and the host calls an engine
//! makes (plan section 19; contracts 7, 8, 9, 12, 13, 14).
//!
//! [`Engine`] is a pure state machine. Its host (`pm-node` inside an
//! Electron utility process, or `pm-cli`) delivers bytes per port with
//! [`Engine::receive`], drives time with [`Engine::tick`], and carries out
//! the returned [`Action`]s: send bytes on a port, close a port, or log. It
//! never blocks on I/O except to read a resync file the host resolved.
//!
//! Ports are assigned by the host out of band ([`Engine::attach`]) with the
//! kind of peer they lead to, and the peer's `hello` must claim that role: a
//! renderer cannot pose as the host to grant leases, relay deadline signals,
//! or answer a `resolveHandle`. Sessions belong to the port that opened
//! them; no other port can address them.

use std::collections::{BTreeMap, BTreeSet};

use pm_protocol::codec::{self, CodecError};
use pm_protocol::common::{ContentHash, HostMicros, RequestId, Revision, SessionId};
use pm_protocol::definitions::{DefinitionSet, DefinitionsApplied};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Outstanding, Request, Response, SessionRole,
};
use pm_protocol::error::{
    CrashReport, EngineError, EngineProcess, ErrorCode, ErrorScope, FailureReport, Retry,
};
use pm_protocol::host::{
    CapabilityHandle, CapabilityKind, HandleAccess, HandleGrant, HandleResolved, MintHandle,
    PortId, ResolveHandle,
};
use pm_protocol::patch::{
    Admission, ConsistencyCheck, ConsistencyResult, DocumentEncoding, PatchBatch, PatchEncoding,
    PatchesApplied, Resync, ResyncSource, RevisionTracker,
};
use pm_protocol::slicing::{DeadlineSignal, SlicingPolicy};
use pm_protocol::version::{
    self, ContractRange, Hello, Negotiated, PeerRole, SOURCE_BUILD, WIRE_VERSION,
};
use serde::Serialize;

use crate::document::{DocumentUpdate, MediaAttempt, MediaTable, decode_canonical};
use crate::inflight::{FailedItems, InFlightTracker, WorkToken};
use crate::jobs::{AGENT_REVISION_WAIT_US, JobDocument, JobService, WaitingFrames, WorkNote};
use crate::lease::{Evict, LeaseBudget, NoEviction};
use crate::present::{FramePresenter, PresentContext};
use crate::slicing::JobSlicer;

/// What a port leads to. Assigned by the host when it hands the engine a
/// port; the peer's `hello` must claim a matching role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PeerKind {
    /// Host services (Electron main, the CLI host). One per launch.
    Host,
    /// A renderer window (or a headless document host worker).
    Renderer,
    /// The other engine process (the direct deadline-signal port).
    PeerEngine,
}

impl PeerKind {
    fn accepts(self, role: PeerRole) -> bool {
        match self {
            PeerKind::Host => matches!(role, PeerRole::Host | PeerRole::Cli),
            PeerKind::Renderer => matches!(role, PeerRole::Renderer | PeerRole::DocumentHost),
            PeerKind::PeerEngine => role == PeerRole::InteractiveEngine,
        }
    }
}

/// Severity of a log line the host should record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Something the engine's host must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Post these bytes (one encoded envelope) on the port.
    Send { port: PortId, bytes: Vec<u8> },
    /// Close the port; the engine has already forgotten it.
    Close { port: PortId },
    /// Record a line in the engine's log.
    Log { level: LogLevel, message: String },
}

/// Decodes canonical patch bodies into leaf patches (RFC 05). The model lane
/// provides the binary decoder; without one, patch batches are refused with
/// an `unsupported` failure and the session keeps its resynced revision.
pub trait PatchBodyDecoder: Send {
    fn decode(&self, encoding: PatchEncoding, body: &[u8]) -> Result<Vec<pm_model::Patch>, String>;
}

/// Engine construction parameters.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub process: EngineProcess,
    /// The protocol build id; drafts interoperate only between equal ids.
    pub build: String,
    /// Largest message this engine accepts.
    pub max_message_bytes: u32,
    /// The contract ranges offered and accepted.
    pub contracts: Vec<ContractRange>,
}

impl EngineConfig {
    pub fn new(process: EngineProcess) -> Self {
        Self {
            process,
            build: SOURCE_BUILD.to_owned(),
            max_message_bytes: codec::MAX_MESSAGE_BYTES as u32,
            contracts: version::local_ranges(),
        }
    }

    fn role(&self) -> PeerRole {
        match self.process {
            EngineProcess::Interactive => PeerRole::InteractiveEngine,
            EngineProcess::Job => PeerRole::JobEngine,
        }
    }
}

#[derive(Debug)]
enum ConnState {
    AwaitHello,
    AwaitWelcome,
    Open(Negotiated),
}

#[derive(Debug)]
struct Connection {
    kind: PeerKind,
    state: ConnState,
    /// Requests this engine sent on the connection.
    outstanding: Outstanding,
    peer: Option<PeerRole>,
}

struct Session {
    port: PortId,
    document: pm_protocol::common::DocumentId,
    role: SessionRole,
    revisions: RevisionTracker,
    /// The canonical document at the applied revision, once resynced.
    mirror: Option<pm_model::Document>,
    definitions: DefinitionSet,
    /// Patch batches could not be applied; the mirror is behind until a resync.
    stale: bool,
    /// The document's media as the host resolved it for this session
    /// (interactive sessions whose presenter renders documents).
    media: MediaTable,
}

/// A call this engine made to the host, keyed by its request id on the host port.
#[derive(Debug)]
enum HostCall {
    /// A renderer's resync whose body is in a cache file.
    ResyncFromFile {
        port: PortId,
        request: RequestId,
        session: SessionId,
        resync: Resync,
        length: u64,
    },
    Lease,
    Mint {
        token: u64,
    },
    /// A `mediaRead` handle for one of a session's assets (document host).
    MediaMint {
        session: SessionId,
        asset: String,
        generation: u64,
    },
    /// Its path, through `resolveHandle`.
    MediaResolve {
        session: SessionId,
        asset: String,
        generation: u64,
    },
    /// A call the job service asked for, answered to it by its token.
    Service {
        token: u64,
    },
}

/// The answer to a [`Engine::mint_handle`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintOutcome {
    pub token: u64,
    pub result: Result<HandleGrant, EngineError>,
}

/// A snapshot of the engine's state for diagnostics and tests.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub process: EngineProcess,
    pub build: String,
    pub ports: Vec<PortStatus>,
    pub sessions: Vec<SessionStatus>,
    pub lease_epoch: Option<u64>,
    pub lease_bytes: u64,
    pub used_bytes: u64,
    pub owed_ack: Option<u64>,
    pub failed_items: usize,
    pub in_flight: usize,
    pub playing: bool,
    pub slicing: Option<SlicingPolicy>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortStatus {
    pub port: u32,
    pub kind: PeerKind,
    pub open: bool,
    pub peer: Option<PeerRole>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub session: u64,
    pub port: u32,
    pub document: String,
    pub role: SessionRole,
    pub revision: u64,
    /// Canonical hash of the mirror, as hex, once resynced.
    pub hash: Option<String>,
    pub stale: bool,
    pub definitions_generation: Option<u64>,
}

/// One engine process.
pub struct Engine {
    config: EngineConfig,
    ports: BTreeMap<PortId, Connection>,
    sessions: BTreeMap<SessionId, Session>,
    next_session: SessionId,
    host: Option<PortId>,
    peer: Option<PortId>,
    lease: LeaseBudget,
    evict: Box<dyn Evict + Send>,
    slicer: Option<JobSlicer>,
    playing: bool,
    /// Viewer sessions whose transport plays (interactive engine): the
    /// first to start sends `playbackStarted`, the last to stop sends
    /// `playbackStopped` (contract 13), so the job engine slices its GPU
    /// work while the user watches.
    playing_viewers: BTreeSet<(PortId, SessionId)>,
    inflight: InFlightTracker,
    failed: FailedItems,
    host_calls: BTreeMap<RequestId, HostCall>,
    next_token: u64,
    minted: Vec<MintOutcome>,
    patch_decoder: Option<Box<dyn PatchBodyDecoder>>,
    presenter: Option<Box<dyn FramePresenter>>,
    /// Document updates for the presenter, handed over after each input.
    pending_updates: Vec<(SessionId, DocumentUpdate)>,
    /// The job engine's render and export service (`jobs`).
    jobs: Option<Box<dyn JobService>>,
    /// Shared snapshots of session mirrors handed to the service, by revision.
    job_docs: BTreeMap<SessionId, (Revision, std::sync::Arc<pm_model::Document>)>,
    /// Agent frame requests waiting for their revision.
    waiting_frames: Vec<WaitingFrames>,
    /// Breadcrumbs of the service's running work, by its token.
    job_work: BTreeMap<u64, WorkToken>,
    logged_once: BTreeSet<String>,
    overdue_logged: Option<u64>,
}

/// A decoded patch body.
enum Decoded {
    Ops(std::sync::Arc<[pm_model::Op]>),
    Patches(std::sync::Arc<[pm_model::Patch]>),
}

/// Collects actions while handling one input.
#[derive(Default)]
struct Out(Vec<Action>);

impl Out {
    fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.0.push(Action::Log {
            level,
            message: message.into(),
        });
    }
}

fn request_error(id: RequestId, code: ErrorCode, message: &str) -> EngineError {
    EngineError::new(code, ErrorScope::Request { id }, message)
}

fn request_name(request: &Request) -> &'static str {
    match request {
        Request::OpenSession(_) => "openSession",
        Request::CloseSession => "closeSession",
        Request::Resync(_) => "resync",
        Request::CheckConsistency(_) => "checkConsistency",
        Request::RenderFrame(_) => "renderFrame",
        Request::Evaluate(_) => "evaluate",
        Request::AgentFrames(_) => "agentFrames",
        Request::SubmitJob(_) => "submitJob",
        Request::CancelJobs(_) => "cancelJobs",
        Request::PushDefinitions(_) => "pushDefinitions",
        Request::GrantLease(_) => "grantLease",
        Request::RequestLease(_) => "requestLease",
        Request::MintHandle(_) => "mintHandle",
        Request::ResolveHandle(_) => "resolveHandle",
        Request::OutputDestination(_) => "outputDestination",
        Request::CommitOutput(_) => "commitOutput",
        Request::HardwareSessions(_) => "hardwareSessions",
        Request::BlenderRender(_) => "blenderRender",
        Request::BlenderStatus => "blenderStatus",
        Request::BlenderModel(_) => "blenderModel",
        Request::OpenProject(_) => "openProject",
        Request::CloseProject(_) => "closeProject",
        Request::Persist(_) => "persist",
        Request::RegisterExtension(_) => "registerExtension",
        Request::ToolCall(_) => "toolCall",
        Request::ProbeMedia(_) => "probeMedia",
    }
}

fn event_name(event: &Event) -> &'static str {
    match event {
        Event::ApplyPatches(_) => "applyPatches",
        Event::PatchesApplied(_) => "patchesApplied",
        Event::Frame(_) => "frame",
        Event::ReleaseFrame(_) => "releaseFrame",
        Event::SurfacePool(_) => "surfacePool",
        Event::Viewer(_) => "viewer",
        Event::Transport(_) => "transport",
        Event::Clock(_) => "clock",
        Event::Deadline(_) => "deadline",
        Event::Slicing(_) => "slicing",
        Event::Failure(_) => "failure",
        Event::Crash(_) => "crash",
        Event::InFlight(_) => "inFlight",
        Event::LeaseUsage(_) => "leaseUsage",
        Event::JobStatus(_) => "jobStatus",
        Event::BlenderProgress(_) => "blenderProgress",
        Event::BlenderSurface(_) => "blenderSurface",
        Event::Fact(_) => "fact",
    }
}

/// The canonical hash of a mirror (`null` for an empty document).
fn mirror_digest(mirror: &pm_model::Document) -> [u8; 32] {
    match mirror.root() {
        Some(root) => pm_model::hash::canonical_digest(root),
        None => pm_model::hash::canonical_digest(&pm_model::Value::Null),
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    pm_model::hash::hex(bytes)
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        let process = config.process;
        Self {
            config,
            ports: BTreeMap::new(),
            sessions: BTreeMap::new(),
            next_session: SessionId::new(1).expect("1 is safe"),
            host: None,
            peer: None,
            lease: LeaseBudget::new(process),
            evict: Box::new(NoEviction),
            slicer: (process == EngineProcess::Job).then(|| JobSlicer::new(SlicingPolicy::DEFAULT)),
            playing: false,
            playing_viewers: BTreeSet::new(),
            inflight: InFlightTracker::default(),
            failed: FailedItems::default(),
            host_calls: BTreeMap::new(),
            next_token: 1,
            minted: Vec::new(),
            patch_decoder: None,
            presenter: None,
            pending_updates: Vec::new(),
            jobs: None,
            job_docs: BTreeMap::new(),
            waiting_frames: Vec::new(),
            job_work: BTreeMap::new(),
            logged_once: BTreeSet::new(),
            overdue_logged: None,
        }
    }

    pub fn process(&self) -> EngineProcess {
        self.config.process
    }

    /// Installs the canonical patch body decoder (RFC 05).
    pub fn set_patch_decoder(&mut self, decoder: Box<dyn PatchBodyDecoder>) {
        self.patch_decoder = Some(decoder);
    }

    /// Installs the viewer presenter (the frame bridge, section 18). Only the
    /// interactive engine presents.
    pub fn set_presenter(&mut self, presenter: Box<dyn FramePresenter>) {
        self.presenter = Some(presenter);
    }

    pub fn has_presenter(&self) -> bool {
        self.presenter.is_some()
    }

    /// Installs the render and export service (contracts 12 and 13). Only
    /// the job engine runs jobs and agent frames.
    pub fn set_job_service(&mut self, service: Box<dyn JobService>) {
        if self.config.process == EngineProcess::Job {
            self.jobs = Some(service);
        }
    }

    pub fn has_job_service(&self) -> bool {
        self.jobs.is_some()
    }

    /// Sends what the job service produced off this thread: its responses,
    /// status events, and host calls. Its host calls `pump` when the service
    /// wakes it; `receive` and `tick` pump too.
    pub fn pump(&mut self) -> Vec<Action> {
        let mut out = Out::default();
        self.drain_jobs(&mut out);
        out.0
    }

    /// Runs `f` with the presenter and a context over the lease; its log
    /// lines join `out`.
    fn with_presenter<R>(
        &mut self,
        out: &mut Out,
        f: impl FnOnce(&mut dyn FramePresenter, &mut PresentContext<'_>) -> R,
    ) -> Option<R> {
        let presenter = self.presenter.as_mut()?;
        let mut ctx = PresentContext::new(&mut self.lease, self.evict.as_mut());
        let result = f(presenter.as_mut(), &mut ctx);
        for (level, message) in ctx.take_logs() {
            out.log(level, message);
        }
        Some(result)
    }

    /// Installs what frees memory when a lease shrinks (the caches).
    pub fn set_evictor(&mut self, evict: Box<dyn Evict + Send>) {
        self.evict = evict;
    }

    pub fn lease(&self) -> &LeaseBudget {
        &self.lease
    }

    pub fn lease_mut(&mut self) -> &mut LeaseBudget {
        &mut self.lease
    }

    /// The job engine's slice gate (contract 13); `None` in the interactive engine.
    pub fn slicer(&self) -> Option<&JobSlicer> {
        self.slicer.as_ref()
    }

    pub fn slicer_mut(&mut self) -> Option<&mut JobSlicer> {
        self.slicer.as_mut()
    }

    /// Whether the interactive engine is playing, as this engine knows it.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn failed_items(&self) -> &FailedItems {
        &self.failed
    }

    /// Takes a new port from the host. On the interactive engine a peer
    /// port is opened with a `hello` at once; every other port waits for
    /// the peer's `hello`.
    pub fn attach(&mut self, port: PortId, kind: PeerKind) -> Vec<Action> {
        let mut out = Out::default();
        if self.ports.contains_key(&port) {
            out.log(
                LogLevel::Warn,
                format!("port {} attached twice; replacing it", port.0),
            );
            self.forget_port(port, &mut out);
        }
        let opens =
            kind == PeerKind::PeerEngine && self.config.process == EngineProcess::Interactive;
        match kind {
            PeerKind::Host => {
                if let Some(old) = self.host.replace(port) {
                    out.log(
                        LogLevel::Warn,
                        format!("host port {} replaced by {}", old.0, port.0),
                    );
                    self.forget_port(old, &mut out);
                    out.0.push(Action::Close { port: old });
                }
            }
            PeerKind::PeerEngine => {
                if let Some(old) = self.peer.replace(port) {
                    self.forget_port(old, &mut out);
                    out.0.push(Action::Close { port: old });
                }
            }
            PeerKind::Renderer => {}
        }
        self.ports.insert(
            port,
            Connection {
                kind,
                state: if opens {
                    ConnState::AwaitWelcome
                } else {
                    ConnState::AwaitHello
                },
                outstanding: Outstanding::default(),
                peer: None,
            },
        );
        if opens {
            let hello = Hello {
                role: self.config.role(),
                wire: WIRE_VERSION,
                build: self.config.build.clone(),
                contracts: self.config.contracts.clone(),
                max_message_bytes: self.config.max_message_bytes,
            };
            self.send_raw(port, Envelope::new(Message::Hello(hello)), &mut out);
        }
        out.0
    }

    /// The port closed (the peer went away). Its sessions close with it.
    pub fn detach(&mut self, port: PortId) -> Vec<Action> {
        let mut out = Out::default();
        self.forget_port(port, &mut out);
        out.0
    }

    fn forget_port(&mut self, port: PortId, out: &mut Out) {
        if self.ports.remove(&port).is_none() {
            return;
        }
        let closed: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.port == port)
            .map(|(id, _)| *id)
            .collect();
        for id in &closed {
            self.sessions.remove(id);
        }
        self.with_presenter(out, |p, ctx| p.port_closed(ctx, port));
        self.viewers_stopped(|(p, _)| *p == port, out);
        self.waiting_frames.retain(|w| w.port != port);
        for id in &closed {
            self.job_docs.remove(id);
        }
        if let Some(jobs) = self.jobs.as_mut() {
            for id in &closed {
                jobs.session_closed(*id);
            }
            jobs.port_closed(port);
        }
        if !closed.is_empty() {
            out.log(
                LogLevel::Info,
                format!("port {} closed with {} session(s)", port.0, closed.len()),
            );
        }
        if self.host == Some(port) {
            self.host = None;
            // Calls to a host that is gone never complete: fail their callers.
            let calls = std::mem::take(&mut self.host_calls);
            for (_, call) in calls {
                self.fail_host_call(
                    call,
                    EngineError::new(
                        ErrorCode::Internal,
                        ErrorScope::Connection,
                        "The host connection closed before it answered.",
                    ),
                    out,
                );
            }
        }
        if self.peer == Some(port) {
            self.peer = None;
        }
        self.host_calls.retain(|_, call| match call {
            HostCall::ResyncFromFile { port: p, .. } => *p != port,
            _ => true,
        });
    }

    /// Handles one message's bytes from `port`.
    pub fn receive(&mut self, port: PortId, bytes: &[u8], now: HostMicros) -> Vec<Action> {
        let mut out = Out::default();
        let Some(conn) = self.ports.get(&port) else {
            out.log(
                LogLevel::Warn,
                format!("message on unknown port {}", port.0),
            );
            return out.0;
        };
        let kind = conn.kind;
        let envelope = match codec::decode(bytes) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.fail_connection(port, EngineError::from(error), &mut out);
                return out.0;
            }
        };
        match &self.ports[&port].state {
            ConnState::AwaitHello => self.on_hello(port, kind, envelope, &mut out),
            ConnState::AwaitWelcome => self.on_welcome(port, envelope, &mut out),
            ConnState::Open(negotiated) => {
                if !negotiated.permits(&envelope.message) {
                    let error = EngineError::new(
                        ErrorCode::UnsupportedVersion,
                        ErrorScope::Connection,
                        "The message belongs to a contract this connection did not negotiate.",
                    );
                    match envelope.message {
                        Message::Request { id, .. } => self.send_error(port, id, error, &mut out),
                        _ => out.log(LogLevel::Warn, error.message),
                    }
                    return out.0;
                }
                self.dispatch(port, kind, envelope, now, &mut out);
            }
        }
        self.flush_document_updates(now, &mut out);
        if self.jobs.is_some() {
            self.release_waiting_frames(now, &mut out);
            self.drain_jobs(&mut out);
        }
        out.0
    }

    fn on_hello(&mut self, port: PortId, kind: PeerKind, envelope: Envelope, out: &mut Out) {
        let Message::Hello(hello) = envelope.message else {
            self.fail_connection(
                port,
                EngineError::new(
                    ErrorCode::UnsupportedVersion,
                    ErrorScope::Connection,
                    "A connection starts with hello.",
                ),
                out,
            );
            return;
        };
        if !kind.accepts(hello.role) {
            self.fail_connection(
                port,
                EngineError::new(
                    ErrorCode::CapabilityRejected,
                    ErrorScope::Connection,
                    "The peer's role does not match the port the host assigned.",
                ),
                out,
            );
            return;
        }
        match version::accept(
            self.config.role(),
            &self.config.build,
            &self.config.contracts,
            self.config.max_message_bytes,
            &hello,
        ) {
            Ok((welcome, negotiated)) => {
                if let Some(conn) = self.ports.get_mut(&port) {
                    conn.state = ConnState::Open(negotiated);
                    conn.peer = Some(hello.role);
                }
                self.send_raw(port, Envelope::new(Message::Welcome(welcome)), out);
                if kind == PeerKind::Host {
                    // A new host gets the full breadcrumb, even if unchanged.
                    self.inflight.reset_reported();
                    self.flush_inflight(out);
                }
                out.log(
                    LogLevel::Info,
                    format!("port {} negotiated with {:?}", port.0, hello.role),
                );
            }
            Err(error) => self.fail_connection(port, EngineError::from(error), out),
        }
    }

    fn on_welcome(&mut self, port: PortId, envelope: Envelope, out: &mut Out) {
        match envelope.message {
            Message::Welcome(welcome) => {
                if welcome.role != PeerRole::JobEngine {
                    self.fail_connection(
                        port,
                        EngineError::new(
                            ErrorCode::CapabilityRejected,
                            ErrorScope::Connection,
                            "The peer port must lead to the job engine.",
                        ),
                        out,
                    );
                    return;
                }
                match version::complete(&self.config.build, &self.config.contracts, &welcome) {
                    Ok(negotiated) => {
                        if let Some(conn) = self.ports.get_mut(&port) {
                            conn.state = ConnState::Open(negotiated);
                            conn.peer = Some(welcome.role);
                        }
                        out.log(LogLevel::Info, format!("peer port {} negotiated", port.0));
                    }
                    Err(error) => {
                        let error = match error {
                            version::WelcomeError::Negotiation(n) => EngineError::from(n),
                            other => EngineError::new(
                                ErrorCode::UnsupportedVersion,
                                ErrorScope::Connection,
                                other.to_string(),
                            ),
                        };
                        self.fail_connection(port, error, out);
                    }
                }
            }
            Message::Error { error, .. } => {
                out.log(
                    LogLevel::Error,
                    format!("peer refused the handshake: {}", error.message),
                );
                self.forget_port(port, out);
                out.0.push(Action::Close { port });
            }
            _ => self.fail_connection(
                port,
                EngineError::new(
                    ErrorCode::UnsupportedVersion,
                    ErrorScope::Connection,
                    "Expected welcome.",
                ),
                out,
            ),
        }
    }

    /// Sends a connection-scope error and closes the port.
    fn fail_connection(&mut self, port: PortId, error: EngineError, out: &mut Out) {
        out.log(
            LogLevel::Warn,
            format!("closing port {}: {}", port.0, error.message),
        );
        let envelope = Envelope::new(Message::Error {
            id: RequestId::ZERO,
            error,
        });
        if let Ok(bytes) = codec::encode(&envelope) {
            out.0.push(Action::Send { port, bytes });
        }
        self.forget_port(port, out);
        out.0.push(Action::Close { port });
    }

    fn dispatch(
        &mut self,
        port: PortId,
        kind: PeerKind,
        envelope: Envelope,
        now: HostMicros,
        out: &mut Out,
    ) {
        let session = envelope.session;
        let revision = envelope.revision;
        match envelope.message {
            Message::Hello(_) | Message::Welcome(_) => {
                self.fail_connection(
                    port,
                    EngineError::new(
                        ErrorCode::InvalidMessage,
                        ErrorScope::Connection,
                        "The handshake already happened on this connection.",
                    ),
                    out,
                );
            }
            Message::Request { id, body } => {
                self.on_request(port, kind, id, body, session, revision, now, out)
            }
            Message::Response { .. } | Message::Error { .. } => {
                self.on_answer(port, envelope.message, now, out)
            }
            Message::Event { body } => self.on_event(port, kind, body, session, revision, now, out),
            Message::Cancel { id } => self.on_cancel(port, id, out),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_request(
        &mut self,
        port: PortId,
        kind: PeerKind,
        id: RequestId,
        body: Request,
        session: Option<SessionId>,
        // Read-your-writes (contract 9): viewer frames go to the presenter
        // with their minimum revision; evaluation and agent frames are not
        // served by this engine yet, and the other served requests carry
        // their revision in the body.
        revision: Option<Revision>,
        now: HostMicros,
        out: &mut Out,
    ) {
        if kind == PeerKind::PeerEngine {
            let error = request_error(
                id,
                ErrorCode::InvalidMessage,
                "The engines exchange only deadline signals on their direct port.",
            );
            self.send_error(port, id, error, out);
            return;
        }
        match body {
            Request::OpenSession(open) => self.open_session(port, id, open, out),
            Request::CloseSession => {
                let Some(sid) = self.owned_session(port, id, session, out) else {
                    return;
                };
                self.sessions.remove(&sid);
                self.with_presenter(out, |p, ctx| p.session_closed(ctx, port, sid));
                self.viewers_stopped(|key| *key == (port, sid), out);
                self.job_docs.remove(&sid);
                self.waiting_frames.retain(|w| w.session != sid);
                if let Some(jobs) = self.jobs.as_mut() {
                    jobs.session_closed(sid);
                }
                self.respond(port, id, Response::Ack, Some(sid), None, out);
            }
            Request::Resync(resync) => self.resync(port, id, session, resync, out),
            Request::CheckConsistency(check) => {
                self.check_consistency(port, id, session, check, out)
            }
            Request::PushDefinitions(update) => {
                let Some(sid) = self.owned_session(port, id, session, out) else {
                    return;
                };
                let result = self
                    .sessions
                    .get_mut(&sid)
                    .expect("owned")
                    .definitions
                    .apply(&update);
                match result {
                    Ok(()) => self.respond(
                        port,
                        id,
                        Response::DefinitionsApplied(DefinitionsApplied {
                            generation: update.generation,
                            failures: Vec::new(),
                        }),
                        Some(sid),
                        None,
                        out,
                    ),
                    Err(error) => {
                        let error =
                            request_error(id, ErrorCode::InvalidMessage, &error.to_string())
                                .with_retry(Retry::AfterResync);
                        self.send_error(port, id, error, out)
                    }
                }
            }
            Request::GrantLease(lease) => {
                if kind != PeerKind::Host {
                    let error = request_error(
                        id,
                        ErrorCode::CapabilityRejected,
                        "Only the host grants leases.",
                    );
                    self.send_error(port, id, error, out);
                    return;
                }
                match self.lease.grant(lease, now, self.evict.as_mut()) {
                    Ok(usage) => {
                        self.overdue_logged = None;
                        self.respond(port, id, Response::LeaseUsage(usage), None, None, out)
                    }
                    Err(error) => {
                        let error =
                            request_error(id, ErrorCode::InvalidMessage, &error.to_string());
                        self.send_error(port, id, error, out)
                    }
                }
            }
            Request::RenderFrame(request)
                if self.presenter.is_some()
                    && kind == PeerKind::Renderer
                    && matches!(
                        request.purpose,
                        pm_protocol::frame::FramePurpose::Viewer
                            | pm_protocol::frame::FramePurpose::Scrub
                    ) =>
            {
                let Some(sid) = self.owned_session(port, id, session, out) else {
                    return;
                };
                let result = self
                    .with_presenter(out, |p, ctx| {
                        p.render(ctx, port, sid, id, request, revision)
                    })
                    .unwrap_or(Ok(()));
                if let Err(error) = result {
                    self.send_error(port, id, *error, out);
                }
            }
            Request::SubmitJob(job) if self.jobs.is_some() && kind == PeerKind::Host => {
                self.submit_job(port, id, job, out)
            }
            Request::CancelJobs(cancel) if self.jobs.is_some() && kind == PeerKind::Host => {
                let cancelled = self.jobs.as_mut().expect("checked").cancel(&cancel);
                self.respond(
                    port,
                    id,
                    Response::JobsCancelled(cancelled),
                    None,
                    None,
                    out,
                );
            }
            Request::AgentFrames(request) if self.jobs.is_some() && kind == PeerKind::Host => {
                self.agent_frames(port, id, session, revision, request, now, out)
            }
            Request::ProbeMedia(probe) if self.jobs.is_some() && kind == PeerKind::Host => {
                let result = self
                    .jobs
                    .as_mut()
                    .expect("checked")
                    .probe_media(port, id, probe);
                if let Err(error) = result {
                    self.send_error(port, id, *error, out);
                }
            }
            Request::RenderFrame(_)
            | Request::Evaluate(_)
            | Request::AgentFrames(_)
            | Request::SubmitJob(_)
            | Request::CancelJobs(_)
            | Request::ProbeMedia(_) => {
                let name = request_name(&body);
                let error = request_error(
                    id,
                    ErrorCode::Unsupported,
                    "The new engine cannot render yet; the current engine handles this.",
                );
                self.log_once(out, format!("unsupported request {name}"));
                self.send_error(port, id, error, out);
            }
            other => {
                let name = request_name(&other);
                let error = request_error(
                    id,
                    ErrorCode::InvalidMessage,
                    "This request is served by the host, not by an engine.",
                );
                out.log(
                    LogLevel::Warn,
                    format!("refused host request {name} on port {}", port.0),
                );
                self.send_error(port, id, error, out);
            }
        }
    }

    fn open_session(&mut self, port: PortId, id: RequestId, open: OpenSession, out: &mut Out) {
        let role_fits = match self.config.process {
            EngineProcess::Interactive => open.role == SessionRole::Interactive,
            EngineProcess::Job => matches!(open.role, SessionRole::Job | SessionRole::JobSnapshot),
        };
        if open.process != self.config.process || !role_fits {
            let error = request_error(
                id,
                ErrorCode::InvalidMessage,
                "That session belongs in the other engine process.",
            );
            self.send_error(port, id, error, out);
            return;
        }
        let sid = self.next_session;
        let Some(next) = sid.next() else {
            let error = request_error(id, ErrorCode::Internal, "Session ids are exhausted.");
            self.send_error(port, id, error, out);
            return;
        };
        self.next_session = next;
        self.sessions.insert(
            sid,
            Session {
                port,
                document: open.document,
                role: open.role,
                revisions: RevisionTracker::new(Revision::ZERO),
                mirror: None,
                definitions: DefinitionSet::default(),
                stale: false,
                media: MediaTable::default(),
            },
        );
        self.respond(
            port,
            id,
            Response::SessionOpened { session: sid },
            Some(sid),
            None,
            out,
        );
    }

    /// The session named by a request, if it exists and belongs to `port`;
    /// otherwise answers with `unknownSession` and returns `None`.
    fn owned_session(
        &mut self,
        port: PortId,
        id: RequestId,
        session: Option<SessionId>,
        out: &mut Out,
    ) -> Option<SessionId> {
        match session {
            Some(sid) if self.sessions.get(&sid).is_some_and(|s| s.port == port) => Some(sid),
            _ => {
                let error = EngineError::new(
                    ErrorCode::UnknownSession,
                    match session {
                        Some(session) => ErrorScope::Session { session },
                        None => ErrorScope::Request { id },
                    },
                    "The session does not exist on this connection; open a new one.",
                )
                .with_retry(Retry::AfterResync);
                self.send_error(port, id, error, out);
                None
            }
        }
    }

    fn resync(
        &mut self,
        port: PortId,
        id: RequestId,
        session: Option<SessionId>,
        resync: Resync,
        out: &mut Out,
    ) {
        let Some(sid) = self.owned_session(port, id, session, out) else {
            return;
        };
        if self.sessions[&sid].document != resync.document
            && self.sessions[&sid].role == SessionRole::Interactive
            && resync.document.is_valid()
        {
            // The window opened another project: an interactive session
            // follows its window, and a resync replaces the whole mirror.
            let session = self.sessions.get_mut(&sid).expect("owned");
            session.document = resync.document.clone();
            session.media.clear();
        }
        if self.sessions[&sid].document != resync.document {
            let error = request_error(
                id,
                ErrorCode::InvalidMessage,
                "The resync names a different document than the session.",
            );
            self.send_error(port, id, error, out);
            return;
        }
        match &resync.source {
            ResyncSource::Inline { body } => {
                let body = body.0.clone();
                let result = self.install_mirror(sid, &resync, &body);
                self.finish_resync(port, id, sid, &resync, result, out);
            }
            ResyncSource::CacheFile { handle, length } => {
                let Some(host) = self.open_host() else {
                    let error = request_error(
                        id,
                        ErrorCode::Internal,
                        "The engine has no host connection to open the resync file.",
                    )
                    .with_retry(Retry::After { ms: 250 });
                    self.send_error(port, id, error, out);
                    return;
                };
                let request = Request::ResolveHandle(ResolveHandle {
                    handle: handle.clone(),
                    session: sid,
                });
                let length = length.get();
                if let Some(call_id) = self.call_host(host, request, Some(sid), out) {
                    self.host_calls.insert(
                        call_id,
                        HostCall::ResyncFromFile {
                            port,
                            request: id,
                            session: sid,
                            resync,
                            length,
                        },
                    );
                }
            }
        }
    }

    /// Parses a resync body, checks its hash, and replaces the mirror.
    fn install_mirror(
        &mut self,
        sid: SessionId,
        resync: &Resync,
        body: &[u8],
    ) -> Result<(), (ErrorCode, String)> {
        let value = match resync.encoding {
            DocumentEncoding::CanonicalJsonV1 => {
                pm_model::json::parse_bytes(body).map_err(|e| {
                    (
                        ErrorCode::InvalidMessage,
                        format!("The resync body is not JSON ({e})."),
                    )
                })?
            }
            DocumentEncoding::CanonicalBinaryV1 => {
                return Err((
                    ErrorCode::Unsupported,
                    "The canonical binary document encoding is not readable yet; resync as canonical JSON.".into(),
                ));
            }
        };
        let shared = self
            .presents_documents()
            .then(|| std::sync::Arc::new(value.clone()));
        let mirror = pm_model::Document::new(Some(value), resync.revision.get());
        let digest = mirror_digest(&mirror);
        if ContentHash(digest) != resync.hash {
            return Err((
                ErrorCode::InvalidMessage,
                "The resync body does not match its canonical hash; send it again.".into(),
            ));
        }
        let session = self.sessions.get_mut(&sid).ok_or((
            ErrorCode::UnknownSession,
            "The session closed during the resync.".into(),
        ))?;
        session.revisions.resynced(resync);
        session.mirror = Some(mirror);
        session.stale = false;
        if let Some(root) = shared {
            self.pending_updates.push((
                sid,
                DocumentUpdate::Replace {
                    root: Some(root),
                    revision: resync.revision,
                },
            ));
        }
        Ok(())
    }

    /// Whether the presenter renders session documents (the document host
    /// forwards updates and resolves media only then).
    fn presents_documents(&self) -> bool {
        self.presenter
            .as_ref()
            .is_some_and(|p| p.renders_documents())
    }

    /// Hands queued document updates to the presenter and starts resolving
    /// the media they introduced.
    fn flush_document_updates(&mut self, now: HostMicros, out: &mut Out) {
        if self.pending_updates.is_empty() {
            return;
        }
        let updates = std::mem::take(&mut self.pending_updates);
        for (sid, update) in updates {
            let Some(port) = self.sessions.get(&sid).map(|s| s.port) else {
                continue;
            };
            self.with_presenter(out, |p, ctx| p.document(ctx, port, sid, update));
            self.sync_media(sid, now, out);
        }
    }

    /// Reconciles a session's media with its mirror and asks the host for
    /// what is new or changed.
    fn sync_media(&mut self, sid: SessionId, now: HostMicros, out: &mut Out) {
        let Some(session) = self.sessions.get_mut(&sid) else {
            return;
        };
        let port = session.port;
        let work = session
            .media
            .reconcile(session.mirror.as_ref().and_then(|m| m.root()));
        for asset in work.removed {
            self.with_presenter(out, |p, ctx| p.media(ctx, port, sid, &asset, None));
        }
        for attempt in work.start {
            self.start_media(sid, attempt, now, out);
        }
    }

    fn start_media(
        &mut self,
        sid: SessionId,
        attempt: MediaAttempt,
        now: HostMicros,
        out: &mut Out,
    ) {
        let request = Request::MintHandle(MintHandle {
            kind: CapabilityKind::MediaRead {
                asset: attempt.asset.as_str().into(),
            },
            session: sid,
        });
        let called = self
            .open_host()
            .and_then(|host| self.call_host(host, request, Some(sid), out));
        match called {
            Some(id) => {
                self.host_calls.insert(
                    id,
                    HostCall::MediaMint {
                        session: sid,
                        asset: attempt.asset,
                        generation: attempt.generation,
                    },
                );
            }
            None => self.media_failed(sid, &attempt.asset, attempt.generation, now, out),
        }
    }

    fn media_failed(
        &mut self,
        sid: SessionId,
        asset: &str,
        generation: u64,
        now: HostMicros,
        out: &mut Out,
    ) {
        let Some(session) = self.sessions.get_mut(&sid) else {
            return;
        };
        let port = session.port;
        if session.media.failed(asset, generation, now.get()) {
            self.with_presenter(out, |p, ctx| p.media(ctx, port, sid, asset, None));
        }
    }

    /// Restarts media whose retry is due.
    fn retry_media(&mut self, now: HostMicros, out: &mut Out) {
        let due: Vec<(SessionId, MediaAttempt)> = self
            .sessions
            .iter_mut()
            .flat_map(|(sid, s)| {
                s.media
                    .due(now.get())
                    .into_iter()
                    .map(|a| (*sid, a))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (sid, attempt) in due {
            self.start_media(sid, attempt, now, out);
        }
    }

    fn finish_resync(
        &mut self,
        port: PortId,
        id: RequestId,
        sid: SessionId,
        resync: &Resync,
        result: Result<(), (ErrorCode, String)>,
        out: &mut Out,
    ) {
        match result {
            Ok(()) => self.respond(
                port,
                id,
                Response::PatchesApplied(PatchesApplied {
                    document: resync.document.clone(),
                    revision: resync.revision,
                }),
                Some(sid),
                Some(resync.revision),
                out,
            ),
            Err((code, message)) => {
                let error = request_error(id, code, &message);
                self.send_error(port, id, error, out)
            }
        }
    }

    fn check_consistency(
        &mut self,
        port: PortId,
        id: RequestId,
        session: Option<SessionId>,
        check: ConsistencyCheck,
        out: &mut Out,
    ) {
        let Some(sid) = self.owned_session(port, id, session, out) else {
            return;
        };
        let state = &self.sessions[&sid];
        let applied = state.revisions.applied();
        let mirror = match &state.mirror {
            Some(mirror) if applied == check.revision && !state.stale => mirror,
            _ => {
                let error = request_error(
                    id,
                    ErrorCode::StaleRevision,
                    "The engine is not at that revision; resync before checking.",
                )
                .with_retry(Retry::AfterResync)
                .with_detail(pm_protocol::error::ErrorDetail::StaleRevision {
                    applied,
                    wanted: check.revision,
                });
                self.send_error(port, id, error, out);
                return;
            }
        };
        let hash = ContentHash(mirror_digest(mirror));
        let result = ConsistencyResult {
            revision: applied,
            matches: hash == check.hash,
            hash,
        };
        self.respond(
            port,
            id,
            Response::Consistency(result),
            Some(sid),
            Some(applied),
            out,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn on_event(
        &mut self,
        port: PortId,
        kind: PeerKind,
        body: Event,
        session: Option<SessionId>,
        revision: Option<Revision>,
        now: HostMicros,
        out: &mut Out,
    ) {
        let name = event_name(&body);
        match (body, kind) {
            (Event::ApplyPatches(batch), PeerKind::Renderer | PeerKind::Host) => {
                self.apply_patches(port, session, batch, out)
            }
            (Event::Slicing(policy), PeerKind::Host) => match self.slicer.as_mut() {
                Some(slicer) => {
                    slicer.set_policy(policy);
                    if let Some(jobs) = self.jobs.as_mut() {
                        jobs.policy(&policy);
                    }
                }
                None => self.log_once(out, "slicing policy sent to the interactive engine".into()),
            },
            (Event::Deadline(signal), PeerKind::Host | PeerKind::PeerEngine) => {
                self.on_deadline(&signal, now, out)
            }
            (Event::Crash(report), PeerKind::Host) => self.on_crash_report(&report, out),
            (
                event @ (Event::ReleaseFrame(_) | Event::Viewer(_) | Event::Transport(_)),
                PeerKind::Renderer,
            ) if self.presenter.is_some() => {
                let Some(sid) =
                    session.filter(|sid| self.sessions.get(sid).is_some_and(|s| s.port == port))
                else {
                    out.log(
                        LogLevel::Warn,
                        format!("{name} for an unknown session on port {}", port.0),
                    );
                    return;
                };
                let transport = match &event {
                    Event::Transport(command) => Some(*command),
                    _ => None,
                };
                self.with_presenter(out, |p, ctx| match event {
                    Event::Viewer(state) => p.viewer(ctx, port, sid, state, revision),
                    Event::ReleaseFrame(release) => p.release(port, sid, release),
                    Event::Transport(command) => p.transport(ctx, port, sid, command),
                    _ => {}
                });
                if let Some(command) = transport {
                    self.viewer_transport(port, sid, command, now, out);
                }
            }
            (
                Event::ReleaseFrame(_) | Event::Viewer(_) | Event::Transport(_),
                PeerKind::Renderer,
            )
            | (Event::BlenderProgress(_) | Event::BlenderSurface(_), PeerKind::Host) => self
                .log_once(
                    out,
                    format!("event {name} is not handled by this engine yet"),
                ),
            (_, kind) => out.log(
                LogLevel::Warn,
                format!("ignored event {name} from a {kind:?} port {}", port.0),
            ),
        }
    }

    /// Frame budget the job engine aligns to while a viewer plays. Viewers do
    /// not report their display's refresh rate yet; 60 Hz is the common case
    /// and only bounds how long alignment trusts a silent engine.
    const VIEWER_FRAME_BUDGET_US: u32 = 16_667;

    /// A viewer's transport changed: tell the job engine when playback
    /// starts or stops anywhere in this engine (contract 13).
    fn viewer_transport(
        &mut self,
        port: PortId,
        sid: SessionId,
        command: pm_protocol::clock::TransportCommand,
        now: HostMicros,
        out: &mut Out,
    ) {
        use pm_protocol::clock::TransportCommand as T;
        let rate = match command {
            T::Play { rate } if rate.num != 0 => Some(rate),
            T::PlayAround { .. } => Some(pm_protocol::common::PlaybackRate::NORMAL),
            T::Play { .. } | T::Pause => None,
            _ => return,
        };
        let was = !self.playing_viewers.is_empty();
        match rate {
            Some(rate) => {
                self.playing_viewers.insert((port, sid));
                if !was {
                    self.emit_deadline(
                        DeadlineSignal::PlaybackStarted {
                            at: now,
                            rate,
                            frame_budget_us: Self::VIEWER_FRAME_BUDGET_US,
                        },
                        out,
                    );
                }
            }
            None => {
                self.playing_viewers.remove(&(port, sid));
                if was && self.playing_viewers.is_empty() {
                    self.emit_deadline(DeadlineSignal::PlaybackStopped { at: now }, out);
                }
            }
        }
    }

    /// Viewers matching `gone` closed: playback stops when none plays.
    fn viewers_stopped(&mut self, gone: impl Fn(&(PortId, SessionId)) -> bool, out: &mut Out) {
        let was = !self.playing_viewers.is_empty();
        self.playing_viewers.retain(|key| !gone(key));
        if was && self.playing_viewers.is_empty() {
            let at = crate::clock::host_now();
            self.emit_deadline(DeadlineSignal::PlaybackStopped { at }, out);
        }
    }

    fn on_deadline(&mut self, signal: &DeadlineSignal, now: HostMicros, out: &mut Out) {
        let Some(slicer) = self.slicer.as_mut() else {
            self.log_once(
                out,
                "deadline signals sent to the interactive engine".into(),
            );
            return;
        };
        slicer.on_signal(signal);
        if let Some(jobs) = self.jobs.as_mut() {
            jobs.signal(signal);
        }
        match signal {
            DeadlineSignal::PlaybackStarted { .. } => self.playing = true,
            DeadlineSignal::PlaybackStopped { .. } => self.playing = false,
            _ => {}
        }
        let _ = now;
    }

    fn on_crash_report(&mut self, report: &CrashReport, out: &mut Out) {
        let added = self.failed.apply(report);
        if added > 0 {
            out.log(
                LogLevel::Warn,
                format!("{added} item(s) marked failed by crash attribution"),
            );
        }
    }

    fn apply_patches(
        &mut self,
        port: PortId,
        session: Option<SessionId>,
        batch: PatchBatch,
        out: &mut Out,
    ) {
        let Some(sid) =
            session.filter(|sid| self.sessions.get(sid).is_some_and(|s| s.port == port))
        else {
            out.log(
                LogLevel::Warn,
                format!("patches for an unknown session on port {}", port.0),
            );
            return;
        };
        let state = &self.sessions[&sid];
        if state.document != batch.document {
            out.log(LogLevel::Warn, "patches for another document; ignored");
            return;
        }
        let admission = state.revisions.admit(&batch);
        match admission {
            Ok(Admission::Duplicate) => {}
            Err(gap) => {
                let error = EngineError::new(
                    ErrorCode::StaleRevision,
                    ErrorScope::Session { session: sid },
                    "Patches skipped a revision; the engine needs a resync.",
                )
                .with_retry(Retry::AfterResync)
                .with_detail(pm_protocol::error::ErrorDetail::StaleRevision {
                    applied: gap.applied,
                    wanted: gap.base,
                });
                self.sessions.get_mut(&sid).expect("exists").stale = true;
                self.report_failure(port, sid, gap.applied, error, out);
            }
            Ok(Admission::Apply) => {
                // A host-installed decoder (tests) produces leaf patches; the
                // built-in one decodes canonicalV1 into operations.
                let decoded = match &self.patch_decoder {
                    Some(decoder) => decoder
                        .decode(batch.encoding, &batch.body.0)
                        .map(|patches| Decoded::Patches(patches.into())),
                    None => decode_canonical(batch.encoding, &batch.body.0)
                        .map(|ops| Decoded::Ops(ops.into())),
                };
                let forward = self.presents_documents();
                let session = self.sessions.get_mut(&sid).expect("exists");
                let applied = session.revisions.applied();
                let result = match (&decoded, session.mirror.as_mut()) {
                    (Ok(Decoded::Patches(patches)), Some(mirror)) => mirror
                        .apply_at(patches, batch.revision.get())
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    (Ok(Decoded::Ops(ops)), Some(mirror)) => mirror
                        .apply_ops_at(ops, batch.revision.get())
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    (Ok(_), None) => Err("no document yet; resync first".into()),
                    (Err(e), _) => Err(e.clone()),
                };
                match result {
                    Ok(()) => {
                        session.revisions.commit(&batch);
                        session.stale = false;
                        if forward {
                            let update = match decoded {
                                Ok(Decoded::Ops(ops)) => DocumentUpdate::Ops {
                                    ops,
                                    revision: batch.revision,
                                },
                                Ok(Decoded::Patches(patches)) => DocumentUpdate::Patches {
                                    patches,
                                    revision: batch.revision,
                                },
                                Err(_) => unreachable!("applied"),
                            };
                            self.pending_updates.push((sid, update));
                        }
                        let applied = PatchesApplied {
                            document: batch.document.clone(),
                            revision: batch.revision,
                        };
                        let envelope = Envelope::new(Message::Event {
                            body: Event::PatchesApplied(applied),
                        })
                        .with_session(sid)
                        .with_revision(batch.revision);
                        self.send_raw(port, envelope, out);
                    }
                    Err(reason) => {
                        session.stale = true;
                        let error = EngineError::new(
                            ErrorCode::InvalidMessage,
                            ErrorScope::Session { session: sid },
                            format!("A patch batch did not apply ({reason}); resync."),
                        )
                        .with_retry(Retry::AfterResync);
                        self.report_failure(port, sid, applied, error, out);
                    }
                }
            }
        }
    }

    fn report_failure(
        &mut self,
        port: PortId,
        sid: SessionId,
        revision: Revision,
        error: EngineError,
        out: &mut Out,
    ) {
        let envelope = Envelope::new(Message::Event {
            body: Event::Failure(FailureReport { revision, error }),
        })
        .with_session(sid)
        .with_revision(revision);
        self.send_raw(port, envelope, out);
    }

    fn on_cancel(&mut self, port: PortId, id: RequestId, out: &mut Out) {
        // Everything but a resync from a file is answered synchronously, so a
        // cancel normally arrives after its answer and has nothing to do.
        let pending = self
            .host_calls
            .iter()
            .find_map(|(call_id, call)| match call {
                HostCall::ResyncFromFile {
                    port: p, request, ..
                } if *p == port && *request == id => Some(*call_id),
                _ => None,
            });
        if let Some(call_id) = pending {
            self.host_calls.remove(&call_id);
            let error = request_error(id, ErrorCode::Cancelled, "The resync was cancelled.");
            self.send_error(port, id, error, out);
        }
    }

    fn on_answer(&mut self, port: PortId, message: Message, now: HostMicros, out: &mut Out) {
        let Some(conn) = self.ports.get_mut(&port) else {
            return;
        };
        let id = match conn.outstanding.receive(&message) {
            Ok(Some(id)) => id,
            Ok(None) => return,
            Err(error) => {
                out.log(
                    LogLevel::Warn,
                    format!("unexpected answer on port {}: {error}", port.0),
                );
                return;
            }
        };
        if Some(port) != self.host {
            return;
        }
        let Some(call) = self.host_calls.remove(&id) else {
            return;
        };
        match message {
            Message::Response { body, .. } => self.complete_host_call(call, body, now, out),
            Message::Error { error, .. } => self.fail_host_call(call, error, out),
            _ => {}
        }
    }

    fn complete_host_call(
        &mut self,
        call: HostCall,
        body: Response,
        now: HostMicros,
        out: &mut Out,
    ) {
        match (call, body) {
            (
                HostCall::ResyncFromFile {
                    port,
                    request,
                    session,
                    resync,
                    length,
                },
                Response::HandleResolved(resolved),
            ) => {
                let result = self
                    .read_resync_file(&resync, &resolved, length)
                    .and_then(|body| self.install_mirror(session, &resync, &body));
                if self.ports.contains_key(&port) {
                    self.finish_resync(port, request, session, &resync, result, out);
                }
            }
            (HostCall::Lease, Response::Lease(lease)) => {
                match self.lease.grant(lease, now, self.evict.as_mut()) {
                    Ok(usage) => {
                        if let Some(host) = self.open_host() {
                            let envelope = Envelope::new(Message::Event {
                                body: Event::LeaseUsage(usage),
                            });
                            self.send_raw(host, envelope, out);
                        }
                    }
                    Err(error) => out.log(LogLevel::Warn, format!("lease answer refused: {error}")),
                }
            }
            (HostCall::Mint { token }, Response::Handle(grant)) => self.minted.push(MintOutcome {
                token,
                result: Ok(grant),
            }),
            (
                HostCall::MediaMint {
                    session,
                    asset,
                    generation,
                },
                Response::Handle(grant),
            ) => {
                let wanted = self
                    .sessions
                    .get_mut(&session)
                    .is_some_and(|s| s.media.minted(&asset, generation));
                if !wanted {
                    return;
                }
                let request = Request::ResolveHandle(ResolveHandle {
                    handle: grant.handle,
                    session,
                });
                let called = self
                    .open_host()
                    .and_then(|host| self.call_host(host, request, Some(session), out));
                match called {
                    Some(id) => {
                        self.host_calls.insert(
                            id,
                            HostCall::MediaResolve {
                                session,
                                asset,
                                generation,
                            },
                        );
                    }
                    None => self.media_failed(session, &asset, generation, now, out),
                }
            }
            (
                HostCall::MediaResolve {
                    session,
                    asset,
                    generation,
                },
                Response::HandleResolved(resolved),
            ) => {
                let readable = matches!(
                    resolved.access,
                    HandleAccess::Read | HandleAccess::ReadWrite
                ) && matches!(&resolved.kind, CapabilityKind::MediaRead { asset: a } if a.as_str() == asset);
                if !readable {
                    out.log(
                        LogLevel::Warn,
                        format!("the host resolved asset {asset} to a handle of another kind"),
                    );
                    self.media_failed(session, &asset, generation, now, out);
                    return;
                }
                let path = std::path::PathBuf::from(&resolved.path);
                let Some(state) = self.sessions.get_mut(&session) else {
                    return;
                };
                let port = state.port;
                if state.media.resolved(&asset, generation, path.clone()) {
                    self.with_presenter(out, |p, ctx| {
                        p.media(ctx, port, session, &asset, Some(path))
                    });
                }
            }
            (HostCall::Service { token }, body) => {
                if let Some(jobs) = self.jobs.as_mut() {
                    jobs.host_answer(token, Ok(body));
                }
            }
            (call, body) => out.log(
                LogLevel::Error,
                format!("host answered {call:?} with {:?}", body.kind()),
            ),
        }
    }

    fn fail_host_call(&mut self, call: HostCall, error: EngineError, out: &mut Out) {
        match call {
            HostCall::ResyncFromFile { port, request, .. } => {
                if self.ports.contains_key(&port) {
                    let error = EngineError {
                        scope: ErrorScope::Request { id: request },
                        ..error
                    };
                    self.send_error(port, request, error, out);
                }
            }
            HostCall::Lease => out.log(
                LogLevel::Warn,
                format!("lease request refused: {}", error.message),
            ),
            HostCall::Mint { token } => self.minted.push(MintOutcome {
                token,
                result: Err(error),
            }),
            HostCall::MediaMint {
                session,
                asset,
                generation,
            }
            | HostCall::MediaResolve {
                session,
                asset,
                generation,
            } => {
                if error.code != ErrorCode::MediaOffline {
                    self.log_once(
                        out,
                        format!(
                            "media for the viewer could not be resolved: {}",
                            error.message
                        ),
                    );
                }
                let now = crate::clock::host_now();
                self.media_failed(session, &asset, generation, now, out);
            }
            HostCall::Service { token } => {
                if let Some(jobs) = self.jobs.as_mut() {
                    jobs.host_answer(token, Err(error));
                }
            }
        }
    }

    fn read_resync_file(
        &self,
        resync: &Resync,
        resolved: &HandleResolved,
        length: u64,
    ) -> Result<Vec<u8>, (ErrorCode, String)> {
        let ResyncSource::CacheFile { handle, .. } = &resync.source else {
            return Err((ErrorCode::Internal, "not a file resync".into()));
        };
        let readable = matches!(
            resolved.access,
            HandleAccess::Read | HandleAccess::ReadWrite
        ) && matches!(resolved.kind, CapabilityKind::Cache { .. });
        if &resolved.handle != handle || !readable {
            return Err((
                ErrorCode::CapabilityRejected,
                "The resync handle does not name a readable cache file.".into(),
            ));
        }
        let bytes = std::fs::read(&resolved.path).map_err(|e| {
            (
                ErrorCode::CapabilityRejected,
                format!("The resync file could not be read ({}).", e.kind()),
            )
        })?;
        if bytes.len() as u64 != length {
            return Err((
                ErrorCode::InvalidMessage,
                format!(
                    "The resync file holds {} bytes, not the {length} announced.",
                    bytes.len()
                ),
            ));
        }
        Ok(bytes)
    }

    /// The host port, if its handshake finished.
    fn open_host(&self) -> Option<PortId> {
        let host = self.host?;
        matches!(self.ports.get(&host)?.state, ConnState::Open(_)).then_some(host)
    }

    fn call_host(
        &mut self,
        host: PortId,
        request: Request,
        session: Option<SessionId>,
        out: &mut Out,
    ) -> Option<RequestId> {
        let conn = self.ports.get_mut(&host)?;
        let id = match conn.outstanding.send(&request) {
            Ok(id) => id,
            Err(error) => {
                out.log(LogLevel::Error, format!("cannot call the host: {error}"));
                return None;
            }
        };
        let mut envelope = Envelope::new(Message::Request { id, body: request });
        envelope.session = session;
        self.send_raw(host, envelope, out);
        Some(id)
    }

    /// Asks the host for a lease large enough for `extra` more bytes. The
    /// host answers with a new lease, applied when it arrives.
    pub fn request_lease(&mut self, extra: u64) -> Vec<Action> {
        let mut out = Out::default();
        let Some(host) = self.open_host() else {
            out.log(LogLevel::Warn, "no host to ask for a lease");
            return out.0;
        };
        let request = Request::RequestLease(self.lease.request_more(extra));
        if let Some(id) = self.call_host(host, request, None, &mut out) {
            self.host_calls.insert(id, HostCall::Lease);
        }
        out.0
    }

    /// Asks the host to mint a handle for `session`. The outcome appears in
    /// [`Engine::take_minted`] with the returned token.
    pub fn mint_handle(&mut self, session: SessionId, kind: CapabilityKind) -> (u64, Vec<Action>) {
        let mut out = Out::default();
        let token = self.next_token;
        self.next_token += 1;
        match self.open_host() {
            Some(host) => {
                let request = Request::MintHandle(MintHandle { kind, session });
                if let Some(id) = self.call_host(host, request, None, &mut out) {
                    self.host_calls.insert(id, HostCall::Mint { token });
                }
            }
            None => self.minted.push(MintOutcome {
                token,
                result: Err(EngineError::new(
                    ErrorCode::Internal,
                    ErrorScope::Connection,
                    "The engine has no host connection.",
                )),
            }),
        }
        (token, out.0)
    }

    pub fn take_minted(&mut self) -> Vec<MintOutcome> {
        std::mem::take(&mut self.minted)
    }

    /// Interactive engine: report a deadline signal (contract 13).
    /// `frameDone` goes straight to the job engine on the peer port; the
    /// rest go to the host, which relays them and adjusts leases.
    pub fn signal_deadline(&mut self, signal: DeadlineSignal) -> Vec<Action> {
        let mut out = Out::default();
        self.emit_deadline(signal, &mut out);
        out.0
    }

    fn emit_deadline(&mut self, signal: DeadlineSignal, out: &mut Out) {
        if self.config.process != EngineProcess::Interactive {
            out.log(
                LogLevel::Error,
                "only the interactive engine sends deadline signals",
            );
            return;
        }
        match signal {
            DeadlineSignal::PlaybackStarted { .. } => self.playing = true,
            DeadlineSignal::PlaybackStopped { .. } => self.playing = false,
            _ => {}
        }
        let target = match signal {
            DeadlineSignal::FrameDone { .. } => self.peer.filter(|p| {
                matches!(
                    self.ports.get(p).map(|c| &c.state),
                    Some(ConnState::Open(_))
                )
            }),
            _ => self.open_host(),
        };
        if let Some(port) = target {
            let envelope = Envelope::new(Message::Event {
                body: Event::Deadline(signal),
            });
            self.send_raw(port, envelope, out);
        }
    }

    /// Starts work on attributable items: refuses items marked failed, and
    /// records the rest as crash suspects until [`Engine::end_work`].
    pub fn begin_work(&mut self, scopes: &[ErrorScope]) -> Result<WorkToken, Box<EngineError>> {
        self.failed.check(scopes)?;
        Ok(self.inflight.enter(scopes))
    }

    pub fn end_work(&mut self, token: WorkToken) {
        self.inflight.leave(token);
    }

    /// Periodic work: lease reports and acknowledgements, the in-flight
    /// breadcrumb. Call every 100 ms or so, and after `begin_work`.
    pub fn tick(&mut self, now: HostMicros) -> Vec<Action> {
        let mut out = Out::default();
        self.retry_media(now, &mut out);
        self.flush_document_updates(now, &mut out);
        if self.jobs.is_some() {
            self.release_waiting_frames(now, &mut out);
            self.drain_jobs(&mut out);
        }
        self.flush_inflight(&mut out);
        let active = !self.sessions.is_empty();
        if let Some(usage) = self.lease.poll_report(now, active)
            && let Some(host) = self.open_host()
        {
            let envelope = Envelope::new(Message::Event {
                body: Event::LeaseUsage(usage),
            });
            self.send_raw(host, envelope, &mut out);
        }
        if self.lease.ack_overdue(now) {
            let epoch = self.lease.owed_ack().map(|e| e.get());
            if self.overdue_logged != epoch {
                self.overdue_logged = epoch;
                out.log(
                    LogLevel::Warn,
                    format!(
                        "lease epoch {epoch:?} not acknowledged in time: {} bytes in use, {} granted",
                        self.lease.used(),
                        self.lease.granted()
                    ),
                );
            }
        }
        out.0
    }

    /// The shared snapshot of a session's mirror at its applied revision.
    fn job_document(&mut self, sid: SessionId) -> Option<JobDocument> {
        let session = self.sessions.get(&sid)?;
        let mirror = session.mirror.as_ref()?;
        let revision = session.revisions.applied();
        let document = match self.job_docs.get(&sid) {
            Some((r, doc)) if *r == revision => doc.clone(),
            _ => {
                let doc = std::sync::Arc::new(mirror.clone());
                self.job_docs.insert(sid, (revision, doc.clone()));
                doc
            }
        };
        Some(JobDocument {
            session: sid,
            revision,
            document,
        })
    }

    fn submit_job(
        &mut self,
        port: PortId,
        id: RequestId,
        job: pm_protocol::priority::SubmitJob,
        out: &mut Out,
    ) {
        if let Err(violation) = pm_protocol::validate::Validate::validate(&job) {
            let error = request_error(id, ErrorCode::InvalidMessage, &violation.to_string());
            self.send_error(port, id, error, out);
            return;
        }
        let document = match &job.spec {
            pm_protocol::priority::JobSpec::Export { session, .. } => {
                if !self
                    .sessions
                    .get(session)
                    .is_some_and(|s| s.port == port && s.role == SessionRole::JobSnapshot)
                {
                    let error = request_error(
                        id,
                        ErrorCode::UnknownSession,
                        "An export renders a job snapshot session this connection opened.",
                    );
                    self.send_error(port, id, error, out);
                    return;
                }
                match self.job_document(*session) {
                    Some(doc) => Some(doc),
                    None => {
                        let error = request_error(
                            id,
                            ErrorCode::StaleRevision,
                            "The job snapshot session has no document yet; resync it first.",
                        )
                        .with_retry(Retry::AfterResync);
                        self.send_error(port, id, error, out);
                        return;
                    }
                }
            }
            pm_protocol::priority::JobSpec::GenerateProxy { .. } => None,
        };
        let result = self
            .jobs
            .as_mut()
            .expect("checked by the caller")
            .submit(port, job, document);
        match result {
            Ok(accepted) => {
                self.respond(port, id, Response::JobAccepted(accepted), None, None, out)
            }
            Err(error) => {
                let error = EngineError {
                    scope: ErrorScope::Request { id },
                    ..*error
                };
                self.send_error(port, id, error, out)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn agent_frames(
        &mut self,
        port: PortId,
        id: RequestId,
        session: Option<SessionId>,
        revision: Option<Revision>,
        request: pm_protocol::priority::AgentFrameRequest,
        now: HostMicros,
        out: &mut Out,
    ) {
        let Some(sid) = self.owned_session(port, id, session, out) else {
            return;
        };
        if let Err(violation) = pm_protocol::validate::Validate::validate(&request) {
            let error = request_error(id, ErrorCode::InvalidMessage, &violation.to_string());
            self.send_error(port, id, error, out);
            return;
        }
        let wanted = revision.unwrap_or(Revision::ZERO);
        self.waiting_frames.push(WaitingFrames {
            port,
            id,
            session: sid,
            revision: wanted,
            request,
            deadline_us: now.get().saturating_add(AGENT_REVISION_WAIT_US),
        });
        self.release_waiting_frames(now, out);
    }

    /// Hands agent frame requests whose revision arrived to the service, and
    /// fails those whose wait bound passed (contract 12 invariant 2).
    fn release_waiting_frames(&mut self, now: HostMicros, out: &mut Out) {
        if self.waiting_frames.is_empty() {
            return;
        }
        let waiting = std::mem::take(&mut self.waiting_frames);
        for w in waiting {
            let ready = self.sessions.get(&w.session).is_some_and(|s| {
                s.mirror.is_some() && !s.stale && s.revisions.satisfies(w.revision)
            });
            if ready {
                let Some(document) = self.job_document(w.session) else {
                    continue;
                };
                let result = self
                    .jobs
                    .as_mut()
                    .expect("frames wait only with a service")
                    .agent_frames(w.port, w.id, document, w.request);
                if let Err(error) = result {
                    let error = EngineError {
                        scope: ErrorScope::Request { id: w.id },
                        ..*error
                    };
                    self.send_error(w.port, w.id, error, out);
                }
            } else if now.get() >= w.deadline_us {
                let error = request_error(
                    w.id,
                    ErrorCode::StaleRevision,
                    "The agent's revision did not reach the job engine in time.",
                )
                .with_retry(Retry::AfterResync);
                self.send_error(w.port, w.id, error, out);
            } else {
                self.waiting_frames.push(w);
            }
        }
    }

    /// Sends the service's outbox and makes its host calls.
    fn drain_jobs(&mut self, out: &mut Out) {
        let Some(jobs) = self.jobs.as_mut() else {
            return;
        };
        let drained = jobs.drain();
        if drained.is_empty() {
            return;
        }
        for (port, envelope) in drained.messages {
            self.send_raw(port, envelope, out);
        }
        for note in drained.work {
            match note {
                WorkNote::Started { token, scopes } => {
                    let work = self.inflight.enter(&scopes);
                    self.job_work.insert(token, work);
                }
                WorkNote::Ended { token } => {
                    if let Some(work) = self.job_work.remove(&token) {
                        self.inflight.leave(work);
                    }
                }
            }
        }
        for call in drained.host_requests {
            let failed = match self.open_host() {
                Some(host) => match self.call_host(host, call.request, call.session, out) {
                    Some(id) => {
                        self.host_calls
                            .insert(id, HostCall::Service { token: call.token });
                        None
                    }
                    None => Some("The host call could not be sent."),
                },
                None => Some("The engine has no host connection."),
            };
            if let Some(message) = failed
                && let Some(jobs) = self.jobs.as_mut()
            {
                jobs.host_answer(
                    call.token,
                    Err(EngineError::new(
                        ErrorCode::Internal,
                        ErrorScope::Connection,
                        message,
                    )),
                );
            }
        }
        self.flush_inflight(out);
    }

    fn flush_inflight(&mut self, out: &mut Out) {
        let Some(host) = self.open_host() else {
            return;
        };
        if let Some(crumb) = self.inflight.take_change() {
            let envelope = Envelope::new(Message::Event {
                body: Event::InFlight(crumb),
            });
            self.send_raw(host, envelope, out);
        }
    }

    pub fn status(&self) -> EngineStatus {
        let lease = self.lease.lease();
        EngineStatus {
            process: self.config.process,
            build: self.config.build.clone(),
            ports: self
                .ports
                .iter()
                .map(|(port, conn)| PortStatus {
                    port: port.0,
                    kind: conn.kind,
                    open: matches!(conn.state, ConnState::Open(_)),
                    peer: conn.peer,
                })
                .collect(),
            sessions: self
                .sessions
                .iter()
                .map(|(id, s)| SessionStatus {
                    session: id.get(),
                    port: s.port.0,
                    document: s.document.as_str().to_owned(),
                    role: s.role,
                    revision: s.revisions.applied().get(),
                    hash: s.mirror.as_ref().map(|m| hex(&mirror_digest(m))),
                    stale: s.stale,
                    definitions_generation: s.definitions.generation().map(|g| g.get()),
                })
                .collect(),
            lease_epoch: lease.map(|l| l.epoch.get()),
            lease_bytes: self.lease.granted(),
            used_bytes: self.lease.used(),
            owed_ack: self.lease.owed_ack().map(|e| e.get()),
            failed_items: self.failed.len(),
            in_flight: self.inflight.suspects().len(),
            playing: self.playing,
            slicing: self.slicer.as_ref().map(|s| *s.governor().policy()),
        }
    }

    fn respond(
        &mut self,
        port: PortId,
        id: RequestId,
        body: Response,
        session: Option<SessionId>,
        revision: Option<Revision>,
        out: &mut Out,
    ) {
        let mut envelope = Envelope::new(Message::Response { id, body });
        envelope.session = session;
        envelope.revision = revision;
        self.send_raw(port, envelope, out);
    }

    fn send_error(&mut self, port: PortId, id: RequestId, error: EngineError, out: &mut Out) {
        self.send_raw(port, Envelope::new(Message::Error { id, error }), out);
    }

    fn send_raw(&mut self, port: PortId, envelope: Envelope, out: &mut Out) {
        let limit = match self.ports.get(&port).map(|c| &c.state) {
            Some(ConnState::Open(negotiated)) => negotiated.max_message_bytes() as usize,
            Some(_) => version::MIN_MESSAGE_LIMIT as usize,
            None => return,
        };
        match codec::encode(&envelope) {
            Ok(bytes)
                if bytes.len() <= limit
                    || matches!(envelope.message, Message::Welcome(_) | Message::Hello(_)) =>
            {
                out.0.push(Action::Send { port, bytes })
            }
            Ok(bytes) => out.log(
                LogLevel::Error,
                format!(
                    "dropped a {}-byte message: the peer accepts at most {limit}",
                    bytes.len()
                ),
            ),
            Err(error) => {
                let error: CodecError = error;
                out.log(
                    LogLevel::Error,
                    format!("could not encode a message: {error}"),
                )
            }
        }
    }

    fn log_once(&mut self, out: &mut Out, message: String) {
        if self.logged_once.insert(message.clone()) {
            out.log(LogLevel::Warn, message);
        }
    }
}

/// Helpers for tests and hosts that drive an engine in-process (pm-cli).
pub mod testing {
    use super::*;

    /// The `hello` a host or renderer sends to open a connection.
    pub fn hello(role: PeerRole) -> Vec<u8> {
        codec::encode(&Envelope::new(Message::Hello(Hello {
            role,
            wire: WIRE_VERSION,
            build: SOURCE_BUILD.to_owned(),
            contracts: version::local_ranges(),
            max_message_bytes: codec::MAX_MESSAGE_BYTES as u32,
        })))
        .expect("hello encodes")
    }

    /// The bytes sent on `port` among `actions`, decoded.
    pub fn sent(actions: &[Action], port: PortId) -> Vec<Envelope> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Send { port: p, bytes } if *p == port => {
                    Some(codec::decode(bytes).expect("engine output decodes"))
                }
                _ => None,
            })
            .collect()
    }

    pub fn closed(actions: &[Action], port: PortId) -> bool {
        actions
            .iter()
            .any(|a| matches!(a, Action::Close { port: p } if *p == port))
    }

    pub fn handle_for(byte: u8) -> CapabilityHandle {
        CapabilityHandle::from_random_bytes([byte; 16])
    }
}
