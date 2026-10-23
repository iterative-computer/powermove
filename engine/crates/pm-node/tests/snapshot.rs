//! Engine snapshots (the Phase 3c render transport), without Electron: this
//! test process plays main (the bridge's rendezvous) and re-runs itself as
//! a child that plays the interactive engine with the document frame
//! source. The child drives the engine through the protocol as main and the
//! kernel's engine connection would, and checks:
//!
//! * a `sync` snapshot of a session with no viewer is the document's frame
//!   at its revision (inline pixels), and a large one comes through a
//!   shared-memory segment main takes once;
//! * read-your-writes: a snapshot asked at a revision the engine has not
//!   reached waits for it (bounded by `wait_ms`), and one that never comes
//!   fails with `timeout`;
//! * only the host asks, and only for a session the named window owns;
//! * the engine never sends anything to the renderer while it serves a
//!   snapshot (a renderer blocked in `sendSync` cannot deadlock it);
//! * scheduling: during playback a `sync` snapshot is answered at once, an
//!   `idle` one only after the transport stops;
//! * Blender compositions: a synchronous capture draws a prepared frame or
//!   fails with today's sentence; an async capture with a Blender wait has
//!   Blender render the frame first (a stand-in for main's supervision) and
//!   draws its surfaces, and Blender's failure, absence, and timeout are
//!   named.
#![cfg(target_os = "macos")]

use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pm_bridge::audit::{ExpectedPeer, SignaturePolicy};
use pm_bridge::host::{self, SurfaceTable};
use pm_bridge::rendezvous::{Rendezvous, SERVICE_PREFIX, random_service_name};
use pm_engine::Action;
use pm_engine::engine::testing::hello;
use pm_node::EngineHost;
use pm_node::present::{BRIDGE_ENV, HOST_CLOCK_ENV, SOURCE_ENV};
use pm_protocol::clock::{SeekMode, TransportCommand, TransportState};
use pm_protocol::codec;
use pm_protocol::color::{ProjectColor, ViewerColorSpace};
use pm_protocol::common::{
    Bytes, ContentHash, PlaybackRate, Rect, RequestId, Revision, Seq, SessionId, Size, Ticks,
};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::{EngineProcess, ErrorCode};
use pm_protocol::frame::{
    EditorCamera, FrameRelease, MediaPreference, PreviewResolution, Projection, Shading3d,
    ViewOverride, ViewerCanvas, ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::patch::{DocumentEncoding, Resync, ResyncSource};
use pm_protocol::snapshot::{
    HostSnapshot, PixelPayload, SnapshotClass, SnapshotOutcome, SnapshotPixels, SnapshotRequest,
    SnapshotResult, SnapshotTarget, SnapshotViewer,
};
use pm_protocol::version::PeerRole;
use serde_json::{Value, json};

const CHILD_ENV: &str = "PM_SNAPSHOT_TEST_CHILD";
const HOST: PortId = PortId(1);
const WINDOW: PortId = PortId(7);

fn run_child(child: &str) -> String {
    run_child_with(child, &[])
}

fn run_child_with(child: &str, envs: &[(&str, &str)]) -> String {
    let name = random_service_name(SERVICE_PREFIX).unwrap();
    let server = Rendezvous::register(&name).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            child,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .envs(envs.iter().copied())
        .env(CHILD_ENV, "1")
        .env(BRIDGE_ENV, &name)
        .env(SOURCE_ENV, "document")
        .env(HOST_CLOCK_ENV, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let expected =
        ExpectedPeer::for_child(child.id() as i32, SignaturePolicy::UnsignedDevBypass).unwrap();
    let table = Arc::new(Mutex::new(SurfaceTable::default()));
    let main = std::thread::spawn(move || {
        let stop = AtomicBool::new(false);
        host::run(
            server,
            &expected,
            Duration::from_secs(60),
            &table,
            &stop,
            &mut |_| {},
        );
    });
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "child failed:\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("CHILD-OK"),
        "child did not finish:\n{stdout}\n{stderr}"
    );
    main.join().unwrap();
    for line in stdout.lines().filter(|l| l.contains("CHILD-OK")) {
        eprintln!("{line}");
    }
    stdout
}

#[test]
fn snapshots_render_the_session_document_at_its_revision() {
    run_child("child_snapshots");
}

#[test]
fn viewer_snapshots_render_the_pinned_canvas_without_changing_the_live_view() {
    run_child("child_viewer_snapshot");
}

#[test]
fn viewer_snapshots_apply_the_pinned_editor_camera_and_local_view() {
    run_child("child_viewer_camera_snapshot");
}

#[test]
fn headless_snapshots_are_scheduled_around_audio_only_playback() {
    run_child("child_headless_scheduling");
}

#[test]
fn snapshots_are_scheduled_around_playback() {
    run_child("child_scheduling");
}

#[test]
fn snapshots_that_need_the_renderer_name_what_it_draws() {
    run_child("child_fallback");
}

#[test]
fn snapshots_send_an_effects_textures_to_main_ahead_of_the_answer() {
    run_child("child_fallback_effect");
}

#[test]
fn frames_carry_their_depth_and_id_surface_when_the_viewer_asks() {
    run_child("child_depth");
}

#[test]
fn blender_compositions_are_captured_as_today() {
    run_child("child_blender");
}

#[test]
fn the_host_snapshots_sessions_it_opened_for_a_window() {
    run_child("child_host_session");
}

#[test]
fn level1_svg_snapshots_draw_the_windows_rasters() {
    run_child("child_svg");
}

#[test]
fn a_snapshot_that_fails_on_fonts_leaves_the_viewer_asking_for_svg_rasters() {
    run_child("child_svg_font_failure");
}

/// The viewer waits for an SVG bucket without a stand-in, but a bucket
/// the window could not draw ends the wait at once (the frame is drawn
/// without the image, as today), is asked for again after a while, and
/// draws when it arrives (pm-node `present::wait`).
#[test]
fn an_svg_raster_the_window_cannot_draw_never_freezes_the_viewer() {
    run_child("child_svg_viewer_failure");
}

/// A bucket the window never answers for: the viewer's wait times out at
/// its bound and the frame is drawn without it.
#[test]
fn an_unanswered_svg_raster_times_out_instead_of_freezing_the_viewer() {
    run_child_with(
        "child_svg_viewer_timeout",
        &[(pm_node::present::EXTERNAL_WAIT_ENV, SVG_WAIT_CAP_MS)],
    );
}

/// The wait cap the SVG timeout child runs with (ms).
const SVG_WAIT_CAP_MS: &str = "1500";

/// Every message the engine sent, by port.
#[derive(Default)]
struct Inbox {
    messages: Mutex<Vec<(PortId, Envelope)>>,
    arrived: Condvar,
}

impl Inbox {
    fn push(&self, actions: Vec<Action>) {
        let mut messages = self.messages.lock().unwrap();
        for action in actions {
            match action {
                Action::Send { port, bytes } => {
                    messages.push((port, codec::decode(&bytes).expect("engine output decodes")));
                }
                Action::Log { level, message } => eprintln!("engine {level:?}: {message}"),
                _ => {}
            }
        }
        self.arrived.notify_all();
    }

    fn wait<T>(
        &self,
        timeout: Duration,
        mut pick: impl FnMut(PortId, &Envelope) -> Option<T>,
    ) -> T {
        self.try_wait(timeout, &mut pick).unwrap_or_else(|| {
            panic!(
                "timed out; inbox: {:?}",
                self.messages
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(p, m)| (p.0, &m.message))
                    .collect::<Vec<_>>()
            )
        })
    }

    fn try_wait<T>(
        &self,
        timeout: Duration,
        pick: &mut impl FnMut(PortId, &Envelope) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + timeout;
        let mut messages = self.messages.lock().unwrap();
        loop {
            if let Some(i) = messages.iter().position(|(p, m)| pick(*p, m).is_some()) {
                let found = pick(messages[i].0, &messages[i].1).unwrap();
                messages.remove(i);
                return Some(found);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            messages = self.arrived.wait_timeout(messages, left).unwrap().0;
        }
    }

    /// Messages to `port` received so far (left in the inbox).
    fn count(&self, port: PortId) -> usize {
        self.messages
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| *p == port)
            .count()
    }
}

