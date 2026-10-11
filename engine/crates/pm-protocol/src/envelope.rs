//! The protocol envelope (contract 9, section 19 "Protocol").
//!
//! Every message on every engine connection is one [`Envelope`]: the wire
//! version, an optional session, an optional revision tag, and one
//! [`Message`]. Messages are requests (answered by exactly one response or
//! error with the same id), events (one-way, either direction), cancels, and
//! the hello/welcome pair that negotiates versions.
//!
//! Revision tagging: on a request, `revision` is the minimum document
//! revision the receiver must have applied before answering
//! (read-your-writes; the receiver waits for patches, then fails with
//! `StaleRevision` after its wait bound). On responses and events it is the
//! revision the answer was computed at.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::blender::{BlenderJobRequest, BlenderModelJob, BlenderProgress, BlenderStatus, BlenderSurface};
use crate::clock::{ClockSample, TransportCommand};
use crate::color::ProjectColor;
use crate::common::{CompositionId, DocumentId, RequestId, Revision, SessionId, Ticks};
use crate::definitions::{DefinitionsApplied, DefinitionsUpdate};
use crate::document_host::{
    CloseProject, ExtensionRegistered, OpenProject, Persist, Persisted, ProjectOpened, RegisterExtension, ToolCall, ToolResult,
};
use crate::error::{CrashReport, EngineError, EngineProcess, FailureReport};
use crate::frame::{FrameDescriptor, FrameQuality, FrameRelease, FrameRequest, ViewOverride, ViewerState};
use crate::host::{
    CommitOutput, HandleGrant, HardwareSessionGrant, HardwareSessionRequest, MintHandle, OutputCommitted, OutputDestination,
    OutputDestinationRequest,
};
use crate::interop::SurfacePool;
use crate::lease::{Lease, LeaseRequest, LeaseUsage};
use crate::patch::{ConsistencyCheck, ConsistencyResult, EngineFact, PatchBatch, PatchesApplied, Resync};
use crate::priority::{AgentFrameRequest, AgentFrameResult, CancelJobs, ImagePayload, JobAccepted, JobStatus, JobsCancelled, SubmitJob};
use crate::scene::EvaluatedScene;
use crate::slicing::{DeadlineSignal, SlicingPolicy};
use crate::validate::{ensure, Validate, Violation};
use crate::version::{ContractId, Hello, Welcome, WIRE_VERSION};

/// One protocol message with its routing and revision tags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    /// Must equal [`WIRE_VERSION`].
    pub wire: u16,
    /// The engine session the message belongs to, for session-bound messages.
    pub session: Option<SessionId>,
    /// Minimum revision (requests) or computed-at revision (responses, events).
    pub revision: Option<Revision>,
    pub message: Message,
}

impl Envelope {
    pub fn new(message: Message) -> Self {
        Self { wire: WIRE_VERSION, session: None, revision: None, message }
    }

    pub fn with_session(mut self, session: SessionId) -> Self {
        self.session = Some(session);
        self
    }

    pub fn with_revision(mut self, revision: Revision) -> Self {
        self.revision = Some(revision);
        self
    }
}

/// The message kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum Message {
    Hello(Hello),
    Welcome(Welcome),
    Request { id: RequestId, body: Request },
    Response { id: RequestId, body: Response },
    Error { id: RequestId, error: EngineError },
    Event { body: Event },
    /// Cancel an outstanding request. The receiver answers the request with
    /// an error coded `Cancelled` unless it already answered.
    Cancel { id: RequestId },
}

/// Engine session roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SessionRole {
    /// The window's live document in the interactive engine.
    Interactive,
    /// The same document stream mirrored in the job engine (agent snapshots, exports).
    Job,
    /// A frozen snapshot for one export or render-queue job.
    JobSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OpenSession {
    pub document: DocumentId,
    pub role: SessionRole,
    pub process: EngineProcess,
    pub color: ProjectColor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EvaluateRequest {
    pub composition: CompositionId,
    pub time: Ticks,
    pub view: Option<ViewOverride>,
}

/// Pixels returned by value (thumbnails, posters, the synchronous
/// compatibility path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RenderedImage {
    pub width: u32,
    pub height: u32,
    pub revision: Revision,
    pub quality: FrameQuality,
    pub image: ImagePayload,
}

