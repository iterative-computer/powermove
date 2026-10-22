//! The Phase 2 tracer bullet in the interactive engine, without Electron:
//! this test process plays main (the bridge's rendezvous and receive loop)
//! and re-runs itself as a child that plays the interactive engine with the
//! document frame source, driven through the protocol like the kernel's
//! engine connection and the viewer would:
//!
//! * a resync, then the viewer state: a paused frame of the document at its
//!   revision, with overlay geometry where the layer is;
//! * canonical patch batches (pm-wasm's stream): a frame per edit, tagged
//!   with the edit's revision, the overlay following the layer;
//! * capability routing: an effect routes the composition to the legacy
//!   viewer (a session `unsupported` failure, no frames); removing it
//!   brings engine frames back; with the kernel's definitions pushed to the
//!   session the engine draws the effect and an adjustment layer;
//! * playback on the host clock at the composition's rate;
//! * a GPU device loss: the device and pool are rebuilt as a new
//!   generation and the paused frame is drawn again;
//! * with the audio device in use (no host-clock override), a composition
//!   with nothing to hear plays on the host clock from Play, as today's
//!   playhead does, without waiting for the device (Phase 2 gate D2).
#![cfg(target_os = "macos")]

use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pm_bridge::audit::{ExpectedPeer, SignaturePolicy};
use pm_bridge::host::{self, HostEvent, SurfaceTable};
use pm_bridge::rendezvous::{Rendezvous, SERVICE_PREFIX, random_service_name};
use pm_engine::Action;
use pm_engine::engine::testing::hello;
use pm_node::EngineHost;
use pm_node::present::{BRIDGE_ENV, HOST_CLOCK_ENV, SEND_DELAY_ENV, SOURCE_ENV};
use pm_protocol::clock::{TransportCommand, TransportState};
use pm_protocol::codec;
use pm_protocol::color::{ProjectColor, ViewerColorSpace};
use pm_protocol::common::{
    Bytes, ContentHash, PlaybackRate, RequestId, Revision, Seq, Size, TICKS_PER_SECOND,
};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::{EngineProcess, ErrorCode, ErrorDetail};
use pm_protocol::frame::{
    FrameDescriptor, FrameRelease, MediaPreference, PreviewResolution, ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::patch::{
    DocumentEncoding, PatchBatch, PatchEncoding, PatchOrigin, Resync, ResyncSource,
};
use pm_protocol::version::PeerRole;
use serde_json::{Value, json};

const CHILD_ENV: &str = "PM_DOCUMENT_TEST_CHILD";
const WINDOW: PortId = PortId(7);

#[test]
fn the_engine_draws_the_document_follows_its_patches_routes_and_recovers() {
    let name = random_service_name(SERVICE_PREFIX).unwrap();
    let server = Rendezvous::register(&name).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_engine",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
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
    let host_table = table.clone();
    let main = std::thread::spawn(move || {
        let stop = AtomicBool::new(false);
        let mut events = Vec::new();
        host::run(
            server,
            &expected,
            Duration::from_secs(60),
            &host_table,
            &stop,
            &mut |e| events.push(e),
        );
        events
    });
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "child failed:\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("CHILD-OK"),
        "child did not finish:\n{stdout}\n{stderr}"
    );
    let events = main.join().unwrap();
    // Two generations: the first pool, and the one made after the device loss.
    let generations: std::collections::BTreeSet<u32> = events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Surface { info, .. } => Some(info.generation),
            _ => None,
        })
        .collect();
    assert_eq!(generations.len(), 2, "{events:?}");
    let retired = events
        .iter()
        .filter(|e| matches!(e, HostEvent::Retire { client_port, .. } if *client_port == WINDOW.0))
        .count();
    assert_eq!(
        retired, 2,
        "the lost pool and the last one are retired: {events:?}"
    );
}

/// Runs `child` (an ignored test of this file) as the interactive engine
/// child with `envs`, against this process as main; returns its stdout.
fn run_child(child: &str, envs: &[(&str, &str)]) -> String {
    let name = random_service_name(SERVICE_PREFIX).unwrap();
    let server = Rendezvous::register(&name).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            child,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .env(BRIDGE_ENV, &name)
        .env_remove(HOST_CLOCK_ENV)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        command.env(k, v);
    }
    let child = command.spawn().unwrap();
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
    main.join().unwrap();
    stdout
}