/// The answer to request `id` on the host port: the outcome or an error.
fn answer(id: u64) -> impl FnMut(PortId, &Envelope) -> Option<Result<SnapshotOutcome, ErrorCode>> {
    move |port, m| {
        if port != HOST {
            return None;
        }
        match &m.message {
            Message::Response {
                id: got,
                body: Response::Snapshot(outcome),
            } if got.get() == id => Some(Ok((**outcome).clone())),
            Message::Error { id: got, error } if got.get() == id => Some(Err(error.code)),
            _ => None,
        }
    }
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn solid(x: f64) -> Value {
    json!({
        "id": "box", "type": "solid", "name": "box", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(x)),
            "position.y": ch(json!(60)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
        "d": { "color": "#E8E2CF", "w": 40, "h": 20 }, "locked_intent": {}
    })
}

fn project(x: f64) -> Value {
    json!({
        "id": "Psnap", "name": "snap", "w": 320, "h": 180, "fps": 30, "dur": 2, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#202020", "position": 0 }] },
        "compId": "Psnap-root", "compName": "snap", "markers": [], "work": [0, 2], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [solid(x)], "assets": {}, "comps": {}
    })
}

struct Child {
    host: EngineHost,
    inbox: Arc<Inbox>,
    session: SessionId,
}

impl Child {
    fn start() -> Child {
        let inbox = Arc::new(Inbox::default());
        let sink_inbox = inbox.clone();
        let mut host = EngineHost::with_sink(
            EngineProcess::Interactive,
            Some(Arc::new(move |actions| sink_inbox.push(actions))),
            |key: &str| std::env::var(key).ok(),
        )
        .unwrap();
        host.attach(HOST.0, "host", None).unwrap();
        inbox.push(host.receive(HOST.0, &hello(PeerRole::Host)));
        host.attach(WINDOW.0, "renderer", Some(WINDOW.0)).unwrap();
        inbox.push(host.receive(WINDOW.0, &hello(PeerRole::Renderer)));
        let mut child = Child {
            host,
            inbox,
            session: SessionId::new(1).unwrap(),
        };
        child.window(Envelope::new(Message::Request {
            id: RequestId::new(1).unwrap(),
            body: Request::OpenSession(OpenSession {
                document: "Psnap".into(),
                role: SessionRole::Interactive,
                process: EngineProcess::Interactive,
                color: ProjectColor::DEFAULT,
            }),
        }));
        child.session = child
            .inbox
            .wait(Duration::from_secs(5), |port, m| match &m.message {
                Message::Response {
                    body: Response::SessionOpened { session },
                    ..
                } if port == WINDOW => Some(*session),
                _ => None,
            });
        child
    }

    fn window(&mut self, envelope: Envelope) {
        let actions = self
            .host
            .receive(WINDOW.0, &codec::encode(&envelope).unwrap());
        self.inbox.push(actions);
    }

    fn host_sends(&mut self, envelope: Envelope) {
        let actions = self
            .host
            .receive(HOST.0, &codec::encode(&envelope).unwrap());
        self.inbox.push(actions);
    }

    fn resync(&mut self, id: u64, revision: u64, doc: &Value) {
        let parsed = pm_model::json::parse(&doc.to_string()).unwrap();
        let session = self.session;
        self.window(
            Envelope::new(Message::Request {
                id: RequestId::new(id).unwrap(),
                body: Request::Resync(Resync {
                    document: "Psnap".into(),
                    revision: Revision::new(revision).unwrap(),
                    encoding: DocumentEncoding::CanonicalJsonV1,
                    source: ResyncSource::Inline {
                        body: Bytes(doc.to_string().into_bytes()),
                    },
                    hash: ContentHash(pm_model::hash::canonical_digest(&parsed)),
                }),
            })
            .with_session(session),
        );
        self.inbox
            .wait(Duration::from_secs(5), |port, m| match &m.message {
                Message::Response {
                    id: got,
                    body: Response::PatchesApplied(_),
                } if port == WINDOW && got.get() == id => Some(()),
                _ => None,
            });
    }

    fn snapshot(
        &mut self,
        id: u64,
        request: SnapshotRequest,
        revision: Option<u64>,
        renderer: PortId,
        session: SessionId,
    ) {
        let mut envelope = Envelope::new(Message::Request {
            id: RequestId::new(id).unwrap(),
            body: Request::Snapshot(HostSnapshot {
                renderer,
                session,
                snapshot: request,
            }),
        });
        envelope.revision = revision.map(|r| Revision::new(r).unwrap());
        self.host_sends(envelope);
    }

    fn event(&mut self, body: Event) {
        let session = self.session;
        self.window(Envelope::new(Message::Event { body }).with_session(session));
    }
}

fn frame_request(
    class: SnapshotClass,
    size: Size,
    pixels: SnapshotPixels,
    wait_ms: u32,
) -> SnapshotRequest {
    SnapshotRequest {
        composition: "Psnap-root".into(),
        time: Ticks::ZERO,
        output_size: size,
        target: SnapshotTarget::Frame {
            motion_blur_samples: 0,
            background: true,
            draft_3d: false,
        },
        pixels,
        class,
        view_seq: Some(Seq::new(4).unwrap()),
        viewer: None,
        wait_ms,
        blender_wait_ms: 0,
        captions_hidden: false,
        uploaded: Vec::new(),
    }
}

fn pixels_of(result: &SnapshotResult) -> Vec<u8> {
    match &result.payload {
        PixelPayload::Inline { bytes } => bytes.0.clone(),
        PixelPayload::SharedMemory { name, length } => {
            pm_bridge::shm::take(name, length.get() as usize).expect("main takes the segment")
        }
    }
}

fn at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

fn rendered(outcome: Result<SnapshotOutcome, ErrorCode>) -> SnapshotResult {
    match outcome {
        Ok(SnapshotOutcome::Rendered(result)) => result,
        other => panic!("expected pixels, got {other:?}"),
    }
}

const BOX: [u8; 4] = [0xE8, 0xE2, 0xCF, 255];
const BACKGROUND: [u8; 4] = [0x20, 0x20, 0x20, 255];

#[test]
#[ignore = "run by snapshots_render_the_session_document_at_its_revision"]
fn child_snapshots() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let mut c = Child::start();
    let session = c.session;
    c.resync(2, 1, &project(100.0));
    let before = c.inbox.count(WINDOW);

    // A sync snapshot of a session with no viewer: inline pixels at the
    // document's revision, the box where the document puts it.
    let small = Size::new(320, 180);
    c.snapshot(
        10,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(1),
        WINDOW,
        session,
    );
    let result = rendered(c.inbox.wait(Duration::from_secs(20), answer(10)));
    assert_eq!(
        (result.width, result.height, result.revision.get()),
        (320, 180, 1)
    );
    assert_eq!(result.view_seq, Some(Seq::new(4).unwrap()));
    assert!(matches!(result.payload, PixelPayload::Inline { .. }));
    let px = pixels_of(&result);
    assert_eq!(at(&px, 320, 120, 70), BOX);
    assert_eq!(at(&px, 320, 10, 10), BACKGROUND);
    assert_eq!(at(&px, 320, 60, 70), BACKGROUND);

    // Larger than the inline limit: a shared-memory segment, taken once,
    // the same frame at twice the scale.
    let big = Size::new(1280, 720);
    c.snapshot(
        11,
        frame_request(SnapshotClass::Async, big, SnapshotPixels::Opaque, 5_000),
        None,
        WINDOW,
        session,
    );
    let result = rendered(c.inbox.wait(Duration::from_secs(20), answer(11)));
    let name = match &result.payload {
        PixelPayload::SharedMemory { name, .. } => name.clone(),
        other => panic!("expected shared memory, got {other:?}"),
    };
    let px = pixels_of(&result);
    assert_eq!(px.len(), 1280 * 720 * 4);
    assert_eq!(at(&px, 1280, 480, 280), BOX);
    assert_eq!(at(&px, 1280, 40, 40), BACKGROUND);
    assert!(pm_bridge::shm::take(&name, 16).is_err(), "taken once");

    // Straight alpha without the background: the box is opaque, the rest
    // transparent.
    let mut clear = frame_request(SnapshotClass::Sync, small, SnapshotPixels::Straight, 5_000);
    clear.target = SnapshotTarget::Frame {
        motion_blur_samples: 0,
        background: false,
        draft_3d: false,
    };
    c.snapshot(12, clear, None, WINDOW, session);
    let px = pixels_of(&rendered(c.inbox.wait(Duration::from_secs(20), answer(12))));
    assert_eq!(at(&px, 320, 120, 70), BOX);
    assert_eq!(at(&px, 320, 10, 10)[3], 0);

    // Read-your-writes: asked at revision 2 before the engine has it, the
    // snapshot waits and shows revision 2's box.
    c.snapshot(
        13,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(2),
        WINDOW,
        session,
    );
    assert!(
        c.inbox
            .try_wait(Duration::from_millis(300), &mut answer(13))
            .is_none(),
        "answered before its revision arrived"
    );
    c.resync(3, 2, &project(200.0));
    let result = rendered(c.inbox.wait(Duration::from_secs(20), answer(13)));
    assert_eq!(result.revision.get(), 2);
    let px = pixels_of(&result);
    assert_eq!(at(&px, 320, 120, 70), BACKGROUND, "the box moved away");
    assert_eq!(at(&px, 320, 220, 70), BOX);

    // The output target is reused only at the matching size and is drawn
    // afresh. Resize up and back after the document revision changed: no
    // pixels from the old location or the transparent capture may survive.
    c.snapshot(
        20,
        frame_request(SnapshotClass::Sync, big, SnapshotPixels::Opaque, 5_000),
        Some(2),
        WINDOW,
        session,
    );
    let px = pixels_of(&rendered(c.inbox.wait(Duration::from_secs(20), answer(20))));
    assert_eq!(at(&px, 1280, 480, 280), BACKGROUND);
    assert_eq!(at(&px, 1280, 800, 280), BOX);
    c.snapshot(
        21,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(2),
        WINDOW,
        session,
    );
    let px = pixels_of(&rendered(c.inbox.wait(Duration::from_secs(20), answer(21))));
    assert_eq!(at(&px, 320, 120, 70), BACKGROUND);
    assert_eq!(at(&px, 320, 200, 70), BOX);
    assert_eq!(at(&px, 320, 10, 10), BACKGROUND);

    // A revision that never comes: a bounded wait, then `timeout`.
    let started = Instant::now();
    c.snapshot(
        14,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 300),
        Some(9),
        WINDOW,
        session,
    );
    assert_eq!(
        c.inbox.wait(Duration::from_secs(5), answer(14)),
        Err(ErrorCode::Timeout)
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );

    // Only for a session the named window owns; only from the host.
    c.snapshot(
        15,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 500),
        None,
        PortId(99),
        session,
    );
    assert_eq!(
        c.inbox.wait(Duration::from_secs(5), answer(15)),
        Err(ErrorCode::UnknownSession)
    );
    c.snapshot(
        16,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 500),
        None,
        WINDOW,
        SessionId::new(77).unwrap(),
    );
    assert_eq!(
        c.inbox.wait(Duration::from_secs(5), answer(16)),
        Err(ErrorCode::UnknownSession)
    );
    c.window(Envelope::new(Message::Request {
        id: RequestId::new(17).unwrap(),
        body: Request::Snapshot(HostSnapshot {
            renderer: WINDOW,
            session,
            snapshot: frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 500),
        }),
    }));
    let refused = c
        .inbox
        .wait(Duration::from_secs(5), |port, m| match &m.message {
            Message::Error { id, error } if port == WINDOW && id.get() == 17 => Some(error.code),
            _ => None,
        });
    assert_eq!(refused, ErrorCode::CapabilityRejected);

    // Layer rasters (`render.raster`'s base): a solid's raster is today's
    // `rasterShape` of its content (a 40×20 box with today's 4 px pad), with
    // today's anchor, box, and selection.
    let raster_request = |crop, content| {
        let mut layer = frame_request(
            SnapshotClass::Sync,
            small,
            SnapshotPixels::Premultiplied,
            2_000,
        );
        layer.target = SnapshotTarget::Layer {
            layer: "box".into(),
            scale: 1.0,
            crop,
            content,
        };
        layer
    };
    c.snapshot(19, raster_request(None, None), None, WINDOW, session);
    let shape = rendered(c.inbox.wait(Duration::from_secs(10), answer(19)));
    assert_eq!((shape.width, shape.height), (48, 28));
    assert_eq!(shape.origin, [4.0, 4.0]);
    assert_eq!(shape.layer_box, Some([48.0, 28.0]));
    let geometry = shape.raster.clone().expect("raster geometry");
    assert_eq!(geometry.canvas, [48, 28]);
    assert_eq!(geometry.selection, Some([-20.0, -10.0, 20.0, 10.0]));
    let px = pixels_of(&shape);
    assert_eq!(at(&px, 48, 24, 14), BOX, "the box's color inside");
    assert_eq!(at(&px, 48, 1, 1)[3], 0, "the pad is transparent");
    // A crop window of a shape is cut from the same raster.
    let window = pm_protocol::common::PixelRect {
        x: 4,
        y: 4,
        width: 10,
        height: 6,
    };
    c.snapshot(
        23,
        raster_request(Some(window), None),
        None,
        WINDOW,
        session,
    );
    let cut = rendered(c.inbox.wait(Duration::from_secs(10), answer(23)));
    assert_eq!((cut.width, cut.height), (10, 6));
    let g = cut.raster.clone().unwrap();
    assert_eq!((g.window, g.cropped, g.canvas), ([4, 4], true, [48, 28]));
    let cut_px = pixels_of(&cut);
    for y in 0..6 {
        for x in 0..10 {
            assert_eq!(at(&cut_px, 10, x, y), at(&px, 48, x + 4, y + 4));
        }
    }
    // A layer object the caller passed (a wrapper's modified copy) is drawn
    // instead of the document's: 80 units wide.
    let mut wide = solid(100.0);
    wide["d"]["w"] = json!(80);
    c.snapshot(
        24,
        raster_request(None, Some(Bytes(wide.to_string().into_bytes()))),
        None,
        WINDOW,
        session,
    );
    let wide = rendered(c.inbox.wait(Duration::from_secs(10), answer(24)));
    assert_eq!((wide.width, wide.height), (88, 28));
    // A layer the document does not have, without an object, is refused.
    let mut missing_layer = raster_request(None, None);
    missing_layer.target = SnapshotTarget::Layer {
        layer: "nope".into(),
        scale: 1.0,
        crop: None,
        content: None,
    };
    c.snapshot(25, missing_layer, None, WINDOW, session);
    assert_eq!(
        c.inbox.wait(Duration::from_secs(5), answer(25)),
        Err(ErrorCode::InvalidMessage)
    );

    // A missing composition is a named failure, not a hang.
    let mut missing = frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 500);
    missing.composition = "nope".into();
    c.snapshot(18, missing, None, WINDOW, session);
    assert_eq!(
        c.inbox.wait(Duration::from_secs(5), answer(18)),
        Err(ErrorCode::InvalidMessage)
    );

    // Serving snapshots sent the renderer nothing but the resync answer:
    // a renderer blocked on one cannot hold the engine up.
    let after = c.inbox.count(WINDOW);
    assert_eq!(
        after, before,
        "the engine wrote to the renderer while serving snapshots"
    );

    // Compatibility pixels (a raster override's result) uploaded in bands
    // on the session's port are assembled and held for the session.
    use pm_protocol::snapshot::{
        CompatFormat, CompatKey, CompatMessage, CompatPixels, CompatSource,
    };
    let id = CompatKey {
        composition: "Psnap-root".into(),
        layer: "box".into(),
        time: Ticks::ZERO,
        source: CompatSource::RasterOverride,
        key: "typewriter:box:1".into(),
    };
    for (row, rows) in [(0u32, 2u32), (2, 2)] {
        c.event(Event::Compat(CompatMessage::Pixels(CompatPixels {
            id: id.clone(),
            revision: Revision::new(2).unwrap(),
            width: 4,
            height: 4,
            format: CompatFormat::Rgba8Premultiplied,
            origin: [0.0, 0.0],
            scale: 1.0,
            row,
            rows,
            data: Bytes(vec![200; (4 * rows * 4) as usize]),
        })));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status: Value = serde_json::from_str(&c.host.status_json()).unwrap();
        if status["presenter"]["compatRastersReceived"] == 1 {
            assert_eq!(status["presenter"]["compatBytes"], 64, "{status}");
            break;
        }
        assert!(Instant::now() < deadline, "{}", status["presenter"]);
        std::thread::sleep(Duration::from_millis(10));
    }
    c.event(Event::Compat(CompatMessage::Forget {
        composition: "Psnap-root".into(),
        layer: "box".into(),
    }));

    let status: Value = serde_json::from_str(&c.host.status_json()).unwrap();
    let presenter = &status["presenter"];
    // Six frames and three layer rasters.
    assert_eq!(presenter["snapshots"], 9, "{presenter}");
    assert_eq!(presenter["snapshotShared"], 2, "{presenter}");
    println!("CHILD-OK");
}

