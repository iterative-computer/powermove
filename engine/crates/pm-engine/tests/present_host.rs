//! The viewer presenter's host calls (the paused viewer's Blender jobs,
//! contract 11): drained on `tick` and `pump`, sent on the host port with
//! their session, answered to the presenter by its token; the host's
//! Blender surface events reach the presenter in the interactive engine.

use std::sync::{Arc, Mutex};

use pm_engine::engine::testing::{hello, sent};
use pm_engine::engine::{Engine, EngineConfig, PeerKind};
use pm_engine::jobs::HostRequest;
use pm_engine::present::{FramePresenter, PresentContext};
use pm_protocol::blender::{BlenderSurface, ResolvedDevice};
use pm_protocol::clock::TransportCommand;
use pm_protocol::codec;
use pm_protocol::common::{
    ByteSize, ContentHash, HostMicros, JobId, PixelRect, RequestId, Revision, SessionId, Size,
};
use pm_protocol::envelope::{Envelope, Event, Message, Request, Response};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode};
use pm_protocol::frame::{FrameRelease, FrameRequest, ViewerState};
use pm_protocol::host::{CapabilityHandle, PortId};
use pm_protocol::version::PeerRole;

const HOST: PortId = PortId(1);

#[derive(Default)]
struct Seen {
    queued: Vec<HostRequest>,
    answers: Vec<(u64, Result<Response, ErrorCode>)>,
    events: Vec<Event>,
}

struct Asker(Arc<Mutex<Seen>>);

impl FramePresenter for Asker {
    fn viewer(
        &mut self,
        _: &mut PresentContext<'_>,
        _: PortId,
        _: SessionId,
        _: ViewerState,
        _: Option<Revision>,
    ) {
    }
    fn release(&mut self, _: PortId, _: SessionId, _: FrameRelease) {}
    fn transport(
        &mut self,
        _: &mut PresentContext<'_>,
        _: PortId,
        _: SessionId,
        _: TransportCommand,
    ) {
    }
    fn render(
        &mut self,
        _: &mut PresentContext<'_>,
        _: PortId,
        _: SessionId,
        _: RequestId,
        _: FrameRequest,
        _: Option<Revision>,
    ) -> Result<(), Box<EngineError>> {
        Ok(())
    }
    fn session_closed(&mut self, _: &mut PresentContext<'_>, _: PortId, _: SessionId) {}
    fn port_closed(&mut self, _: &mut PresentContext<'_>, _: PortId) {}

    fn drain_host_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.0.lock().unwrap().queued)
    }
    fn host_answer(&mut self, token: u64, answer: Result<Response, EngineError>) {
        self.0
            .lock()
            .unwrap()
            .answers
            .push((token, answer.map_err(|e| e.code)));
    }
    fn host_event(&mut self, event: &Event) {
        self.0.lock().unwrap().events.push(event.clone());
    }
}

fn at(us: u64) -> HostMicros {
    HostMicros::new(us).unwrap()
}

fn engine(seen: &Arc<Mutex<Seen>>) -> Engine {
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Interactive));
    engine.set_presenter(Box::new(Asker(seen.clone())));
    engine.attach(HOST, PeerKind::Host);
    engine.receive(HOST, &hello(PeerRole::Host), at(0));
    engine
}

fn requests(out: &[pm_engine::Action]) -> Vec<(RequestId, Request, Option<SessionId>)> {
    sent(out, HOST)
        .into_iter()
        .filter_map(|e| match e.message {
            Message::Request { id, body } => Some((id, body, e.session)),
            _ => None,
        })
        .collect()
}