/// Requests. Each names its direction; a peer answers only requests it serves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum Request {
    /// Renderer or host → engine. Answer: `sessionOpened`. Follow with a resync.
    OpenSession(OpenSession),
    /// Renderer or host → engine (session-bound). Answer: `ack`.
    CloseSession,
    /// Renderer → engine (session-bound). Answer: `patchesApplied`.
    Resync(Resync),
    /// Renderer → engine (session-bound). Answer: `consistency`.
    CheckConsistency(ConsistencyCheck),
    /// Renderer → engine (session-bound). Answer: `frame` (viewer, scrub) or `image`.
    RenderFrame(FrameRequest),
    /// Renderer, CLI → engine (session-bound). Answer: `scene`.
    Evaluate(EvaluateRequest),
    /// Host → job engine (session-bound). Answer: `agentFrames`.
    AgentFrames(AgentFrameRequest),
    /// Host → job engine. Answer: `jobAccepted`; progress follows as `jobStatus` events.
    SubmitJob(SubmitJob),
    /// Host → job engine. Answer: `jobsCancelled`.
    CancelJobs(CancelJobs),
    /// Renderer or host → engine (session-bound). Answer: `definitionsApplied`.
    PushDefinitions(DefinitionsUpdate),
    /// Host → engine. Answer: `leaseUsage` with `ack` once usage fits.
    GrantLease(Lease),
    /// Engine → host. Answer: `lease`.
    RequestLease(LeaseRequest),
    /// Engine → host. Answer: `handle`.
    MintHandle(MintHandle),
    /// Job engine → host. Answer: `outputDestination`.
    OutputDestination(OutputDestinationRequest),
    /// Job engine → host. Answer: `outputCommitted`.
    CommitOutput(CommitOutput),
    /// Engine → host. Answer: `hardwareSessions`.
    HardwareSessions(HardwareSessionRequest),
    /// Job engine → host. Answer: `ack`; surfaces and progress follow as events.
    BlenderRender(BlenderJobRequest),
    /// Any → host. Answer: `blenderStatus`.
    BlenderStatus,
    /// Renderer or document host → host. Answer: `ack`; the result follows as a `fact` event.
    BlenderModel(BlenderModelJob),
    /// Host → document host pool. Answer: `projectOpened`.
    OpenProject(OpenProject),
    /// Host → document host. Answer: `ack`.
    CloseProject(CloseProject),
    /// Host → document host. Answer: `persisted`.
    Persist(Persist),
    /// Host → document host. Answer: `extensionRegistered`.
    RegisterExtension(RegisterExtension),
    /// Host → document host. Answer: `toolResult`.
    ToolCall(ToolCall),
}

/// Responses, one per request kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum Response {
    Ack,
    SessionOpened { session: SessionId },
    PatchesApplied(PatchesApplied),
    Consistency(ConsistencyResult),
    Frame(FrameDescriptor),
    Image(RenderedImage),
    Scene(Box<EvaluatedScene>),
    AgentFrames(AgentFrameResult),
    JobAccepted(JobAccepted),
    JobsCancelled(JobsCancelled),
    DefinitionsApplied(DefinitionsApplied),
    LeaseUsage(LeaseUsage),
    Lease(Lease),
    Handle(HandleGrant),
    OutputDestination(OutputDestination),
    OutputCommitted(OutputCommitted),
    HardwareSessions(HardwareSessionGrant),
    BlenderStatus(BlenderStatus),
    ProjectOpened(ProjectOpened),
    Persisted(Persisted),
    ExtensionRegistered(ExtensionRegistered),
    ToolResult(ToolResult),
}