#[test]
fn a_silent_composition_plays_from_play_without_waiting_for_the_audio_device() {
    let stdout = run_child("child_silent_play", &[(SOURCE_ENV, "document")]);
    assert!(
        stdout.contains("CHILD-OK"),
        "child did not finish:\n{stdout}"
    );
}

#[test]
fn headless_playback_draws_nothing_and_a_view_joins_its_running_clock() {
    let stdout = run_child(
        "child_silent_play",
        &[(SOURCE_ENV, "document"), ("PM_HEADLESS_TEST", "1")],
    );
    assert!(
        stdout.contains("CHILD-OK"),
        "child did not finish: {stdout}"
    );
}

/// A composition the renderer cannot draw (an effect without the kernel's
/// definitions) still plays on the engine's clock (amendment 21: nothing
/// hands the transport to another engine): the view is told why it draws
/// nothing, again when a viewer attaches, and no frame is drawn.
#[test]
fn an_unsupported_composition_plays_on_the_engine_clock_and_says_why() {
    for headless in ["0", "1"] {
        let mut envs = vec![(SOURCE_ENV, "document"), ("PM_UNSUPPORTED_TEST", "1")];
        if headless == "1" {
            envs.push(("PM_HEADLESS_TEST", "1"));
        }
        let stdout = run_child("child_silent_play", &envs);
        assert!(
            stdout.contains("CHILD-OK"),
            "child did not finish (headless {headless}): {stdout}"
        );
    }
}

/// A paused frame leaves from the completion thread once the GPU finishes
/// it, while the render thread goes on to the next batch. When that batch
/// makes the composition unsupported, the frame drawn before it (held in
/// flight here by `SEND_DELAY_ENV`) reaches the viewer before the report,
/// and the report names the newest frame seq the view was given, so the
/// viewer can tell every older frame from a later decision.
#[test]
fn an_unsupported_report_follows_the_frames_drawn_before_it() {
    let stdout = run_child(
        "child_report_after_in_flight_frame",
        &[
            (SOURCE_ENV, "document"),
            (HOST_CLOCK_ENV, "1"),
            (SEND_DELAY_ENV, "600"),
        ],
    );
    assert!(
        stdout.contains("CHILD-OK"),
        "child did not finish:\n{stdout}"
    );
}

/// The message of the next unsupported failure.
fn unsupported(envelope: &Envelope) -> Option<String> {
    match &envelope.message {
        Message::Event {
            body: Event::Failure(f),
        } if f.error.code == ErrorCode::Unsupported => Some(f.error.message.clone()),
        _ => None,
    }
}