#[test]
#[ignore = "run by viewer_snapshots_render_the_pinned_canvas_without_changing_the_live_view"]
fn child_viewer_snapshot() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let mut c = Child::start();
    c.resync(2, 1, &project(100.0));
    c.event(Event::Viewer(viewer_state()));
    let initial = c
        .inbox
        .wait(Duration::from_secs(20), |port, m| match &m.message {
            Message::Event {
                body: Event::Frame(d),
            } if port == WINDOW => Some(d.clone()),
            _ => None,
        });
    assert_eq!(initial.view_seq, Seq::new(1).unwrap());
    c.event(Event::ReleaseFrame(FrameRelease {
        pool_index: initial.pool_index,
        seq: initial.seq,
    }));
    let mut pinned = frame_request(
        SnapshotClass::Sync,
        Size::new(320, 180),
        SnapshotPixels::Opaque,
        5_000,
    );
    pinned.view_seq = None;
    pinned.viewer = Some(SnapshotViewer {
        view: None,
        canvas: ViewerCanvas {
            width: 320,
            height: 180,
            crop: Rect {
                x: 80.0,
                y: 40.0,
                width: 160.0,
                height: 90.0,
            },
            css: Rect {
                x: 17.0,
                y: 23.0,
                width: 640.0,
                height: 360.0,
            },
        },
        view_seq: Seq::new(5).unwrap(),
    });
    c.snapshot(50, pinned, Some(1), WINDOW, c.session);
    let pinned = rendered(c.inbox.wait(Duration::from_secs(20), answer(50)));
    assert_eq!(pinned.view_seq, Some(Seq::new(5).unwrap()));
    assert_eq!((pinned.width, pinned.height), (320, 180));
    let pixels = pixels_of(&pinned);
    assert_eq!(at(&pixels, 320, 80, 60), BOX);
    assert_eq!(at(&pixels, 320, 0, 0), BACKGROUND);

    // A later live frame still belongs to the original viewer state.
    c.event(Event::Transport(TransportCommand::Seek {
        time: Ticks::from_whole_seconds(1),
        mode: SeekMode::Exact,
    }));
    let later = c
        .inbox
        .wait(Duration::from_secs(20), |port, m| match &m.message {
            Message::Event {
                body: Event::Frame(d),
            } if port == WINDOW && d.seq != initial.seq => Some(d.clone()),
            _ => None,
        });
    assert_eq!(later.view_seq, Seq::new(1).unwrap());
    c.event(Event::ReleaseFrame(FrameRelease {
        pool_index: later.pool_index,
        seq: later.seq,
    }));

    // The next ordinary capture still uses the full composition and its
    // legacy sequence.
    c.snapshot(
        51,
        frame_request(
            SnapshotClass::Sync,
            Size::new(320, 180),
            SnapshotPixels::Opaque,
            5_000,
        ),
        Some(1),
        WINDOW,
        c.session,
    );
    let ordinary = rendered(c.inbox.wait(Duration::from_secs(20), answer(51)));
    assert_eq!(ordinary.view_seq, Some(Seq::new(4).unwrap()));
    let pixels = pixels_of(&ordinary);
    assert_eq!(at(&pixels, 320, 120, 70), BOX);
    assert_eq!(at(&pixels, 320, 80, 60), BACKGROUND);

    // The pinned canvas uses the live viewer's top-down color pass. On a
    // full canvas it must retain the same exact 8-bit colors as an ordinary
    // snapshot, while keeping its own selected sequence.
    let mut full = frame_request(
        SnapshotClass::Sync,
        Size::new(320, 180),
        SnapshotPixels::Opaque,
        5_000,
    );
    full.view_seq = None;
    full.viewer = Some(SnapshotViewer {
        view: None,
        canvas: ViewerCanvas {
            width: 320,
            height: 180,
            crop: Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 180.0,
            },
            css: Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 180.0,
            },
        },
        view_seq: Seq::new(6).unwrap(),
    });
    c.snapshot(52, full, Some(1), WINDOW, c.session);
    let full = rendered(c.inbox.wait(Duration::from_secs(20), answer(52)));
    assert_eq!(full.view_seq, Some(Seq::new(6).unwrap()));
    assert_eq!(pixels_of(&full), pixels);

    // Project Float32 raises render intermediates, while the live viewer
    // still writes its composition into an F16 display working texture.
    // At this opacity, ordinary Float32 rounds white on black to code 95;
    // the viewer's F16 display texture rounds the same pixel to code 94.
    let mut high_precision = project(100.0);
    high_precision["color"] =
        json!({ "mode": "standard", "linearBlending": false, "precision": "float32" });
    high_precision["backgroundFill"]["stops"][0]["color"] = json!("#000000");
    high_precision["layers"][0]["d"]["color"] = json!("#FFFFFF");
    high_precision["layers"][0]["p"]["opacity"] = ch(json!(37.060));
    c.resync(53, 2, &high_precision);
    c.snapshot(
        54,
        frame_request(
            SnapshotClass::Sync,
            Size::new(320, 180),
            SnapshotPixels::Opaque,
            5_000,
        ),
        Some(2),
        WINDOW,
        c.session,
    );
    let ordinary_float32 = rendered(c.inbox.wait(Duration::from_secs(20), answer(54)));
    assert_eq!(ordinary_float32.view_seq, Some(Seq::new(4).unwrap()));
    let mut high_precision_pin = frame_request(
        SnapshotClass::Sync,
        Size::new(320, 180),
        SnapshotPixels::Opaque,
        5_000,
    );
    high_precision_pin.viewer = Some(SnapshotViewer {
        view: None,
        canvas: ViewerCanvas {
            width: 320,
            height: 180,
            crop: Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 180.0,
            },
            css: Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 180.0,
            },
        },
        view_seq: Seq::new(7).unwrap(),
    });
    high_precision_pin.view_seq = None;
    c.snapshot(55, high_precision_pin, Some(2), WINDOW, c.session);
    let high_precision_viewer = rendered(c.inbox.wait(Duration::from_secs(20), answer(55)));
    assert_eq!(high_precision_viewer.view_seq, Some(Seq::new(7).unwrap()));
    assert_eq!(
        at(&pixels_of(&ordinary_float32), 320, 120, 70),
        [95, 95, 95, 255]
    );
    assert_eq!(
        at(&pixels_of(&high_precision_viewer), 320, 120, 70),
        [94, 94, 94, 255]
    );
    println!("CHILD-OK");
}

