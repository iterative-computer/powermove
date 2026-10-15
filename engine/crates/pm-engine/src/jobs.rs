//! The job engine's render and export service hook (contracts 12, 13, 14;
//! plan sections 17 and 20).
//!
//! The job engine's host (`pm-node`) installs a [`JobService`]. The engine
//! routes the host's `submitJob`, `cancelJobs`, and `agentFrames` requests to
//! it with an immutable snapshot of the session's document, forwards every
//! deadline signal and slicing policy, and tells it when sessions and ports
//! close. The service renders on its own threads and never touches the
//! engine's state: what it wants sent (responses, `jobStatus` events) and the
//! calls it makes to host services (`mintHandle`, `resolveHandle`,
//! `outputDestination`, `commitOutput`) wait in its outbox until the engine
//! drains it ([`crate::Engine::pump`], and on every `receive` and `tick`).
//! Host answers come back through [`JobService::host_answer`], keyed by the
//! service's own token, so the engine's request ids and its correlation of
//! host answers stay its own.
//!
//! Agent frames carry the agent's revision (read-your-writes, contract 12
//! invariant 2): a request for a revision the session has not applied waits
//! in the engine until a resync or patch batch reaches it, and fails with
//! `staleRevision` after [`AGENT_REVISION_WAIT_US`].

use std::sync::Arc;

use pm_protocol::common::{RequestId, Revision, SessionId};
use pm_protocol::envelope::{Envelope, Request, Response};
use pm_protocol::error::{EngineError, ErrorScope};
use pm_protocol::host::PortId;
use pm_protocol::priority::{AgentFrameRequest, CancelJobs, JobAccepted, JobsCancelled, SubmitJob};
use pm_protocol::slicing::{DeadlineSignal, SlicingPolicy};

/// How long an agent frame request waits for its revision before it fails
/// with `staleRevision` (contract 12 invariant 2's wait bound).
pub const AGENT_REVISION_WAIT_US: u64 = 2_000_000;

/// A frozen view of a session's document: what a job or an agent frame
/// request renders. Edits after the snapshot never reach it (section 17:
/// users keep editing; edits never invalidate a running export).
#[derive(Clone)]
pub struct JobDocument {
    pub session: SessionId,
    pub revision: Revision,
    pub document: Arc<pm_model::Document>,
}

/// A host services call the service wants made, answered through
/// [`JobService::host_answer`] with the same token.
#[derive(Debug, Clone, PartialEq)]
pub struct HostRequest {
    pub token: u64,
    pub request: Request,
    /// The envelope session: handles and outputs are bound to it.
    pub session: Option<SessionId>,
}

/// Work the service starts or ends, for crash attribution breadcrumbs
/// (contract 7): the engine reports what is in flight to the host.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkNote {
    Started { token: u64, scopes: Vec<ErrorScope> },
    Ended { token: u64 },
}

/// What the service has for the engine to send.
#[derive(Default)]
pub struct Drained {
    /// Envelopes for ports (responses to agent frames, job status events).
    pub messages: Vec<(PortId, Envelope)>,
    pub host_requests: Vec<HostRequest>,
    pub work: Vec<WorkNote>,
}

impl Drained {
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.host_requests.is_empty() && self.work.is_empty()
    }
}

/// Runs jobs and agent frames for the job engine.
pub trait JobService: Send {
    /// A job from the host on `host`. `document` is the snapshot of the
    /// export's session when the spec names one the host owns. Answer with
    /// acceptance (status events follow through the outbox) or an error.
    fn submit(
        &mut self,
        host: PortId,
        job: SubmitJob,
        document: Option<JobDocument>,
    ) -> Result<JobAccepted, Box<EngineError>>;

    /// Cancels one job or every job of an agent run.
    fn cancel(&mut self, cancel: &CancelJobs) -> JobsCancelled;

    /// Agent frames for `document`; the service answers `id` on `port` with
    /// `Response::AgentFrames` (or an error) through its outbox.
    fn agent_frames(
        &mut self,
        port: PortId,
        id: RequestId,
        document: JobDocument,
        request: AgentFrameRequest,
    ) -> Result<(), Box<EngineError>>;

    /// A deadline signal (contract 13) after the engine's own slicer saw it.
    fn signal(&mut self, signal: &DeadlineSignal);

    /// The slicing policy in force.
    fn policy(&mut self, policy: &SlicingPolicy);

    /// The answer to a [`HostRequest`].
    fn host_answer(&mut self, token: u64, answer: Result<Response, EngineError>);

    /// Takes everything waiting to be sent.
    fn drain(&mut self) -> Drained;

    /// A session closed: work on it that has not started is dropped.
    fn session_closed(&mut self, session: SessionId);

    /// A port closed: requests from it are not answered.
    fn port_closed(&mut self, port: PortId);
}

/// An agent frame request waiting for its revision.
pub(crate) struct WaitingFrames {
    pub port: PortId,
    pub id: RequestId,
    pub session: SessionId,
    pub revision: Revision,
    pub request: AgentFrameRequest,
    pub deadline_us: u64,
}