#[test]
fn presenter_calls_go_to_the_host_and_answers_come_back_by_token() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut engine = engine(&seen);
    let session = SessionId::new(7).unwrap();
    seen.lock().unwrap().queued = vec![
        HostRequest {
            token: 41,
            request: Request::BlenderStatus,
            session: Some(session),
        },
        HostRequest {
            token: 42,
            request: Request::BlenderStatus,
            session: Some(session),
        },
    ];
    // `tick` drains them (the utility process calls it every 100 ms).
    let out = engine.tick(at(1_000));
    let calls = requests(&out);
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|(_, body, s)| matches!(body, Request::BlenderStatus) && *s == Some(session))
    );
    // The host answers the second first: each answer reaches its token.
    let status = Response::BlenderStatus(pm_protocol::blender::BlenderStatus {
        available: true,
        version: Some("4.5.14".into()),
        error: None,
    });
    let answer = |id: RequestId, body: Response| {
        codec::encode(&Envelope::new(Message::Response { id, body })).unwrap()
    };
    engine.receive(HOST, &answer(calls[1].0, status.clone()), at(2_000));
    let error = Envelope::new(Message::Error {
        id: calls[0].0,
        error: EngineError::new(
            ErrorCode::BlenderMissing,
            pm_protocol::error::ErrorScope::Request { id: calls[0].0 },
            "no Blender",
        ),
    });
    engine.receive(HOST, &codec::encode(&error).unwrap(), at(3_000));
    let s = seen.lock().unwrap();
    assert_eq!(
        s.answers,
        [(42, Ok(status)), (41, Err(ErrorCode::BlenderMissing))]
    );
    drop(s);
    // `pump` drains too.
    seen.lock().unwrap().queued = vec![HostRequest {
        token: 43,
        request: Request::BlenderStatus,
        session: None,
    }];
    assert_eq!(requests(&engine.pump()).len(), 1);
}

#[test]
fn blender_surfaces_from_the_host_reach_the_interactive_presenter() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut engine = engine(&seen);
    let surface = BlenderSurface {
        job: JobId::new("preview-blender-1"),
        request_index: 0,
        key: ContentHash([3; 32]),
        size: Size::new(8, 8),
        crop: PixelRect {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        },
        handle: CapabilityHandle::from_random_bytes([1; 16]),
        bytes: ByteSize::new(100).unwrap(),
        device: ResolvedDevice::Cpu,
    };
    let event = Envelope::new(Message::Event {
        body: Event::BlenderSurface(surface.clone()),
    });
    engine.receive(HOST, &codec::encode(&event).unwrap(), at(1_000));
    assert_eq!(
        seen.lock().unwrap().events,
        [Event::BlenderSurface(surface)]
    );
}

#[test]
fn a_call_without_a_host_connection_is_answered_at_once() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Interactive));
    engine.set_presenter(Box::new(Asker(seen.clone())));
    seen.lock().unwrap().queued = vec![HostRequest {
        token: 9,
        request: Request::BlenderStatus,
        session: None,
    }];
    let _ = engine.tick(at(1_000));
    assert_eq!(
        seen.lock().unwrap().answers,
        [(9, Err(ErrorCode::Internal))]
    );
}

