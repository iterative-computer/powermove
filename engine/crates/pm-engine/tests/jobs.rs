//! The job engine's routing to its render and export service (contracts 12,
//! 13, 14): submissions with frozen snapshots, cancellation, agent frames
//! held for their revision, host calls relayed by token, the outbox, and
//! breadcrumbs.

use std::sync::{Arc, Mutex};

use pm_engine::engine::testing::{hello, sent};
use pm_engine::engine::{Action, Engine, EngineConfig, PeerKind};
use pm_engine::jobs::{
    AGENT_REVISION_WAIT_US, Drained, HostRequest, JobDocument, JobService, WorkNote,
};
use pm_protocol::codec;
use pm_protocol::color::{OutputColor, Precision, ProjectColor, QuickTimeTag};
use pm_protocol::common::{
    Bytes, ContentHash, FrameRate, HostMicros, RequestId, Revision, SessionId, Size, TickRange,
    Ticks,
};
use pm_protocol::envelope::{Envelope, Event, Message, OpenSession, Request, Response, SessionRole};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::host::{CacheKind, CapabilityKind, MintHandle, OutputChoice, PortId};
use pm_protocol::patch::{DocumentEncoding, Resync, ResyncSource};
use pm_protocol::priority::*;
use pm_protocol::slicing::{DeadlineSignal, SlicingPolicy};
use pm_protocol::version::PeerRole;

const HOST: PortId = PortId(1);
const WINDOW: PortId = PortId(2);

fn at(us: u64) -> HostMicros {
    HostMicros::new(us).unwrap()
}

fn request(id: u64, body: Request, session: Option<SessionId>, revision: Option<u64>) -> Vec<u8> {
    let mut envelope = Envelope::new(Message::Request {
        id: RequestId::new(id).unwrap(),
        body,
    });
    envelope.session = session;
    envelope.revision = revision.map(|r| Revision::new(r).unwrap());
    codec::encode(&envelope).unwrap()
}

#[derive(Default)]
struct Seen {
    submitted: Vec<(SubmitJob, Option<(SessionId, u64, String)>)>,
    frames: Vec<(PortId, RequestId, u64, AgentFrameRequest)>,
    cancels: Vec<CancelJobs>,
    signals: Vec<DeadlineSignal>,
    policies: usize,
    answers: Vec<(u64, Result<Response, EngineError>)>,
    closed_sessions: Vec<SessionId>,
    closed_ports: Vec<PortId>,
    outbox: Drained,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Seen>>);

impl JobService for Fake {
    fn submit(
        &mut self,
        _host: PortId,
        job: SubmitJob,
        document: Option<JobDocument>,
    ) -> Result<JobAccepted, EngineError> {
        let accepted = JobAccepted {
            job: job.job.clone(),
            state: JobState::Queued,
            position: 0,
        };
        let doc = document.map(|d| {
            let name = pm_model::json::to_canonical_string(&d.document.to_value().unwrap());
            (d.session, d.revision.get(), name)
        });
        self.0.lock().unwrap().submitted.push((job, doc));
        Ok(accepted)
    }

    fn cancel(&mut self, cancel: &CancelJobs) -> JobsCancelled {
        self.0.lock().unwrap().cancels.push(cancel.clone());
        JobsCancelled {
            jobs: vec!["job-1".into()],
        }
    }

    fn agent_frames(
        &mut self,
        port: PortId,
        id: RequestId,
        document: JobDocument,
        request: AgentFrameRequest,
    ) -> Result<(), EngineError> {
        self.0
            .lock()
            .unwrap()
            .frames
            .push((port, id, document.revision.get(), request));
        Ok(())
    }

    fn signal(&mut self, signal: &DeadlineSignal) {
        self.0.lock().unwrap().signals.push(*signal);
    }

    fn policy(&mut self, _: &SlicingPolicy) {
        self.0.lock().unwrap().policies += 1;
    }

    fn host_answer(&mut self, token: u64, answer: Result<Response, EngineError>) {
        self.0.lock().unwrap().answers.push((token, answer));
    }