#[test]
#[ignore = "run by viewer_snapshots_apply_the_pinned_editor_camera_and_local_view"]
fn child_viewer_camera_snapshot() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let mut c = Child::start();
    c.resync(2, 1, &blender_project());
    let size = Size::new(320, 180);
    let mut capture = frame_request(SnapshotClass::Sync, size, SnapshotPixels::Opaque, 5_000);
    capture.target = SnapshotTarget::Frame {
        motion_blur_samples: 0,
        background: true,
        draft_3d: true,
    };
    capture.view_seq = None;
    capture.viewer = Some(SnapshotViewer {
        view: Some(ViewOverride {
            camera: None,
            shading: Shading3d::Solid,
            xray: false,
            local_view: vec![],
        }),
        canvas: ViewerCanvas {
            width: size.width,
            height: size.height,
            crop: Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 180.0,
            },
            css: Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 180.0,
            },
        },
        view_seq: Seq::new(7).unwrap(),
    });
    c.snapshot(60, capture.clone(), Some(1), WINDOW, c.session);
    let default = rendered(c.inbox.wait(Duration::from_secs(20), answer(60)));
    assert_eq!(default.view_seq, Some(Seq::new(7).unwrap()));
    let default_pixels = pixels_of(&default);

    let selected = capture.viewer.as_mut().unwrap();
    selected.view_seq = Seq::new(8).unwrap();
    selected.view.as_mut().unwrap().camera = Some(EditorCamera {
        view: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -10.0, 1.0,
        ],
        projection: Projection::Orthographic {
            half_height: 0.6,
            near: 0.01,
            far: 100.0,
        },
    });
    c.snapshot(61, capture.clone(), Some(1), WINDOW, c.session);
    let editor = rendered(c.inbox.wait(Duration::from_secs(20), answer(61)));
    assert_eq!(editor.view_seq, Some(Seq::new(8).unwrap()));
    assert_ne!(
        pixels_of(&editor),
        default_pixels,
        "the editor camera changes the frame"
    );

    let selected = capture.viewer.as_mut().unwrap();
    selected.view_seq = Seq::new(9).unwrap();
    selected.view.as_mut().unwrap().local_view = vec!["cam".into()];
    c.snapshot(62, capture, Some(1), WINDOW, c.session);
    let local = rendered(c.inbox.wait(Duration::from_secs(20), answer(62)));
    assert_ne!(
        pixels_of(&local),
        pixels_of(&editor),
        "Local View hides the object"
    );
    println!("CHILD-OK");
}

fn viewer_state() -> ViewerState {
    ViewerState {
        headless: None,
        composition: "Psnap-root".into(),
        size: Size::new(640, 360),
        device_pixel_ratio: 2.0,
        zoom: 1.0,
        pan: [0.0, 0.0],
        resolution: PreviewResolution::Full,
        media: MediaPreference::Automatic,
        color_space: ViewerColorSpace::Srgb,
        view: None,
        play_every_frame: false,
        edr: false,
        view_seq: Seq::new(1).unwrap(),
        vsync: None,
        depth_id: None,
        chrome_selection: None,
        canvas: None,
        surface_epoch: None,
    }
}

#[test]
#[ignore = "run by snapshots_are_scheduled_around_playback"]
fn child_scheduling() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    scheduling(false);
}

#[test]
#[ignore = "run by headless_snapshots_are_scheduled_around_audio_only_playback"]
fn child_headless_scheduling() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    scheduling(true);
}

fn scheduling(headless: bool) {
    let mut c = Child::start();
    let session = c.session;
    let mut state = viewer_state();
    state.headless = Some(headless);
    if headless {
        c.event(Event::Viewer(state.clone()));
        // A cold Play must survive the document/media pending state.
        c.event(Event::Transport(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        }));
    }
    c.resync(2, 1, &project(100.0));
    if !headless {
        c.event(Event::Viewer(state));
    }
    if headless {
        c.inbox
            .wait(Duration::from_secs(20), |_, m| match &m.message {
                Message::Event {
                    body: Event::Clock(clock),
                } if m.revision == Some(Revision::new(1).unwrap())
                    && clock.state == TransportState::Playing =>
                {
                    Some(())
                }
                _ => None,
            });
        c.event(Event::Transport(TransportCommand::Pause));
        c.inbox
            .wait(Duration::from_secs(5), |_, m| match &m.message {
                Message::Event {
                    body: Event::Clock(clock),
                } if clock.state == TransportState::Stopped => Some(()),
                _ => None,
            });
        // A newer document while paused must publish a fresh readiness
        // clock, even though the composition remains supported and ready.
        c.resync(3, 2, &project(100.0));
        c.inbox
            .wait(Duration::from_secs(5), |_, m| match &m.message {
                Message::Event {
                    body: Event::Clock(_),
                } if m.revision == Revision::new(2).ok() => Some(()),
                _ => None,
            });
        // Presentation warms the GPU before the timed section below. A
        // headless session has drawn nothing: warm it with an offscreen
        // capture too, without allocating a presentation pool.
        c.snapshot(
            19,
            frame_request(
                SnapshotClass::Sync,
                Size::new(160, 90),
                SnapshotPixels::Opaque,
                10_000,
            ),
            None,
            WINDOW,
            session,
        );
        rendered(c.inbox.wait(Duration::from_secs(20), answer(19)));
    } else {
        let first = c
            .inbox
            .wait(Duration::from_secs(20), |port, m| match &m.message {
                Message::Event {
                    body: Event::Frame(d),
                } if port == WINDOW => Some(d.clone()),
                _ => None,
            });
        c.event(Event::ReleaseFrame(FrameRelease {
            pool_index: first.pool_index,
            seq: first.seq,
        }));
    }
    c.event(Event::Transport(TransportCommand::Play {
        rate: PlaybackRate::NORMAL,
    }));
    c.inbox
        .wait(Duration::from_secs(5), |_, m| match &m.message {
            Message::Event {
                body: Event::Clock(s),
            } if s.state == TransportState::Playing => Some(()),
            _ => None,
        });
    // Release every frame the viewer is sent, as the viewer would.
    let small = Size::new(160, 90);
    c.snapshot(
        20,
        frame_request(SnapshotClass::Idle, small, SnapshotPixels::Opaque, 10_000),
        None,
        WINDOW,
        session,
    );
    let asked = Instant::now();
    c.snapshot(
        21,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 10_000),
        None,
        WINDOW,
        session,
    );
    if headless {
        c.snapshot(
            22,
            frame_request(SnapshotClass::Async, small, SnapshotPixels::Opaque, 10_000),
            None,
            WINDOW,
            session,
        );
    }
    let mut sent_frames = 0;
    let mut async_at = None;
    let mut sync_at = None;
    let mut idle_early = false;
    while asked.elapsed() < Duration::from_millis(800) {
        let release = c.inbox.try_wait(
            Duration::from_millis(5),
            &mut |port, m: &Envelope| match &m.message {
                Message::Event {
                    body: Event::Frame(d),
                } if port == WINDOW => Some(FrameRelease {
                    pool_index: d.pool_index,
                    seq: d.seq,
                }),
                _ => None,
            },
        );
        if let Some(release) = release {
            sent_frames += 1;
            c.event(Event::ReleaseFrame(release));
        }
        if headless
            && async_at.is_none()
            && c.inbox.try_wait(Duration::ZERO, &mut answer(22)).is_some()
        {
            async_at = Some(asked.elapsed());
        }
        if sync_at.is_none() && c.inbox.try_wait(Duration::ZERO, &mut answer(21)).is_some() {
            sync_at = Some(asked.elapsed());
        }
        if c.inbox.try_wait(Duration::ZERO, &mut answer(20)).is_some() {
            idle_early = true;
        }
    }
    let sync_at = sync_at.expect("the sync snapshot was answered during playback");
    assert!(
        sync_at < Duration::from_millis(500),
        "sync answered after {sync_at:?}"
    );
    if headless {
        assert!(
            async_at.is_some(),
            "foreground async snapshot must not wait on a nonexistent presentation deadline"
        );
    }
    assert!(!idle_early, "an idle snapshot ran during playback");
    c.event(Event::Transport(TransportCommand::Pause));
    let result = rendered(c.inbox.wait(Duration::from_secs(5), answer(20)));
    if headless {
        assert_eq!(sent_frames, 0, "snapshots allocate no presentation frames");
    } else {
        // Presentation stats reach a drawing viewer at most twice a second.
        let stats = c
            .inbox
            .wait(Duration::from_secs(5), |port, m| match &m.message {
                Message::Event {
                    body: Event::RenderStats(s),
                } if port == WINDOW && s.snapshots.get() >= 1 => Some(s.clone()),
                _ => None,
            });
        assert!(stats.frames.get() >= 2, "{stats:?}");
        assert!(stats.timed_frames.get() >= 1, "{stats:?}");
        assert!(stats.frame_us_avg > 0, "{stats:?}");
    }
    assert_eq!((result.width, result.height), (160, 90));
    println!("CHILD-OK sync={sync_at:?}");
}

fn shader_project() -> Value {
    let code = "uniform samplerCube uEnv;\nvoid main(){ fragColor = vec4(texture(uEnv, vec3(uv, 1.0)).rgb + vec3(uv, 0.0), 1.0); }";
    let mut doc = project(100.0);
    doc["layers"] = json!([{
        "id": "sky", "type": "shader", "name": "sky", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(0)),
            "position.y": ch(json!(0)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
        "d": { "code": code, "uniforms": {}, "w": 80, "h": 45 }, "locked_intent": {}
    }]);
    doc
}

/// The renderer's pixels for a fallback request: one value everywhere.
fn bands(request: &pm_protocol::shader_fallback::ShaderFallbackRequest, value: u16) -> Vec<Event> {
    use pm_protocol::shader_fallback::{ShaderFallbackMessage, ShaderFallbackPixels};
    let row: Vec<u8> = (0..request.width)
        .flat_map(|_| [value, value, value, 0x3c00])
        .flat_map(u16::to_le_bytes)
        .collect();
    let half = request.height / 2;
    [(0, half), (half, request.height - half)]
        .into_iter()
        .map(|(from, rows)| {
            Event::ShaderFallback(ShaderFallbackMessage::Pixels(ShaderFallbackPixels {
                key: request.key.clone(),
                width: request.width,
                height: request.height,
                row: from,
                rows,
                rgba16f: Bytes((0..rows).flat_map(|_| row.clone()).collect()),
            }))
        })
        .collect()
}

