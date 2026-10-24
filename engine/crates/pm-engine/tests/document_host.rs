//! The interactive engine's document host (plan section 6; contracts 5 and
//! 14): the canonical patch stream pm-wasm receives applied to the
//! session's mirror with the built-in `canonicalV1` decoder, every change
//! forwarded to a presenter that renders documents, in revision order, and
//! the document's media resolved through the host (a `mediaRead` handle
//! for the session, then `resolveHandle`), with retries for media the host
//! cannot resolve yet.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pm_engine::document::{DocumentUpdate, MEDIA_RETRY_US};
use pm_engine::engine::testing::{hello, sent};
use pm_engine::engine::{Engine, EngineConfig, PeerKind};
use pm_engine::present::{FramePresenter, PresentContext};
use pm_protocol::codec;
use pm_protocol::color::ProjectColor;
use pm_protocol::common::{Bytes, ContentHash, HostMicros, RequestId, Revision, SessionId};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::frame::{FrameRelease, FrameRequest, ViewerState};
use pm_protocol::host::{
    CapabilityHandle, CapabilityKind, HandleAccess, HandleGrant, HandleResolved, PortId,
};
use pm_protocol::patch::{
    DocumentEncoding, PatchBatch, PatchEncoding, PatchOrigin, Resync, ResyncSource,
};
use pm_protocol::version::PeerRole;

const HOST: PortId = PortId(1);
const WINDOW: PortId = PortId(2);

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Document(u64, Option<pm_model::Value>),
    Media(String, Option<PathBuf>),
}

#[derive(Clone, Default)]
struct Recorder {
    seen: Arc<Mutex<Vec<Seen>>>,
    mirror: Arc<Mutex<pm_model::Document>>,
}

impl FramePresenter for Recorder {
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
        _: pm_protocol::clock::TransportCommand,
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
    fn renders_documents(&self) -> bool {
        true
    }
    fn document(
        &mut self,
        _: &mut PresentContext<'_>,
        _: PortId,
        _: SessionId,
        update: DocumentUpdate,
        _: bool,
    ) {
        let mut mirror = self.mirror.lock().unwrap();
        update.apply_to(&mut mirror).unwrap();
        self.seen
            .lock()
            .unwrap()
            .push(Seen::Document(update.revision().get(), mirror.to_value()));
    }
    fn media(
        &mut self,
        _: &mut PresentContext<'_>,
        _: PortId,
        _: SessionId,
        asset: &str,
        path: Option<PathBuf>,
    ) {
        self.seen
            .lock()
            .unwrap()
            .push(Seen::Media(asset.into(), path));
    }
}

fn at(us: u64) -> HostMicros {
    HostMicros::new(us).unwrap()
}

fn encode(envelope: Envelope) -> Vec<u8> {
    codec::encode(&envelope).unwrap()
}

fn request(id: u64, body: Request, session: Option<SessionId>) -> Vec<u8> {
    let mut envelope = Envelope::new(Message::Request {
        id: RequestId::new(id).unwrap(),
        body,
    });
    envelope.session = session;
    encode(envelope)
}

fn engine_with(recorder: &Recorder) -> (Engine, SessionId) {
    let mut engine = Engine::new(EngineConfig::new(EngineProcess::Interactive));
    engine.set_presenter(Box::new(recorder.clone()));
    for (port, kind, role) in [
        (HOST, PeerKind::Host, PeerRole::Host),
        (WINDOW, PeerKind::Renderer, PeerRole::Renderer),
    ] {
        engine.attach(port, kind);
        engine.receive(port, &hello(role), at(0));
    }
    let out = engine.receive(
        WINDOW,
        &request(
            1,
            Request::OpenSession(OpenSession {
                document: "project-a".into(),
                role: SessionRole::Interactive,
                process: EngineProcess::Interactive,
                color: ProjectColor::DEFAULT,
            }),
            None,
        ),
        at(1),
    );
    let session = sent(&out, WINDOW)
        .into_iter()
        .find_map(|e| match e.message {
            Message::Response {
                body: Response::SessionOpened { session },
                ..
            } => Some(session),
            _ => None,
        })
        .unwrap();
    (engine, session)
}

fn resync(document: &str, revision: u64, text: &str) -> Request {
    let value = pm_model::json::parse(text).unwrap();
    Request::Resync(Resync {
        document: document.into(),
        revision: Revision::new(revision).unwrap(),
        encoding: DocumentEncoding::CanonicalJsonV1,
        source: ResyncSource::Inline {
            body: Bytes(text.as_bytes().to_vec()),
        },
        hash: ContentHash(pm_model::hash::canonical_digest(&value)),
    })
}

