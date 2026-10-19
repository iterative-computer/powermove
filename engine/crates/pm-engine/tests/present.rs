//! Viewer traffic reaches the installed presenter only for sessions the
//! port owns; everything else keeps today's answers.

use std::sync::{Arc, Mutex};

use pm_engine::engine::testing::{hello, sent};
use pm_engine::engine::{Engine, EngineConfig, PeerKind};
use pm_engine::lease::Category;
use pm_engine::present::{FramePresenter, PresentContext};
use pm_protocol::clock::TransportCommand;
use pm_protocol::codec;
use pm_protocol::color::{OutputColor, Precision, ViewerColorSpace};
use pm_protocol::common::{
    ByteSize, HostMicros, PlaybackRate, RequestId, Revision, Seq, SessionId, Size, Ticks,
};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::frame::{
    FramePurpose, FrameRelease, FrameRequest, MediaPreference, MotionBlurKind, PreviewResolution,
    ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::lease::{Lease, LeaseReason};
use pm_protocol::priority::PriorityClass;
use pm_protocol::version::PeerRole;

const HOST: PortId = PortId(1);
const WINDOW: PortId = PortId(2);
const OTHER: PortId = PortId(3);

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Viewer(PortId, SessionId, u64, Option<Revision>),
    Release(PortId, SessionId, FrameRelease),
    Transport(PortId, SessionId, TransportCommand),
    Render(PortId, SessionId, RequestId, FramePurpose),
    SessionClosed(PortId, SessionId),
    PortClosed(PortId),
}

struct Recorder {
    calls: Arc<Mutex<Vec<Call>>>,
    pool_bytes: u64,
}

impl FramePresenter for Recorder {
    fn viewer(
        &mut self,
        ctx: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        state: ViewerState,
        revision: Option<Revision>,
    ) {
        if !ctx.charge_pool(self.pool_bytes) {
            ctx.charge_over(self.pool_bytes);
            ctx.log(pm_engine::LogLevel::Warn, "pool over the lease");
        }
        self.calls.lock().unwrap().push(Call::Viewer(
            port,
            session,
            state.view_seq.get(),
            revision,
        ));
    }

    fn release(&mut self, port: PortId, session: SessionId, release: FrameRelease) {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Release(port, session, release));
    }

    fn transport(
        &mut self,
        _: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        command: TransportCommand,
    ) {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Transport(port, session, command));
    }

    fn render(
        &mut self,
        _: &mut PresentContext<'_>,
        port: PortId,
        session: SessionId,
        id: RequestId,
        request: FrameRequest,
        _: Option<Revision>,
    ) -> Result<(), Box<EngineError>> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Render(port, session, id, request.purpose));
        if request.output_size.width == 13 {
            return Err(Box::new(EngineError::new(
                ErrorCode::Unsupported,
                ErrorScope::Request { id },
                "thirteen",
            )));
        }
        Ok(())
    }

    fn session_closed(&mut self, ctx: &mut PresentContext<'_>, port: PortId, session: SessionId) {
        ctx.release_pool(self.pool_bytes);
        self.calls
            .lock()
            .unwrap()
            .push(Call::SessionClosed(port, session));
    }

    fn port_closed(&mut self, _: &mut PresentContext<'_>, port: PortId) {
        self.calls.lock().unwrap().push(Call::PortClosed(port));
    }
}

fn at(us: u64) -> HostMicros {
    HostMicros::new(us).unwrap()
}

fn rid(n: u64) -> RequestId {
    RequestId::new(n).unwrap()
}

fn send(engine: &mut Engine, port: PortId, envelope: Envelope) -> Vec<pm_engine::Action> {
    engine.receive(port, &codec::encode(&envelope).unwrap(), at(10))
}

fn request(id: u64, body: Request, session: Option<SessionId>) -> Envelope {
    let mut envelope = Envelope::new(Message::Request { id: rid(id), body });
    envelope.session = session;
    envelope
}

fn event(body: Event, session: Option<SessionId>) -> Envelope {
    let mut envelope = Envelope::new(Message::Event { body });
    envelope.session = session;
    envelope
}

