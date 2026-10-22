//! The interactive engine's viewer presenter end to end, without Electron:
//! this test process plays main (the bridge's rendezvous and receive loop),
//! and re-runs itself as a child that plays the interactive engine with the
//! bridge test source, driven through the protocol like a viewer would.
#![cfg(target_os = "macos")]

use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pm_bridge::audit::{ExpectedPeer, SignaturePolicy};
use pm_bridge::host::{self, HostEvent, SurfaceTable};
use pm_bridge::rendezvous::{Rendezvous, SERVICE_PREFIX, random_service_name};
use pm_bridge::wire::PixelFormat;
use pm_engine::Action;
use pm_engine::engine::testing::hello;
use pm_node::EngineHost;
use pm_node::present::{BRIDGE_ENV, REMOTE_LAYER_ENV, SOURCE_ENV};
use pm_protocol::clock::{TransportCommand, TransportState};
use pm_protocol::codec;
use pm_protocol::color::{ProjectColor, ViewerColorSpace};
use pm_protocol::common::{PlaybackRate, RequestId, Revision, Seq, Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::EngineProcess;
use pm_protocol::frame::{
    FrameDescriptor, FramePurpose, FrameRelease, FrameRequest, MediaPreference, MotionBlurKind,
    PreviewResolution, ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::interop::SurfaceFormat;
use pm_protocol::priority::PriorityClass;
use pm_protocol::version::PeerRole;

const CHILD_ENV: &str = "PM_PRESENT_TEST_CHILD";
const WINDOW: PortId = PortId(7);

#[test]
fn the_presenter_hands_pools_to_main_and_frames_to_the_viewer() {
    present(&[]);
}

/// The C′ prototype is compiled only with the `remote-layer` feature
/// (measurement builds). Asked for it, a build without it says so and
/// presents through the viewer as usual: the child's frames still arrive as
/// viewer events, and main is never asked to host a remote layer.
#[test]
#[cfg(not(feature = "remote-layer"))]
fn a_build_without_c_prime_presents_through_the_viewer_when_asked_for_it() {
    let events = present(&[(REMOTE_LAYER_ENV, "1")]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, HostEvent::RemoteLayer { .. })),
        "{events:?}"
    );
}

/// Runs the child engine against this process playing main, checks what
/// main saw, and returns main's events.
fn present(extra_env: &[(&str, &str)]) -> Vec<HostEvent> {
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
        .env(SOURCE_ENV, "test-pattern")
        .envs(extra_env.iter().copied())
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
            Duration::from_secs(20),
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

    // Main: one check-in, generation 1 (bgra) and 2 (rgbaf16) of 6 surfaces
    // each (ring 2 + lead 2 + 2), both retired, then the engine's goodbye.
    assert!(
        matches!(events[0], HostEvent::Accepted { .. }),
        "{events:?}"
    );
    let surfaces: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Surface { info, .. } => Some(*info),
            _ => None,
        })
        .collect();
    assert_eq!(surfaces.len(), 18, "{events:?}");
    for info in &surfaces[..4] {
        assert_eq!(
            (info.client_port, info.generation, info.pool_size),
            (WINDOW.0, 1, 4)
        );
        assert_eq!(
            (info.width, info.height, info.format),
            (1280, 720, PixelFormat::Bgra8)
        );
    }
    for (range, generation, count) in [(4..10, 2, 6), (10..14, 3, 4)] {
        for info in &surfaces[range] {
            assert_eq!(
                (info.generation, info.pool_size, info.format),
                (generation, count, PixelFormat::Bgra8)
            );
        }
    }
    for info in &surfaces[14..] {
        assert_eq!(
            (info.generation, info.pool_size, info.format),
            (4, 4, PixelFormat::Rgba16F)
        );
    }
    let retired: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Retire {
                client_port,
                generation,
            } if *client_port == WINDOW.0 => Some(*generation),
            _ => None,
        })
        .collect();
    assert_eq!(retired, vec![1, 2, 3, 4]);
    assert!(events.contains(&HostEvent::Goodbye));
    assert_eq!(events.last(), Some(&HostEvent::Gone));
    assert_eq!(
        table.lock().unwrap().len(),
        18,
        "main holds surfaces until Chromium releases them"
    );
    events
}