/// One-way messages. Each names its direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum Event {
    /// pm-wasm → engine (session-bound): canonical patches, in revision order.
    ApplyPatches(PatchBatch),
    /// Engine → renderer: coalesced acknowledgement of applied patches.
    PatchesApplied(PatchesApplied),
    /// Engine → host → viewer: a presented frame (playback and scrubbing).
    Frame(FrameDescriptor),
    /// Viewer → engine: release a frame.
    ReleaseFrame(FrameRelease),
    /// Engine → host: the surface pool (sent once per generation).
    SurfacePool(SurfacePool),
    /// Viewer → engine: size, zoom, resolution, view override.
    Viewer(ViewerState),
    /// Renderer → engine: transport commands.
    Transport(TransportCommand),
    /// Engine → renderer, host, job engine: the playback clock.
    Clock(ClockSample),
    /// Interactive engine → host → job engine: deadline signals.
    Deadline(DeadlineSignal),
    /// Host → job engine: the slicing policy in force.
    Slicing(SlicingPolicy),
    /// Engine → renderer: an isolated failure.
    Failure(FailureReport),
    /// Host → renderer: an engine crashed and is restarting.
    Crash(CrashReport),
    /// Engine → host: periodic lease usage.
    LeaseUsage(LeaseUsage),
    /// Job engine → host → renderer: job status.
    JobStatus(JobStatus),
    /// Host → job engine: Blender progress.
    BlenderProgress(BlenderProgress),
    /// Host → job engine: a finished Blender surface.
    BlenderSurface(BlenderSurface),
    /// Engine → renderer: a fact that may become a document write.
    Fact(EngineFact),
}

/// Response kinds, for correlating answers with requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResponseKind {
    Ack,
    SessionOpened,
    PatchesApplied,
    Consistency,
    FrameOrImage,
    Scene,
    AgentFrames,
    JobAccepted,
    JobsCancelled,
    DefinitionsApplied,
    LeaseUsage,
    Lease,
    Handle,
    OutputDestination,
    OutputCommitted,
    HardwareSessions,
    BlenderStatus,
    ProjectOpened,
    Persisted,
    ExtensionRegistered,
    ToolResult,
}

impl Request {
    /// The response kind that answers this request.
    pub const fn expects(&self) -> ResponseKind {
        match self {
            Request::OpenSession(_) => ResponseKind::SessionOpened,
            Request::CloseSession | Request::BlenderRender(_) | Request::BlenderModel(_) | Request::CloseProject(_) => ResponseKind::Ack,
            Request::Resync(_) => ResponseKind::PatchesApplied,
            Request::CheckConsistency(_) => ResponseKind::Consistency,
            Request::RenderFrame(_) => ResponseKind::FrameOrImage,
            Request::Evaluate(_) => ResponseKind::Scene,
            Request::AgentFrames(_) => ResponseKind::AgentFrames,
            Request::SubmitJob(_) => ResponseKind::JobAccepted,
            Request::CancelJobs(_) => ResponseKind::JobsCancelled,
            Request::PushDefinitions(_) => ResponseKind::DefinitionsApplied,
            Request::GrantLease(_) => ResponseKind::LeaseUsage,
            Request::RequestLease(_) => ResponseKind::Lease,
            Request::MintHandle(_) => ResponseKind::Handle,
            Request::OutputDestination(_) => ResponseKind::OutputDestination,
            Request::CommitOutput(_) => ResponseKind::OutputCommitted,
            Request::HardwareSessions(_) => ResponseKind::HardwareSessions,
            Request::BlenderStatus => ResponseKind::BlenderStatus,
            Request::OpenProject(_) => ResponseKind::ProjectOpened,
            Request::Persist(_) => ResponseKind::Persisted,
            Request::RegisterExtension(_) => ResponseKind::ExtensionRegistered,
            Request::ToolCall(_) => ResponseKind::ToolResult,
        }
    }