/// The canonical binary batch taking `before` to `after`, as pm-wasm emits it.
fn batch(document: &str, base: u64, revision: u64, before: &str, after: &str) -> Event {
    let ops = pm_model::ops::diff_ops(
        Some(&pm_model::json::parse(before).unwrap()),
        Some(&pm_model::json::parse(after).unwrap()),
    );
    Event::ApplyPatches(PatchBatch {
        document: document.into(),
        base_revision: Revision::new(base).unwrap(),
        revision: Revision::new(revision).unwrap(),
        origin: PatchOrigin::Edit,
        encoding: PatchEncoding::CanonicalV1,
        patch_count: ops.len() as u32,
        body: Bytes(pm_model::binary::encode(&ops).unwrap()),
        provisional: false,
    })
}

fn event(body: Event, session: SessionId) -> Vec<u8> {
    let mut envelope = Envelope::new(Message::Event { body });
    envelope.session = Some(session);
    encode(envelope)
}

/// The host's pending requests among `out`.
fn host_calls(out: &[pm_engine::Action]) -> Vec<(RequestId, Request)> {
    sent(out, HOST)
        .into_iter()
        .filter_map(|e| match e.message {
            Message::Request { id, body } => Some((id, body)),
            _ => None,
        })
        .collect()
}

fn handle(byte: u8) -> CapabilityHandle {
    CapabilityHandle::from_random_bytes([byte; 16])
}

fn answer(id: RequestId, body: Response) -> Vec<u8> {
    encode(Envelope::new(Message::Response { id, body }))
}