/// Collects the engine's asynchronous actions.
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
                Action::Send { port, .. } if port != WINDOW => {}
                Action::Send { bytes, .. } => {
                    messages.push(codec::decode(&bytes).expect("engine output decodes"));
                }
                Action::Log { level, message } => eprintln!("engine {level:?}: {message}"),
                Action::Close { .. } => {}
            }
        }
        self.arrived.notify_all();
    }

    /// Waits for the first message `pick` accepts, removing it and every
    /// message before it from the inbox (and returning those too).
    fn wait<T>(
        &self,
        timeout: Duration,
        mut pick: impl FnMut(&Envelope) -> Option<T>,
    ) -> (T, Vec<Envelope>) {
        let deadline = Instant::now() + timeout;
        let mut messages = self.messages.lock().unwrap();
        loop {
            if let Some(i) = messages.iter().position(|m| pick(m).is_some()) {
                let found = pick(&messages[i]).unwrap();
                let before: Vec<Envelope> = messages.drain(..=i).collect();
                return (found, before);
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

fn state(view_seq: u64, edr: bool, pan: [f64; 2]) -> ViewerState {
    ViewerState {
        headless: None,
        composition: "main".into(),
        size: Size::new(1280, 720),
        device_pixel_ratio: 2.0,
        zoom: 1.0 / 6.0,
        pan,
        resolution: PreviewResolution::Automatic,
        media: MediaPreference::Automatic,
        color_space: ViewerColorSpace::Srgb,
        view: None,
        play_every_frame: false,
        edr,
        view_seq: Seq::new(view_seq).unwrap(),
        vsync: None,
        depth_id: None,
        chrome_selection: None,
    }
}

/// The child: an interactive engine with the presenter, driven like a viewer.
#[test]
#[ignore = "run by the_presenter_hands_pools_to_main_and_frames_to_the_viewer"]
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
    // A host port with a lease, so the rgbaf16 pool fits.
    host.attach(1, "host", None).unwrap();
    host.receive(1, &hello(PeerRole::Host));
    let grant = Envelope::new(Message::Request {
        id: RequestId::new(1).unwrap(),
        body: Request::GrantLease(pm_protocol::lease::Lease {
            process: EngineProcess::Interactive,
            epoch: Seq::new(1).unwrap(),
            bytes: pm_protocol::common::ByteSize::new(4 << 30).unwrap(),
            reason: pm_protocol::lease::LeaseReason::Initial,
            ack_deadline_ms: 1000,
        }),
    });
    host.receive(1, &codec::encode(&grant).unwrap());
    host.attach(WINDOW.0, "renderer", None).unwrap();
    inbox.push(host.receive(WINDOW.0, &hello(PeerRole::Renderer)));
    let open = Envelope::new(Message::Request {
        id: RequestId::new(1).unwrap(),
        body: Request::OpenSession(OpenSession {
            document: "doc".into(),
            role: SessionRole::Interactive,
            process: EngineProcess::Interactive,
            color: ProjectColor::DEFAULT,
        }),
    });
    send(&mut host, open);
    let (session, _) = inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Response {
            body: Response::SessionOpened { session },
            ..
        } => Some(*session),
        _ => None,
    });
    let event = |body: Event, revision: Option<u64>| {
        let mut envelope = Envelope::new(Message::Event { body }).with_session(session);
        envelope.revision = revision.map(|r| Revision::new(r).unwrap());
        envelope
    };
    let release = |host: &mut EngineHost, d: &FrameDescriptor| {
        let body = Event::ReleaseFrame(FrameRelease {
            pool_index: d.pool_index,
            seq: d.seq,
        });
        send(host, event(body, None));
    };

    // Viewer state: the four-slot paused pool, then a paused frame.
    send(
        &mut host,
        event(Event::Viewer(state(1, false, [0.0, 0.0])), Some(5)),
    );
    let (pool, _) = inbox.wait(Duration::from_secs(10), |m| match &m.message {
        Message::Event {
            body: Event::SurfacePool(p),
        } => Some(p.clone()),
        _ => None,
    });
    assert_eq!(pool.generation.get(), 1);
    assert_eq!(pool.surfaces.len(), 4);
    assert_eq!(pool.surfaces[0].format, SurfaceFormat::Bgra8Unorm);
    let (first, _) = inbox.wait(Duration::from_secs(5), frame);
    assert_eq!(
        (first.seq.get(), first.view_seq.get(), first.revision.get()),
        (1, 1, 5)
    );
    assert_eq!(first.duration, None);
    assert_eq!(first.pool_generation.get(), 1);
    let marker = first.overlay.as_ref().unwrap().layers[0].bounds[0];
    release(&mut host, &first);

    // A view change renders a new paused frame tagged with it.
    send(
        &mut host,
        event(Event::Viewer(state(2, false, [10.0, 0.0])), None),
    );
    let (second, _) = inbox.wait(Duration::from_secs(5), frame);
    assert_eq!((second.seq.get(), second.view_seq.get()), (2, 2));
    assert_eq!(
        second.overlay.as_ref().unwrap().layers[0].bounds[0][0],
        marker[0] + 20.0
    );
    release(&mut host, &second);

    // Play: a clock sample, then timed frames a frame apart, each released
    // by "the viewer" as it arrives, for about a second.
    send(
        &mut host,
        event(
            Event::Transport(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            }),
            None,
        ),
    );
    let (sample, _) = inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Event {
            body: Event::Clock(c),
        } if c.state == TransportState::Playing => Some(*c),
        _ => None,
    });
    let frame_ticks = TICKS_PER_SECOND / 60;
    let mut timed = Vec::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1000) {
        let (d, _) = inbox.wait(Duration::from_secs(2), frame);
        release(&mut host, &d);
        timed.push(d);
    }
    assert!(timed.len() >= 50, "{} frames in a second", timed.len());
    for pair in timed.windows(2) {
        assert_eq!(
            pair[1].pts.get() - pair[0].pts.get(),
            frame_ticks,
            "consecutive frames"
        );
        assert_eq!(pair[1].seq.get(), pair[0].seq.get() + 1);
    }
    assert!(
        timed
            .iter()
            .all(|d| d.duration == Some(Ticks::new(frame_ticks).unwrap()))
    );
    // Playback starts at the paused frame (media 0). The first sample is
    // taken during the preroll: it names where the run starts, at a host
    // time still ahead, rather than the media before it the line would
    // extrapolate to (clock.rs, `run_starts_at`).
    assert_eq!(timed[0].pts.get(), 0);
    assert_eq!(sample.media_time.get(), 0);

    // Without releases the engine stops at the pool: never more than 6 held.
    let held_start = inbox.take().into_iter().filter_map(|m| frame(&m)).count();
    std::thread::sleep(Duration::from_millis(300));
    let unreleased = held_start + inbox.take().into_iter().filter_map(|m| frame(&m)).count();
    assert!(
        unreleased <= 6,
        "{unreleased} frames rendered without releases"
    );
    let status: serde_json::Value = serde_json::from_str(&host.status_json()).unwrap();
    assert!(
        status["presenter"]["poolWaits"].as_u64().unwrap() > 0,
        "{status}"
    );

    // Pause: a stopped sample. Every slot is still held, so the paused frame
    // waits for the next pool.
    send(
        &mut host,
        event(Event::Transport(TransportCommand::Pause), None),
    );
    inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Event {
            body: Event::Clock(c),
        } if c.state == TransportState::Stopped => Some(()),
        _ => None,
    });

    // An EDR display: a new generation in rgbaf16; the old one is retired.
    send(
        &mut host,
        event(Event::Viewer(state(3, true, [0.0, 0.0])), None),
    );
    let (pool2, _) = inbox.wait(Duration::from_secs(10), |m| match &m.message {
        Message::Event {
            body: Event::SurfacePool(p),
        } if p.generation.get() == 4 => Some(p.clone()),
        _ => None,
    });
    assert_eq!(pool2.surfaces[0].format, SurfaceFormat::Rgba16Float);
    assert_eq!(pool2.surfaces.len(), 4);
    let (hdr, _) = inbox.wait(Duration::from_secs(5), |m| {
        frame(m).filter(|d| d.pool_generation == pool2.generation)
    });
    assert_eq!(
        (hdr.view_seq.get(), hdr.format, hdr.duration),
        (3, SurfaceFormat::Rgba16Float, None)
    );
    release(&mut host, &hdr);

    // A scrub request is answered with its frame.
    let request = FrameRequest {
        composition: "main".into(),
        time: Ticks::new(30 * frame_ticks).unwrap(),
        purpose: FramePurpose::Scrub,
        priority: PriorityClass::Interactive,
        output_size: Size::new(1280, 720),
        region: None,
        scale: 1.0,
        resolution: PreviewResolution::Full,
        media: MediaPreference::Automatic,
        color: pm_protocol::color::OutputColor::Viewer {
            space: ViewerColorSpace::Srgb,
        },
        precision: pm_protocol::color::Precision::Float16,
        motion_blur: MotionBlurKind::PreviewPaused,
        view: None,
        draft_3d: true,
        draft_render: false,
        depth_id: false,
        overlay: true,
    };
    let mut envelope = Envelope::new(Message::Request {
        id: RequestId::new(9).unwrap(),
        body: Request::RenderFrame(request),
    })
    .with_session(session);
    envelope.revision = Some(Revision::new(6).unwrap());
    send(&mut host, envelope);
    let (scrub, _) = inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Response {
            id,
            body: Response::Frame(d),
        } if id.get() == 9 => Some(d.clone()),
        _ => None,
    });
    assert_eq!(
        (scrub.pts.get(), scrub.revision.get(), scrub.duration),
        (30 * frame_ticks, 6, None)
    );

    // Closing the session retires generation 4; dropping the engine says goodbye.
    let close = Envelope::new(Message::Request {
        id: RequestId::new(10).unwrap(),
        body: Request::CloseSession,
    })
    .with_session(session);
    send(&mut host, close);
    let status: serde_json::Value = serde_json::from_str(&host.status_json()).unwrap();
    let presenter = &status["presenter"];
    assert_eq!(presenter["checkedIn"], true);
    assert_eq!(presenter["pools"], 4);
    assert!(
        presenter["releaseMismatches"].as_u64().unwrap() == 0,
        "{presenter}"
    );
    if std::env::var(REMOTE_LAYER_ENV).as_deref() == Ok("1") && !cfg!(feature = "remote-layer") {
        assert_eq!(presenter["remoteFrames"], 0, "{presenter}");
        assert!(
            presenter["remoteError"]
                .as_str()
                .is_some_and(|e| e.contains("built without the C′ prototype")),
            "{presenter}"
        );
    }
    drop(host);
    println!("CHILD-OK frames={}", timed.len());
}