    /// The contract that owns this request.
    pub const fn contract(&self) -> ContractId {
        match self {
            Request::OpenSession(_) | Request::CloseSession => ContractId::Protocol,
            Request::Resync(_) | Request::CheckConsistency(_) => ContractId::Patch,
            Request::RenderFrame(_) => ContractId::Frame,
            Request::Evaluate(_) => ContractId::EvaluatedScene,
            Request::AgentFrames(_) | Request::SubmitJob(_) | Request::CancelJobs(_) => ContractId::Priority,
            Request::PushDefinitions(_) => ContractId::Definitions,
            Request::GrantLease(_) | Request::RequestLease(_) => ContractId::Lease,
            Request::MintHandle(_) | Request::OutputDestination(_) | Request::CommitOutput(_) | Request::HardwareSessions(_) => {
                ContractId::HostServices
            }
            Request::BlenderRender(_) | Request::BlenderStatus | Request::BlenderModel(_) => ContractId::Blender,
            Request::OpenProject(_) | Request::CloseProject(_) | Request::Persist(_) | Request::RegisterExtension(_) | Request::ToolCall(_) => {
                ContractId::DocumentHost
            }
        }
    }

    /// Requests that act on one engine session and must carry it.
    pub const fn session_bound(&self) -> bool {
        matches!(
            self,
            Request::CloseSession
                | Request::Resync(_)
                | Request::CheckConsistency(_)
                | Request::RenderFrame(_)
                | Request::Evaluate(_)
                | Request::AgentFrames(_)
                | Request::PushDefinitions(_)
        )
    }
}

impl Response {
    pub const fn kind(&self) -> ResponseKind {
        match self {
            Response::Ack => ResponseKind::Ack,
            Response::SessionOpened { .. } => ResponseKind::SessionOpened,
            Response::PatchesApplied(_) => ResponseKind::PatchesApplied,
            Response::Consistency(_) => ResponseKind::Consistency,
            Response::Frame(_) | Response::Image(_) => ResponseKind::FrameOrImage,
            Response::Scene(_) => ResponseKind::Scene,
            Response::AgentFrames(_) => ResponseKind::AgentFrames,
            Response::JobAccepted(_) => ResponseKind::JobAccepted,
            Response::JobsCancelled(_) => ResponseKind::JobsCancelled,
            Response::DefinitionsApplied(_) => ResponseKind::DefinitionsApplied,
            Response::LeaseUsage(_) => ResponseKind::LeaseUsage,
            Response::Lease(_) => ResponseKind::Lease,
            Response::Handle(_) => ResponseKind::Handle,
            Response::OutputDestination(_) => ResponseKind::OutputDestination,
            Response::OutputCommitted(_) => ResponseKind::OutputCommitted,
            Response::HardwareSessions(_) => ResponseKind::HardwareSessions,
            Response::BlenderStatus(_) => ResponseKind::BlenderStatus,
            Response::ProjectOpened(_) => ResponseKind::ProjectOpened,
            Response::Persisted(_) => ResponseKind::Persisted,
            Response::ExtensionRegistered(_) => ResponseKind::ExtensionRegistered,
            Response::ToolResult(_) => ResponseKind::ToolResult,
        }
    }
}

impl Event {
    pub const fn contract(&self) -> ContractId {
        match self {
            Event::ApplyPatches(_) | Event::PatchesApplied(_) | Event::Fact(_) => ContractId::Patch,
            Event::Frame(_) | Event::ReleaseFrame(_) | Event::Viewer(_) => ContractId::Frame,
            Event::SurfacePool(_) => ContractId::GpuInterop,
            Event::Transport(_) | Event::Clock(_) => ContractId::Clock,
            Event::Deadline(_) | Event::Slicing(_) => ContractId::Slicing,
            Event::Failure(_) | Event::Crash(_) => ContractId::Errors,
            Event::LeaseUsage(_) => ContractId::Lease,
            Event::JobStatus(_) => ContractId::Priority,
            Event::BlenderProgress(_) | Event::BlenderSurface(_) => ContractId::Blender,
        }
    }