    fn drain(&mut self) -> Drained {
        std::mem::take(&mut self.0.lock().unwrap().outbox)
    }

    fn session_closed(&mut self, session: SessionId) {
        self.0.lock().unwrap().closed_sessions.push(session);
    }

    fn port_closed(&mut self, port: PortId) {
        self.0.lock().unwrap().closed_ports.push(port);
    }
}

fn engine() -> (Engine, Fake) {
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Job));
    let fake = Fake::default();
    engine.set_job_service(Box::new(fake.clone()));
    assert!(engine.has_job_service());
    for (port, kind, role) in [
        (HOST, PeerKind::Host, PeerRole::Host),
        (WINDOW, PeerKind::Renderer, PeerRole::Renderer),
    ] {
        engine.attach(port, kind);
        engine.receive(port, &hello(role), at(0));
    }
    (engine, fake)
}

fn answer(actions: &[Action], port: PortId) -> Message {
    sent(actions, port)
        .into_iter()
        .map(|e| e.message)
        .find(|m| matches!(m, Message::Response { .. } | Message::Error { .. }))
        .unwrap_or_else(|| panic!("no answer on {port:?}: {actions:?}"))
}

fn code(m: &Message) -> ErrorCode {
    match m {
        Message::Error { error, .. } => error.code,
        other => panic!("expected an error, got {other:?}"),
    }
}

/// Opens a session on `port` and resyncs it to `text` at `revision`.
fn snapshot(engine: &mut Engine, port: PortId, role: SessionRole, text: &str, revision: u64) -> SessionId {
    let out = engine.receive(
        port,
        &request(
            10,
            Request::OpenSession(OpenSession {
                document: "doc".into(),
                role,
                process: EngineProcess::Job,
                color: ProjectColor::DEFAULT,
            }),
            None,
            None,
        ),
        at(1),
    );
    let Message::Response {
        body: Response::SessionOpened { session },
        ..
    } = answer(&out, port)
    else {
        panic!()
    };
    let value = pm_model::json::parse(text).unwrap();
    let out = engine.receive(
        port,
        &request(
            11,
            Request::Resync(Resync {
                document: "doc".into(),
                revision: Revision::new(revision).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(text.as_bytes().to_vec()),
                },
                hash: ContentHash(pm_model::hash::canonical_digest(&value)),
            }),
            Some(session),
            None,
        ),
        at(2),
    );
    assert!(matches!(answer(&out, port), Message::Response { .. }), "{out:?}");
    session
}

fn settings() -> ExportSettings {
    ExportSettings {
        format: ExportFormat::H264,
        alpha: AlphaOutput::None,
        quality: QualityTier::High,
        size: Size::new(320, 180),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::from_whole_seconds(1),
        },
        audio: true,
        motion_blur: false,
        color: OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        precision: Precision::Float32,
        dither: true,
        draft_from_proxies: false,
    }
}

fn agent_export(session: SessionId) -> SubmitJob {
    SubmitJob {
        job: "job-1".into(),
        class: PriorityClass::AgentExport,
        run: Some("run-1".into()),
        spec: JobSpec::Export {
            session,
            composition: "C".into(),
            settings: settings(),
        },
        output: Some(OutputChoice::AgentAutomatic {
            suggested_name: "Cut".into(),
            extension: "mp4".into(),
        }),
    }
}

