//! The engine core against the protocol: handshake and roles, sessions,
//! resync and consistency, host calls, leases, deadline signals, crash
//! attribution, and hostile input.

use pm_engine::engine::testing::{closed, handle_for, hello, sent};
use pm_engine::engine::{Action, Engine, EngineConfig, PatchBodyDecoder, PeerKind};
use pm_protocol::codec;
use pm_protocol::color::ProjectColor;
use pm_protocol::common::{
    ByteSize, Bytes, ContentHash, HostMicros, PlaybackRate, RequestId, Revision, Seq, SessionId,
};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::{CrashReport, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::host::{CacheKind, CapabilityKind, HandleAccess, HandleResolved, PortId};
use pm_protocol::lease::{Lease, LeaseReason};
use pm_protocol::patch::{
    ConsistencyCheck, DocumentEncoding, PatchBatch, PatchEncoding, PatchOrigin, Resync,
    ResyncSource,
};
use pm_protocol::slicing::{DeadlineSignal, SlicingPolicy};
use pm_protocol::version::PeerRole;

struct TempDirectory(std::path::PathBuf);
impl TempDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pm-engine-resync-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const HOST: PortId = PortId(1);
const WINDOW: PortId = PortId(2);
const OTHER_WINDOW: PortId = PortId(3);
const PEER: PortId = PortId(4);

fn at(us: u64) -> HostMicros {
    HostMicros::new(us).unwrap()
}

fn rid(n: u64) -> RequestId {
    RequestId::new(n).unwrap()
}

fn encode(envelope: Envelope) -> Vec<u8> {
    codec::encode(&envelope).unwrap()
}

fn request(id: u64, body: Request, session: Option<SessionId>) -> Vec<u8> {
    let mut envelope = Envelope::new(Message::Request { id: rid(id), body });
    envelope.session = session;
    encode(envelope)
}

fn event(body: Event, session: Option<SessionId>) -> Vec<u8> {
    let mut envelope = Envelope::new(Message::Event { body });
    envelope.session = session;
    encode(envelope)
}

/// An engine with an open host port and an open window port.
fn connected(process: EngineProcess) -> Engine {
    let mut engine = Engine::new(EngineConfig::new(process));
    for (port, kind, role) in [
        (HOST, PeerKind::Host, PeerRole::Host),
        (WINDOW, PeerKind::Renderer, PeerRole::Renderer),
    ] {
        engine.attach(port, kind);
        let out = engine.receive(port, &hello(role), at(0));
        assert!(
            matches!(sent(&out, port)[0].message, Message::Welcome(_)),
            "{out:?}"
        );
    }
    engine
}

fn only_answer(actions: &[Action], port: PortId) -> Message {
    let mut messages: Vec<_> = sent(actions, port)
        .into_iter()
        .map(|e| e.message)
        .filter(|m| matches!(m, Message::Response { .. } | Message::Error { .. }))
        .collect();
    assert_eq!(messages.len(), 1, "{actions:?}");
    messages.remove(0)
}

fn error_code(message: &Message) -> ErrorCode {
    match message {
        Message::Error { error, .. } => error.code,
        other => panic!("expected an error, got {other:?}"),
    }
}

fn open_session(engine: &mut Engine, port: PortId, id: u64) -> SessionId {
    let role = match engine.process() {
        EngineProcess::Interactive => SessionRole::Interactive,
        EngineProcess::Job => SessionRole::Job,
    };
    let out = engine.receive(
        port,
        &request(
            id,
            Request::OpenSession(OpenSession {
                document: "doc".into(),
                role,
                process: engine.process(),
                color: ProjectColor::DEFAULT,
            }),
            None,
        ),
        at(1),
    );
    match only_answer(&out, port) {
        Message::Response {
            body: Response::SessionOpened { session },
            ..
        } => session,
        other => panic!("{other:?}"),
    }
}

fn document(text: &str) -> (Vec<u8>, ContentHash) {
    let value = pm_model::json::parse(text).unwrap();
    (
        text.as_bytes().to_vec(),
        ContentHash(pm_model::hash::canonical_digest(&value)),
    )
}

fn resync(revision: u64, source: ResyncSource, hash: ContentHash) -> Request {
    Request::Resync(Resync {
        document: "doc".into(),
        revision: Revision::new(revision).unwrap(),
        encoding: DocumentEncoding::CanonicalJsonV1,
        source,
        hash,
    })
}

#[test]
fn ports_negotiate_only_with_the_role_the_host_assigned() {
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Job));
    engine.attach(HOST, PeerKind::Host);
    engine.attach(WINDOW, PeerKind::Renderer);
    // A renderer posing as the host is refused and its port closed.
    let out = engine.receive(WINDOW, &hello(PeerRole::Host), at(0));
    assert_eq!(
        error_code(&sent(&out, WINDOW)[0].message),
        ErrorCode::CapabilityRejected
    );
    assert!(closed(&out, WINDOW));
    assert!(
        engine
            .receive(WINDOW, &hello(PeerRole::Renderer), at(0))
            .iter()
            .any(|a| matches!(a, Action::Log { .. })),
        "a closed port is forgotten"
    );
    // Anything before hello closes the connection.
    let out = engine.receive(
        HOST,
        &event(Event::Slicing(SlicingPolicy::DEFAULT), None),
        at(0),
    );
    assert_eq!(
        error_code(&sent(&out, HOST)[0].message),
        ErrorCode::UnsupportedVersion
    );
    assert!(closed(&out, HOST));
    // A different build cannot speak draft contracts.
    engine.attach(HOST, PeerKind::Host);
    let mut other_build = codec::decode(&hello(PeerRole::Host)).unwrap();
    if let Message::Hello(hello) = &mut other_build.message {
        hello.build = "protocol-0000000000000000".into();
    }
    let out = engine.receive(HOST, &encode(other_build), at(0));
    assert_eq!(
        error_code(&sent(&out, HOST)[0].message),
        ErrorCode::UnsupportedVersion
    );
    assert!(closed(&out, HOST));
    // The real host negotiates every contract.
    engine.attach(HOST, PeerKind::Host);
    let out = engine.receive(HOST, &hello(PeerRole::Host), at(0));
    let Message::Welcome(welcome) = &sent(&out, HOST)[0].message else {
        panic!("{out:?}")
    };
    assert_eq!(welcome.role, PeerRole::JobEngine);
    assert_eq!(welcome.contracts.len(), 15);
    // A second hello is a protocol violation.
    let out = engine.receive(HOST, &hello(PeerRole::Host), at(0));
    assert!(closed(&out, HOST));
}