#[test]
#[ignore = "run by snapshots_that_need_the_renderer_name_what_it_draws"]
fn child_fallback() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let mut c = Child::start();
    let session = c.session;
    c.resync(2, 1, &shader_project());
    let small = Size::new(320, 180);
    // The engine does not wait on the renderer: it names the program.
    c.snapshot(
        30,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(1),
        WINDOW,
        session,
    );
    let needs = match c.inbox.wait(Duration::from_secs(20), answer(30)) {
        Ok(SnapshotOutcome::NeedsRenderer {
            revision,
            fallback,
            compat,
            svg,
        }) => {
            assert_eq!(revision.get(), 1);
            assert!(compat.is_empty());
            assert!(svg.is_empty());
            fallback
        }
        other => panic!("expected needsRenderer, got {other:?}"),
    };
    assert_eq!(needs.len(), 1);
    let request = needs[0].clone();
    assert_eq!(
        (request.layer.as_str(), request.width, request.height),
        ("sky", 80, 45)
    );
    // The renderer draws it, posts it on the session's port, and asks again.
    for band in bands(&request, 0x3800) {
        c.event(band);
    }
    let mut again = frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000);
    again.uploaded = vec![request.key.clone()];
    c.snapshot(31, again, Some(1), WINDOW, session);
    let px = pixels_of(&rendered(c.inbox.wait(Duration::from_secs(20), answer(31))));
    // binary16 0x3800 = 0.5 over the layer's box, the background elsewhere.
    assert_eq!(at(&px, 320, 40, 20), [128, 128, 128, 255]);
    assert_eq!(at(&px, 320, 200, 120), BACKGROUND);

    // Pixels that reach the engine after the request (two ports race): the
    // job waits for them rather than naming the program again. (A move
    // alone reuses the pixels drawn above: the fallback key covers the
    // program, its size, and its uniforms.)
    c.resync(3, 2, &{
        let mut doc = shader_project();
        doc["layers"][0]["p"]["position.x"] = ch(json!(160));
        // Another size is another program draw (another fallback key).
        doc["layers"][0]["d"]["w"] = json!(100);
        doc
    });
    c.snapshot(
        32,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(2),
        WINDOW,
        session,
    );
    let request = match c.inbox.wait(Duration::from_secs(20), answer(32)) {
        Ok(SnapshotOutcome::NeedsRenderer { fallback, .. }) => fallback[0].clone(),
        other => panic!("expected needsRenderer, got {other:?}"),
    };
    let mut again = frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000);
    again.uploaded = vec![request.key.clone()];
    c.snapshot(33, again, Some(2), WINDOW, session);
    assert!(
        c.inbox
            .try_wait(Duration::from_millis(200), &mut answer(33))
            .is_none(),
        "answered before the pixels arrived"
    );
    for band in bands(&request, 0x3400) {
        c.event(band);
    }
    let px = pixels_of(&rendered(c.inbox.wait(Duration::from_secs(20), answer(33))));
    // binary16 0x3400 = 0.25 → 64, where the layer moved.
    assert_eq!(at(&px, 320, 200, 20), [64, 64, 64, 255]);
    assert_eq!(at(&px, 320, 40, 20), BACKGROUND);
    println!("CHILD-OK");
}

/// An effect only the runner draws: the built-in Invert with a cube
/// sampler (amendment 9: WebGL samples it as (0, 0, 0, 1)).
fn cube_effect_definitions() -> pm_protocol::definitions::DefinitionsUpdate {
    let mut set = pm_render::builtin_definitions();
    let mut cube = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    cube.id = "acme.cube".into();
    cube.origin = pm_protocol::definitions::DefinitionOrigin::Store;
    cube.flags.backdrop = true;
    let glsl = cube.glsl.as_mut().unwrap();
    glsl.source = glsl.source.replacen(
        "void main(){",
        "uniform samplerCube u_env;\nvoid main(){",
        1,
    );
    let end = glsl.source.rfind('}').unwrap();
    glsl.source
        .insert_str(end, "  o += texture(u_env, vec3(0.0, 0.0, 1.0)) * 0.0;\n");
    set.definitions.push(cube);
    pm_protocol::definitions::DefinitionsUpdate {
        generation: Seq::new(1).unwrap(),
        full: true,
        upserts: set.definitions,
        removals: vec![],
    }
}

#[test]
#[ignore = "run by snapshots_send_an_effects_textures_to_main_ahead_of_the_answer"]
fn child_fallback_effect() {
    use pm_protocol::shader_fallback::ShaderFallbackMessage;
    use std::collections::BTreeSet;
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let mut c = Child::start();
    let session = c.session;
    c.window(
        Envelope::new(Message::Request {
            id: RequestId::new(2).unwrap(),
            body: Request::PushDefinitions(cube_effect_definitions()),
        })
        .with_session(session),
    );
    c.inbox
        .wait(Duration::from_secs(5), |port, m| match &m.message {
            Message::Response {
                body: Response::DefinitionsApplied(_),
                ..
            } if port == WINDOW => Some(()),
            _ => None,
        });
    let mut doc = project(100.0);
    doc["layers"][0]["fx"] = json!([{ "id": "fx1", "type": "acme.cube", "on": true, "p": {} }]);
    c.resync(3, 1, &doc);
    let small = Size::new(320, 180);
    c.snapshot(
        40,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(1),
        WINDOW,
        session,
    );
    // The bands come to main (the host port) before the answer: the scan
    // meets them first. Each is (key, texture, first row, rows).
    let mut before = BTreeSet::new();
    let mut to_window = 0;
    let mut answer40 = answer(40);
    let outcome = c.inbox.wait(Duration::from_secs(20), |port, m| {
        if let Message::Event {
            body: Event::ShaderFallback(ShaderFallbackMessage::Input(i)),
        } = &m.message
        {
            if port == HOST {
                before.insert((i.key.clone(), i.texture, i.row, i.rows, i.width, i.height));
            } else {
                to_window += 1;
            }
            return None;
        }
        answer40(port, m)
    });
    let request = match outcome {
        Ok(SnapshotOutcome::NeedsRenderer { fallback, .. }) => {
            assert_eq!(fallback.len(), 1);
            fallback[0].clone()
        }
        other => panic!("expected needsRenderer, got {other:?}"),
    };
    assert_eq!((request.layer.as_str(), request.textures), ("box", 2));
    // Never on the renderer's port: it may be the thread blocked on this.
    assert_eq!(to_window, 0);
    for texture in 0..2 {
        let rows: u32 = before
            .iter()
            .filter(|b| b.0 == request.key && b.1 == texture)
            .map(|b| {
                assert_eq!((b.4, b.5), (request.width, request.height));
                b.3
            })
            .sum();
        assert_eq!(rows, request.height, "texture {texture}: {before:?}");
    }
    // The renderer draws it with those textures, posts it on its port, and
    // asks again.
    for band in bands(&request, 0x3800) {
        c.event(band);
    }
    let mut again = frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000);
    again.uploaded = vec![request.key.clone()];
    c.snapshot(41, again, Some(1), WINDOW, session);
    let px = pixels_of(&rendered(c.inbox.wait(Duration::from_secs(20), answer(41))));
    // binary16 0x3800 = 0.5 everywhere the runner's pixels cover: an
    // effect with a backdrop draws into the layer's composition-sized
    // buffer.
    assert_eq!((request.width, request.height), (320, 180));
    assert_eq!(at(&px, 320, 120, 70), [128, 128, 128, 255]);
    assert_eq!(at(&px, 320, 300, 160), [128, 128, 128, 255]);
    println!("CHILD-OK");
}

#[test]
#[ignore = "run by frames_carry_their_depth_and_id_surface_when_the_viewer_asks"]
fn child_depth() {
    use pm_protocol::interop::PoolRole;
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let mut c = Child::start();
    c.resync(2, 1, &project(100.0));
    // Without the ask: one color pool, frames without depth.
    c.event(Event::Viewer(viewer_state()));
    let pool = c
        .inbox
        .wait(Duration::from_secs(20), |port, m| match &m.message {
            Message::Event {
                body: Event::SurfacePool(p),
            } if port == WINDOW => Some(p.clone()),
            _ => None,
        });
    assert_ne!(pool.role, Some(PoolRole::DepthId));
    let first = c
        .inbox
        .wait(Duration::from_secs(20), |port, m| match &m.message {
            Message::Event {
                body: Event::Frame(d),
            } if port == WINDOW => Some(d.clone()),
            _ => None,
        });
    assert!(first.depth_id.is_none());
    c.event(Event::ReleaseFrame(FrameRelease {
        pool_index: first.pool_index,
        seq: first.seq,
    }));
    // The 3D chrome asks: each color slot owns an rgbaf16 depth/ID and
    // selection-coverage pair, retained until that color frame is released.
    let mut asking = viewer_state();
    asking.view_seq = Seq::new(2).unwrap();
    asking.depth_id = Some(true);
    c.event(Event::Viewer(asking));
    let depth_pool = c
        .inbox
        .wait(Duration::from_secs(20), |port, m| match &m.message {
            Message::Event {
                body: Event::SurfacePool(p),
            } if port == WINDOW && p.role == Some(PoolRole::DepthId) => Some(p.clone()),
            _ => None,
        });
    assert_eq!(depth_pool.surfaces.len(), pool.surfaces.len() * 2);
    assert!(
        depth_pool
            .surfaces
            .iter()
            .all(|s| s.format == pm_protocol::interop::SurfaceFormat::Rgba16Float)
    );
    assert_eq!(depth_pool.surfaces[0].size, pool.surfaces[0].size);
    let framed = c
        .inbox
        .wait(Duration::from_secs(20), |port, m| match &m.message {
            Message::Event {
                body: Event::Frame(d),
            } if port == WINDOW && d.view_seq.get() == 2 => Some(d.clone()),
            _ => None,
        });
    let depth = framed
        .depth_id
        .expect("a frame of a view that asks names its depth surface");
    assert_eq!(depth.pool_generation, depth_pool.generation);
    assert_eq!(depth.seq, framed.seq);
    assert_eq!(depth.pool_index, framed.pool_index * 2);
    assert_eq!(depth.coverage_pool_index, Some(depth.pool_index + 1));
    assert!((depth.pool_index + 1) < depth_pool.surfaces.len() as u32);
    c.event(Event::ReleaseFrame(FrameRelease {
        pool_index: framed.pool_index,
        seq: framed.seq,
    }));
    // Asking no more: frames go on without depth.
    let mut done = viewer_state();
    done.view_seq = Seq::new(3).unwrap();
    c.event(Event::Viewer(done));
    let plain = c
        .inbox
        .wait(Duration::from_secs(20), |port, m| match &m.message {
            Message::Event {
                body: Event::Frame(d),
            } if port == WINDOW && d.view_seq.get() == 3 => Some(d.clone()),
            _ => None,
        });
    assert!(plain.depth_id.is_none());
    let status: Value = serde_json::from_str(&c.host.status_json()).unwrap();
    assert_eq!(
        status["presenter"]["depthPools"], 1,
        "{}",
        status["presenter"]
    );
    assert!(status["presenter"]["depthFrames"].as_u64().unwrap() >= 1);
    println!("CHILD-OK");
}

/// A 3D layer (`mkLayer`'s transform channels) of extension `definition`.
fn layer3d(id: &str, definition: &str, data: Value, position: [f64; 3], w: u32, h: u32) -> Value {
    json!({
        "id": id, "type": "extension", "name": id, "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": true, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "perspective": ch(json!(50)), "position.z": ch(json!(position[2])), "anchor.z": ch(json!(0)),
            "scale.z": ch(json!(100)), "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)),
            "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)), "orientation.z": ch(json!(0)),
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(position[0])),
            "position.y": ch(json!(position[1])), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
        "d": { "definition": definition, "version": 1, "w": w, "h": h, "params": {}, "data": data },
        "locked_intent": {}
    })
}