/// The interactive engine's probe service (pm-node's probes-only
/// `JobEngineService`): it answers `probeMedia` and nothing else. Every call
/// it receives is recorded so a test sees what leaked into it.
#[derive(Clone, Default)]
struct Probes(Arc<Mutex<Vec<&'static str>>>);

impl Probes {
    fn calls(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
    fn note(&self, call: &'static str) {
        self.0.lock().unwrap().push(call);
    }
}

impl pm_engine::jobs::JobService for Probes {
    fn submit(
        &mut self,
        _: PortId,
        _: pm_protocol::priority::SubmitJob,
        _: Option<pm_engine::jobs::JobDocument>,
    ) -> Result<pm_protocol::priority::JobAccepted, Box<EngineError>> {
        self.note("submit");
        panic!("the interactive engine submitted a job to its probe service");
    }
    fn cancel(
        &mut self,
        _: &pm_protocol::priority::CancelJobs,
    ) -> pm_protocol::priority::JobsCancelled {
        self.note("cancel");
        pm_protocol::priority::JobsCancelled { jobs: Vec::new() }
    }
    fn agent_frames(
        &mut self,
        _: PortId,
        _: RequestId,
        _: pm_engine::jobs::JobDocument,
        _: pm_protocol::priority::AgentFrameRequest,
    ) -> Result<(), Box<EngineError>> {
        self.note("agent_frames");
        panic!("the interactive engine sent agent frames to its probe service");
    }
    fn probe_media(
        &mut self,
        _: PortId,
        _: RequestId,
        _: pm_protocol::media::ProbeMedia,
    ) -> Result<(), Box<EngineError>> {
        self.note("probe_media");
        Ok(())
    }
    fn signal(&mut self, _: &pm_protocol::slicing::DeadlineSignal) {
        self.note("signal");
    }
    fn policy(&mut self, _: &pm_protocol::slicing::SlicingPolicy) {
        self.note("policy");
    }
    fn host_answer(&mut self, _: u64, _: Result<Response, EngineError>) {
        self.note("host_answer");
    }
    fn host_event(&mut self, _: &Event) {
        self.note("host_event");
    }
    fn drain(&mut self) -> pm_engine::jobs::Drained {
        pm_engine::jobs::Drained::default()
    }
    fn session_closed(&mut self, _: SessionId) {
        self.note("session_closed");
    }
    fn port_closed(&mut self, _: PortId) {
        self.note("port_closed");
    }
}

fn blender_surface(job: &str) -> BlenderSurface {
    BlenderSurface {
        job: JobId::new(job),
        request_index: 0,
        key: ContentHash([5; 32]),
        size: Size::new(8, 8),
        crop: PixelRect {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        },
        handle: CapabilityHandle::from_random_bytes([2; 16]),
        bytes: ByteSize::new(100).unwrap(),
        device: ResolvedDevice::Cpu,
    }
}

/// Regression: the interactive engine also holds a probe service (so a
/// retired job utility need not start for media facts). The paused viewer's
/// Blender surfaces and progress must still reach the presenter, never the
/// probe service, or Material Preview draws nothing.
#[test]
fn blender_events_reach_the_presenter_when_the_interactive_engine_has_a_probe_service() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let probes = Probes::default();
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Interactive));
    engine.set_presenter(Box::new(Asker(seen.clone())));
    engine.set_job_service(Box::new(probes.clone()));
    assert!(!engine.has_job_service());
    engine.attach(HOST, PeerKind::Host);
    engine.receive(HOST, &hello(PeerRole::Host), at(0));

    let surface = Event::BlenderSurface(blender_surface("preview-blender-2"));
    let progress = Event::BlenderProgress(pm_protocol::blender::BlenderProgress {
        job: JobId::new("preview-blender-2"),
        completed: 1,
        total: 2,
    });
    for (n, event) in [progress.clone(), surface.clone()].into_iter().enumerate() {
        let envelope = Envelope::new(Message::Event { body: event });
        engine.receive(
            HOST,
            &codec::encode(&envelope).unwrap(),
            at(1_000 + n as u64),
        );
    }
    // Periodic work and wake-ups do not reach job-only paths either.
    let _ = engine.tick(at(2_000));
    let _ = engine.pump();

    assert_eq!(seen.lock().unwrap().events, [progress, surface]);
    assert!(probes.calls().is_empty(), "{:?}", probes.calls());
}

/// The probe service still answers `probeMedia` in the interactive engine,
/// while jobs, cancels, and agent frames stay the job engine's.
#[test]
fn the_interactive_probe_service_takes_probes_and_refuses_jobs() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let probes = Probes::default();
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Interactive));
    engine.set_presenter(Box::new(Asker(seen)));
    engine.set_job_service(Box::new(probes.clone()));
    engine.attach(HOST, PeerKind::Host);
    engine.receive(HOST, &hello(PeerRole::Host), at(0));
    let request = |id: u64, body: Request| {
        let mut envelope = Envelope::new(Message::Request {
            id: RequestId::new(id).unwrap(),
            body,
        });
        if matches!(
            envelope.message,
            Message::Request {
                body: Request::AgentFrames(_),
                ..
            }
        ) {
            envelope.session = Some(SessionId::new(1).unwrap());
            envelope.revision = Some(Revision::new(1).unwrap());
        }
        codec::encode(&envelope).unwrap()
    };
    let probe = pm_protocol::media::ProbeMedia {
        handle: CapabilityHandle::from_random_bytes([3; 16]),
        interpret: None,
    };
    engine.receive(HOST, &request(1, Request::ProbeMedia(probe)), at(1));
    let frames = pm_protocol::priority::AgentFrameRequest::render_frames_default(
        "run-1".into(),
        vec![pm_protocol::priority::AgentFrame {
            composition: "C".into(),
            time: pm_protocol::common::Ticks::ZERO,
        }],
    );
    let refused = engine.receive(HOST, &request(2, Request::AgentFrames(frames)), at(2));
    let cancel = pm_protocol::priority::CancelJobs::Run {
        run: "run-1".into(),
    };
    let cancelled = engine.receive(HOST, &request(3, Request::CancelJobs(cancel)), at(3));
    for out in [&refused, &cancelled] {
        let errors: Vec<ErrorCode> = sent(out, HOST)
            .into_iter()
            .filter_map(|e| match e.message {
                Message::Error { error, .. } => Some(error.code),
                _ => None,
            })
            .collect();
        assert_eq!(errors, [ErrorCode::Unsupported]);
    }
    assert_eq!(probes.calls(), ["probe_media"]);
}