#[test]
fn garbage_closes_the_port_without_a_panic() {
    let mut engine = connected(EngineProcess::Interactive);
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for round in 0..512 {
        let len = (round % 97) + 1;
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        let _ = engine.receive(WINDOW, &bytes, at(0));
        engine.attach(WINDOW, PeerKind::Renderer);
        let _ = engine.receive(WINDOW, &hello(PeerRole::Renderer), at(0));
    }
    let out = engine.receive(WINDOW, &[0xc1], at(0));
    assert_eq!(
        error_code(&sent(&out, WINDOW)[0].message),
        ErrorCode::InvalidMessage
    );
    assert!(closed(&out, WINDOW));
}

#[test]
fn sessions_belong_to_their_process_and_port() {
    let mut engine = connected(EngineProcess::Interactive);
    engine.attach(OTHER_WINDOW, PeerKind::Renderer);
    engine.receive(OTHER_WINDOW, &hello(PeerRole::Renderer), at(0));
    let session = open_session(&mut engine, WINDOW, 1);
    // The job engine's session roles are refused here.
    let out = engine.receive(
        WINDOW,
        &request(
            2,
            Request::OpenSession(OpenSession {
                document: "doc".into(),
                role: SessionRole::JobSnapshot,
                process: EngineProcess::Job,
                color: ProjectColor::DEFAULT,
            }),
            None,
        ),
        at(1),
    );
    assert_eq!(
        error_code(&only_answer(&out, WINDOW)),
        ErrorCode::InvalidMessage
    );
    // Another window cannot touch it.
    let out = engine.receive(
        OTHER_WINDOW,
        &request(1, Request::CloseSession, Some(session)),
        at(1),
    );
    assert_eq!(
        error_code(&only_answer(&out, OTHER_WINDOW)),
        ErrorCode::UnknownSession
    );
    assert_eq!(engine.status().sessions.len(), 1);
    // The window closes its own session.
    let out = engine.receive(
        WINDOW,
        &request(3, Request::CloseSession, Some(session)),
        at(1),
    );
    assert!(matches!(
        only_answer(&out, WINDOW),
        Message::Response {
            body: Response::Ack,
            ..
        }
    ));
    // Closing the port closes its sessions.
    open_session(&mut engine, WINDOW, 4);
    open_session(&mut engine, WINDOW, 5);
    assert_eq!(engine.status().sessions.len(), 2);
    engine.detach(WINDOW);
    assert!(engine.status().sessions.is_empty());
}