#[test]
#[ignore = "run by a_silent_composition_plays_from_play_without_waiting_for_the_audio_device"]
fn child_silent_play() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let inbox = Arc::new(Inbox::default());
    let sink_inbox = inbox.clone();
    let mut host = EngineHost::with_sink(
        EngineProcess::Interactive,
        Some(Arc::new(move |actions| sink_inbox.push(actions))),
        |key: &str| std::env::var(key).ok(),
    )
    .unwrap();
    let send = |host: &mut EngineHost, envelope: Envelope| {
        let actions = host.receive(WINDOW.0, &codec::encode(&envelope).unwrap());
        inbox.push(actions);
    };
    host.attach(1, "host", None).unwrap();
    host.receive(1, &hello(PeerRole::Host));
    host.attach(WINDOW.0, "renderer", None).unwrap();
    inbox.push(host.receive(WINDOW.0, &hello(PeerRole::Renderer)));
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(1).unwrap(),
            body: Request::OpenSession(OpenSession {
                document: "Ptracer".into(),
                role: SessionRole::Interactive,
                process: EngineProcess::Interactive,
                color: ProjectColor::DEFAULT,
            }),
        }),
    );
    let session = inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Response {
            body: Response::SessionOpened { session },
            ..
        } => Some(*session),
        _ => None,
    });
    let event = |body: Event| Envelope::new(Message::Event { body }).with_session(session);
    // A solid and nothing else: nothing to hear. Unsupported: an effect
    // without definitions (the renderer cannot draw it).
    let unsupported_test = std::env::var("PM_UNSUPPORTED_TEST").is_ok();
    let fx = if unsupported_test {
        json!([{ "id": "fx1", "type": "blur", "on": true, "params": {} }])
    } else {
        json!([])
    };
    let doc = project(solid(100.0, fx));
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(2).unwrap(),
            body: Request::Resync(Resync {
                document: "Ptracer".into(),
                revision: Revision::new(1).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(doc.to_string().into_bytes()),
                },
                hash: ContentHash(pm_model::hash::canonical_digest(&model(&doc))),
            }),
        })
        .with_session(session),
    );
    let headless = std::env::var("PM_HEADLESS_TEST").is_ok();
    let mut viewer = state(1);
    viewer.headless = Some(headless);
    send(&mut host, event(Event::Viewer(viewer)));
    if unsupported_test {
        let why = inbox.wait(Duration::from_secs(10), unsupported);
        assert!(
            why.starts_with("The new engine cannot draw this composition yet:"),
            "{why}"
        );
    }
    if headless {
        // A headless clock carries the revision only while the engine draws
        // the composition: the renderer learns the route from it.
        let tag = if unsupported_test {
            None
        } else {
            Revision::new(1).ok()
        };
        inbox.wait(Duration::from_secs(5), |m| match &m.message {
            Message::Event {
                body: Event::Clock(_),
            } if m.revision == tag => Some(()),
            _ => None,
        });
        let status: Value = serde_json::from_str(&host.status_json()).unwrap();
        assert_eq!(status["presenter"]["pools"], 0);
        assert_eq!(status["presenter"]["frames"], 0);
    } else if unsupported_test {
        // Nothing to draw: the failure above is what the viewer shows.
    } else {
        let first = inbox.wait(Duration::from_secs(20), frame);
        send(
            &mut host,
            event(Event::ReleaseFrame(FrameRelease {
                pool_index: first.pool_index,
                seq: first.seq,
            })),
        );
    }
    // Give a warmed audio thread time to decide (it must not open the
    // device for a silent composition).
    std::thread::sleep(Duration::from_millis(300));
    let pressed = pm_node::present::render::now_us();
    send(
        &mut host,
        event(Event::Transport(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })),
    );
    let run = inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Event {
            body: Event::Clock(c),
        } if c.state == TransportState::Playing => Some(*c),
        _ => None,
    });
    let lead = run.host_time.get() as i64 - pressed;
    // The host-clock line's frame pre-roll (50 ms), not an output's
    // latency (160 ms on the reference machine's Bluetooth output).
    assert!(
        (0..=pm_node::present::clock::PREROLL_US + 20_000).contains(&lead),
        "the run starts {lead} µs after Play"
    );
    assert_eq!(run.output_latency_us, 0, "{run:?}");
    if headless {
        std::thread::sleep(Duration::from_millis(150));
        let status: Value = serde_json::from_str(&host.status_json()).unwrap();
        assert_eq!(status["presenter"]["frames"], 0);
        inbox.take();
        send(&mut host, event(Event::Viewer(state(2))));
        if unsupported_test {
            // The viewer that attaches is told why it draws nothing.
            let why = inbox.wait(Duration::from_secs(10), unsupported);
            assert!(why.contains("cannot draw"), "{why}");
        } else {
            let joined = inbox.wait(Duration::from_secs(20), frame);
            assert!(
                joined.duration.is_some(),
                "joins timed playback, without a pause or seek"
            );
            assert!(joined.pts.get() > 0);
        }
        let clock = inbox.wait(Duration::from_secs(2), |m| match &m.message {
            Message::Event {
                body: Event::Clock(c),
            } if c.state == TransportState::Playing => Some(*c),
            _ => None,
        });
        assert_eq!(clock.epoch, run.epoch);
    }
    send(&mut host, event(Event::Transport(TransportCommand::Pause)));
    inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Event {
            body: Event::Clock(c),
        } if c.state == TransportState::Stopped => Some(()),
        _ => None,
    });
    let status: Value = serde_json::from_str(&host.status_json()).unwrap();
    let presenter = &status["presenter"];
    if unsupported_test {
        assert_eq!(presenter["frames"], 0, "{presenter}");
        assert!(
            inbox.take().iter().filter_map(frame).next().is_none(),
            "no frame is drawn for an unsupported composition"
        );
    }
    assert_eq!(presenter["silentRuns"], 1, "{presenter}");
    assert_eq!(presenter["hostClockRuns"], 0, "{presenter}");
    if unsupported_test && headless {
        // Viewerless again, and an edit makes the composition drawable: the
        // clock comes tagged with the new revision (no frame says so).
        let mut away = state(3);
        away.headless = Some(true);
        send(&mut host, event(Event::Viewer(away)));
        let drawable = project(solid(100.0, json!([])));
        inbox.take();
        send(
            &mut host,
            Envelope::new(Message::Request {
                id: RequestId::new(3).unwrap(),
                body: Request::Resync(Resync {
                    document: "Ptracer".into(),
                    revision: Revision::new(2).unwrap(),
                    encoding: DocumentEncoding::CanonicalJsonV1,
                    source: ResyncSource::Inline {
                        body: Bytes(drawable.to_string().into_bytes()),
                    },
                    hash: ContentHash(pm_model::hash::canonical_digest(&model(&drawable))),
                }),
            })
            .with_session(session),
        );
        inbox.wait(Duration::from_secs(10), |m| match &m.message {
            Message::Event {
                body: Event::Clock(_),
            } if m.revision == Revision::new(2).ok() => Some(()),
            Message::Event {
                body: Event::Clock(_),
            } if m.revision.is_some() => panic!("a clock tagged {:?}", m.revision),
            _ => None,
        });
    }
    // The view's audio thread never opened a device.
    for audio in presenter["audio"]
        .as_object()
        .into_iter()
        .flat_map(|a| a.values())
    {
        assert!(audio["device"].is_null(), "{presenter}");
    }
    drop(host);
    println!("CHILD-OK lead={lead}");
}