fn setup(pool_bytes: u64) -> (Engine, Arc<Mutex<Vec<Call>>>, SessionId) {
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Interactive));
    let calls = Arc::new(Mutex::new(Vec::new()));
    engine.set_presenter(Box::new(Recorder {
        calls: calls.clone(),
        pool_bytes,
    }));
    for (port, kind, role) in [
        (HOST, PeerKind::Host, PeerRole::Host),
        (WINDOW, PeerKind::Renderer, PeerRole::Renderer),
        (OTHER, PeerKind::Renderer, PeerRole::Renderer),
    ] {
        engine.attach(port, kind);
        engine.receive(port, &hello(role), at(0));
    }
    let out = send(
        &mut engine,
        WINDOW,
        request(
            1,
            Request::OpenSession(OpenSession {
                document: "doc".into(),
                process: EngineProcess::Interactive,
                role: SessionRole::Interactive,
                color: pm_protocol::color::ProjectColor::DEFAULT,
            }),
            None,
        ),
    );
    let session = match &sent(&out, WINDOW)[0].message {
        Message::Response {
            body: Response::SessionOpened { session },
            ..
        } => *session,
        other => panic!("{other:?}"),
    };
    (engine, calls, session)
}

fn viewer_state(view_seq: u64) -> ViewerState {
    ViewerState {
        composition: "main".into(),
        size: Size::new(1600, 900),
        device_pixel_ratio: 2.0,
        zoom: 0.5,
        pan: [0.0, 0.0],
        resolution: PreviewResolution::Automatic,
        media: MediaPreference::Automatic,
        color_space: ViewerColorSpace::Srgb,
        view: None,
        play_every_frame: false,
        edr: false,
        view_seq: Seq::new(view_seq).unwrap(),
        vsync: None,
        depth_id: None,
    }
}

fn frame_request(purpose: FramePurpose, width: u32) -> FrameRequest {
    FrameRequest {
        composition: "main".into(),
        time: Ticks::ZERO,
        purpose,
        priority: PriorityClass::Interactive,
        output_size: Size::new(width, 900),
        region: None,
        scale: 1.0,
        resolution: PreviewResolution::Full,
        media: MediaPreference::Automatic,
        color: OutputColor::Viewer {
            space: ViewerColorSpace::Srgb,
        },
        precision: Precision::Float16,
        motion_blur: MotionBlurKind::PreviewPaused,
        view: None,
        draft_3d: true,
        draft_render: false,
        depth_id: false,
        overlay: true,
    }
}

#[test]
fn viewer_traffic_for_an_owned_session_reaches_the_presenter() {
    let (mut engine, calls, session) = setup(4096);
    let grant = Lease {
        process: EngineProcess::Interactive,
        ack_deadline_ms: 1000,
        epoch: Seq::new(1).unwrap(),
        bytes: ByteSize::new(1 << 30).unwrap(),
        reason: LeaseReason::Initial,
    };
    send(
        &mut engine,
        HOST,
        request(1, Request::GrantLease(grant), None),
    );

    let mut viewer = event(Event::Viewer(viewer_state(3)), Some(session));
    viewer.revision = Some(Revision::new(7).unwrap());
    send(&mut engine, WINDOW, viewer);
    let release = FrameRelease {
        pool_index: 2,
        seq: Seq::new(9).unwrap(),
    };
    send(
        &mut engine,
        WINDOW,
        event(Event::ReleaseFrame(release), Some(session)),
    );
    let play = TransportCommand::Play {
        rate: PlaybackRate::NORMAL,
    };
    send(
        &mut engine,
        WINDOW,
        event(Event::Transport(play), Some(session)),
    );
    let out = send(
        &mut engine,
        WINDOW,
        request(
            5,
            Request::RenderFrame(frame_request(FramePurpose::Scrub, 1600)),
            Some(session),
        ),
    );
    assert!(
        sent(&out, WINDOW).is_empty(),
        "the presenter answers later: {out:?}"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Call::Viewer(WINDOW, session, 3, Some(Revision::new(7).unwrap())),
            Call::Release(WINDOW, session, release),
            Call::Transport(WINDOW, session, play),
            Call::Render(WINDOW, session, rid(5), FramePurpose::Scrub),
        ]
    );
    assert_eq!(engine.lease().used_in(Category::SurfacePool), 4096);
}

