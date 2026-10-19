//! The shader UI's layer reports through the interactive engine, without
//! Electron (the parent plays main, a child plays the engine, as
//! `document.rs`): with the engine presenting, the WebGL compositor draws
//! nothing, so the engine reports what today's compositor fills into
//! `GL.errors` and `uiState.getShaderMeta` (plan section 11): a shader layer
//! whose code does not compile (`ShaderCompile`, layer scope, the author's
//! line numbers) and an extension layer whose definition is missing
//! (`DefinitionMissing`, layer scope), at the frame's revision and before
//! the frame; every revision presented repeats what still holds, and
//! nothing is repeated within one.
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
use pm_protocol::definitions::{DefinitionKind, DefinitionRemoval, DefinitionsUpdate};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorDetail, ErrorScope};
use pm_protocol::frame::{
    FrameDescriptor, FrameRelease, MediaPreference, PreviewResolution, ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::patch::{
    DocumentEncoding, PatchBatch, PatchEncoding, PatchOrigin, Resync, ResyncSource,
};
use pm_protocol::version::PeerRole;
use serde_json::{Value, json};

const CHILD_ENV: &str = "PM_LAYER_REPORTS_TEST_CHILD";
const WINDOW: PortId = PortId(7);

#[test]
fn shader_compile_errors_and_missing_layers_reach_the_shader_ui() {
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

fn extension(id: &str, definition: &str) -> Value {
    let mut layer = shader("");
    layer["id"] = json!(id);
    layer["name"] = json!(id);
    layer["type"] = json!("extension");
    layer["d"] = json!({ "definition": definition, "w": 80, "h": 45, "params": {} });
    layer
}

fn project(layers: Vec<Value>) -> Value {
    json!({
        "id": "Ptracer", "name": "tracer", "w": 320, "h": 180, "fps": 30, "dur": 2, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#202020", "position": 0 }] },
        "compId": "Ptracer-root", "compName": "tracer", "markers": [], "work": [0, 2], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
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
        depth_id: None,
    }
}

/// The failure reports up to (not including) the next frame, and the frame.
fn until_frame(inbox: &Inbox) -> (Vec<(u64, EngineError)>, FrameDescriptor) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut messages = inbox.messages.lock().unwrap();
    loop {
        if let Some(i) = messages.iter().position(|m| frame(m).is_some()) {
            let drained: Vec<Envelope> = messages.drain(..=i).collect();
            let reports = drained
                .iter()
                .filter_map(|m| match &m.message {
                    Message::Event {
                        body: Event::Failure(f),
                    } => Some((f.revision.get(), f.error.clone())),
                    _ => None,
                })
                .collect();
            return (reports, frame(drained.last().unwrap()).unwrap());
        }
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "timed out waiting for a frame");
        messages = inbox.arrived.wait_timeout(messages, left).unwrap().0;
    }
}

fn layer_of(error: &EngineError) -> &str {
    match &error.scope {
        ErrorScope::Layer { layer } => layer.as_str(),
        other => panic!("not a layer report: {other:?}"),
    }
}

#[test]
#[ignore = "run by shader_compile_errors_and_missing_layers_reach_the_shader_ui"]
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
    let push = |host: &mut EngineHost, id: u64, update: DefinitionsUpdate| {
        send(
            host,
            Envelope::new(Message::Request {
                id: RequestId::new(id).unwrap(),
                body: Request::PushDefinitions(update),
            })
            .with_session(session),
        );
        inbox.wait(Duration::from_secs(5), |m| match &m.message {
            Message::Response {
                body: Response::DefinitionsApplied(applied),
                ..
            } => Some(applied.generation),
            _ => None,
        })
    };

    // The kernel's set, then `acme.gone` disposed (marked missing).
    push(
        &mut host,
        2,
        DefinitionsUpdate {
            generation: Seq::new(1).unwrap(),
            full: true,
            upserts: vec![],
            removals: vec![],
        },
    );
    push(
        &mut host,
        5,
        DefinitionsUpdate {
            generation: Seq::new(2).unwrap(),
            full: false,
            upserts: vec![],
            removals: vec![DefinitionRemoval {
                id: "acme.gone".into(),
                kind: DefinitionKind::Layer,
            }],
        },
    );
    // A shader layer whose second line does not compile, and the missing
    // extension layer.
    let broken = "void main(){\n  fragColor = vec4(nope, 1.0);\n}";
    let layers = |code: &str| vec![shader(code), extension("gone", "acme.gone")];
    let doc = project(layers(broken));
    let text = model(&doc);
    send(
        &mut host,
        Envelope::new(Message::Request {
            id: RequestId::new(3).unwrap(),
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
    let (reports, first) = until_frame(&inbox);
    assert_eq!(first.revision.get(), 1, "the engine presents (not routed)");
    assert_eq!(reports.len(), 2, "{reports:#?}");
    assert!(reports.iter().all(|(r, _)| *r == 1));
    let compile = &reports
        .iter()
        .find(|(_, e)| layer_of(e) == "sky")
        .unwrap()
        .1;
    assert_eq!(compile.code, ErrorCode::ShaderCompile);
    assert!(
        compile
            .message
            .starts_with("Shader layer sky does not compile"),
        "{}",
        compile.message
    );
    let Some(ErrorDetail::ShaderCompile { diagnostics }) = &compile.detail else {
        panic!("no diagnostics: {compile:?}");
    };
    // The author's line: the header and the joining newline are removed.
    assert!(
        diagnostics
            .iter()
            .any(|d| d.line == 2 && d.message.contains("nope")),
        "{diagnostics:?}"
    );
    let missing = &reports
        .iter()
        .find(|(_, e)| layer_of(e) == "gone")
        .unwrap()
        .1;
    assert_eq!(missing.code, ErrorCode::DefinitionMissing);
    release(&mut host, &first);

    // Another frame at the same revision repeats nothing.
    send(&mut host, event(Event::Viewer(state(2))));
    let (reports, same) = until_frame(&inbox);
    assert_eq!(same.revision.get(), 1);
    assert!(reports.is_empty(), "{reports:#?}");
    release(&mut host, &same);

    // The code fixed at revision 2: only the missing layer is repeated.
    let fixed = "void main(){\n  fragColor = vec4(uv, 0.0, 1.0);\n}";
    let doc2 = project(layers(fixed));
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
    let (reports, second) = loop {
        let (reports, f) = until_frame(&inbox);
        if f.revision.get() == 2 {
            break (reports, f);
        }
        release(&mut host, &f);
    };
    let repeated: Vec<_> = reports
        .iter()
        .map(|(r, e)| (*r, layer_of(e), e.code))
        .collect();
    assert_eq!(repeated, vec![(2, "gone", ErrorCode::DefinitionMissing)]);
    release(&mut host, &second);

    // The definition arrives (same revision): nothing holds, nothing is sent.
    let mut cube = pm_render::shader_layers::builtin_layer_definitions()
        .into_iter()
        .find(|d| d.id.as_str() == "powermove.3d.studio-cube")
        .expect("Studio Cube");
    cube.id = "acme.gone".into();
    push(
        &mut host,
        4,
        DefinitionsUpdate {
            generation: Seq::new(3).unwrap(),
            full: false,
            upserts: vec![cube],
            removals: vec![],
        },
    );
    let (reports, third) = until_frame(&inbox);
    assert_eq!(third.revision.get(), 2);
    assert!(reports.is_empty(), "{reports:#?}");
    release(&mut host, &third);
    println!("CHILD-OK");
}