#[derive(Default)]
struct Inbox {
    messages: Mutex<Vec<Envelope>>,
    arrived: Condvar,
}

impl Inbox {
    fn push(&self, actions: Vec<Action>) {
        let mut messages = self.messages.lock().unwrap();
        for action in actions {
            match action {
                Action::Send { port, bytes } if port == WINDOW => {
                    messages.push(codec::decode(&bytes).expect("engine output decodes"));
                }
                Action::Log { level, message } => eprintln!("engine {level:?}: {message}"),
                _ => {}
            }
        }
        self.arrived.notify_all();
    }

    fn wait<T>(&self, timeout: Duration, mut pick: impl FnMut(&Envelope) -> Option<T>) -> T {
        let deadline = Instant::now() + timeout;
        let mut messages = self.messages.lock().unwrap();
        loop {
            if let Some(i) = messages.iter().position(|m| pick(m).is_some()) {
                let found = pick(&messages[i]).unwrap();
                messages.drain(..=i);
                return found;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "timed out; inbox: {:?}",
                messages.iter().map(|m| &m.message).collect::<Vec<_>>()
            );
            messages = self.arrived.wait_timeout(messages, left).unwrap().0;
        }
    }

    fn take(&self) -> Vec<Envelope> {
        std::mem::take(&mut *self.messages.lock().unwrap())
    }
}

fn frame(envelope: &Envelope) -> Option<FrameDescriptor> {
    match &envelope.message {
        Message::Event {
            body: Event::Frame(d),
        } => Some(d.clone()),
        _ => None,
    }
}

fn failure(envelope: &Envelope) -> Option<ErrorCode> {
    match &envelope.message {
        Message::Event {
            body: Event::Failure(f),
        } => Some(f.error.code),
        _ => None,
    }
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn solid(x: f64, fx: Value) -> Value {
    json!({
        "id": "box", "type": "solid", "name": "box", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(x)),
            "position.y": ch(json!(60)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": fx, "transitionIn": null, "transitionOut": null, "masks": [],
        "d": { "color": "#E8E2CF", "w": 40, "h": 20 }, "locked_intent": {}
    })
}

fn project(layer: Value) -> Value {
    json!({
        "id": "Ptracer", "name": "tracer", "w": 320, "h": 180, "fps": 30, "dur": 2, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#202020", "position": 0 }] },
        "compId": "Ptracer-root", "compName": "tracer", "markers": [], "work": [0, 2], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer], "assets": {}, "comps": {}
    })
}