/// A composition Blender renders (`render3d.enabled`): a camera, a sun,
/// and one box.
fn blender_project() -> Value {
    let camera = layer3d(
        "cam",
        "powermove.3d.camera",
        json!({
            "camera": { "projection": "perspective", "p": {
                "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
                "fov": ch(json!(45)), "zoom": ch(json!(1)), "near": ch(json!(0.01)), "far": ch(json!(1000))
            } },
            "environment": { "p": {
                "ambient": ch(json!(0.5)), "ambientColor": ch(json!("#FFFFFF")), "exposure": ch(json!(1))
            }, "background": null, "shadows": false }
        }),
        [0.0, 0.0, 4.0],
        0,
        0,
    );
    let sun = layer3d(
        "sun",
        "powermove.3d.light",
        json!({ "light": { "type": "sun", "castShadow": false, "p": {
            "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
            "color": ch(json!("#FFFFFF")), "intensity": ch(json!(3)), "distance": ch(json!(0)),
            "decay": ch(json!(2)), "angle": ch(json!(45)), "penumbra": ch(json!(0.4)),
            "width": ch(json!(1)), "height": ch(json!(1))
        } } }),
        [-3.0, 5.0, 4.0],
        0,
        0,
    );
    let object = layer3d(
        "o",
        "powermove.3d.object",
        json!({ "object": {
            "source": { "primitive": "box", "parameters": {} },
            "material": { "p": {
                "color": ch(json!("#FF8040")), "roughness": ch(json!(0.4)), "metalness": ch(json!(0.1)),
                "emissive": ch(json!("#000000")), "emissiveIntensity": ch(json!(1)), "opacity": ch(json!(1))
            }, "maps": {}, "doubleSided": true },
            "useSourceMaterials": false, "slots": [], "castShadow": true, "receiveShadow": true
        } }),
        [0.0, 0.0, 0.0],
        320,
        180,
    );
    let mut p = project(100.0);
    p["layers"] = json!([camera, sun, object]);
    p["render3d"] = json!({ "enabled": true, "engine": "eevee", "samples": 8 });
    p
}

/// How the stand-in host services answer `blenderRender`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum BlenderMode {
    /// Every surface opaque magenta.
    Render,
    /// Blender failed.
    Fail,
    /// Blender is not installed.
    Missing,
    /// Never answers until cancelled.
    Hold,
}

/// Main's Blender supervision as the engine sees it (contract 11 over the
/// host port; `main/engine/blender.ts`): the Blender Surfaces area in a
/// temporary directory, surfaces written as 16-bit PNGs before the answer.
struct StandInHost {
    area: std::path::PathBuf,
    mode: BlenderMode,
    renders: Vec<String>,
    cancels: Vec<String>,
    held: Vec<(RequestId, String)>,
}

const MAGENTA: [u8; 4] = [255, 0, 255, 255];

impl StandInHost {
    fn new() -> StandInHost {
        let area = std::env::temp_dir().join(format!("pm-snapshot-blender-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&area);
        std::fs::create_dir_all(&area).unwrap();
        StandInHost {
            area,
            mode: BlenderMode::Render,
            renders: Vec::new(),
            cancels: Vec::new(),
            held: Vec::new(),
        }
    }

    fn reply(c: &mut Child, id: RequestId, answer: Result<Response, (ErrorCode, &str)>) {
        use pm_protocol::error::{EngineError, ErrorScope};
        let message = match answer {
            Ok(body) => Message::Response { id, body },
            Err((code, text)) => Message::Error {
                id,
                error: EngineError::new(code, ErrorScope::Request { id }, text),
            },
        };
        c.host_sends(Envelope::new(message));
    }

    fn serve(&mut self, c: &mut Child, id: RequestId, session: Option<SessionId>, body: Request) {
        use pm_protocol::blender::{BlenderStatus, BlenderSurface, ResolvedDevice};
        use pm_protocol::common::{ByteSize, PixelRect};
        use pm_protocol::host::{
            CacheKind, CapabilityHandle, CapabilityKind, HandleAccess, HandleGrant, HandleResolved,
        };
        use pm_protocol::priority::{CancelJobs, JobsCancelled};
        match body {
            Request::BlenderStatus => Self::reply(
                c,
                id,
                Ok(Response::BlenderStatus(BlenderStatus {
                    available: true,
                    version: Some("4.5.14".into()),
                    error: None,
                })),
            ),
            Request::MintHandle(mint) => Self::reply(
                c,
                id,
                Ok(Response::Handle(HandleGrant {
                    handle: CapabilityHandle::from_random_bytes([9; 16]),
                    kind: mint.kind,
                    content_hash: None,
                })),
            ),
            Request::ResolveHandle(r) => Self::reply(
                c,
                id,
                Ok(Response::HandleResolved(HandleResolved {
                    handle: r.handle,
                    kind: CapabilityKind::Cache {
                        cache: CacheKind::BlenderSurfaces,
                    },
                    access: HandleAccess::ReadWrite,
                    path: self.area.to_string_lossy().into_owned(),
                })),
            ),
            Request::BlenderRender(job) => {
                self.renders.push(job.job.0.clone());
                match self.mode {
                    BlenderMode::Fail => {
                        Self::reply(c, id, Err((ErrorCode::BlenderFailed, "Blender crashed")))
                    }
                    BlenderMode::Missing => Self::reply(
                        c,
                        id,
                        Err((ErrorCode::BlenderMissing, "Install Blender 4.5 or later.")),
                    ),
                    BlenderMode::Hold => self.held.push((id, job.job.0.clone())),
                    BlenderMode::Render => {
                        for (i, sr) in job.surfaces.iter().enumerate() {
                            let (w, h) = (sr.size.width, sr.size.height);
                            let rgba: Vec<u16> = (0..w * h)
                                .flat_map(|_| [65_535, 0, 65_535, 65_535])
                                .collect();
                            let png = pm_blender::surface::encode_rgba16(w, h, &rgba, &[]);
                            let name = format!("{}.png", pm_blender::key::hex(&sr.key));
                            std::fs::write(self.area.join(name), &png).unwrap();
                            let event = Event::BlenderSurface(BlenderSurface {
                                job: job.job.clone(),
                                request_index: i as u32,
                                key: sr.key,
                                size: sr.size,
                                crop: PixelRect {
                                    x: 0,
                                    y: 0,
                                    width: w,
                                    height: h,
                                },
                                handle: job.output.clone(),
                                bytes: ByteSize::new(png.len() as u64).unwrap(),
                                device: ResolvedDevice::Cpu,
                            });
                            let mut envelope = Envelope::new(Message::Event { body: event });
                            envelope.session = session;
                            c.host_sends(envelope);
                        }
                        Self::reply(c, id, Ok(Response::Ack));
                    }
                }
            }
            Request::CancelJobs(CancelJobs::Job { job }) => {
                self.cancels.push(job.0.clone());
                if let Some(i) = self.held.iter().position(|(_, j)| *j == job.0) {
                    let (held, _) = self.held.remove(i);
                    Self::reply(
                        c,
                        held,
                        Err((ErrorCode::Cancelled, "Blender request cancelled")),
                    );
                }
                Self::reply(
                    c,
                    id,
                    Ok(Response::JobsCancelled(JobsCancelled { jobs: vec![job] })),
                );
            }
            other => {
                eprintln!("stand-in host: unsupported {other:?}");
                Self::reply(
                    c,
                    id,
                    Err((ErrorCode::Unsupported, "not served by this test")),
                );
            }
        }
    }

    /// Serves the engine's host requests until `done` matches.
    fn serve_until<T>(
        &mut self,
        c: &mut Child,
        timeout: Duration,
        mut done: impl FnMut(PortId, &Envelope) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + timeout;
        loop {
            let actions = c.host.pump();
            c.inbox.push(actions);
            let request = c
                .inbox
                .try_wait(Duration::ZERO, &mut |port, m| match &m.message {
                    Message::Request { id, body } if port == HOST => {
                        Some((*id, m.session, body.clone()))
                    }
                    _ => None,
                });
            if let Some((id, session, body)) = request {
                self.serve(c, id, session, body);
                continue;
            }
            if let Some(found) = c.inbox.try_wait(Duration::from_millis(5), &mut done) {
                return found;
            }
            assert!(Instant::now() < deadline, "timed out serving Blender");
        }
    }
}

impl StandInHost {
    /// Serves the engine's host requests while `waiting` holds.
    fn serve_while(&mut self, c: &mut Child, timeout: Duration, waiting: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + timeout;
        while waiting(self) {
            assert!(Instant::now() < deadline, "timed out serving Blender");
            let actions = c.host.pump();
            c.inbox.push(actions);
            let request =
                c.inbox
                    .try_wait(Duration::from_millis(5), &mut |port, m| match &m.message {
                        Message::Request { id, body } if port == HOST => {
                            Some((*id, m.session, body.clone()))
                        }
                        _ => None,
                    });
            if let Some((id, session, body)) = request {
                self.serve(c, id, session, body);
            }
        }
    }
}

impl Drop for StandInHost {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.area);
    }
}

fn blender_capture(wait_ms: u32, blender_wait_ms: u32) -> SnapshotRequest {
    let mut r = frame_request(
        SnapshotClass::Async,
        Size::new(320, 180),
        SnapshotPixels::Opaque,
        wait_ms,
    );
    r.blender_wait_ms = blender_wait_ms;
    r
}

fn with_samples(samples: u32) -> Value {
    let mut p = blender_project();
    p["render3d"]["samples"] = json!(samples);
    p
}