#[test]
fn exports_reach_the_service_with_a_frozen_snapshot() {
    let (mut engine, fake) = engine();
    let sid = snapshot(&mut engine, HOST, SessionRole::JobSnapshot, r#"{"id":"P","n":1}"#, 7);
    let out = engine.receive(HOST, &request(20, Request::SubmitJob(agent_export(sid)), None, None), at(3));
    match answer(&out, HOST) {
        Message::Response {
            body: Response::JobAccepted(a),
            ..
        } => assert_eq!(a.job, "job-1".into()),
        other => panic!("{other:?}"),
    }
    let seen = fake.0.lock().unwrap();
    let (job, doc) = &seen.submitted[0];
    assert_eq!(job.job, "job-1".into());
    let (s, revision, text) = doc.clone().unwrap();
    assert_eq!((s, revision), (sid, 7));
    assert!(text.contains("\"n\":1"), "{text}");
}

#[test]
fn submissions_are_checked_before_the_service_sees_them() {
    let (mut engine, fake) = engine();
    // A renderer cannot submit jobs.
    let window_session = snapshot(&mut engine, WINDOW, SessionRole::JobSnapshot, "{}", 1);
    let out = engine.receive(WINDOW, &request(20, Request::SubmitJob(agent_export(window_session)), None, None), at(3));
    assert_eq!(code(&answer(&out, WINDOW)), ErrorCode::Unsupported);
    // The host cannot export a session another port owns, or a live job session.
    let out = engine.receive(HOST, &request(21, Request::SubmitJob(agent_export(window_session)), None, None), at(3));
    assert_eq!(code(&answer(&out, HOST)), ErrorCode::UnknownSession);
    let live = snapshot(&mut engine, HOST, SessionRole::Job, "{}", 1);
    let out = engine.receive(HOST, &request(22, Request::SubmitJob(agent_export(live)), None, None), at(3));
    assert_eq!(code(&answer(&out, HOST)), ErrorCode::UnknownSession);
    assert!(fake.0.lock().unwrap().submitted.is_empty());
}

#[test]
fn agent_frames_wait_for_the_agents_revision() {
    let (mut engine, fake) = engine();
    let sid = snapshot(&mut engine, HOST, SessionRole::Job, r#"{"r":4}"#, 4);
    let frames = AgentFrameRequest::render_frames_default(
        "run-1".into(),
        vec![AgentFrame {
            composition: "C".into(),
            time: Ticks::ZERO,
        }],
    );
    // At or behind the applied revision: straight to the service.
    engine.receive(HOST, &request(30, Request::AgentFrames(frames.clone()), Some(sid), Some(4)), at(10));
    assert_eq!(fake.0.lock().unwrap().frames.len(), 1);
    // Ahead: held until the revision arrives.
    let out = engine.receive(HOST, &request(31, Request::AgentFrames(frames.clone()), Some(sid), Some(6)), at(11));
    assert!(sent(&out, HOST).is_empty(), "{out:?}");
    assert_eq!(fake.0.lock().unwrap().frames.len(), 1);
    // The resync to revision 6 releases it.
    let value = pm_model::json::parse(r#"{"r":6}"#).unwrap();
    engine.receive(
        HOST,
        &request(
            32,
            Request::Resync(Resync {
                document: "doc".into(),
                revision: Revision::new(6).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(br#"{"r":6}"#.to_vec()),
                },
                hash: ContentHash(pm_model::hash::canonical_digest(&value)),
            }),
            Some(sid),
            None,
        ),
        at(12),
    );
    {
        let seen = fake.0.lock().unwrap();
        assert_eq!(seen.frames.len(), 2);
        assert_eq!((seen.frames[1].1, seen.frames[1].2), (RequestId::new(31).unwrap(), 6));
    }
    // A revision that never arrives fails with staleRevision after the bound.
    engine.receive(HOST, &request(33, Request::AgentFrames(frames.clone()), Some(sid), Some(9)), at(20));
    assert!(engine.tick(at(20 + AGENT_REVISION_WAIT_US / 2)).iter().all(|a| !matches!(a, Action::Send { .. })));
    let out = engine.tick(at(21 + AGENT_REVISION_WAIT_US));
    assert_eq!(code(&answer(&out, HOST)), ErrorCode::StaleRevision);
    // Frames name a session the asking port owns.
    let out = engine.receive(HOST, &request(34, Request::AgentFrames(frames), Some(SessionId::new(99).unwrap()), None), at(30));
    assert_eq!(code(&answer(&out, HOST)), ErrorCode::UnknownSession);
}

#[test]
fn host_calls_relay_by_token_and_the_outbox_drains() {
    let (mut engine, fake) = engine();
    let sid = snapshot(&mut engine, HOST, SessionRole::JobSnapshot, "{}", 1);
    {
        let mut seen = fake.0.lock().unwrap();
        seen.outbox.host_requests.push(HostRequest {
            token: 77,
            request: Request::MintHandle(MintHandle {
                kind: CapabilityKind::Cache {
                    cache: CacheKind::Scratch,
                },
                session: sid,
            }),
            session: Some(sid),
        });
        seen.outbox.messages.push((
            HOST,
            Envelope::new(Message::Event {
                body: Event::JobStatus(JobStatus {
                    job: "job-1".into(),
                    class: PriorityClass::AgentExport,
                    state: JobState::Running,
                    progress: 0.5,
                    path: None,
                    error: None,
                }),
            }),
        ));
        seen.outbox.work.push(WorkNote::Started {
            token: 5,
            scopes: vec![ErrorScope::Asset { asset: "clip".into() }],
        });
    }
    let out = engine.pump();
    let to_host = sent(&out, HOST);
    let call_id = to_host
        .iter()
        .find_map(|e| match &e.message {
            Message::Request {
                id,
                body: Request::MintHandle(_),
            } => Some(*id),
            _ => None,
        })
        .expect("the mint went to the host");
    assert!(to_host.iter().any(|e| matches!(
        &e.message,
        Message::Event {
            body: Event::JobStatus(s)
        } if s.progress == 0.5
    )));
    assert!(
        to_host.iter().any(|e| matches!(&e.message, Message::Event { body: Event::InFlight(f) } if !f.suspects.is_empty())),
        "the breadcrumb names the asset in flight"
    );
    // The host's answer reaches the service by its token.
    let reply = Envelope::new(Message::Error {
        id: call_id,
        error: EngineError::new(ErrorCode::CapabilityRejected, ErrorScope::Request { id: call_id }, "no"),
    });
    engine.receive(HOST, &codec::encode(&reply).unwrap(), at(40));
    let seen = fake.0.lock().unwrap();
    assert_eq!(seen.answers.len(), 1);
    assert_eq!(seen.answers[0].0, 77);
    assert!(seen.answers[0].1.is_err());
}

#[test]
fn cancels_signals_and_closes_reach_the_service() {
    let (mut engine, fake) = engine();
    let sid = snapshot(&mut engine, HOST, SessionRole::JobSnapshot, "{}", 1);
    let out = engine.receive(
        HOST,
        &request(40, Request::CancelJobs(CancelJobs::Run { run: "run-1".into() }), None, None),
        at(1),
    );
    assert!(matches!(
        answer(&out, HOST),
        Message::Response {
            body: Response::JobsCancelled(_),
            ..
        }
    ));
    let signal = DeadlineSignal::PlaybackStarted {
        at: at(5),
        rate: pm_protocol::common::PlaybackRate::NORMAL,
        frame_budget_us: 16_666,
    };
    engine.receive(
        HOST,
        &codec::encode(&Envelope::new(Message::Event {
            body: Event::Deadline(signal),
        }))
        .unwrap(),
        at(6),
    );
    engine.receive(
        HOST,
        &codec::encode(&Envelope::new(Message::Event {
            body: Event::Slicing(SlicingPolicy::DEFAULT),
        }))
        .unwrap(),
        at(6),
    );
    let mut close = Envelope::new(Message::Request {
        id: RequestId::new(41).unwrap(),
        body: Request::CloseSession,
    });
    close.session = Some(sid);
    engine.receive(HOST, &codec::encode(&close).unwrap(), at(7));
    engine.detach(WINDOW);
    let seen = fake.0.lock().unwrap();
    assert_eq!(seen.cancels, vec![CancelJobs::Run { run: "run-1".into() }]);
    assert_eq!(seen.signals, vec![signal]);
    assert_eq!(seen.policies, 1);
    assert_eq!(seen.closed_sessions, vec![sid]);
    assert_eq!(seen.closed_ports, vec![WINDOW]);
}

#[test]
fn the_interactive_engine_never_runs_jobs() {
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Interactive));
    engine.set_job_service(Box::new(Fake::default()));
    assert!(!engine.has_job_service());
}