fn model(v: &Value) -> pm_model::Value {
    pm_model::json::parse(&v.to_string()).unwrap()
}

fn state(view_seq: u64) -> ViewerState {
    ViewerState {
        headless: None,
        composition: "Ptracer-root".into(),
        size: Size::new(1280, 720),
        device_pixel_ratio: 2.0,
        zoom: 2.0,
        pan: [0.0, 0.0],
        resolution: PreviewResolution::Full,
        media: MediaPreference::Automatic,
        color_space: ViewerColorSpace::Srgb,
        view: None,
        play_every_frame: false,
        edr: false,
        view_seq: Seq::new(view_seq).unwrap(),
        vsync: None,
        depth_id: None,
        chrome_selection: None,
    }
}

#[test]
#[ignore = "run by the_engine_draws_the_document_follows_its_patches_routes_and_recovers"]
fn child_engine() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let inbox = Arc::new(Inbox::default());
    let sink_inbox = inbox.clone();
    let env = |key: &str| match key {
        "POWERMOVE_BRIDGE_LEAD" => Some("2".to_owned()),
        other => std::env::var(other).ok(),
    };
    let mut host = EngineHost::with_sink(
        EngineProcess::Interactive,
        Some(Arc::new(move |actions| sink_inbox.push(actions))),
        env,
    )
    .unwrap();
    let send = |host: &mut EngineHost, envelope: Envelope| {
        let actions = host.receive(WINDOW.0, &codec::encode(&envelope).unwrap());
        inbox.push(actions);
    };
    host.attach(1, "host", None).unwrap();
    host.receive(1, &hello(PeerRole::Host));
    host.attach(WINDOW.0, "renderer", None).unwrap();
    inbox.push(host.receive(WINDOW.0, &hello(PeerRole::Renderer)));
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(1).unwrap(),
            body: Request::OpenSession(OpenSession {
                document: "Ptracer".into(),
                role: SessionRole::Interactive,
                process: EngineProcess::Interactive,
                color: ProjectColor::DEFAULT,
            }),
        }),
    );
    let session = inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Response {
            body: Response::SessionOpened { session },
            ..
        } => Some(*session),
        _ => None,
    });
    let event = |body: Event| Envelope::new(Message::Event { body }).with_session(session);
    let release = |host: &mut EngineHost, d: &FrameDescriptor| {
        send(
            host,
            event(Event::ReleaseFrame(FrameRelease {
                pool_index: d.pool_index,
                seq: d.seq,
            })),
        );
    };

    // The document, then the viewer.
    let mut doc = project(solid(100.0, json!([])));
    let text = model(&doc);
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(2).unwrap(),
            body: Request::Resync(Resync {
                document: "Ptracer".into(),
                revision: Revision::new(1).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(doc.to_string().into_bytes()),
                },
                hash: ContentHash(pm_model::hash::canonical_digest(&text)),
            }),
        })
        .with_session(session),
    );
    send(&mut host, event(Event::Viewer(state(1))));
    let first = inbox.wait(Duration::from_secs(20), frame);
    assert_eq!(first.revision.get(), 1);
    assert_eq!(first.duration, None);
    let overlay = first.overlay.clone().unwrap();
    // The composition (320×180 at zoom 2, DPR 2) is centered in the frame.
    assert_eq!(
        (
            overlay.viewport.x,
            overlay.viewport.y,
            overlay.viewport.width,
            overlay.viewport.height
        ),
        (0.0, 0.0, 1280.0, 720.0)
    );
    let quad = &overlay.layers[0];
    assert_eq!(quad.layer.as_str(), "box");
    let x0 = quad.bounds[0][0];
    release(&mut host, &first);

    // A patch batch (pm-wasm's canonical binary) moves the layer.
    let mut revision = 1;
    let mut edit = |host: &mut EngineHost, next: Value, doc: &mut Value| {
        let ops = pm_model::ops::diff_ops(Some(&model(doc)), Some(&model(&next)));
        let body = pm_model::binary::encode(&ops).unwrap();
        let batch = PatchBatch {
            document: "Ptracer".into(),
            base_revision: Revision::new(revision).unwrap(),
            revision: Revision::new(revision + 1).unwrap(),
            origin: PatchOrigin::Edit,
            encoding: PatchEncoding::CanonicalV1,
            patch_count: ops.len() as u32,
            body: Bytes(body),
        };
        revision += 1;
        *doc = next;
        send(host, event(Event::ApplyPatches(batch)));
        revision
    };
    let r = edit(&mut host, project(solid(130.0, json!([]))), &mut doc);
    let moved = inbox.wait(Duration::from_secs(10), |m| {
        frame(m).filter(|d| d.revision.get() == r)
    });
    let dx = moved.overlay.as_ref().unwrap().layers[0].bounds[0][0] - x0;
    assert!(
        (dx - 30.0 * 4.0).abs() < 1e-6,
        "the overlay follows the layer: {dx}"
    );
    release(&mut host, &moved);

    // An effect routes the composition to the legacy viewer.
    let fx = json!([{ "id": "fx1", "type": "blur", "on": true, "params": {} }]);
    let routed_revision = edit(&mut host, project(solid(130.0, fx)), &mut doc);
    let code = inbox.wait(Duration::from_secs(10), failure);
    assert_eq!(code, ErrorCode::Unsupported);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        inbox
            .take()
            .iter()
            .filter_map(frame)
            .all(|d| d.revision.get() < routed_revision),
        "no engine frames while routed"
    );
    // Without it, engine frames come back.
    let r = edit(&mut host, project(solid(130.0, json!([]))), &mut doc);
    let back = inbox.wait(Duration::from_secs(10), |m| {
        frame(m).filter(|d| d.revision.get() == r)
    });
    release(&mut host, &back);

    // With the kernel's definitions (contract 10) the engine draws the same
    // effect: the session's set reaches the presenter's renderer.
    let builtins = pm_render::builtin_definitions();
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(3).unwrap(),
            body: Request::PushDefinitions(pm_protocol::definitions::DefinitionsUpdate {
                generation: Seq::new(1).unwrap(),
                full: true,
                upserts: builtins.definitions,
                removals: vec![],
            }),
        })
        .with_session(session),
    );
    let applied = inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Response {
            body: Response::DefinitionsApplied(applied),
            ..
        } => Some(applied.clone()),
        _ => None,
    });
    assert_eq!(applied.generation, Seq::new(1).unwrap());
    let blurred =
        json!([{ "id": "fx1", "type": "blur", "on": true, "p": { "amount": ch(json!(8)) } }]);
    let r = edit(&mut host, project(solid(130.0, blurred)), &mut doc);
    let drawn = inbox.wait(Duration::from_secs(20), |m| {
        frame(m).filter(|d| d.revision.get() == r)
    });
    release(&mut host, &drawn);
    // An adjustment layer with effects is drawn too (not routed).
    let mut adjusted = project(solid(130.0, json!([])));
    let mut adjustment = solid(
        0.0,
        json!([{ "id": "fx2", "type": "invert", "on": true, "p": { "amount": ch(json!(100)) } }]),
    );
    adjustment["id"] = json!("adj");
    adjustment["type"] = json!("adjustment");
    adjustment["d"] = json!({});
    adjusted["layers"]
        .as_array_mut()
        .unwrap()
        .insert(0, adjustment);
    let r = edit(&mut host, adjusted, &mut doc);
    let inverted = inbox.wait(Duration::from_secs(20), |m| {
        frame(m).filter(|d| d.revision.get() == r)
    });
    release(&mut host, &inverted);
    let r = edit(&mut host, project(solid(130.0, json!([]))), &mut doc);
    let plain = inbox.wait(Duration::from_secs(10), |m| {
        frame(m).filter(|d| d.revision.get() == r)
    });
    release(&mut host, &plain);
    // A definition that does not compile is reported for the shader UI.
    let source = "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main(){ o = nope; }\n";
    let broken: pm_protocol::definitions::DefinitionSnapshot = serde_json::from_value(json!({
        "id": "broken", "kind": "effect", "version": 0, "origin": "local", "renderer": null,
        "glsl": { "source": source, "bodyLineOffset": 0, "bodyUtf16Len": source.len(), "bodyUtf8Len": source.len() },
        "params": [],
        "flags": { "passes": 1, "keepOrig": false, "backdrop": false, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": false }
    }))
    .unwrap();
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(4).unwrap(),
            body: Request::PushDefinitions(pm_protocol::definitions::DefinitionsUpdate {
                generation: Seq::new(2).unwrap(),
                full: false,
                upserts: vec![broken],
                removals: vec![],
            }),
        })
        .with_session(session),
    );
    // Reported at the definitions generation it was translated at (the
    // kernel drops one older than its latest push of that definition).
    let at = inbox.wait(Duration::from_secs(10), |m| match &m.message {
        Message::Event {
            body: Event::Failure(report),
        } if report.error.code == ErrorCode::ShaderCompile => Some(report.revision),
        _ => None,
    });
    assert_eq!(at.get(), 2, "the generation of the push that broke it");

    // Playback on the host clock at 30 fps.
    send(
        &mut host,
        event(Event::Transport(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })),
    );
    inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Event {
            body: Event::Clock(c),
        } if c.state == TransportState::Playing => Some(()),
        _ => None,
    });
    let mut timed = Vec::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(700) {
        let d = inbox.wait(Duration::from_secs(3), frame);
        release(&mut host, &d);
        if d.duration.is_some() {
            timed.push(d);
        }
    }
    assert!(timed.len() >= 15, "{} timed frames", timed.len());
    for pair in timed.windows(2) {
        assert_eq!(pair[1].pts.get() - pair[0].pts.get(), TICKS_PER_SECOND / 30);
    }
    send(&mut host, event(Event::Transport(TransportCommand::Pause)));
    inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Event {
            body: Event::Clock(c),
        } if c.state == TransportState::Stopped => Some(()),
        _ => None,
    });
    for d in inbox.take().iter().filter_map(frame) {
        release(&mut host, &d);
    }

    // A GPU device loss: a new pool generation and the paused frame again.
    let generation = timed.last().unwrap().pool_generation.get();
    assert!(host.simulate_gpu_loss());
    let lost = inbox.wait(Duration::from_secs(10), failure);
    assert_eq!(lost, ErrorCode::DeviceLost);
    let pool = inbox.wait(Duration::from_secs(20), |m| match &m.message {
        Message::Event {
            body: Event::SurfacePool(p),
        } => Some(p.generation.get()),
        _ => None,
    });
    assert!(pool > generation);
    let again = inbox.wait(Duration::from_secs(20), |m| {
        frame(m).filter(|d| d.pool_generation.get() == pool)
    });
    assert_eq!(again.revision.get(), r);
    release(&mut host, &again);
    let status: Value = serde_json::from_str(&host.status_json()).unwrap();
    let presenter = &status["presenter"];
    assert_eq!(presenter["deviceLosses"], 1, "{presenter}");
    assert!(
        presenter["routedLegacy"].as_u64().unwrap() >= 1,
        "{presenter}"
    );
    assert_eq!(presenter["renderErrors"], 0, "{presenter}");

    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(30).unwrap(),
            body: Request::CloseSession,
        })
        .with_session(session),
    );
    drop(host);
    println!("CHILD-OK timed={}", timed.len());
}