#[test]
fn another_ports_session_never_reaches_the_presenter() {
    let (mut engine, calls, session) = setup(0);
    let out = send(
        &mut engine,
        OTHER,
        event(Event::Viewer(viewer_state(1)), Some(session)),
    );
    assert!(out.iter().any(|a| matches!(a, pm_engine::Action::Log { message, .. } if message.contains("unknown session"))));
    let out = send(
        &mut engine,
        OTHER,
        request(
            2,
            Request::RenderFrame(frame_request(FramePurpose::Viewer, 1600)),
            Some(session),
        ),
    );
    assert!(
        matches!(&sent(&out, OTHER)[0].message, Message::Error { error, .. } if error.code == ErrorCode::UnknownSession)
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_presenter_refusal_is_answered_and_other_purposes_stay_unsupported() {
    let (mut engine, _calls, session) = setup(0);
    let out = send(
        &mut engine,
        WINDOW,
        request(
            2,
            Request::RenderFrame(frame_request(FramePurpose::Viewer, 13)),
            Some(session),
        ),
    );
    assert!(
        matches!(&sent(&out, WINDOW)[0].message, Message::Error { id, error } if *id == rid(2) && error.message == "thirteen")
    );
    let mut thumbnail = frame_request(FramePurpose::Thumbnail, 1600);
    thumbnail.priority = PriorityClass::Interactive;
    let out = send(
        &mut engine,
        WINDOW,
        request(3, Request::RenderFrame(thumbnail), Some(session)),
    );
    assert!(
        matches!(&sent(&out, WINDOW)[0].message, Message::Error { error, .. } if error.code == ErrorCode::Unsupported)
    );
}

#[test]
fn closing_a_session_or_its_port_tells_the_presenter() {
    let (mut engine, calls, session) = setup(100);
    send(
        &mut engine,
        WINDOW,
        event(Event::Viewer(viewer_state(1)), Some(session)),
    );
    let out = send(
        &mut engine,
        WINDOW,
        request(2, Request::CloseSession, Some(session)),
    );
    assert!(matches!(
        &sent(&out, WINDOW)[0].message,
        Message::Response {
            body: Response::Ack,
            ..
        }
    ));
    assert_eq!(engine.lease().used_in(Category::SurfacePool), 0);
    engine.detach(WINDOW);
    let calls = calls.lock().unwrap();
    assert_eq!(calls[1], Call::SessionClosed(WINDOW, session));
    assert_eq!(calls[2], Call::PortClosed(WINDOW));
}

#[test]
fn a_pool_past_the_lease_is_charged_over_and_logged() {
    let (mut engine, _calls, session) = setup(1 << 20);
    // No lease granted yet: the pool does not fit, so it is charged over.
    let out = send(
        &mut engine,
        WINDOW,
        event(Event::Viewer(viewer_state(1)), Some(session)),
    );
    assert!(out.iter().any(
        |a| matches!(a, pm_engine::Action::Log { message, .. } if message == "pool over the lease")
    ));
    assert_eq!(engine.lease().used_in(Category::SurfacePool), 1 << 20);
}

/// The host's deadline signals among `actions` (contract 13).
fn deadlines(actions: &[pm_engine::Action]) -> Vec<pm_protocol::slicing::DeadlineSignal> {
    sent(actions, HOST)
        .into_iter()
        .filter_map(|e| match e.message {
            Message::Event {
                body: Event::Deadline(signal),
            } => Some(signal),
            _ => None,
        })
        .collect()
}

fn open_viewer(engine: &mut Engine, port: PortId, id: u64) -> SessionId {
    let out = send(
        engine,
        port,
        request(
            id,
            Request::OpenSession(OpenSession {
                document: "doc".into(),
                process: EngineProcess::Interactive,
                role: SessionRole::Interactive,
                color: pm_protocol::color::ProjectColor::DEFAULT,
            }),
            None,
        ),
    );
    match &sent(&out, port)[0].message {
        Message::Response {
            body: Response::SessionOpened { session },
            ..
        } => *session,
        other => panic!("{other:?}"),
    }
}

/// Playback in any viewer tells the host (which relays it to the job
/// engine) so agent snapshots and exports slice their GPU work; the last
/// viewer to stop, close, or go away ends it.
#[test]
fn viewer_playback_sends_playback_started_and_stopped_to_the_host() {
    use pm_protocol::slicing::DeadlineSignal as D;
    let (mut engine, _calls, a) = setup(0);
    let b = open_viewer(&mut engine, OTHER, 9);
    let play = |rate| Event::Transport(TransportCommand::Play { rate });
    let out = send(
        &mut engine,
        WINDOW,
        event(play(PlaybackRate::NORMAL), Some(a)),
    );
    assert_eq!(
        deadlines(&out),
        vec![D::PlaybackStarted {
            at: at(10),
            rate: PlaybackRate::NORMAL,
            frame_budget_us: 16_667
        }]
    );
    assert!(engine.is_playing());
    // A second viewer joining, seeking, or a repeated play sends nothing.
    assert!(
        deadlines(&send(
            &mut engine,
            OTHER,
            event(play(PlaybackRate::NORMAL), Some(b))
        ))
        .is_empty()
    );
    assert!(
        deadlines(&send(
            &mut engine,
            WINDOW,
            event(play(PlaybackRate::NORMAL), Some(a))
        ))
        .is_empty()
    );
    let seek = Event::Transport(TransportCommand::Step { frames: 1 });
    assert!(deadlines(&send(&mut engine, WINDOW, event(seek, Some(a)))).is_empty());
    // One viewer pausing leaves the other playing.
    let pause = Event::Transport(TransportCommand::Pause);
    assert!(deadlines(&send(&mut engine, WINDOW, event(pause, Some(a)))).is_empty());
    // The last one going away stops playback.
    let out = engine.detach(OTHER);
    assert!(matches!(deadlines(&out)[..], [D::PlaybackStopped { .. }]));
    assert!(!engine.is_playing());
    // Reverse playback counts; closing the session of a playing viewer stops it.
    let out = send(
        &mut engine,
        WINDOW,
        event(play(PlaybackRate { num: -2, den: 1 }), Some(a)),
    );
    assert!(matches!(
        deadlines(&out)[..],
        [D::PlaybackStarted {
            rate: PlaybackRate { num: -2, den: 1 },
            ..
        }]
    ));
    let out = send(
        &mut engine,
        WINDOW,
        request(3, Request::CloseSession, Some(a)),
    );
    assert!(matches!(deadlines(&out)[..], [D::PlaybackStopped { .. }]));
}

/// A presenter that sizes a cache from the lease and reserves it.
struct LeaseWatcher {
    seen: Arc<Mutex<Vec<u64>>>,
    reserved: u64,
}

impl FramePresenter for LeaseWatcher {
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
    fn lease_changed(&mut self, ctx: &mut PresentContext<'_>) {
        ctx.release_frame_cache(self.reserved);
        self.reserved = ctx.granted() / 4;
        assert!(ctx.reserve_frame_cache(self.reserved));
        self.seen.lock().unwrap().push(ctx.granted());
    }
}

/// Every grant reaches the presenter after the ledger took it, so a cache
/// sized from the lease follows grows and shrinks; a shrink's owed
/// acknowledgement goes out once the presenter's reservation fits.
#[test]
fn every_lease_grant_reaches_the_presenter() {
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Interactive));
    let seen = Arc::new(Mutex::new(Vec::new()));
    engine.set_presenter(Box::new(LeaseWatcher {
        seen: seen.clone(),
        reserved: 0,
    }));
    engine.attach(HOST, PeerKind::Host);
    engine.receive(HOST, &hello(PeerRole::Host), at(0));
    for (epoch, bytes) in [(1, 1u64 << 30), (2, 1 << 28)] {
        let grant = Lease {
            process: EngineProcess::Interactive,
            ack_deadline_ms: 1000,
            epoch: Seq::new(epoch).unwrap(),
            bytes: ByteSize::new(bytes).unwrap(),
            reason: LeaseReason::Initial,
        };
        send(
            &mut engine,
            HOST,
            request(epoch, Request::GrantLease(grant), None),
        );
        assert_eq!(engine.lease().used_in(Category::FrameCache), bytes / 4);
    }
    assert_eq!(*seen.lock().unwrap(), vec![1 << 30, 1 << 28]);
    assert!(engine.lease().fits());
}