    pub const fn session_bound(&self) -> bool {
        matches!(
            self,
            Event::ApplyPatches(_) | Event::PatchesApplied(_) | Event::Frame(_) | Event::ReleaseFrame(_) | Event::Viewer(_) | Event::Transport(_)
        )
    }
}

impl Message {
    /// The contract a message belongs to; it must have been negotiated.
    pub const fn contract(&self) -> ContractId {
        match self {
            Message::Hello(_) | Message::Welcome(_) | Message::Cancel { .. } | Message::Response { .. } => ContractId::Protocol,
            Message::Error { .. } => ContractId::Errors,
            Message::Request { body, .. } => body.contract(),
            Message::Event { body } => body.contract(),
        }
    }
}

impl Validate for Request {
    fn validate(&self) -> Result<(), Violation> {
        match self {
            Request::OpenSession(open) => ensure(open.document.is_valid(), ContractId::Protocol, || "invalid document id".into()),
            Request::RenderFrame(request) => request.validate(),
            Request::Evaluate(request) => ensure(request.composition.is_valid(), ContractId::EvaluatedScene, || "invalid composition id".into()),
            Request::AgentFrames(request) => request.validate(),
            Request::PushDefinitions(update) => update.validate(),
            Request::GrantLease(lease) => lease.validate(),
            Request::OutputDestination(request) => request.validate(),
            Request::HardwareSessions(request) => request.validate(),
            Request::BlenderRender(job) => job.validate(),
            Request::BlenderModel(job) => job.validate(),
            Request::SubmitJob(job) => job.validate(),
            Request::ToolCall(call) => call.validate(),
            Request::CloseSession
            | Request::Resync(_)
            | Request::CheckConsistency(_)
            | Request::RequestLease(_)
            | Request::MintHandle(_)
            | Request::CommitOutput(_)
            | Request::BlenderStatus
            | Request::CancelJobs(_)
            | Request::OpenProject(_)
            | Request::CloseProject(_)
            | Request::Persist(_)
            | Request::RegisterExtension(_) => Ok(()),
        }
    }
}

impl Validate for Response {
    fn validate(&self) -> Result<(), Violation> {
        match self {
            Response::Scene(scene) => scene.validate(),
            Response::LeaseUsage(usage) => usage.validate(),
            Response::Lease(lease) => lease.validate(),
            _ => Ok(()),
        }
    }
}

impl Validate for Event {
    fn validate(&self) -> Result<(), Violation> {
        match self {
            Event::ApplyPatches(batch) => batch.validate(),
            Event::SurfacePool(pool) => pool.validate(),
            Event::Transport(command) => command.validate(),
            Event::Clock(sample) => sample.validate(),
            Event::Slicing(policy) => policy.validate(),
            Event::Deadline(signal) => signal.validate(),
            Event::LeaseUsage(usage) => usage.validate(),
            Event::JobStatus(status) => status.validate(),
            Event::BlenderSurface(surface) => surface.validate(),
            Event::Viewer(state) => state.validate(),
            Event::Failure(report) => report.error.validate(),
            _ => Ok(()),
        }
    }
}

impl Validate for Envelope {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Protocol;
        ensure(self.wire == WIRE_VERSION, C, || format!("wire version {} is not {WIRE_VERSION}", self.wire))?;
        match &self.message {
            Message::Hello(_) | Message::Welcome(_) => {
                ensure(self.session.is_none() && self.revision.is_none(), C, || "hello and welcome carry no session or revision".into())
            }
            Message::Request { body, .. } => {
                ensure(!body.session_bound() || self.session.is_some(), C, || "this request needs a session".into())?;
                body.validate()
            }
            Message::Response { body, .. } => body.validate(),
            Message::Event { body } => {
                ensure(!body.session_bound() || self.session.is_some(), C, || "this event needs a session".into())?;
                body.validate()
            }
            Message::Error { error, .. } => error.validate(),
            Message::Cancel { .. } => Ok(()),
        }
    }
}

