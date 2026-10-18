//! The decision 19 shader fallback runner through the interactive engine,
//! without Electron (the parent plays main, a child plays the engine, as
//! `document.rs`): a shader layer whose only sampler is a `samplerCube`
//! (amendment 9: the bind check refuses it, WebGL draws it with
//! (0, 0, 0, 1)) is not routed: the engine asks the viewer's renderer for
//! its pixels (`ShaderFallbackMessage::Requests`), takes them back in row
//! bands, and presents the frame, reporting the layer to the Shader panel
//! as not headless-renderable (layer scope, at the frame's revision, before
//! the frame) and logging it once. A runner failure is reported and the
//! layer draws nothing, as WebGL draws a program it refuses; nothing is
//! routed to the legacy viewer (amendment 21). An effect only the runner
//! draws is asked for with its textures (input and backdrop, in row bands
//! after the request), and the frame draws the runner's pixels.
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
use pm_protocol::codec;
use pm_protocol::color::{ProjectColor, ViewerColorSpace};
use pm_protocol::common::{Bytes, ContentHash, RequestId, Revision, Seq, Size};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::{EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::frame::{
    FrameDescriptor, FrameRelease, MediaPreference, PreviewResolution, ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::patch::{
    DocumentEncoding, PatchBatch, PatchEncoding, PatchOrigin, Resync, ResyncSource,
};
use pm_protocol::shader_fallback::{ShaderFallbackMessage, ShaderFallbackPixels};
use pm_protocol::version::PeerRole;
use serde_json::{Value, json};

const CHILD_ENV: &str = "PM_SHADER_FALLBACK_TEST_CHILD";
const WINDOW: PortId = PortId(7);

#[test]
fn programs_only_the_runner_draws_take_its_pixels_through_the_engine() {
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
    main.join().unwrap();
}

#[derive(Default)]
struct Inbox {
    messages: Mutex<Vec<Envelope>>,
    logs: Mutex<Vec<String>>,
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
                Action::Log { level, message } => {
                    eprintln!("engine {level:?}: {message}");
                    self.logs.lock().unwrap().push(message);
                }
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

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn shader(code: &str) -> Value {
    json!({
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
    })
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
    }
}

#[test]
#[ignore = "run by programs_only_the_runner_draws_take_its_pixels_through_the_engine"]
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
    host.attach(1, "host").unwrap();
    host.receive(1, &hello(PeerRole::Host));
    host.attach(WINDOW.0, "renderer").unwrap();
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

    // A shader layer only the fallback runner draws.
    let code = "uniform samplerCube uEnv;\nvoid main(){ fragColor = vec4(texture(uEnv, vec3(uv, 1.0)).rgb + vec3(uv, 0.0), 1.0); }";
    let doc = project(shader(code));
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
    let requests = inbox.wait(Duration::from_secs(20), |m| match &m.message {
        Message::Event {
            body: Event::ShaderFallback(ShaderFallbackMessage::Requests { requests }),
        } => Some(requests.clone()),
        Message::Event {
            body: Event::Failure(f),
        } => panic!("expected a fallback request, got {:?}", f.error),
        _ => None,
    });
    assert_eq!(requests.len(), 1);
    let r = &requests[0];
    assert_eq!(r.layer, "sky");
    assert_eq!(r.stage, "bind");
    // The layer's 80 × 45 at the viewer's output scale (zoom 2, DPR 2).
    assert_eq!((r.width, r.height), (320, 180));
    // No frame until the pixels arrive.
    std::thread::sleep(Duration::from_millis(200));
    assert!(inbox.take().iter().filter_map(frame).next().is_none());
    // The runner's pixels in two bands (any order).
    let row = |v: u16| -> Vec<u8> {
        (0..r.width)
            .flat_map(|_| [v, v, v, 0x3c00])
            .flat_map(u16::to_le_bytes)
            .collect()
    };
    let band = |from: u32, rows: u32, v: u16| {
        event(Event::ShaderFallback(ShaderFallbackMessage::Pixels(
            ShaderFallbackPixels {
                key: r.key.clone(),
                width: r.width,
                height: r.height,
                row: from,
                rows,
                rgba16f: Bytes((0..rows).flat_map(|_| row(v)).collect()),
            },
        )))
    };
    send(&mut host, band(100, 80, 0x3800));
    send(&mut host, band(0, 100, 0x3400));
    // Reported (plan section 11: "reported and marked not
    // headless-renderable"), at the frame's revision, before the frame.
    let report = inbox.wait(Duration::from_secs(20), |m| match &m.message {
        Message::Event {
            body: Event::Failure(f),
        } => Some(f.clone()),
        Message::Event {
            body: Event::Frame(_),
        } => panic!("the frame came before the layer's report"),
        _ => None,
    });
    assert_eq!(report.revision.get(), 1);
    assert_eq!(report.error.code, ErrorCode::NotHeadlessRenderable);
    assert_eq!(
        report.error.scope,
        ErrorScope::Layer {
            layer: "sky".into()
        }
    );
    assert!(
        report.error.message.contains("WebGL fallback")
            && report.error.message.contains("headless exports"),
        "{}",
        report.error.message
    );
    let drawn = inbox.wait(Duration::from_secs(20), frame);
    assert_eq!(drawn.revision.get(), 1);
    let logged: Vec<String> = inbox
        .logs
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.contains("fallback runner") && m.contains("not headless-renderable"))
        .cloned()
        .collect();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(logged[0].starts_with("layer sky "), "{}", logged[0]);
    release(&mut host, &drawn);
    // A runner failure is reported and the layer draws nothing (another
    // program); the frame is presented, not routed.
    let doc2 = project(shader(&code.replace("1.0)", "0.5)")));
    let ops = pm_model::ops::diff_ops(Some(&model(&doc)), Some(&model(&doc2)));
    let body = pm_model::binary::encode(&ops).unwrap();
    send(
        &mut host,
        event(Event::ApplyPatches(PatchBatch {
            document: "Ptracer".into(),
            base_revision: Revision::new(1).unwrap(),
            revision: Revision::new(2).unwrap(),
            origin: PatchOrigin::Edit,
            encoding: PatchEncoding::CanonicalV1,
            patch_count: ops.len() as u32,
            body: Bytes(body),
        })),
    );
    let next = inbox.wait(Duration::from_secs(20), |m| match &m.message {
        Message::Event {
            body: Event::ShaderFallback(ShaderFallbackMessage::Requests { requests }),
        } => Some(requests[0].key.clone()),
        _ => None,
    });
    assert_ne!(next, r.key);
    send(
        &mut host,
        event(Event::ShaderFallback(ShaderFallbackMessage::Failed {
            key: next,
            reason: "compile".into(),
            message: "the program does not link".into(),
        })),
    );
    let mut reported = Vec::new();
    let refused = inbox.wait(Duration::from_secs(20), |m| match &m.message {
        Message::Event {
            body: Event::Failure(f),
        } => {
            assert_ne!(
                f.error.code,
                ErrorCode::Unsupported,
                "routed: {:?}",
                f.error
            );
            reported.push(f.error.clone());
            None
        }
        Message::Event {
            body: Event::Frame(d),
        } if d.revision.get() == 2 => Some(d.clone()),
        _ => None,
    });
    assert!(
        reported.iter().any(|e| e.code == ErrorCode::ShaderCompile
            && e.scope
                == ErrorScope::Layer {
                    layer: "sky".into()
                }
            && e.message.contains("does not link")),
        "{reported:?}"
    );
    release(&mut host, &refused);

    // An effect only the runner draws: asked for with its textures.
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
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(3).unwrap(),
            body: Request::PushDefinitions(pm_protocol::definitions::DefinitionsUpdate {
                generation: Seq::new(1).unwrap(),
                full: true,
                upserts: set.definitions,
                removals: vec![],
            }),
        })
        .with_session(session),
    );
    inbox.wait(Duration::from_secs(5), |m| match &m.message {
        Message::Response {
            body: Response::DefinitionsApplied(_),
            ..
        } => Some(()),
        _ => None,
    });
    let doc3 = project(solid(
        130.0,
        json!([{ "id": "fx1", "type": "acme.cube", "on": true, "p": {} }]),
    ));
    let ops = pm_model::ops::diff_ops(Some(&model(&doc2)), Some(&model(&doc3)));
    let body = pm_model::binary::encode(&ops).unwrap();
    send(
        &mut host,
        event(Event::ApplyPatches(PatchBatch {
            document: "Ptracer".into(),
            base_revision: Revision::new(2).unwrap(),
            revision: Revision::new(3).unwrap(),
            origin: PatchOrigin::Edit,
            encoding: PatchEncoding::CanonicalV1,
            patch_count: ops.len() as u32,
            body: Bytes(body),
        })),
    );
    let effect = inbox.wait(Duration::from_secs(20), |m| match &m.message {
        Message::Event {
            body: Event::ShaderFallback(ShaderFallbackMessage::Requests { requests }),
        } => Some(requests[0].clone()),
        Message::Event {
            body: Event::Failure(f),
        } if f.error.code == ErrorCode::Unsupported => panic!("routed: {:?}", f.error),
        _ => None,
    });
    assert_eq!((effect.layer.as_str(), effect.textures), ("box", 2));
    // Both textures follow in row bands covering every row.
    let mut rows = [0u32; 2];
    while rows.iter().any(|r| *r < effect.height) {
        let input = inbox.wait(Duration::from_secs(20), |m| match &m.message {
            Message::Event {
                body: Event::ShaderFallback(ShaderFallbackMessage::Input(i)),
            } => Some(i.clone()),
            _ => None,
        });
        assert_eq!(input.key, effect.key);
        assert_eq!((input.width, input.height), (effect.width, effect.height));
        rows[input.texture as usize] += input.rows;
    }
    assert_eq!(rows, [effect.height, effect.height]);
    let answer = |from: u32, rows: u32| {
        event(Event::ShaderFallback(ShaderFallbackMessage::Pixels(
            ShaderFallbackPixels {
                key: effect.key.clone(),
                width: effect.width,
                height: effect.height,
                row: from,
                rows,
                rgba16f: Bytes(
                    (0..rows * effect.width)
                        .flat_map(|_| [0x3800u16, 0, 0, 0x3c00])
                        .flat_map(u16::to_le_bytes)
                        .collect(),
                ),
            },
        )))
    };
    send(&mut host, answer(0, effect.height));
    let shown = inbox.wait(Duration::from_secs(20), |m| {
        frame(m).filter(|d| d.revision.get() == 3)
    });
    release(&mut host, &shown);
    println!("CHILD-OK");
}