#[test]
#[ignore = "run by blender_compositions_are_captured_as_today"]
fn child_blender() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let mut c = Child::start();
    let session = c.session;
    let mut host = StandInHost::new();
    c.resync(2, 1, &blender_project());
    // Today's synchronous capture draws the frame Blender prepared or fails;
    // there is none, so it fails with today's error, never native shading
    // labelled final.
    let small = Size::new(320, 180);
    c.snapshot(
        10,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(1),
        WINDOW,
        session,
    );
    let outcome = c.inbox.wait(Duration::from_secs(20), answer(10));
    assert_eq!(outcome.err(), Some(ErrorCode::BlenderFailed));
    // A draft 3D capture draws native shading and says so.
    let mut draft = frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000);
    draft.target = SnapshotTarget::Frame {
        motion_blur_samples: 0,
        background: true,
        draft_3d: true,
    };
    c.snapshot(11, draft, Some(1), WINDOW, session);
    let native = rendered(c.inbox.wait(Duration::from_secs(20), answer(11)));
    assert!(native.quality.draft_3d);
    assert_ne!(at(&pixels_of(&native), 320, 160, 90), MAGENTA);
    assert!(host.renders.is_empty(), "no Blender for those");

    // Today's `snapshotAsync`: Blender renders the frame first (with a 1 s
    // wait that Blender's time does not count against), then the capture
    // draws Blender's surfaces.
    let started = Instant::now();
    c.snapshot(12, blender_capture(1_000, 60_000), Some(1), WINDOW, session);
    let result = rendered(host.serve_until(&mut c, Duration::from_secs(30), answer(12)));
    let elapsed = started.elapsed();
    assert!(!result.quality.draft_3d);
    assert_eq!(result.revision.get(), 1);
    let px = pixels_of(&result);
    assert_eq!(
        at(&px, 320, 160, 90),
        MAGENTA,
        "Blender's surface, not native shading"
    );
    assert_eq!(host.renders.len(), 1);
    assert!(
        host.renders[0].starts_with(&format!("capture-{}-{}-", WINDOW.0, session.get())),
        "{:?}",
        host.renders
    );
    // A synchronous capture of the same frame now draws the prepared frame
    // (today's prepared frame), without Blender.
    c.snapshot(
        13,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(1),
        WINDOW,
        session,
    );
    let again = rendered(host.serve_until(&mut c, Duration::from_secs(20), answer(13)));
    assert_eq!(at(&pixels_of(&again), 320, 160, 90), MAGENTA);
    // And an async capture of it asks Blender for nothing.
    c.snapshot(14, blender_capture(1_000, 60_000), Some(1), WINDOW, session);
    let cached = rendered(host.serve_until(&mut c, Duration::from_secs(20), answer(14)));
    assert_eq!(at(&pixels_of(&cached), 320, 160, 90), MAGENTA);
    assert_eq!(host.renders.len(), 1);

    // Blender failing fails the capture with its reason.
    c.resync(3, 2, &with_samples(16));
    host.mode = BlenderMode::Fail;
    c.snapshot(15, blender_capture(1_000, 60_000), Some(2), WINDOW, session);
    assert_eq!(
        host.serve_until(&mut c, Duration::from_secs(20), answer(15)),
        Err(ErrorCode::BlenderFailed)
    );
    // Blender not installed: today's `blenderMissing`.
    host.mode = BlenderMode::Missing;
    c.snapshot(16, blender_capture(1_000, 60_000), Some(2), WINDOW, session);
    assert_eq!(
        host.serve_until(&mut c, Duration::from_secs(20), answer(16)),
        Err(ErrorCode::BlenderMissing)
    );
    // Blender that never finishes: the capture fails at its Blender bound
    // and Blender is stopped.
    host.mode = BlenderMode::Hold;
    let held = Instant::now();
    c.snapshot(17, blender_capture(500, 1_500), Some(2), WINDOW, session);
    assert_eq!(
        host.serve_until(&mut c, Duration::from_secs(20), answer(17)),
        Err(ErrorCode::Timeout)
    );
    let bounded = held.elapsed();
    assert!(
        bounded >= Duration::from_millis(1_400) && bounded < Duration::from_secs(5),
        "{bounded:?}"
    );
    host.serve_while(&mut c, Duration::from_secs(10), |h| h.cancels.is_empty());
    assert_eq!(host.cancels.len(), 1);
    assert_eq!(host.cancels[0], host.renders[host.renders.len() - 1]);
    assert!(
        host.held.is_empty(),
        "the held Blender request was answered as cancelled"
    );
    println!(
        "CHILD-OK blender: refused when not prepared, native with draft 3D, prepared by Blender in {} ms, prepared frame reused, failure, missing and timeout ({} ms) named",
        elapsed.as_millis(),
        bounded.as_millis()
    );
}

#[test]
#[ignore = "run by the_host_snapshots_sessions_it_opened_for_a_window"]
fn child_host_session() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    // Main opens a session of its own (a window's sync snapshot that cannot
    // wait for the renderer's session: none yet, or its resync is staged).
    let mut c = Child::start();
    let window_session = c.session;
    c.host_sends(Envelope::new(Message::Request {
        id: RequestId::new(40).unwrap(),
        body: Request::OpenSession(OpenSession {
            document: "Psnap".into(),
            role: SessionRole::Interactive,
            process: EngineProcess::Interactive,
            color: ProjectColor::DEFAULT,
        }),
    }));
    let host_session = c
        .inbox
        .wait(Duration::from_secs(5), |port, m| match &m.message {
            Message::Response {
                id,
                body: Response::SessionOpened { session },
            } if port == HOST && id.get() == 40 => Some(*session),
            _ => None,
        });
    let doc = project(100.0);
    let parsed = pm_model::json::parse(&doc.to_string()).unwrap();
    c.host_sends(
        Envelope::new(Message::Request {
            id: RequestId::new(41).unwrap(),
            body: Request::Resync(Resync {
                document: "Psnap".into(),
                revision: Revision::new(5).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(doc.to_string().into_bytes()),
                },
                hash: ContentHash(pm_model::hash::canonical_digest(&parsed)),
            }),
        })
        .with_session(host_session),
    );
    c.inbox
        .wait(Duration::from_secs(5), |port, m| match &m.message {
            Message::Response {
                id,
                body: Response::PatchesApplied(_),
            } if port == HOST && id.get() == 41 => Some(()),
            _ => None,
        });
    let small = Size::new(320, 180);
    c.snapshot(
        42,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(5),
        HOST,
        host_session,
    );
    let result = rendered(c.inbox.wait(Duration::from_secs(20), answer(42)));
    assert_eq!(result.revision.get(), 5);
    let px = pixels_of(&result);
    assert_eq!(at(&px, 320, 120, 70), BOX);
    // The host cannot name itself for a window's own session, nor a window
    // for its own session.
    c.snapshot(
        43,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 500),
        None,
        HOST,
        window_session,
    );
    assert_eq!(
        c.inbox.wait(Duration::from_secs(5), answer(43)),
        Err(ErrorCode::UnknownSession)
    );
    c.snapshot(
        44,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 500),
        None,
        WINDOW,
        host_session,
    );
    assert_eq!(
        c.inbox.wait(Duration::from_secs(5), answer(44)),
        Err(ErrorCode::UnknownSession)
    );
    println!("CHILD-OK host session");
}

/// A 320 × 180 project with one level 1 SVG image layer (`art`, 64 × 36,
/// `contain` in an 80 × 45 box at (160, 90)) over the background.
fn svg_project() -> Value {
    let mut doc = project(100.0);
    doc["layers"] = json!([{
        "id": "art", "type": "image", "name": "art", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(160)),
            "position.y": ch(json!(90)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
        "d": { "asset": "svg1", "fit": "contain", "w": 80, "h": 45 }, "locked_intent": {}
    }]);
    doc["assets"] = json!({ "svg1": { "id": "svg1", "name": "art.svg", "kind": "image", "format": "svg", "w": 64, "h": 36, "dur": 0, "persisted": true } });
    doc
}

/// Answers the engine's media requests with `path` until `done` holds.
fn serve_media<T>(
    c: &mut Child,
    path: &std::path::Path,
    timeout: Duration,
    done: impl FnMut(PortId, &Envelope) -> Option<T>,
) -> T {
    try_serve_media(c, path, timeout, done).expect("timed out serving media")
}

/// [`serve_media`], or `None` once `timeout` passed.
fn try_serve_media<T>(
    c: &mut Child,
    path: &std::path::Path,
    timeout: Duration,
    mut done: impl FnMut(PortId, &Envelope) -> Option<T>,
) -> Option<T> {
    use pm_protocol::error::{EngineError, ErrorScope};
    use pm_protocol::host::{CapabilityHandle, HandleAccess, HandleGrant, HandleResolved};
    let deadline = Instant::now() + timeout;
    loop {
        let actions = c.host.pump();
        c.inbox.push(actions);
        let request = c
            .inbox
            .try_wait(Duration::ZERO, &mut |port, m| match &m.message {
                Message::Request { id, body } if port == HOST => Some((*id, body.clone())),
                _ => None,
            });
        if let Some((id, body)) = request {
            let message = match body {
                Request::MintHandle(mint) => Message::Response {
                    id,
                    body: Response::Handle(HandleGrant {
                        handle: CapabilityHandle::from_random_bytes([7; 16]),
                        kind: mint.kind,
                        content_hash: None,
                    }),
                },
                Request::ResolveHandle(r) => Message::Response {
                    id,
                    body: Response::HandleResolved(HandleResolved {
                        handle: r.handle,
                        kind: pm_protocol::host::CapabilityKind::MediaRead {
                            asset: "svg1".into(),
                        },
                        access: HandleAccess::Read,
                        path: path.to_string_lossy().into_owned(),
                    }),
                },
                other => Message::Error {
                    id,
                    error: EngineError::new(
                        ErrorCode::Unsupported,
                        ErrorScope::Request { id },
                        format!("not served here: {other:?}"),
                    ),
                },
            };
            c.host_sends(Envelope::new(message));
            continue;
        }
        if let Some(found) = c.inbox.try_wait(Duration::from_millis(5), &mut done) {
            return Some(found);
        }
        if Instant::now() >= deadline {
            return None;
        }
    }
}