#[test]
#[ignore = "run by an_unsupported_report_follows_the_frames_drawn_before_it"]
fn child_report_after_in_flight_frame() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let inbox = Arc::new(Inbox::default());
    let sink_inbox = inbox.clone();
    let mut host = EngineHost::with_sink(
        EngineProcess::Interactive,
        Some(Arc::new(move |actions| sink_inbox.push(actions))),
        |key: &str| std::env::var(key).ok(),
    )
    .unwrap();
    let send = |host: &mut EngineHost, envelope: Envelope| {
        let actions = host.receive(WINDOW.0, &codec::encode(&envelope).unwrap());
        inbox.push(actions);
    };
    host.attach(1, "host", None).unwrap();
    host.receive(1, &hello(PeerRole::Host));
    host.attach(WINDOW.0, "renderer", None).unwrap();
    inbox.push(host.receive(WINDOW.0, &hello(PeerRole::Renderer)));
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(1).unwrap(),
            body: Request::OpenSession(OpenSession {
                document: "Ptracer".into(),
                role: SessionRole::Interactive,
                process: EngineProcess::Interactive,
                color: ProjectColor::DEFAULT,
            }),
        }),
    );
    let session = inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Response {
            body: Response::SessionOpened { session },
            ..
        } => Some(*session),
        _ => None,
    });
    let event = |body: Event| Envelope::new(Message::Event { body }).with_session(session);
    let mut doc = project(solid(100.0, json!([])));
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(2).unwrap(),
            body: Request::Resync(Resync {
                document: "Ptracer".into(),
                revision: Revision::new(1).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(doc.to_string().into_bytes()),
                },
                hash: ContentHash(pm_model::hash::canonical_digest(&model(&doc))),
            }),
        })
        .with_session(session),
    );
    send(&mut host, event(Event::Viewer(state(1))));
    let first = inbox.wait(Duration::from_secs(20), frame);
    assert_eq!(first.revision.get(), 1);
    send(
        &mut host,
        event(Event::ReleaseFrame(FrameRelease {
            pool_index: first.pool_index,
            seq: first.seq,
        })),
    );
    let mut revision = 1;
    let mut edit = |host: &mut EngineHost, next: Value, doc: &mut Value| {
        let ops = pm_model::ops::diff_ops(Some(&model(doc)), Some(&model(&next)));
        let batch = PatchBatch {
            document: "Ptracer".into(),
            base_revision: Revision::new(revision).unwrap(),
            revision: Revision::new(revision + 1).unwrap(),
            origin: PatchOrigin::Edit,
            encoding: PatchEncoding::CanonicalV1,
            patch_count: ops.len() as u32,
            body: Bytes(pm_model::binary::encode(&ops).unwrap()),
        };
        revision += 1;
        *doc = next;
        send(host, event(Event::ApplyPatches(batch)));
        revision
    };

    // A drawable edit: its paused frame is drawn and waits in the
    // completion thread (600 ms) while the next batch makes the composition
    // unsupported (an effect without the kernel's definitions).
    let drawn = edit(&mut host, project(solid(130.0, json!([]))), &mut doc);
    std::thread::sleep(Duration::from_millis(150));
    let fx = json!([{ "id": "fx1", "type": "blur", "on": true, "params": {} }]);
    let routed = edit(&mut host, project(solid(130.0, fx)), &mut doc);
    // Everything the viewer receives, in order, until the report and for a
    // while after it (a frame drawn before the report and sent after it
    // would arrive in this time).
    let mut seen: Vec<Envelope> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !seen.iter().any(|m| unsupported(m).is_some()) {
        assert!(Instant::now() < deadline, "no report: {seen:?}");
        std::thread::sleep(Duration::from_millis(5));
        seen.extend(inbox.take());
    }
    std::thread::sleep(Duration::from_millis(1_200));
    seen.extend(inbox.take());
    let report = seen.iter().position(|m| unsupported(m).is_some()).unwrap();
    let Message::Event {
        body: Event::Failure(failure),
    } = &seen[report].message
    else {
        unreachable!()
    };
    assert_eq!(failure.revision.get(), routed);
    let Some(ErrorDetail::Unsupported { last_frame }) = failure.error.detail.clone() else {
        panic!("the report names the view's newest frame: {failure:?}");
    };
    // The frame of the drawable edit was in flight when the batch that made
    // the composition unsupported arrived; it reaches the viewer first.
    let in_flight = seen[..report]
        .iter()
        .filter_map(frame)
        .find(|d| d.revision.get() == drawn)
        .unwrap_or_else(|| {
            panic!(
                "the in-flight frame (revision {drawn}) arrives before the report: {:?}",
                seen.iter().map(|m| &m.message).collect::<Vec<_>>()
            )
        });
    assert!(
        in_flight.seq <= last_frame,
        "the report covers the frame drawn before it: {} > {}",
        in_flight.seq.get(),
        last_frame.get()
    );
    let late: Vec<_> = seen[report + 1..].iter().filter_map(frame).collect();
    assert!(
        late.is_empty(),
        "no frame reaches the viewer after the report: {late:?}"
    );
    println!("CHILD-OK drawn={drawn} routed={routed}");
    drop(host);
}