/// A response or error that does not match an outstanding request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CorrelationError {
    #[error("no outstanding request {0}")]
    Unknown(RequestId),
    #[error("request {id} expects {expected:?}, got {got:?}")]
    WrongKind { id: RequestId, expected: ResponseKind, got: ResponseKind },
    #[error("request ids are exhausted on this connection")]
    Exhausted,
}

/// One direction's outstanding requests: allocates ids and checks answers.
#[derive(Debug, Default, Clone)]
pub struct Outstanding {
    next: RequestId,
    pending: std::collections::BTreeMap<RequestId, ResponseKind>,
}

impl Outstanding {
    /// Allocates an id for `request` and records what must answer it.
    pub fn send(&mut self, request: &Request) -> Result<RequestId, CorrelationError> {
        let id = self.next;
        self.next = id.next().ok_or(CorrelationError::Exhausted)?;
        self.pending.insert(id, request.expects());
        Ok(id)
    }

    /// Checks an incoming response, error, or cancel acknowledgement and
    /// retires the request. Errors always answer a pending request.
    pub fn receive(&mut self, message: &Message) -> Result<Option<RequestId>, CorrelationError> {
        match message {
            Message::Response { id, body } => {
                let expected = *self.pending.get(id).ok_or(CorrelationError::Unknown(*id))?;
                if expected != body.kind() {
                    return Err(CorrelationError::WrongKind { id: *id, expected, got: body.kind() });
                }
                self.pending.remove(id);
                Ok(Some(*id))
            }
            Message::Error { id, .. } => {
                self.pending.remove(id).ok_or(CorrelationError::Unknown(*id))?;
                Ok(Some(*id))
            }
            _ => Ok(None),
        }
    }

    pub fn is_pending(&self, id: RequestId) -> bool {
        self.pending.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_correlate_with_their_response_kind() {
        let mut outstanding = Outstanding::default();
        let id = outstanding.send(&Request::BlenderStatus).unwrap();
        let wrong = Message::Response { id, body: Response::Ack };
        assert!(matches!(outstanding.receive(&wrong), Err(CorrelationError::WrongKind { .. })));
        assert!(outstanding.is_pending(id));
        let right = Message::Response {
            id,
            body: Response::BlenderStatus(BlenderStatus { available: false, version: None, error: None }),
        };
        assert_eq!(outstanding.receive(&right), Ok(Some(id)));
        assert_eq!(outstanding.receive(&right), Err(CorrelationError::Unknown(id)));
        let second = outstanding.send(&Request::CloseSession).unwrap();
        assert_ne!(id, second);
        let error = Message::Error {
            id: second,
            error: EngineError::new(crate::error::ErrorCode::Cancelled, crate::error::ErrorScope::Request { id: second }, "Cancelled."),
        };
        assert_eq!(outstanding.receive(&error), Ok(Some(second)));
        assert!(outstanding.is_empty());
    }

    #[test]
    fn session_bound_messages_need_a_session() {
        let envelope = Envelope::new(Message::Request { id: RequestId::ZERO, body: Request::CloseSession });
        assert!(envelope.validate().is_err());
        assert!(envelope.with_session(SessionId::ZERO).validate().is_ok());
        let hello = Envelope::new(Message::Hello(Hello {
            role: crate::version::PeerRole::Renderer,
            wire: WIRE_VERSION,
            build: "dev".into(),
            contracts: crate::version::local_ranges(),
            max_message_bytes: crate::codec::MAX_MESSAGE_BYTES as u32,
        }));
        assert!(hello.validate().is_ok());
        assert!(hello.with_session(SessionId::ZERO).validate().is_err());
    }
}