#[test]
#[ignore = "run by level1_svg_snapshots_draw_the_windows_rasters"]
fn child_svg() {
    use pm_protocol::snapshot::CompatMessage;
    use pm_protocol::svg_raster::SvgRasterPixels;
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("pm-snapshot-svg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("art.svg");
    std::fs::write(
        &path,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="36"><rect width="64" height="36" fill="#4C8DFF"/></svg>"##,
    )
    .unwrap();
    let mut c = Child::start();
    let session = c.session;
    c.resync(2, 1, &svg_project());
    let small = Size::new(320, 180);
    // The engine never waits on the renderer: it names the bucket.
    c.snapshot(
        40,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(1),
        WINDOW,
        session,
    );
    let svg = match serve_media(&mut c, &path, Duration::from_secs(20), answer(40)) {
        Ok(SnapshotOutcome::NeedsRenderer {
            fallback,
            compat,
            svg,
            ..
        }) => {
            assert!(fallback.is_empty() && compat.is_empty());
            svg
        }
        other => panic!("expected needsRenderer, got {other:?}"),
    };
    assert_eq!(svg.len(), 1);
    let key = svg[0].key.clone();
    assert_eq!(
        (key.width, key.height, svg[0].asset.as_str()),
        (80, 45, "svg1")
    );
    assert_eq!(
        key.content,
        pm_render::vector::svg::content_hash(&path).unwrap()
    );
    // The window draws it, posts it in two bands, and asks again naming it.
    let fill = [0x4C, 0x8D, 0xFF, 0xFF];
    for (row, rows) in [(0, 20), (20, 25)] {
        c.event(Event::Compat(CompatMessage::SvgPixels(SvgRasterPixels {
            key: key.clone(),
            row,
            rows,
            data: Bytes(fill.repeat((key.width * rows) as usize)),
        })));
    }
    let mut again = frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000);
    again.uploaded = vec![key.upload_key()];
    c.snapshot(41, again, Some(1), WINDOW, session);
    let px = pixels_of(&rendered(serve_media(
        &mut c,
        &path,
        Duration::from_secs(20),
        answer(41),
    )));
    assert_eq!(at(&px, 320, 160, 90), fill, "the window's raster");
    assert_eq!(at(&px, 320, 10, 10), BACKGROUND);
    // Kept: the next snapshot of that bucket draws at once.
    c.snapshot(
        42,
        frame_request(SnapshotClass::Async, small, SnapshotPixels::Opaque, 5_000),
        Some(1),
        WINDOW,
        session,
    );
    let px = pixels_of(&rendered(serve_media(
        &mut c,
        &path,
        Duration::from_secs(20),
        answer(42),
    )));
    assert_eq!(at(&px, 320, 160, 90), fill);

    // Posted bands that reach the engine after the request (two ports
    // race): the job waits for them instead of naming them again.
    let mut moved = svg_project();
    moved["layers"][0]["p"]["scale.x"] = ch(json!(200));
    moved["layers"][0]["p"]["scale.y"] = ch(json!(200));
    c.resync(3, 2, &moved);
    c.snapshot(
        43,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000),
        Some(2),
        WINDOW,
        session,
    );
    let key2 = match serve_media(&mut c, &path, Duration::from_secs(20), answer(43)) {
        Ok(SnapshotOutcome::NeedsRenderer { svg, .. }) => svg[0].key.clone(),
        other => panic!("expected needsRenderer, got {other:?}"),
    };
    assert_eq!((key2.width, key2.height), (160, 90));
    let mut again = frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 5_000);
    again.uploaded = vec![key2.upload_key()];
    c.snapshot(44, again, Some(2), WINDOW, session);
    assert!(
        c.inbox
            .try_wait(Duration::from_millis(200), &mut answer(44))
            .is_none(),
        "answered before the raster arrived"
    );
    let green = [0x20, 0xC0, 0x40, 0xFF];
    c.event(Event::Compat(CompatMessage::SvgPixels(SvgRasterPixels {
        key: key2.clone(),
        row: 0,
        rows: key2.height,
        data: Bytes(green.repeat((key2.width * key2.height) as usize)),
    })));
    let px = pixels_of(&rendered(serve_media(
        &mut c,
        &path,
        Duration::from_secs(20),
        answer(44),
    )));
    assert_eq!(at(&px, 320, 160, 90), green);
    println!("CHILD-OK");
}

#[test]
#[ignore = "run by a_snapshot_that_fails_on_fonts_leaves_the_viewer_asking_for_svg_rasters"]
fn child_svg_font_failure() {
    use pm_protocol::snapshot::CompatMessage;
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("pm-snapshot-svg-fonts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("art.svg");
    std::fs::write(
        &path,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="36"><rect width="64" height="36" fill="#4C8DFF"/></svg>"##,
    )
    .unwrap();
    let mut c = Child::start();
    let session = c.session;
    // A text layer over the SVG: this engine has no font service, so the
    // snapshot's render meets a font failure and returns early.
    let mut doc = svg_project();
    doc["layers"].as_array_mut().unwrap().push(json!({
        "id": "title", "type": "text", "name": "title", "from": 0, "dur": 2, "on": true,
        "lock": false, "shy": false, "collapsed": true, "threeD": false, "solo": false,
        "color": "#4C8DFF", "blend": "normal", "mblur": false, "parent": null, "group": null,
        "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(160)),
            "position.y": ch(json!(40)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
        "d": {
            "text": "Fonts", "boxWidth": ch(json!(0)), "boxHeight": ch(json!(0)),
            "font": "Helvetica", "weight": 700, "size": 24, "tracking": 0, "leading": 1.2,
            "color": "#F2E8D0", "align": "center", "italic": false
        },
        "locked_intent": {}
    }));
    c.resync(2, 1, &doc);
    let small = Size::new(320, 180);
    c.snapshot(
        50,
        frame_request(SnapshotClass::Sync, small, SnapshotPixels::Opaque, 20_000),
        Some(1),
        WINDOW,
        session,
    );
    let outcome = serve_media(&mut c, &path, Duration::from_secs(40), answer(50));
    assert_eq!(
        outcome.err(),
        Some(ErrorCode::Unsupported),
        "the snapshot fails on its fonts"
    );
    // The viewer shares the snapshot's renderer: its misses still ask the
    // window for rasters (a renderer left waiting fails them silently).
    c.resync(3, 2, &svg_project());
    c.event(Event::Viewer(viewer_state()));
    let asked = serve_media(&mut c, &path, Duration::from_secs(20), |port, m| {
        if port != WINDOW {
            return None;
        }
        match &m.message {
            Message::Event {
                body: Event::Compat(CompatMessage::SvgRequests { requests }),
            } => Some(requests.clone()),
            _ => None,
        }
    });
    assert!(
        asked.iter().any(|r| r.asset == "svg1"),
        "the viewer asked for the SVG's raster: {asked:?}"
    );
    let presenter: Value = serde_json::from_str(&c.host.status_json()).unwrap();
    println!(
        "CHILD-OK svg font failure ({} raster requests; presenter {})",
        asked.len(),
        presenter["presenter"]
    );
}

/// A viewer frame from the window port.
fn viewer_frame(port: PortId, m: &Envelope) -> Option<pm_protocol::frame::FrameDescriptor> {
    match &m.message {
        Message::Event {
            body: Event::Frame(d),
        } if port == WINDOW => Some(d.clone()),
        _ => None,
    }
}

/// The viewer's SVG raster requests from the window port.
fn svg_asks(port: PortId, m: &Envelope) -> Option<Vec<pm_protocol::svg_raster::SvgRasterRequest>> {
    use pm_protocol::snapshot::CompatMessage;
    match &m.message {
        Message::Event {
            body: Event::Compat(CompatMessage::SvgRequests { requests }),
        } if port == WINDOW => Some(requests.clone()),
        _ => None,
    }
}

/// The SVG project in the viewer: its first raster ask (no frame first).
fn svg_viewer(c: &mut Child, path: &std::path::Path) -> pm_protocol::svg_raster::SvgRasterKey {
    c.resync(2, 1, &svg_project());
    c.event(Event::Viewer(viewer_state()));
    let asked = serve_media(c, path, Duration::from_secs(20), |port, m| {
        assert!(
            viewer_frame(port, m).is_none(),
            "a frame before the bucket was answered (a stand-in)"
        );
        svg_asks(port, m)
    });
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].asset, "svg1");
    asked[0].key.clone()
}

fn presenter_stats(c: &Child) -> Value {
    let status: Value = serde_json::from_str(&c.host.status_json()).unwrap();
    status["presenter"].clone()
}

fn svg_file(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-snapshot-svg-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("art.svg");
    std::fs::write(
        &path,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="36"><rect width="64" height="36" fill="#4C8DFF"/></svg>"##,
    )
    .unwrap();
    path
}

#[test]
#[ignore = "run by an_svg_raster_the_window_cannot_draw_never_freezes_the_viewer"]
fn child_svg_viewer_failure() {
    use pm_protocol::snapshot::CompatMessage;
    use pm_protocol::svg_raster::SvgRasterPixels;
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let path = svg_file("viewer-failure");
    let mut c = Child::start();
    let key = svg_viewer(&mut c, &path);
    // Waiting for the bucket: no frame meanwhile.
    assert!(
        try_serve_media(&mut c, &path, Duration::from_millis(300), viewer_frame).is_none(),
        "a frame while the bucket is outstanding"
    );
    // The window cannot draw it: the frame is drawn at once, without it.
    let t = Instant::now();
    c.event(Event::Compat(CompatMessage::SvgFailed {
        key: key.clone(),
        reason: "decode".into(),
        message: "Chromium could not decode this SVG".into(),
    }));
    let first = serve_media(&mut c, &path, Duration::from_secs(20), viewer_frame);
    let shown = t.elapsed();
    assert!(shown < Duration::from_secs(1), "the frame took {shown:?}");
    assert_eq!(first.revision.get(), 1);
    assert!(
        !first.quality.stand_in,
        "drawn without the image, not a stand-in"
    );
    let stats = presenter_stats(&c);
    assert_eq!(stats["svgFailedFrames"], json!(1), "{stats}");
    assert_eq!(stats["externalWaitTimeouts"], json!(0), "{stats}");
    c.event(Event::ReleaseFrame(FrameRelease {
        pool_index: first.pool_index,
        seq: first.seq,
    }));
    // Paused, nothing draws again, yet the bucket it lacks is asked for
    // again once the retry is due (5 s after the failure).
    let again = serve_media(&mut c, &path, Duration::from_secs(20), |port, m| {
        assert!(viewer_frame(port, m).is_none(), "drawn again for nothing");
        svg_asks(port, m)
    });
    let retried = t.elapsed();
    assert!(
        retried >= Duration::from_millis(4_500) && retried < Duration::from_secs(8),
        "asked again after {retried:?}"
    );
    assert_eq!(again[0].key, key);
    // Now it arrives: the view draws it.
    let fill = [0x4C, 0x8D, 0xFF, 0xFF];
    let t = Instant::now();
    c.event(Event::Compat(CompatMessage::SvgPixels(SvgRasterPixels {
        key: key.clone(),
        row: 0,
        rows: key.height,
        data: Bytes(fill.repeat((key.width * key.height) as usize)),
    })));
    let drawn = serve_media(&mut c, &path, Duration::from_secs(20), viewer_frame);
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert!(drawn.seq.get() > first.seq.get());
    let stats = presenter_stats(&c);
    assert_eq!(stats["svgFailedFrames"], json!(1), "drawn with it: {stats}");
    println!("CHILD-OK svg viewer failure (shown {shown:?}, asked again {retried:?})");
}

#[test]
#[ignore = "run by an_unanswered_svg_raster_times_out_instead_of_freezing_the_viewer"]
fn child_svg_viewer_timeout() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let cap = Duration::from_millis(SVG_WAIT_CAP_MS.parse().unwrap());
    let path = svg_file("viewer-timeout");
    let mut c = Child::start();
    let _key = svg_viewer(&mut c, &path);
    let t = Instant::now();
    // Never answered: drawn without it at the bound, counted from the ask.
    let drawn = serve_media(&mut c, &path, Duration::from_secs(20), viewer_frame);
    let waited = t.elapsed();
    assert!(
        waited + Duration::from_millis(100) >= cap && waited < cap + Duration::from_secs(2),
        "drawn after {waited:?} (bound {cap:?})"
    );
    assert!(!drawn.quality.stand_in);
    let stats = presenter_stats(&c);
    assert_eq!(stats["externalWaitTimeouts"], json!(1), "{stats}");
    assert_eq!(stats["svgFailedFrames"], json!(1), "{stats}");
    println!("CHILD-OK svg viewer timeout (drawn after {waited:?})");
}