#[test]
fn an_inline_resync_installs_the_mirror_after_checking_its_hash() {
    let mut engine = connected(EngineProcess::Interactive);
    let session = open_session(&mut engine, WINDOW, 1);
    let (body, hash) = document(r#"{"comps":{},"layers":[1,2,3],"name":"Ad"}"#);
    let out = engine.receive(
        WINDOW,
        &request(
            2,
            resync(
                7,
                ResyncSource::Inline {
                    body: Bytes(body.clone()),
                },
                ContentHash([0; 32]),
            ),
            Some(session),
        ),
        at(2),
    );
    assert_eq!(
        error_code(&only_answer(&out, WINDOW)),
        ErrorCode::InvalidMessage
    );
    let out = engine.receive(
        WINDOW,
        &request(
            3,
            resync(7, ResyncSource::Inline { body: Bytes(body) }, hash),
            Some(session),
        ),
        at(2),
    );
    let answer = sent(&out, WINDOW).remove(0);
    assert_eq!(answer.revision, Revision::new(7).ok());
    assert!(matches!(
        answer.message,
        Message::Response { body: Response::PatchesApplied(ref applied), .. } if applied.revision.get() == 7
    ));
    let status = engine.status();
    assert_eq!(status.sessions[0].revision, 7);
    assert_eq!(
        status.sessions[0].hash.as_deref(),
        Some(pm_model::hash::hex(&hash.0).as_str())
    );

    // The consistency check compares canonical hashes at one revision.
    let check = |hash: ContentHash, revision: u64| {
        request(
            9,
            Request::CheckConsistency(ConsistencyCheck {
                document: "doc".into(),
                revision: Revision::new(revision).unwrap(),
                hash,
            }),
            Some(session),
        )
    };
    let out = engine.receive(WINDOW, &check(hash, 7), at(3));
    assert!(matches!(
        only_answer(&out, WINDOW),
        Message::Response { body: Response::Consistency(ref r), .. } if r.matches && r.hash == hash
    ));
    let out = engine.receive(WINDOW, &check(ContentHash([1; 32]), 7), at(3));
    assert!(matches!(
        only_answer(&out, WINDOW),
        Message::Response { body: Response::Consistency(ref r), .. } if !r.matches
    ));
    let out = engine.receive(WINDOW, &check(hash, 8), at(3));
    assert_eq!(
        error_code(&only_answer(&out, WINDOW)),
        ErrorCode::StaleRevision
    );
}

#[test]
fn a_file_resync_resolves_its_handle_with_the_host_only() {
    let mut engine = connected(EngineProcess::Job);
    let session = open_session(&mut engine, WINDOW, 1);
    let (body, hash) = document(r#"{"layers":[],"name":"Big project"}"#);
    let dir = TempDirectory::new();
    let file = dir.path().join("resync.json");
    std::fs::write(&file, &body).unwrap();
    let handle = handle_for(0x5a);
    let source = ResyncSource::CacheFile {
        handle: handle.clone(),
        length: ByteSize::new(body.len() as u64).unwrap(),
    };
    let out = engine.receive(
        WINDOW,
        &request(2, resync(3, source.clone(), hash), Some(session)),
        at(1),
    );
    assert!(
        sent(&out, WINDOW).is_empty(),
        "the answer waits for the host"
    );
    let call = sent(&out, HOST).remove(0);
    let Message::Request {
        id: call_id,
        body: Request::ResolveHandle(resolve),
    } = call.message
    else {
        panic!("{call:?}")
    };
    assert_eq!(resolve.handle, handle);
    assert_eq!(resolve.session, session);
    let resolved = |path: &std::path::Path| {
        encode(Envelope::new(Message::Response {
            id: call_id,
            body: Response::HandleResolved(HandleResolved {
                handle: handle.clone(),
                kind: CapabilityKind::Cache {
                    cache: CacheKind::Scratch,
                },
                access: HandleAccess::ReadWrite,
                path: path.to_str().unwrap().into(),
            }),
        }))
    };
    // A renderer cannot answer the engine's host call.
    let out = engine.receive(
        WINDOW,
        &resolved(std::path::Path::new("/etc/passwd")),
        at(2),
    );
    assert!(sent(&out, WINDOW).is_empty());
    assert_eq!(engine.status().sessions[0].revision, 0);
    // The host can.
    let mut out = engine.receive(HOST, &resolved(&file), at(3));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while sent(&out, WINDOW).is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
        out.extend(engine.tick(at(3)));
    }
    assert!(matches!(
        only_answer(&out, WINDOW),
        Message::Response { id, body: Response::PatchesApplied(_) } if id == rid(2)
    ));
    assert_eq!(engine.status().sessions[0].revision, 3);
    // The host refusing the handle fails the renderer's request.
    let out = engine.receive(
        WINDOW,
        &request(4, resync(4, source, hash), Some(session)),
        at(4),
    );
    let Message::Request { id: call_id, .. } = sent(&out, HOST).remove(0).message else {
        panic!()
    };
    let refusal = encode(Envelope::new(Message::Error {
        id: call_id,
        error: pm_protocol::error::EngineError::new(
            ErrorCode::CapabilityRejected,
            ErrorScope::Connection,
            "capability handle bound to another session",
        ),
    }));
    let out = engine.receive(HOST, &refusal, at(5));
    assert_eq!(
        error_code(&only_answer(&out, WINDOW)),
        ErrorCode::CapabilityRejected
    );
}

#[test]
fn a_second_session_resyncing_to_a_checked_hash_shares_the_mirror_without_reading_it() {
    // Main's snapshot session for a window and the window's own session
    // carry the same document. With a 180 MB document each parse is about
    // 1.8 GB (C1): the second session takes the checked mirror instead.
    let mut engine = connected(EngineProcess::Interactive);
    let window = open_session(&mut engine, WINDOW, 1);
    let (body, hash) = document(r#"{"comps":{},"layers":[{"id":"a","v":1}],"name":"Big"}"#);
    let out = engine.receive(
        WINDOW,
        &request(
            2,
            resync(5, ResyncSource::Inline { body: Bytes(body) }, hash),
            Some(window),
        ),
        at(2),
    );
    assert!(matches!(
        only_answer(&out, WINDOW),
        Message::Response {
            body: Response::PatchesApplied(_),
            ..
        }
    ));
    // The host's session for the window: a file resync of the same hash at
    // a later revision is answered at once, with no handle to resolve.
    let host_session = open_session(&mut engine, HOST, 3);
    let source = ResyncSource::CacheFile {
        handle: handle_for(0x31),
        length: ByteSize::new(1).unwrap(),
    };
    let out = engine.receive(
        HOST,
        &request(4, resync(6, source.clone(), hash), Some(host_session)),
        at(3),
    );
    assert!(
        !sent(&out, HOST).iter().any(|e| matches!(
            e.message,
            Message::Request {
                body: Request::ResolveHandle(_),
                ..
            }
        )),
        "{out:?}"
    );
    assert!(matches!(
        only_answer(&out, HOST),
        Message::Response { id, body: Response::PatchesApplied(ref applied) } if id == rid(4) && applied.revision.get() == 6
    ));
    let status = engine.status();
    let shared = status
        .sessions
        .iter()
        .find(|s| s.session == host_session.get())
        .unwrap();
    assert_eq!(shared.revision, 6);
    assert_eq!(
        shared.hash.as_deref(),
        Some(pm_model::hash::hex(&hash.0).as_str())
    );
    // A session with a mirror of its own (and render caches validated
    // against its own revisions) never takes another session's tree.
    let out = engine.receive(
        WINDOW,
        &request(
            9,
            resync(
                8,
                ResyncSource::CacheFile {
                    handle: handle_for(0x32),
                    length: ByteSize::new(1).unwrap(),
                },
                hash,
            ),
            Some(window),
        ),
        at(3),
    );
    assert!(
        sent(&out, HOST).iter().any(|e| matches!(
            e.message,
            Message::Request {
                body: Request::ResolveHandle(_),
                ..
            }
        )),
        "{out:?}"
    );
    // A different hash still reads its body.
    let (_, other) = document(r#"{"comps":{},"layers":[],"name":"Other"}"#);
    let out = engine.receive(
        HOST,
        &request(5, resync(7, source, other), Some(host_session)),
        at(4),
    );
    assert!(sent(&out, HOST).iter().any(|e| matches!(
        e.message,
        Message::Request {
            body: Request::ResolveHandle(_),
            ..
        }
    )));
}

#[test]
fn only_the_host_grants_leases() {
    let mut engine = connected(EngineProcess::Job);
    let lease = Lease {
        process: EngineProcess::Job,
        epoch: Seq::new(1).unwrap(),
        bytes: ByteSize::new(1 << 30).unwrap(),
        reason: LeaseReason::Initial,
        ack_deadline_ms: 500,
    };
    let out = engine.receive(WINDOW, &request(1, Request::GrantLease(lease), None), at(0));
    assert_eq!(
        error_code(&only_answer(&out, WINDOW)),
        ErrorCode::CapabilityRejected
    );
    let out = engine.receive(HOST, &request(1, Request::GrantLease(lease), None), at(0));
    assert!(matches!(
        only_answer(&out, HOST),
        Message::Response { body: Response::LeaseUsage(ref u), .. } if u.ack && u.epoch.get() == 1
    ));
    assert_eq!(engine.status().lease_bytes, 1 << 30);
    // Periodic reports go to the host while sessions are open.
    open_session(&mut engine, WINDOW, 2);
    let out = engine.tick(at(6_000_000));
    assert!(sent(&out, HOST).iter().any(|e| matches!(
        e.message,
        Message::Event {
            body: Event::LeaseUsage(_)
        }
    )));
    // Asking for more goes to the host, and its answer is applied.
    let out = engine.request_lease(1 << 30);
    let Message::Request {
        id,
        body: Request::RequestLease(asked),
    } = sent(&out, HOST).remove(0).message
    else {
        panic!()
    };
    assert_eq!(asked.wanted.get(), 1 << 30);
    let bigger = Lease {
        epoch: Seq::new(2).unwrap(),
        bytes: ByteSize::new(2 << 30).unwrap(),
        reason: LeaseReason::Requested,
        ..lease
    };
    engine.receive(
        HOST,
        &encode(Envelope::new(Message::Response {
            id,
            body: Response::Lease(bigger),
        })),
        at(7_000_000),
    );
    assert_eq!(engine.status().lease_bytes, 2 << 30);
}

#[test]
fn the_job_engine_follows_relayed_and_direct_deadline_signals() {
    let mut job = connected(EngineProcess::Job);
    let mut interactive = connected(EngineProcess::Interactive);
    job.attach(PEER, PeerKind::PeerEngine);
    let opening = interactive.attach(PEER, PeerKind::PeerEngine);
    let hello = sent(&opening, PEER).remove(0);
    assert!(
        matches!(hello.message, Message::Hello(ref h) if h.role == PeerRole::InteractiveEngine)
    );
    let welcome = job.receive(PEER, &codec::encode(&hello).unwrap(), at(0));
    let welcome = sent(&welcome, PEER).remove(0);
    interactive.receive(PEER, &codec::encode(&welcome).unwrap(), at(0));
    assert!(interactive.status().ports.iter().all(|p| p.open));

    // The policy comes from the host only.
    let policy = SlicingPolicy {
        yield_after_miss_us: 50_000,
        ..SlicingPolicy::DEFAULT
    };
    job.receive(
        WINDOW,
        &event(Event::Slicing(SlicingPolicy::DEFAULT), None),
        at(0),
    );
    job.receive(HOST, &event(Event::Slicing(policy), None), at(0));
    assert_eq!(job.status().slicing, Some(policy));

    // Playback starts: the interactive engine tells the host, which relays.
    let started = DeadlineSignal::PlaybackStarted {
        at: at(1_000),
        rate: PlaybackRate::NORMAL,
        frame_budget_us: 16_666,
    };
    let out = interactive.signal_deadline(started);
    let relayed = sent(&out, HOST).remove(0);
    assert!(sent(&out, PEER).is_empty());
    // A renderer cannot fake it.
    job.receive(WINDOW, &codec::encode(&relayed).unwrap(), at(1_000));
    assert!(!job.status().playing);
    job.receive(HOST, &codec::encode(&relayed).unwrap(), at(1_000));
    assert!(job.status().playing);
    // Per-frame signals take the direct port.
    for seq in 0..2u64 {
        let out = interactive.signal_deadline(DeadlineSignal::FrameDone {
            at: at(4_000 + seq * 16_666),
            frame_seq: Seq::new(seq).unwrap(),
            tick: at(1_000 + seq * 16_666),
            next_submit: at(17_666 + seq * 16_666),
            late: false,
        });
        assert!(sent(&out, HOST).is_empty());
        let frame = sent(&out, PEER).remove(0);
        job.receive(
            PEER,
            &codec::encode(&frame).unwrap(),
            at(5_000 + seq * 16_666),
        );
    }
    let budget = job.slicer().unwrap().governor().budget(at(22_000));
    assert!(
        matches!(
            budget,
            pm_protocol::slicing::SliceBudget::Sliced {
                deadline: Some(_),
                ..
            }
        ),
        "{budget:?}"
    );
    // A request on the peer port is refused.
    let out = job.receive(
        PEER,
        &request(1, Request::CloseSession, Some(SessionId::new(1).unwrap())),
        at(0),
    );
    assert_eq!(
        error_code(&only_answer(&out, PEER)),
        ErrorCode::InvalidMessage
    );
}

#[test]
fn crash_attribution_refuses_failed_items_and_breadcrumbs_reach_the_host() {
    let mut engine = connected(EngineProcess::Interactive);
    let asset = ErrorScope::Asset {
        asset: "clip".into(),
    };
    let token = engine
        .begin_work(&[asset.clone(), ErrorScope::Layer { layer: "l1".into() }])
        .unwrap();
    let out = engine.tick(at(0));
    let crumbs: Vec<_> = sent(&out, HOST)
        .into_iter()
        .filter_map(|e| match e.message {
            Message::Event {
                body: Event::InFlight(f),
            } => Some(f),
            _ => None,
        })
        .collect();
    assert_eq!(crumbs.len(), 1);
    assert_eq!(crumbs[0].suspects.len(), 2);
    assert!(sent(&engine.tick(at(1)), HOST).is_empty(), "unchanged");
    engine.end_work(token);
    let report = CrashReport {
        process: EngineProcess::Interactive,
        attempt: 2,
        exit: "signal 11".into(),
        suspects: vec![asset.clone()],
        marked_failed: vec![asset.clone()],
        restart_in_ms: 500,
    };
    // Only the host's attribution counts.
    engine.receive(WINDOW, &event(Event::Crash(report.clone()), None), at(2));
    assert!(engine.begin_work(std::slice::from_ref(&asset)).is_ok());
    engine.receive(HOST, &event(Event::Crash(report), None), at(2));
    let refused = engine.begin_work(&[asset]).unwrap_err();
    assert_eq!(refused.code, ErrorCode::CrashAttributed);
    assert_eq!(engine.status().failed_items, 1);
}

#[test]
fn render_requests_route_back_to_the_current_engine() {
    let mut engine = connected(EngineProcess::Interactive);
    let session = open_session(&mut engine, WINDOW, 1);
    let out = engine.receive(
        WINDOW,
        &request(
            2,
            Request::Evaluate(pm_protocol::envelope::EvaluateRequest {
                composition: "c".into(),
                time: pm_protocol::common::Ticks::ZERO,
                view: None,
            }),
            Some(session),
        ),
        at(0),
    );
    assert_eq!(
        error_code(&only_answer(&out, WINDOW)),
        ErrorCode::Unsupported
    );
    // Requests for the host are refused by an engine.
    let out = engine.receive(WINDOW, &request(3, Request::BlenderStatus, None), at(0));
    assert_eq!(
        error_code(&only_answer(&out, WINDOW)),
        ErrorCode::InvalidMessage
    );
}

struct JsonPatches;

impl PatchBodyDecoder for JsonPatches {
    fn decode(&self, _: PatchEncoding, body: &[u8]) -> Result<Vec<pm_model::Patch>, String> {
        let value = pm_model::json::parse_bytes(body).map_err(|e| e.to_string())?;
        pm_model::patch::patches_from_value(&value).map_err(|e| format!("{e:?}"))
    }
}

#[test]
fn patch_batches_follow_the_applied_revision() {
    let mut engine = connected(EngineProcess::Interactive);
    let session = open_session(&mut engine, WINDOW, 1);
    let (body, hash) = document(r#"{"name":"A"}"#);
    engine.receive(
        WINDOW,
        &request(
            2,
            resync(1, ResyncSource::Inline { body: Bytes(body) }, hash),
            Some(session),
        ),
        at(0),
    );
    let batch = |base: u64, revision: u64| {
        event(
            Event::ApplyPatches(PatchBatch {
                document: "doc".into(),
                base_revision: Revision::new(base).unwrap(),
                revision: Revision::new(revision).unwrap(),
                origin: PatchOrigin::Edit,
                encoding: PatchEncoding::CanonicalV1,
                patch_count: 1,
                body: Bytes(br#"[{"path":["name"],"exists":true,"value":"B"}]"#.to_vec()),
                provisional: false,
            }),
            Some(session),
        )
    };
    // The built-in decoder reads canonicalV1 (pm-model's binary patches):
    // a body that is not one is refused, and the session waits for a resync.
    let out = engine.receive(WINDOW, &batch(1, 2), at(1));
    let failure = sent(&out, WINDOW).remove(0);
    assert!(matches!(
        failure.message,
        Message::Event { body: Event::Failure(ref f) } if f.error.code == ErrorCode::InvalidMessage
    ));
    assert!(engine.status().sessions[0].stale);

    engine.set_patch_decoder(Box::new(JsonPatches));
    let out = engine.receive(WINDOW, &batch(1, 2), at(2));
    let applied = sent(&out, WINDOW).remove(0);
    assert!(
        matches!(
            applied.message,
            Message::Event { body: Event::PatchesApplied(ref a) } if a.revision.get() == 2
        ),
        "{applied:?}"
    );
    let (_, expected) = document(r#"{"name":"B"}"#);
    assert_eq!(
        engine.status().sessions[0].hash.as_deref(),
        Some(pm_model::hash::hex(&expected.0).as_str())
    );
    // Duplicates are ignored; gaps ask for a resync.
    assert!(sent(&engine.receive(WINDOW, &batch(1, 2), at(3)), WINDOW).is_empty());
    let out = engine.receive(WINDOW, &batch(5, 6), at(3));
    assert!(matches!(
        sent(&out, WINDOW).remove(0).message,
        Message::Event { body: Event::Failure(ref f) } if f.error.code == ErrorCode::StaleRevision
    ));
}

fn wait_for_answer(engine: &mut Engine, mut actions: Vec<Action>, port: PortId) -> Vec<Action> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while sent(&actions, port)
        .iter()
        .all(|e| !matches!(e.message, Message::Response { .. } | Message::Error { .. }))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "document reader did not answer"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
        actions.extend(engine.tick(at(5)));
    }
    actions
}

#[test]
fn large_ingestion_is_async_and_cancellation_never_installs_its_result() {
    let mut engine = connected(EngineProcess::Interactive);
    let sid = open_session(&mut engine, WINDOW, 1);
    let (body, hash) = document(&format!("{{\"payload\":\"{}\"}}", "x".repeat(128 * 1024)));
    let start = std::time::Instant::now();
    let out = engine.receive(
        WINDOW,
        &request(
            2,
            resync(
                7,
                ResyncSource::Inline {
                    body: Bytes(body.clone()),
                },
                hash,
            ),
            Some(sid),
        ),
        at(2),
    );
    assert!(
        sent(&out, WINDOW).is_empty(),
        "large ingestion must leave the event loop"
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(100));
    let cancel = encode(Envelope::new(Message::Cancel { id: rid(2) }));
    let out = engine.receive(WINDOW, &cancel, at(3));
    assert_eq!(error_code(&only_answer(&out, WINDOW)), ErrorCode::Cancelled);
    // Completion of cancelled work cannot install a mirror or emit another answer.
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        engine.tick(at(4));
    }
    assert_eq!(engine.status().sessions[0].revision, 0);
    let out = engine.receive(
        WINDOW,
        &request(
            3,
            resync(8, ResyncSource::Inline { body: Bytes(body) }, hash),
            Some(sid),
        ),
        at(4),
    );
    let out = wait_for_answer(&mut engine, out, WINDOW);
    assert!(matches!(
        only_answer(&out, WINDOW),
        Message::Response {
            body: Response::PatchesApplied(_),
            ..
        }
    ));
    assert_eq!(engine.status().sessions[0].revision, 8);
    assert_eq!(
        engine.status().sessions[0].hash,
        Some(pm_model::hash::hex(&hash.0))
    );
}

#[test]
fn reconnect_installs_a_verified_checkpoint_without_parsing_then_accepts_the_patch_suffix() {
    let mut engine = connected(EngineProcess::Interactive);
    let sid = open_session(&mut engine, WINDOW, 1);
    let (body, hash) = document(r#"{"value":1}"#);
    engine.receive(
        WINDOW,
        &request(
            2,
            resync(1, ResyncSource::Inline { body: Bytes(body) }, hash),
            Some(sid),
        ),
        at(1),
    );
    let batch = |base, revision, value| PatchBatch {
        document: "doc".into(),
        base_revision: Revision::new(base).unwrap(),
        revision: Revision::new(revision).unwrap(),
        origin: PatchOrigin::Edit,
        encoding: PatchEncoding::CanonicalV1,
        patch_count: 1,
        body: Bytes(
            pm_model::binary::encode(&[pm_model::Op::Leaf(pm_model::Patch {
                path: vec!["value".into()],
                exists: true,
                value: Some(pm_model::Value::from(value)),
            })])
            .unwrap(),
        ),
        provisional: false,
    };
    engine.receive(
        WINDOW,
        &event(Event::ApplyPatches(batch(1, 2, 2)), Some(sid)),
        at(2),
    );
    engine.detach(WINDOW);
    engine.attach(WINDOW, PeerKind::Renderer);
    engine.receive(WINDOW, &hello(PeerRole::Renderer), at(3));
    let new = open_session(&mut engine, WINDOW, 3);
    let out = engine.receive(
        WINDOW,
        &request(
            4,
            resync(
                1,
                ResyncSource::Inline {
                    body: Bytes(Vec::new()),
                },
                hash,
            ),
            Some(new),
        ),
        at(4),
    );
    assert!(matches!(
        only_answer(&out, WINDOW),
        Message::Response {
            body: Response::PatchesApplied(_),
            ..
        }
    ));
    engine.receive(
        WINDOW,
        &event(Event::ApplyPatches(batch(1, 2, 2)), Some(new)),
        at(5),
    );
    engine.receive(
        WINDOW,
        &event(Event::ApplyPatches(batch(2, 3, 3)), Some(new)),
        at(6),
    );
    let (_, expected) = document(r#"{"value":3}"#);
    assert_eq!(
        engine.status().sessions[0].hash,
        Some(pm_model::hash::hex(&expected.0))
    );
}