#[test]
fn the_patch_stream_reaches_the_mirror_and_the_presenter_in_order() {
    let recorder = Recorder::default();
    let (mut engine, session) = engine_with(&recorder);
    let a = r#"{"assets":{},"layers":[{"id":"l1","x":1}],"name":"A"}"#;
    let b = r#"{"assets":{},"layers":[{"id":"l1","x":2}],"name":"A"}"#;
    let c = r#"{"assets":{},"layers":[{"id":"l2"},{"id":"l1","x":2}],"name":"A"}"#;
    engine.receive(
        WINDOW,
        &request(2, resync("project-a", 1, a), Some(session)),
        at(2),
    );
    let out = engine.receive(
        WINDOW,
        &event(batch("project-a", 1, 2, a, b), session),
        at(3),
    );
    assert!(sent(&out, WINDOW).iter().any(|e| matches!(
        e.message,
        Message::Event { body: Event::PatchesApplied(ref p) } if p.revision.get() == 2
    )));
    engine.receive(
        WINDOW,
        &event(batch("project-a", 2, 3, b, c), session),
        at(4),
    );
    let seen = recorder.seen.lock().unwrap().clone();
    let documents: Vec<_> = seen
        .iter()
        .filter_map(|s| match s {
            Seen::Document(r, v) => Some((*r, v.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        documents,
        vec![
            (1, Some(pm_model::json::parse(a).unwrap())),
            (2, Some(pm_model::json::parse(b).unwrap())),
            (3, Some(pm_model::json::parse(c).unwrap())),
        ]
    );
    let expected = ContentHash(pm_model::hash::canonical_digest(
        &pm_model::json::parse(c).unwrap(),
    ));
    assert_eq!(
        engine.status().sessions[0].hash.as_deref(),
        Some(pm_model::hash::hex(&expected.0).as_str())
    );
    assert!(!engine.status().sessions[0].stale);
}

#[test]
fn a_resync_naming_another_project_rebinds_an_interactive_session() {
    let recorder = Recorder::default();
    let (mut engine, session) = engine_with(&recorder);
    engine.receive(
        WINDOW,
        &request(2, resync("project-a", 1, r#"{"name":"A"}"#), Some(session)),
        at(2),
    );
    let out = engine.receive(
        WINDOW,
        &request(3, resync("project-b", 7, r#"{"name":"B"}"#), Some(session)),
        at(3),
    );
    assert!(sent(&out, WINDOW).iter().any(|e| matches!(
        e.message,
        Message::Response { body: Response::PatchesApplied(ref p), .. } if p.document.as_str() == "project-b" && p.revision.get() == 7
    )));
    assert_eq!(engine.status().sessions[0].document, "project-b");
    // Patches for the old project are ignored now.
    engine.receive(
        WINDOW,
        &event(
            batch("project-a", 7, 8, r#"{"name":"B"}"#, r#"{"name":"C"}"#),
            session,
        ),
        at(4),
    );
    assert_eq!(engine.status().sessions[0].revision, 7);
}

#[test]
fn media_resolves_through_host_handles_and_retries_when_offline() {
    let recorder = Recorder::default();
    let (mut engine, session) = engine_with(&recorder);
    let doc =
        r#"{"assets":{"clip":{"id":"clip","name":"clip.mov"},"logo":{"id":"logo"}},"layers":[]}"#;
    let out = engine.receive(
        WINDOW,
        &request(2, resync("project-a", 1, doc), Some(session)),
        at(10),
    );
    let mut calls = host_calls(&out);
    assert_eq!(calls.len(), 2, "{calls:?}");
    let assets: Vec<String> = calls
        .iter()
        .map(|(_, body)| match body {
            Request::MintHandle(m) => {
                assert_eq!(m.session, session);
                match &m.kind {
                    CapabilityKind::MediaRead { asset } => asset.as_str().to_owned(),
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(assets, ["clip", "logo"]);

    // The clip mints, then resolves.
    let (clip_id, _) = calls.remove(0);
    let grant = Response::Handle(HandleGrant {
        handle: handle(1),
        kind: CapabilityKind::MediaRead {
            asset: "clip".into(),
        },
        content_hash: None,
    });
    let out = engine.receive(HOST, &answer(clip_id, grant), at(11));
    let (resolve_id, resolve) = host_calls(&out).remove(0);
    assert!(
        matches!(resolve, Request::ResolveHandle(ref r) if r.handle == handle(1) && r.session == session)
    );
    let resolved = Response::HandleResolved(HandleResolved {
        handle: handle(1),
        kind: CapabilityKind::MediaRead {
            asset: "clip".into(),
        },
        access: HandleAccess::Read,
        path: "/store/objects/ab/clip".into(),
    });
    engine.receive(HOST, &answer(resolve_id, resolved), at(12));

    // The logo is offline (not bound yet): the presenter hears so, and the
    // engine tries again after the first retry delay.
    let (logo_id, _) = calls.remove(0);
    let offline = encode(Envelope::new(Message::Error {
        id: logo_id,
        error: EngineError::new(ErrorCode::MediaOffline, ErrorScope::Connection, "offline"),
    }));
    engine.receive(HOST, &offline, at(13));
    let seen: Vec<Seen> = recorder
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| matches!(s, Seen::Media(..)))
        .cloned()
        .collect();
    assert_eq!(
        seen,
        [
            Seen::Media("clip".into(), Some("/store/objects/ab/clip".into())),
            Seen::Media("logo".into(), None),
        ]
    );
    let now = pm_engine::clock::host_now().get();
    assert!(host_calls(&engine.tick(at(now))).is_empty());
    let out = engine.tick(at(now + MEDIA_RETRY_US[1] + 1));
    let retry = host_calls(&out);
    assert_eq!(retry.len(), 1);
    assert!(
        matches!(&retry[0].1, Request::MintHandle(m) if matches!(&m.kind, CapabilityKind::MediaRead { asset } if asset.as_str() == "logo"))
    );

    // An unchanged record is not resolved again; a relinked one is, and an
    // asset that leaves the document goes offline for the presenter.
    let relinked = r#"{"assets":{"clip":{"id":"clip","name":"clip.mov","link":{"path":"/Volumes/A/clip.mov"}}},"layers":[]}"#;
    let out = engine.receive(
        WINDOW,
        &event(batch("project-a", 1, 2, doc, relinked), session),
        at(now + 20_000_000),
    );
    let calls = host_calls(&out);
    assert_eq!(calls.len(), 1);
    assert!(
        matches!(&calls[0].1, Request::MintHandle(m) if matches!(&m.kind, CapabilityKind::MediaRead { asset } if asset.as_str() == "clip"))
    );
    assert!(
        recorder
            .seen
            .lock()
            .unwrap()
            .contains(&Seen::Media("logo".into(), None))
    );
}

#[test]
fn viewer_playback_is_signalled_to_the_host() {
    let recorder = Recorder::default();
    let (mut engine, session) = engine_with(&recorder);
    let transport = |command| event(Event::Transport(command), session);
    let out = engine.receive(
        WINDOW,
        &transport(pm_protocol::clock::TransportCommand::Play {
            rate: pm_protocol::common::PlaybackRate::NORMAL,
        }),
        at(5),
    );
    let deadline = |out: &[pm_engine::Action]| {
        sent(out, HOST)
            .into_iter()
            .filter_map(|e| match e.message {
                Message::Event {
                    body: Event::Deadline(signal),
                } => Some(signal),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(matches!(
        deadline(&out)[..],
        [pm_protocol::slicing::DeadlineSignal::PlaybackStarted { .. }]
    ));
    assert!(engine.is_playing());
    // A second Play while playing says nothing new.
    let out = engine.receive(
        WINDOW,
        &transport(pm_protocol::clock::TransportCommand::Play {
            rate: pm_protocol::common::PlaybackRate::NORMAL,
        }),
        at(6),
    );
    assert!(deadline(&out).is_empty());
    let out = engine.receive(
        WINDOW,
        &transport(pm_protocol::clock::TransportCommand::Pause),
        at(7),
    );
    assert!(matches!(
        deadline(&out)[..],
        [pm_protocol::slicing::DeadlineSignal::PlaybackStopped { .. }]
    ));
    assert!(!engine.is_playing());
}
