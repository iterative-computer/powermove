//! The job engine's render and export service end to end, without Electron:
//! this test plays host services (main) over the protocol — handles for media
//! and outputs, `open(wx, 0600)` temporary files, commits with unique names —
//! and drives a job engine `EngineHost` with its lanes, as the utility
//! process does (`onActions` → `pump`).
#![cfg(target_os = "macos")]
// The fake host answers with protocol errors by value, as main does.
#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pm_engine::Action;
use pm_engine::engine::testing::hello;
use pm_node::EngineHost;
use pm_protocol::codec;
use pm_protocol::color::{OutputColor, Precision, ProjectColor, QuickTimeTag};
use pm_protocol::common::{
    Bytes, ContentHash, FrameRate, HostMicros, JobId, PlaybackRate, RequestId, Revision, Seq,
    SessionId, Size, TICKS_PER_SECOND, TickRange, Ticks,
};
use pm_protocol::envelope::{
    Envelope, Event, Message, OpenSession, Request, Response, SessionRole,
};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::host::{
    CacheKind, CapabilityHandle, CapabilityKind, HandleAccess, HandleGrant, HandleResolved,
    OutputChoice, OutputCommitted, OutputDestination, PortId,
};
use pm_protocol::patch::{DocumentEncoding, Resync, ResyncSource};
use pm_protocol::priority::*;
use pm_protocol::slicing::DeadlineSignal;
use pm_protocol::version::PeerRole;
use serde_json::{Value, json};

const HOST: PortId = PortId(1);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn fixture(name: &str) -> PathBuf {
    repo().join("apps/desktop/e2e/fixtures").join(name)
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn layer(id: &str, ty: &str, d: Value, x: f64, y: f64, dur: f64) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in [
        ("perspective", 50.0),
        ("position.z", 0.0),
        ("anchor.z", 0.0),
        ("scale.z", 100.0),
        ("rotation.x", 0.0),
        ("rotation.y", 0.0),
        ("orientation.x", 0.0),
        ("orientation.y", 0.0),
        ("orientation.z", 0.0),
        ("anchor.x", 0.0),
        ("anchor.y", 0.0),
        ("position.x", x),
        ("position.y", y),
        ("scale.x", 100.0),
        ("scale.y", 100.0),
        ("rotation", 0.0),
        ("opacity", 100.0),
        ("skew", 0.0),
    ] {
        p.insert(k.into(), ch(json!(v)));
    }
    json!({
        "id": id, "type": ty, "name": id, "from": 0, "dur": dur, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": p, "fx": [], "transitionIn": null, "transitionOut": null,
        "masks": [], "d": d, "locked_intent": {}
    })
}

/// A 320 × 180 project: a solid, a PNG, an H.264 clip with sound, a WAV tone.
fn project(extra_text_layer: bool) -> Value {
    let dur = 1.0;
    let mut layers = vec![];
    if extra_text_layer {
        layers.push(layer(
            "t",
            "text",
            json!({ "text": "Routed" }),
            10.0,
            10.0,
            dur,
        ));
    }
    layers.push(layer(
        "img",
        "image",
        json!({ "asset": "still", "fit": "contain", "w": 80, "h": 45 }),
        250.0,
        40.0,
        dur,
    ));
    layers.push(layer(
        "vid",
        "video",
        json!({ "asset": "clip", "fit": "cover", "w": 160, "h": 90, "trim": 0, "speed": 1, "embeddedAudio": true }),
        100.0,
        100.0,
        dur,
    ));
    let mut tone = layer(
        "tone",
        "audio",
        json!({ "asset": "tone", "gain": 1, "trim": 0, "fadeIn": 0, "fadeOut": 0 }),
        0.0,
        0.0,
        dur,
    );
    tone["from"] = json!(0.25);
    layers.push(tone);
    layers.push(layer(
        "back",
        "solid",
        json!({ "color": "#E83A5E", "w": 320, "h": 180 }),
        0.0,
        0.0,
        dur,
    ));
    let asset = |id: &str, kind: &str, format: &str| json!({ "id": id, "name": id, "kind": kind, "format": format, "w": 320, "h": 180, "dur": 2, "fps": 30, "persisted": true });
    json!({
        "id": "Ptest", "name": "test", "w": 320, "h": 180, "fps": 30, "dur": dur, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#14161C", "position": 0 }] },
        "compId": "Ptest-root", "compName": "test", "markers": [], "work": [0, dur], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": layers,
        "assets": {
            "still": asset("still", "image", "png"),
            "clip": asset("clip", "video", "mp4"),
            "tone": asset("tone", "audio", "wav"),
        },
        "comps": {}
    })
}

/// Host services as main serves them, plus everything the engine sent.
struct Host {
    engine: EngineHost,
    pending: Arc<(Mutex<bool>, Condvar)>,
    dir: PathBuf,
    media: HashMap<String, PathBuf>,
    handles: HashMap<CapabilityHandle, (CapabilityKind, PathBuf)>,
    outputs: HashMap<CapabilityHandle, (JobId, String)>,
    next_handle: u8,
    next_id: u64,
    answers: BTreeMap<u64, Message>,
    statuses: Vec<JobStatus>,
    host_requests: Vec<&'static str>,
    /// The editor window's answer to `compatFrame` asks (level 1 SVG
    /// rasters): a fill colour, or the host's error words.
    svg_window: Result<[u8; 4], String>,
    compat_asks: Vec<pm_protocol::snapshot::CompatFrameRequest>,
}

impl Host {
    fn new(name: &str, env: &[(&str, &str)]) -> Host {
        let pending = Arc::new((Mutex::new(false), Condvar::new()));
        let p2 = pending.clone();
        let sink: pm_node::ActionSink = Arc::new(move |actions: Vec<Action>| {
            // The service only wakes the host; it never sends directly.
            assert!(actions.is_empty(), "{actions:?}");
            *p2.0.lock().unwrap() = true;
            p2.1.notify_all();
        });
        let env: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut engine =
            EngineHost::with_sink(EngineProcess::Job, Some(sink), |k| env.get(k).cloned()).unwrap();
        engine.attach(HOST.0, "host").unwrap();
        let out = engine.receive(HOST.0, &hello(PeerRole::Host));
        let dir = std::env::temp_dir().join(format!("pm-node-jobs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut host = Host {
            engine,
            pending,
            dir,
            media: [
                ("still", fixture("still-alpha.png")),
                ("clip", fixture("h264-aac.mp4")),
                ("tone", fixture("tone.wav")),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            handles: HashMap::new(),
            outputs: HashMap::new(),
            next_handle: 1,
            next_id: 100,
            answers: BTreeMap::new(),
            statuses: vec![],
            host_requests: vec![],
            svg_window: Err("This layer is drawn by an extension in the editor; open the project in a window to export it.".into()),
            compat_asks: vec![],
        };
        host.handle(out);
        host
    }

    fn mint(&mut self, kind: CapabilityKind, path: PathBuf) -> CapabilityHandle {
        let h = CapabilityHandle::from_random_bytes([self.next_handle; 16]);
        self.next_handle += 1;
        self.handles.insert(h.clone(), (kind, path));
        h
    }

    fn send_request(
        &mut self,
        body: Request,
        session: Option<SessionId>,
        revision: Option<u64>,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let mut envelope = Envelope::new(Message::Request {
            id: RequestId::new(id).unwrap(),
            body,
        });
        envelope.session = session;
        envelope.revision = revision.map(|r| Revision::new(r).unwrap());
        let out = self
            .engine
            .receive(HOST.0, &codec::encode(&envelope).unwrap());
        self.handle(out);
        id
    }

    fn send_event(&mut self, body: Event) {
        let out = self.engine.receive(
            HOST.0,
            &codec::encode(&Envelope::new(Message::Event { body })).unwrap(),
        );
        self.handle(out);
    }

    fn reply(&mut self, id: RequestId, answer: Result<Response, EngineError>) {
        let message = match answer {
            Ok(body) => Message::Response { id, body },
            Err(error) => Message::Error { id, error },
        };
        let out = self
            .engine
            .receive(HOST.0, &codec::encode(&Envelope::new(message)).unwrap());
        self.handle(out);
    }

    /// Serves what the engine sent to the host port.
    fn handle(&mut self, actions: Vec<Action>) {
        for action in actions {
            let Action::Send { port, bytes } = action else {
                continue;
            };
            assert_eq!(port, HOST);
            let envelope = codec::decode(&bytes).unwrap();
            match envelope.message {
                Message::Request { id, body } => {
                    let answer = self.serve(body, envelope.session);
                    self.reply(id, answer);
                }
                Message::Response { id, .. } | Message::Error { id, .. } => {
                    self.answers.insert(id.get(), envelope.message);
                }
                Message::Event {
                    body: Event::JobStatus(status),
                } => self.statuses.push(status),
                _ => {}
            }
        }
    }

    fn serve(
        &mut self,
        body: Request,
        session: Option<SessionId>,
    ) -> Result<Response, EngineError> {
        let err = |code, m: &str| Err(EngineError::new(code, ErrorScope::Connection, m));
        match body {
            Request::MintHandle(m) => {
                self.host_requests.push("mintHandle");
                if let CapabilityKind::Cache {
                    cache: CacheKind::Scratch,
                } = &m.kind
                {
                    let path = self.dir.join("scratch");
                    std::fs::create_dir_all(&path).unwrap();
                    let handle = self.mint(m.kind.clone(), path);
                    return Ok(Response::Handle(HandleGrant {
                        handle,
                        kind: m.kind,
                        content_hash: None,
                    }));
                }
                let CapabilityKind::MediaRead { asset } = &m.kind else {
                    return err(ErrorCode::Unsupported, "only media");
                };
                let Some(path) = self.media.get(asset.as_str()).cloned() else {
                    return err(ErrorCode::MediaOffline, "offline");
                };
                let handle = self.mint(m.kind.clone(), path);
                Ok(Response::Handle(HandleGrant {
                    handle,
                    kind: m.kind,
                    content_hash: None,
                }))
            }
            Request::ResolveHandle(r) => {
                self.host_requests.push("resolveHandle");
                let Some((kind, path)) = self.handles.get(&r.handle).cloned() else {
                    return err(ErrorCode::CapabilityRejected, "unknown");
                };
                let access = match kind {
                    CapabilityKind::OutputWrite { .. } => HandleAccess::Write,
                    CapabilityKind::Cache { .. } => HandleAccess::ReadWrite,
                    _ => HandleAccess::Read,
                };
                Ok(Response::HandleResolved(HandleResolved {
                    handle: r.handle,
                    kind,
                    access,
                    path: path.to_string_lossy().into_owned(),
                }))
            }
            Request::OutputDestination(d) => {
                self.host_requests.push("outputDestination");
                assert!(session.is_some(), "outputs are bound to the job's session");
                let OutputChoice::AgentAutomatic {
                    suggested_name,
                    extension,
                } = &d.choice
                else {
                    return err(ErrorCode::Unsupported, "agent outputs only here");
                };
                assert_eq!(d.overwrite, pm_protocol::host::OverwritePolicy::UniqueName);
                let temp = self
                    .dir
                    .join(format!(".powermove-{:016x}.part", self.next_handle));
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&temp)
                        .unwrap();
                }
                let handle = self.mint(CapabilityKind::OutputWrite { job: d.job.clone() }, temp);
                self.outputs.insert(
                    handle.clone(),
                    (d.job.clone(), format!("{suggested_name}.{extension}")),
                );
                Ok(Response::OutputDestination(OutputDestination {
                    job: d.job,
                    temporary: handle,
                }))
            }
            Request::CommitOutput(c) => {
                self.host_requests.push("commitOutput");
                let (_, temp) = self.handles.remove(&c.temporary).unwrap();
                let (_, name) = self.outputs.remove(&c.temporary).unwrap();
                let size = std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
                if !c.commit
                    || size == 0
                    || c.verification.as_ref().map(|v| v.bytes.get()) != Some(size)
                {
                    let _ = std::fs::remove_file(&temp);
                    return Ok(Response::OutputCommitted(OutputCommitted {
                        job: c.job,
                        path: None,
                    }));
                }
                // Unique names that never replace anything: hard links.
                for attempt in 0.. {
                    let candidate = if attempt == 0 {
                        self.dir.join(&name)
                    } else {
                        let (stem, ext) = name.rsplit_once('.').unwrap();
                        self.dir.join(format!("{stem} ({}).{ext}", attempt + 1))
                    };
                    if std::fs::hard_link(&temp, &candidate).is_ok() {
                        std::fs::remove_file(&temp).unwrap();
                        return Ok(Response::OutputCommitted(OutputCommitted {
                            job: c.job,
                            path: Some(candidate.to_string_lossy().into_owned()),
                        }));
                    }
                }
                unreachable!()
            }
            // A window's fallback runner (main relays it): every texel
            // blue, after checking the textures arrived as files.
            Request::ShaderFallback(f) => {
                self.host_requests.push("shaderFallback");
                assert!(session.is_some(), "bound to the job's session");
                let Some((_, area)) = self.handles.get(&f.area).cloned() else {
                    return err(ErrorCode::CapabilityRejected, "unknown area");
                };
                let r = &f.request;
                let bytes = u64::from(r.width) * u64::from(r.height) * 8;
                for name in &f.textures {
                    assert_eq!(
                        std::fs::metadata(area.join(name)).unwrap().len(),
                        bytes,
                        "{name}"
                    );
                }
                let out: Vec<u8> = (0..r.width * r.height)
                    .flat_map(|_| [0u16, 0, 0x3c00, 0x3c00])
                    .flat_map(u16::to_le_bytes)
                    .collect();
                std::fs::write(area.join(&f.output), out).unwrap();
                Ok(Response::Ack)
            }
            Request::CompatFrame(ask) => {
                self.host_requests.push("compatFrame");
                self.compat_asks.push(ask.clone());
                let svg = ask.svg.clone().expect("an SVG raster");
                let fill = match &self.svg_window {
                    Ok(fill) => *fill,
                    Err(words) => return err(ErrorCode::NotHeadlessRenderable, words),
                };
                let k = svg.key;
                Ok(Response::CompatFrame(Box::new(
                    pm_protocol::snapshot::CompatFrame {
                        key: format!("{}:{}x{}", k.content, k.width, k.height),
                        width: k.width,
                        height: k.height,
                        format: pm_protocol::snapshot::CompatFormat::Rgba8Premultiplied,
                        origin: [0.0, 0.0],
                        scale: 1.0,
                        payload: pm_protocol::snapshot::PixelPayload::Inline {
                            bytes: Bytes(fill.repeat((k.width * k.height) as usize)),
                        },
                    },
                )))
            }
            other => err(ErrorCode::InvalidMessage, &format!("unexpected {other:?}")),
        }
    }

    /// Pumps the engine until `done` holds or `timeout` passes.
    fn run_until(&mut self, timeout: Duration, done: impl FnMut(&Host) -> bool) -> bool {
        self.run_until_with(timeout, |_| {}, done)
    }

    /// As `run_until`, calling `each` at least every 10 ms.
    fn run_until_with(
        &mut self,
        timeout: Duration,
        mut each: impl FnMut(&mut Host),
        mut done: impl FnMut(&Host) -> bool,
    ) -> bool {
        let end = Instant::now() + timeout;
        loop {
            each(self);
            if done(self) {
                return true;
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            {
                let (lock, cv) = &*self.pending;
                let mut woken = lock.lock().unwrap();
                if !*woken {
                    woken = cv
                        .wait_timeout(woken, left.min(Duration::from_millis(10)))
                        .unwrap()
                        .0;
                }
                *woken = false;
            }
            let out = self.engine.pump();
            self.handle(out);
            let out = self.engine.tick();
            self.handle(out);
        }
    }

    fn snapshot(&mut self, doc: &Value, role: SessionRole, revision: u64) -> SessionId {
        let id = self.send_request(
            Request::OpenSession(OpenSession {
                document: "doc".into(),
                role,
                process: EngineProcess::Job,
                color: ProjectColor::DEFAULT,
            }),
            None,
            None,
        );
        let Some(Message::Response {
            body: Response::SessionOpened { session },
            ..
        }) = self.answers.remove(&id)
        else {
            panic!("no session")
        };
        self.resync(session, doc, revision);
        session
    }

    /// Replaces `session`'s document at `revision`.
    fn resync(&mut self, session: SessionId, doc: &Value, revision: u64) {
        let text = doc.to_string();
        let value = pm_model::json::parse(&text).unwrap();
        let id = self.send_request(
            Request::Resync(Resync {
                document: "doc".into(),
                revision: Revision::new(revision).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(text.into_bytes()),
                },
                hash: ContentHash(pm_model::hash::canonical_digest(&value)),
            }),
            Some(session),
            None,
        );
        assert!(matches!(
            self.answers.remove(&id),
            Some(Message::Response { .. })
        ));
    }

    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn settings(format: ExportFormat) -> ExportSettings {
    ExportSettings {
        format,
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
        dither: matches!(format, ExportFormat::H264),
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}

fn agent_export(
    job: &str,
    run: &str,
    session: SessionId,
    format: ExportFormat,
    ext: &str,
) -> SubmitJob {
    SubmitJob {
        job: job.into(),
        class: PriorityClass::AgentExport,
        run: Some(run.into()),
        spec: JobSpec::Export {
            session,
            composition: "Ptest-root".into(),
            settings: settings(format),
        },
        output: Some(OutputChoice::AgentAutomatic {
            suggested_name: "Launch cut".into(),
            extension: ext.into(),
        }),
    }
}

fn terminal(host: &Host, job: &str) -> Option<JobStatus> {
    host.statuses
        .iter()
        .rev()
        .find(|s| s.job.as_str() == job && s.state.is_terminal())
        .cloned()
}

#[test]
fn agent_exports_render_encode_verify_and_commit_through_host_services() {
    let mut host = Host::new("export", &[]);
    let sid = host.snapshot(&project(false), SessionRole::JobSnapshot, 3);
    let id = host.send_request(
        Request::SubmitJob(agent_export(
            "job-a",
            "run-1",
            sid,
            ExportFormat::H264,
            "mp4",
        )),
        None,
        None,
    );
    assert!(matches!(
        host.answers.get(&id),
        Some(Message::Response {
            body: Response::JobAccepted(_),
            ..
        })
    ));
    // A second export of the same name gets a unique name from the host.
    host.send_request(
        Request::SubmitJob(agent_export(
            "job-b",
            "run-1",
            sid,
            ExportFormat::ProRes {
                profile: ProResProfile::Hq,
            },
            "mp4",
        )),
        None,
        None,
    );
    assert!(
        host.run_until(Duration::from_secs(120), |h| terminal(h, "job-a").is_some()
            && terminal(h, "job-b").is_some())
    );
    let a = terminal(&host, "job-a").unwrap();
    let b = terminal(&host, "job-b").unwrap();
    assert_eq!(a.state, JobState::Done, "{a:?}");
    assert_eq!(b.state, JobState::Done, "{b:?}");
    let pa = PathBuf::from(a.path.unwrap());
    let pb = PathBuf::from(b.path.unwrap());
    assert_ne!(pa, pb);
    let names = host.files();
    assert_eq!(
        names,
        vec![
            "Launch cut (2).mp4".to_string(),
            "Launch cut.mp4".to_string()
        ],
        "{names:?}"
    );
    for p in [&pa, &pb] {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(p).unwrap();
        assert!(meta.len() > 1000);
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
    // Running → progress → done, in order, never backwards.
    let a_states: Vec<_> = host
        .statuses
        .iter()
        .filter(|s| s.job.as_str() == "job-a")
        .collect();
    assert_eq!(a_states[0].state, JobState::Running);
    assert!(a_states.windows(2).all(|w| w[0].progress <= w[1].progress));
    assert!(
        host.host_requests.contains(&"mintHandle") && host.host_requests.contains(&"commitOutput")
    );
}

#[test]
fn unsupported_exports_fail_as_unsupported_without_an_output() {
    let mut host = Host::new("route", &[]);
    let sid = host.snapshot(&project(true), SessionRole::JobSnapshot, 1);
    host.send_request(
        Request::SubmitJob(agent_export(
            "job-r",
            "run-2",
            sid,
            ExportFormat::H264,
            "mp4",
        )),
        None,
        None,
    );
    assert!(host.run_until(Duration::from_secs(60), |h| terminal(h, "job-r").is_some()));
    let status = terminal(&host, "job-r").unwrap();
    assert_eq!(status.state, JobState::Failed);
    assert_eq!(status.error.unwrap().code, ErrorCode::Unsupported);
    assert!(
        !host.host_requests.contains(&"outputDestination"),
        "routing happens before any output"
    );
    assert!(host.files().is_empty());
}

#[test]
fn cancelling_the_run_stops_its_export_and_removes_the_temporary_file() {
    let mut host = Host::new("cancel", &[]);
    let mut doc = project(false);
    doc["dur"] = json!(20.0);
    let sid = host.snapshot(&doc, SessionRole::JobSnapshot, 1);
    let mut job = agent_export(
        "job-c",
        "run-3",
        sid,
        ExportFormat::ProRes {
            profile: ProResProfile::P4444Xq,
        },
        "mov",
    );
    if let JobSpec::Export { settings, .. } = &mut job.spec {
        settings.range.end = Ticks::from_whole_seconds(20);
    }
    host.send_request(Request::SubmitJob(job), None, None);
    assert!(host.run_until(Duration::from_secs(60), |h| {
        h.statuses
            .iter()
            .any(|s| s.job.as_str() == "job-c" && s.progress > 0.0)
    }));
    let id = host.send_request(
        Request::CancelJobs(CancelJobs::Run {
            run: "run-3".into(),
        }),
        None,
        None,
    );
    match host.answers.get(&id) {
        Some(Message::Response {
            body: Response::JobsCancelled(c),
            ..
        }) => assert_eq!(c.jobs, vec![JobId::from("job-c")]),
        other => panic!("{other:?}"),
    }
    assert!(host.run_until(Duration::from_secs(60), |h| terminal(h, "job-c").is_some()));
    assert_eq!(terminal(&host, "job-c").unwrap().state, JobState::Cancelled);
    assert!(host.files().is_empty(), "{:?}", host.files());
}

fn frames_request(n: usize) -> AgentFrameRequest {
    AgentFrameRequest::render_frames_default(
        "run-f".into(),
        (0..n)
            .map(|i| AgentFrame {
                composition: "Ptest-root".into(),
                time: Ticks::new(i as i64 * TICKS_PER_SECOND / 5).unwrap(),
            })
            .collect(),
    )
}

fn frame_images(message: &Message) -> Vec<(u32, u32, Vec<u8>)> {
    let Message::Response {
        body: Response::AgentFrames(result),
        ..
    } = message
    else {
        panic!("{message:?}")
    };
    result
        .outcomes
        .iter()
        .map(|o| match o {
            AgentFrameOutcome::Rendered {
                width,
                height,
                image: ImagePayload::Inline { bytes },
                ..
            } => (*width, *height, bytes.0.clone()),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn agent_frames_answer_with_jpeg_images_at_the_agent_revision() {
    let mut host = Host::new("frames", &[]);
    let sid = host.snapshot(&project(false), SessionRole::Job, 5);
    let id = host.send_request(Request::AgentFrames(frames_request(4)), Some(sid), Some(5));
    assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    let images = frame_images(&host.answers[&id]);
    assert_eq!(images.len(), 4);
    for (w, h, jpeg) in &images {
        assert_eq!((*w, *h), (320, 180), "never enlarged past the composition");
        let decoded = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (320, 180));
        // The red background solid shows in the top-left corner.
        let px = decoded.to_rgb8().get_pixel(2, 2).0;
        assert!(px[0] > 180 && px[1] < 100, "{px:?}");
    }
    // A smaller longest edge, and an unsupported composition fails per frame.
    let mut small = frames_request(1);
    small.max_dimension = 160;
    let id = host.send_request(Request::AgentFrames(small), Some(sid), Some(5));
    assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    assert_eq!(frame_images(&host.answers[&id])[0].0, 160);
    let routed = host.snapshot(&project(true), SessionRole::Job, 1);
    let id = host.send_request(
        Request::AgentFrames(frames_request(1)),
        Some(routed),
        Some(1),
    );
    assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    let Message::Response {
        body: Response::AgentFrames(result),
        ..
    } = &host.answers[&id]
    else {
        panic!()
    };
    assert!(matches!(
        &result.outcomes[0],
        AgentFrameOutcome::Failed { error, .. } if error.code == ErrorCode::Unsupported
    ));
}

fn at_now() -> HostMicros {
    pm_engine::clock::host_now()
}

/// While the interactive engine plays, agent frames still render, sliced
/// into bands under the 2 ms bound, aligned to the gap after each frame.
#[test]
fn agent_frames_during_playback_are_sliced_and_still_answer() {
    let mut host = Host::new("playback", &[]);
    let sid = host.snapshot(&project(false), SessionRole::Job, 1);
    // Calibrate the pass while idle, as a first agent turn would (its first
    // frame is the cold one).
    let id = host.send_request(Request::AgentFrames(frames_request(2)), Some(sid), Some(1));
    assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    host.send_event(Event::Deadline(DeadlineSignal::PlaybackStarted {
        at: at_now(),
        rate: PlaybackRate::NORMAL,
        frame_budget_us: 16_666,
    }));
    let id = host.send_request(Request::AgentFrames(frames_request(4)), Some(sid), Some(1));
    // A 60 Hz interactive engine reporting each frame on the direct port.
    let mut seq = 0u64;
    let mut last = Instant::now();
    let answered = host.run_until_with(
        Duration::from_secs(20),
        |h| {
            if last.elapsed() >= Duration::from_micros(16_666) {
                last = Instant::now();
                seq += 1;
                let now = at_now();
                h.send_event(Event::Deadline(DeadlineSignal::FrameDone {
                    at: now,
                    frame_seq: Seq::new(seq).unwrap(),
                    tick: now,
                    next_submit: HostMicros::new(now.get() + 12_000).unwrap(),
                    late: false,
                }));
            }
        },
        |h| h.answers.contains_key(&id),
    );
    let status: Value = serde_json::from_str(&host.engine.status_json()).unwrap();
    let gate = status["jobs"]["gate"].clone();
    eprintln!("gate during playback: {gate}");
    if cfg!(debug_assertions) && !answered {
        // Unoptimized builds measure a 16-row band above the 2 ms bound
        // (wgpu's own processing is ten times slower): the batch waits for
        // playback to stop rather than risk a dropped frame.
        assert!(
            gate["lastUnitTooLargeUs"].as_f64().unwrap() > 2000.0,
            "{gate}"
        );
        host.send_event(Event::Deadline(DeadlineSignal::PlaybackStopped {
            at: at_now(),
        }));
        assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
        assert_eq!(frame_images(&host.answers[&id]).len(), 4);
        return;
    }
    assert!(answered, "agent frames answer during playback");
    assert_eq!(frame_images(&host.answers[&id]).len(), 4);
    assert!(gate["maxPlaybackUs"].as_f64().unwrap() <= 2000.0, "{gate}");
    // Every frame's submissions were sliced: a frame whose bands all fit the
    // 1 ms target goes in one command buffer, others in several.
    assert!(gate["duringPlayback"].as_u64().unwrap() >= 4, "{gate}");
    assert!(gate["slicedSubmissions"].as_u64().unwrap() >= 4, "{gate}");
    host.send_event(Event::Deadline(DeadlineSignal::PlaybackStopped {
        at: at_now(),
    }));
}

fn frame_outcome_codes(message: &Message) -> Vec<Option<ErrorCode>> {
    let Message::Response {
        body: Response::AgentFrames(result),
        ..
    } = message
    else {
        panic!("{message:?}")
    };
    result
        .outcomes
        .iter()
        .map(|o| match o {
            AgentFrameOutcome::Failed { error, .. } => Some(error.code),
            _ => None,
        })
        .collect()
}

/// Batches the engine cannot render never hold the GPU gate: a text layer
/// is routed before any GPU work, an offline clip fails its frames after
/// rendering, and the supported batch after each one answers at once, as an
/// export after them does.
#[test]
fn unsupported_agent_frames_never_block_the_batches_after_them() {
    let mut host = Host::new("unblock", &[]);
    let good = host.snapshot(&project(false), SessionRole::Job, 1);
    let text = host.snapshot(&project(true), SessionRole::Job, 2);
    let mut doc = project(false);
    doc["layers"][1]["d"]["asset"] = json!("gone");
    doc["assets"]["gone"] = doc["assets"]["clip"].clone();
    doc["assets"]["gone"]["id"] = json!("gone");
    let offline = host.snapshot(&doc, SessionRole::Job, 3);
    for (session, revision) in [(text, 2), (offline, 3)] {
        let id = host.send_request(
            Request::AgentFrames(frames_request(2)),
            Some(session),
            Some(revision),
        );
        assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
        let codes = frame_outcome_codes(&host.answers[&id]);
        assert!(
            codes
                .iter()
                .all(|c| c.is_some_and(|c| c != ErrorCode::Cancelled)),
            "{codes:?}"
        );
        let started = Instant::now();
        let id = host.send_request(Request::AgentFrames(frames_request(2)), Some(good), Some(1));
        assert!(
            host.run_until(Duration::from_secs(20), |h| h.answers.contains_key(&id)),
            "the supported batch after an unsupported one answers"
        );
        assert_eq!(frame_images(&host.answers[&id]).len(), 2);
        eprintln!(
            "supported batch after the failed one: {:?}",
            started.elapsed()
        );
    }
    // An export after them still runs to the end.
    let snapshot = host.snapshot(&project(false), SessionRole::JobSnapshot, 4);
    host.send_request(
        Request::SubmitJob(agent_export(
            "job-after",
            "run-u",
            snapshot,
            ExportFormat::H264,
            "mp4",
        )),
        None,
        None,
    );
    assert!(host.run_until(Duration::from_secs(120), |h| {
        terminal(h, "job-after").is_some()
    }));
    assert_eq!(terminal(&host, "job-after").unwrap().state, JobState::Done);
}

/// A render that fails after the gate admitted it (a frame nothing routed
/// first) releases the admission on the shared gate, idle and while the
/// interactive engine plays, so every lane after it is admitted at once.
#[test]
fn a_render_that_fails_after_admission_releases_the_shared_gate() {
    use pm_export::render::{GpuGate, JobError, MediaPaths, render_frame};
    use pm_node::jobs::gate::{Gate, LaneGate, SNAPSHOT_LANE, USER_LANE};
    use pm_render::{RenderRequest, Renderer};
    use std::sync::atomic::AtomicBool;

    let mut renderer = Renderer::headless().unwrap();
    let mut doc = project(true);
    // Only the text layer and the solid: nothing the host must resolve.
    doc["layers"] = json!([doc["layers"][0].clone(), doc["layers"][4].clone()]);
    let text = pm_model::json::parse(&doc.to_string()).unwrap();
    let text = pm_model::Document::new(Some(text), 1);
    let request = RenderRequest::sized(Size::new(320, 180), Precision::Float16);
    let gate = Arc::new(Gate::new());
    let snap = LaneGate {
        gate: gate.clone(),
        lane: SNAPSHOT_LANE,
    };
    let user = LaneGate {
        gate: gate.clone(),
        lane: USER_LANE,
    };
    let no = AtomicBool::new(false);
    for playing in [false, true] {
        if playing {
            gate.signal(&DeadlineSignal::PlaybackStarted {
                at: at_now(),
                rate: PlaybackRate::NORMAL,
                frame_budget_us: 16_666,
            });
        }
        let failed = render_frame(
            &mut renderer,
            &text,
            "Ptest-root",
            Ticks::ZERO,
            &request,
            &MediaPaths::default(),
            &snap,
            42,
            &no,
        );
        assert!(
            matches!(failed, Err(JobError::Unsupported(_))),
            "{:?}",
            failed.err()
        );
        let started = Instant::now();
        let granted = user.admit(7, 12, &no);
        assert!(granted.is_some());
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "admitted after {:?} (playing {playing})",
            started.elapsed()
        );
        user.finished(7, granted.unwrap(), 50.0);
    }
    assert_eq!(gate.stats().abandoned, 2);
}

/// A relinked asset (its record changed) is resolved again within the same
/// session; an unchanged one is not.
#[test]
fn a_relinked_asset_is_resolved_again_in_the_same_session() {
    let mut host = Host::new("relink", &[]);
    let doc = project(false);
    let sid = host.snapshot(&doc, SessionRole::Job, 1);
    let mints = |h: &Host| {
        h.host_requests
            .iter()
            .filter(|r| **r == "mintHandle")
            .count()
    };
    let id = host.send_request(Request::AgentFrames(frames_request(1)), Some(sid), Some(1));
    assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    let first = mints(&host);
    assert_eq!(first, 3, "still, clip, tone");
    // The same document at a new revision: nothing is resolved again.
    host.resync(sid, &doc, 2);
    let id = host.send_request(Request::AgentFrames(frames_request(1)), Some(sid), Some(2));
    assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    assert_eq!(mints(&host), first);
    // Relink the still to another file: only it is resolved again, and the
    // frame shows the new file.
    let mut relinked = doc.clone();
    relinked["assets"]["still"]["name"] = json!("still-relinked.png");
    let before = frame_images(&host.answers[&id]).remove(0).2;
    host.media.insert("still".into(), fixture("still-red.png"));
    host.resync(sid, &relinked, 3);
    let id = host.send_request(Request::AgentFrames(frames_request(1)), Some(sid), Some(3));
    assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    assert_eq!(mints(&host), first + 1);
    let after = frame_images(&host.answers[&id]).remove(0).2;
    // The image layer (80 × 45 around 250, 40) shows the other file.
    let pixels = |jpeg: &[u8]| {
        image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .unwrap()
            .to_rgb8()
    };
    let (a, b) = (pixels(&before), pixels(&after));
    let changed = (215..285)
        .flat_map(|x| (22..58).map(move |y| (x, y)))
        .map(|(x, y)| {
            let (p, q) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
            (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap()
        })
        .max()
        .unwrap();
    assert!(
        changed > 40,
        "the relinked still did not change the frame ({changed})"
    );
}

/// Interpret Footage's facts: the host mints a `mediaRead` handle for the
/// job engine (session 0) and asks `probeMedia`; the engine resolves the
/// handle through the host and answers off its thread. An unknown handle is
/// refused.
#[test]
fn media_facts_resolve_their_handle_through_the_host() {
    use pm_protocol::media::{MediaKind, ProbeMedia};
    let mut host = Host::new("probe", &[]);
    let handle = host.mint(
        CapabilityKind::MediaRead {
            asset: "clip".into(),
        },
        fixture("hevc-alpha.mov"),
    );
    let id = host.send_request(
        Request::ProbeMedia(ProbeMedia {
            handle,
            interpret: Some(r#"{"alpha":"premultiplied"}"#.into()),
        }),
        None,
        None,
    );
    assert!(host.run_until(Duration::from_secs(20), |h| h.answers.contains_key(&id)));
    let Some(Message::Response {
        body: Response::MediaFacts(facts),
        ..
    }) = host.answers.get(&id)
    else {
        panic!("{:?}", host.answers.get(&id));
    };
    assert_eq!(facts.kind, MediaKind::Video);
    let alpha = facts.settings.iter().find(|s| s.member == "alpha").unwrap();
    assert_eq!(alpha.value, "premultiplied");
    assert_eq!(facts.display_size, Some([320, 180]));
    assert!(host.host_requests.contains(&"resolveHandle"));

    let id = host.send_request(
        Request::ProbeMedia(ProbeMedia {
            handle: CapabilityHandle::from_random_bytes([0xEE; 16]),
            interpret: None,
        }),
        None,
        None,
    );
    assert!(host.run_until(Duration::from_secs(20), |h| h.answers.contains_key(&id)));
    assert!(matches!(
        host.answers.get(&id),
        Some(Message::Error { error, .. }) if error.code == ErrorCode::CapabilityRejected
    ));

    // Probes sent back to back queue on the probe lane (one thread, in
    // order) and every one is answered with its own reading.
    let ids: Vec<_> = ["straight", "premultiplied", "ignore", "straight", "ignore"]
        .iter()
        .map(|alpha| {
            let handle = host.mint(
                CapabilityKind::MediaRead {
                    asset: "clip".into(),
                },
                fixture("hevc-alpha.mov"),
            );
            let id = host.send_request(
                Request::ProbeMedia(ProbeMedia {
                    handle,
                    interpret: Some(format!(r#"{{"alpha":"{alpha}"}}"#)),
                }),
                None,
                None,
            );
            (id, *alpha)
        })
        .collect();
    assert!(host.run_until(Duration::from_secs(60), |h| {
        ids.iter().all(|(id, _)| h.answers.contains_key(id))
    }));
    for (id, alpha) in ids {
        let Some(Message::Response {
            body: Response::MediaFacts(facts),
            ..
        }) = host.answers.get(&id)
        else {
            panic!("{:?}", host.answers.get(&id));
        };
        let value = &facts
            .settings
            .iter()
            .find(|s| s.member == "alpha")
            .unwrap()
            .value;
        assert_eq!(value, alpha);
    }
}

/// The project with Invert on its background solid (the top-left corner).
fn inverted_project() -> Value {
    let mut doc = project(false);
    let back = doc["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "back")
        .unwrap();
    back["fx"] =
        json!([{ "id": "fx1", "type": "invert", "on": true, "p": { "amount": ch(json!(100)) } }]);
    doc
}

/// `invert` replaced by a fork that paints the layer blue (an override by
/// id: the same id, the fork's GLSL).
fn blue_invert_fork() -> pm_protocol::definitions::DefinitionSnapshot {
    let mut fork = pm_render::builtin_definitions()
        .definitions
        .into_iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap();
    fork.origin = pm_protocol::definitions::DefinitionOrigin::Fork;
    let glsl = fork.glsl.as_mut().unwrap();
    let body = "\n      vec4 c = texture(u_tex, v_st); o = vec4(0.0, 0.0, c.a, c.a);\n}";
    let at = glsl.source.rfind("void main(){").unwrap() + "void main(){".len();
    glsl.source.truncate(at);
    glsl.source.push_str(body);
    glsl.body_utf16_len = body.encode_utf16().count() as u32;
    glsl.body_utf8_len = body.len() as u32;
    fork
}

impl Host {
    /// Pushes a definitions update to `session`; the engine's answer.
    fn push_definitions(
        &mut self,
        session: SessionId,
        update: pm_protocol::definitions::DefinitionsUpdate,
    ) -> Message {
        let id = self.send_request(Request::PushDefinitions(update), Some(session), None);
        // The compile worker answers once the update is compiled.
        assert!(self.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
        self.answers.remove(&id).expect("answered")
    }

    /// The top-left pixel of each frame of a batch on `session`.
    fn corner_pixels(
        &mut self,
        session: SessionId,
        revision: u64,
    ) -> Vec<Result<[u8; 3], ErrorCode>> {
        let id = self.send_request(
            Request::AgentFrames(frames_request(2)),
            Some(session),
            Some(revision),
        );
        assert!(self.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
        let Message::Response {
            body: Response::AgentFrames(result),
            ..
        } = &self.answers[&id]
        else {
            panic!("{:?}", self.answers[&id])
        };
        result
            .outcomes
            .iter()
            .map(|o| match o {
                AgentFrameOutcome::Rendered {
                    image: ImagePayload::Inline { bytes },
                    ..
                } => {
                    let decoded =
                        image::load_from_memory_with_format(&bytes.0, image::ImageFormat::Jpeg)
                            .unwrap();
                    Ok(decoded.to_rgb8().get_pixel(2, 2).0)
                }
                AgentFrameOutcome::Failed { error, .. } => Err(error.code),
                other => panic!("{other:?}"),
            })
            .collect()
    }
}

fn near(px: [u8; 3], want: [u8; 3]) -> bool {
    px.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 6)
}

/// Contract 10 in the job engine: agent frames and exports draw effects
/// with the definitions their session holds when the work is submitted (the
/// window's winning registrations), an override by id draws the fork's
/// GLSL from the next batch, disposing it restores the built-in, a removed
/// definition is skipped with its data kept (as a missing effect), and a
/// session without definitions reports the effect unsupported instead of
/// drawing from a stale or built-in set.
#[test]
fn agent_frames_and_exports_draw_the_definitions_their_session_holds() {
    use pm_protocol::definitions::{DefinitionKind, DefinitionRemoval, DefinitionsUpdate};
    let mut host = Host::new("definitions", &[]);
    let doc = inverted_project();
    let sid = host.snapshot(&doc, SessionRole::Job, 1);
    // No definitions pushed: the effect cannot be drawn, and nothing is
    // drawn without it.
    let none = host.corner_pixels(sid, 1);
    assert!(
        none.iter().all(|p| *p == Err(ErrorCode::Unsupported)),
        "{none:?}"
    );
    let builtins = pm_render::builtin_definitions();
    let update = |generation: u64, full: bool, upserts, removals| DefinitionsUpdate {
        generation: Seq::new(generation).unwrap(),
        full,
        upserts,
        removals,
    };
    // The window's set (built-ins): acknowledged, then drawn.
    let applied = host.push_definitions(sid, update(1, true, builtins.definitions.clone(), vec![]));
    assert!(
        matches!(&applied, Message::Response { body: Response::DefinitionsApplied(a), .. } if a.generation.get() == 1),
        "{applied:?}"
    );
    // #E83A5E inverted.
    let inverted = [0x17, 0xC5, 0xA1];
    for px in host.corner_pixels(sid, 1) {
        assert!(near(px.unwrap(), inverted), "{px:?}");
    }
    // A fork registers `invert` (a delta): the next batch draws the fork.
    host.push_definitions(sid, update(2, false, vec![blue_invert_fork()], vec![]));
    for px in host.corner_pixels(sid, 1) {
        assert!(near(px.unwrap(), [0, 0, 255]), "the override wins: {px:?}");
    }
    // The fork is disposed: the built-in registration is restored.
    let original = builtins
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    host.push_definitions(sid, update(3, false, vec![original], vec![]));
    for px in host.corner_pixels(sid, 1) {
        assert!(near(px.unwrap(), inverted), "restored: {px:?}");
    }
    // No registration remains: missing, skipped with its data kept.
    host.push_definitions(
        sid,
        update(
            4,
            false,
            vec![],
            vec![DefinitionRemoval {
                kind: DefinitionKind::Effect,
                id: "invert".into(),
            }],
        ),
    );
    for px in host.corner_pixels(sid, 1) {
        assert!(near(px.unwrap(), [0xE8, 0x3A, 0x5E]), "skipped: {px:?}");
    }
    // An export from a snapshot session with the window's set draws every
    // effect (it was routed before definitions reached the job engine).
    let snapshot = host.snapshot(&doc, SessionRole::JobSnapshot, 1);
    host.push_definitions(
        snapshot,
        update(1, true, builtins.definitions.clone(), vec![]),
    );
    host.send_request(
        Request::SubmitJob(agent_export(
            "job-fx",
            "run-d",
            snapshot,
            ExportFormat::H264,
            "mp4",
        )),
        None,
        None,
    );
    assert!(host.run_until(Duration::from_secs(120), |h| {
        terminal(h, "job-fx").is_some()
    }));
    let status = terminal(&host, "job-fx").unwrap();
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);
    // Without definitions the same export is refused before anything is
    // written.
    let bare = host.snapshot(&doc, SessionRole::JobSnapshot, 1);
    host.send_request(
        Request::SubmitJob(agent_export(
            "job-bare",
            "run-d",
            bare,
            ExportFormat::H264,
            "mp4",
        )),
        None,
        None,
    );
    assert!(host.run_until(Duration::from_secs(60), |h| {
        terminal(h, "job-bare").is_some()
    }));
    let status = terminal(&host, "job-bare").unwrap();
    assert_eq!(status.state, JobState::Failed);
    assert_eq!(
        status.error.as_ref().map(|e| e.code),
        Some(ErrorCode::Unsupported)
    );
}

fn raw_effect(id: &str, source: &str) -> pm_protocol::definitions::DefinitionSnapshot {
    serde_json::from_value(json!({
        "id": id, "kind": "effect", "version": 0, "origin": "local", "renderer": null,
        "glsl": { "source": source, "bodyLineOffset": 0, "bodyUtf16Len": source.encode_utf16().count(), "bodyUtf8Len": source.len() },
        "params": [],
        "flags": { "passes": 1, "keepOrig": false, "backdrop": false, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": false }
    }))
    .unwrap()
}

/// The job engine compiles what it is pushed before acknowledging it
/// (`validate_effect`): a program WebGL refuses too fails with its
/// diagnostics in the author's lines, one only the WebGL fallback runner
/// draws is reported as not headless-renderable (valid in the app, never a
/// compile error), and valid programs and the built-ins pass.
#[test]
fn pushed_definitions_are_compiled_before_they_are_acknowledged() {
    use pm_protocol::definitions::DefinitionsUpdate;
    let mut host = Host::new("compile", &[]);
    let sid = host.snapshot(&project(false), SessionRole::Job, 1);
    let good = raw_effect(
        "acme.gain",
        "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nout vec4 o;\nvoid main(){ o = texture(u_tex, v_st) * 2.0; }\n",
    );
    let broken = raw_effect(
        "acme.broken",
        "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main(){\n  o = nope;\n}\n",
    );
    let runner = raw_effect(
        "acme.sky",
        "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nuniform samplerCube u_sky;\nout vec4 o;\nvoid main(){ o = texture(u_tex, v_st) + texture(u_sky, vec3(0.0, 0.0, 1.0)); }\n",
    );
    let mut upserts = pm_render::builtin_definitions().definitions;
    upserts.extend([good, broken, runner]);
    let answer = host.push_definitions(
        sid,
        DefinitionsUpdate {
            generation: Seq::new(1).unwrap(),
            full: true,
            upserts,
            removals: vec![],
        },
    );
    let Message::Response {
        body: Response::DefinitionsApplied(applied),
        ..
    } = answer
    else {
        panic!("{answer:?}")
    };
    assert_eq!(applied.generation.get(), 1);
    let mut failures: Vec<_> = applied
        .failures
        .iter()
        .map(|e| match &e.scope {
            ErrorScope::Definition { definition } => (definition.as_str().to_owned(), e.code),
            other => panic!("{other:?}"),
        })
        .collect();
    failures.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        failures,
        vec![
            ("acme.broken".to_owned(), ErrorCode::ShaderCompile),
            ("acme.sky".to_owned(), ErrorCode::NotHeadlessRenderable),
        ]
    );
    let broken = applied
        .failures
        .iter()
        .find(|e| e.code == ErrorCode::ShaderCompile)
        .unwrap();
    let Some(pm_protocol::error::ErrorDetail::ShaderCompile { diagnostics }) = &broken.detail
    else {
        panic!("{broken:?}")
    };
    assert!(
        diagnostics
            .iter()
            .any(|d| d.line == 5 && d.message.contains("nope")),
        "{diagnostics:?}"
    );
}

/// Contract 14 and amendment 21 in the job engine: an effect only the WebGL
/// runner draws is not routed and not skipped. The job asks the host to run
/// it in a window's runner, with its textures as files in the session's
/// scratch area, and draws the pixels the host writes back: agent frames
/// and an export alike.
#[test]
fn effects_only_the_webgl_runner_draws_run_in_a_window_through_the_host() {
    use pm_protocol::definitions::DefinitionsUpdate;
    let mut host = Host::new("fallback", &[]);
    let mut doc = project(false);
    let back = doc["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "back")
        .unwrap();
    back["fx"] = json!([{ "id": "fx1", "type": "acme.cube", "on": true, "p": {} }]);
    let mut set = pm_render::builtin_definitions();
    let mut cube = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    cube.id = "acme.cube".into();
    cube.origin = pm_protocol::definitions::DefinitionOrigin::Store;
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
    let update = DefinitionsUpdate {
        generation: Seq::new(1).unwrap(),
        full: true,
        upserts: set.definitions,
        removals: vec![],
    };
    let sid = host.snapshot(&doc, SessionRole::Job, 1);
    host.push_definitions(sid, update.clone());
    for px in host.corner_pixels(sid, 1) {
        assert!(
            near(px.unwrap(), [0, 0, 255]),
            "the runner's pixels: {px:?}"
        );
    }
    assert!(host.host_requests.contains(&"shaderFallback"));
    // The scratch files are gone once read.
    let left = std::fs::read_dir(host.dir.join("scratch")).unwrap().count();
    assert_eq!(left, 0);
    // An export draws it too.
    let snapshot = host.snapshot(&doc, SessionRole::JobSnapshot, 1);
    host.push_definitions(snapshot, update);
    host.send_request(
        Request::SubmitJob(agent_export(
            "job-cube",
            "run-c",
            snapshot,
            ExportFormat::H264,
            "mp4",
        )),
        None,
        None,
    );
    assert!(host.run_until(Duration::from_secs(120), |h| {
        terminal(h, "job-cube").is_some()
    }));
    let status = terminal(&host, "job-cube").unwrap();
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);
}

/// Level 1 SVG images (decision S1): the job engine asks host services for
/// each raster (the window that owns the document draws it with today's
/// code), once per bucket, and with no window the frame fails with the
/// host's words instead of drawing anything else.
#[test]
fn level1_svg_frames_get_their_rasters_from_the_documents_window() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="36"><rect width="64" height="36" fill="#4C8DFF"/></svg>"##;
    let mut host = Host::new("svg", &[]);
    let path = host.dir.join("art.svg");
    std::fs::write(&path, svg).unwrap();
    host.media.insert("art".into(), path);
    let mut doc = project(false);
    // First in the list: the top layer, clear of the others.
    doc["layers"].as_array_mut().unwrap().insert(
        0,
        layer(
            "svg",
            "image",
            json!({ "asset": "art", "fit": "contain", "w": 80, "h": 45 }),
            270.0,
            140.0,
            1.0,
        ),
    );
    doc["assets"]["art"] = json!({ "id": "art", "name": "art.svg", "kind": "image", "format": "svg", "w": 64, "h": 36, "dur": 0, "persisted": true });
    host.svg_window = Ok([0x4C, 0x8D, 0xFF, 0xFF]);
    let sid = host.snapshot(&doc, SessionRole::Job, 2);
    let id = host.send_request(Request::AgentFrames(frames_request(2)), Some(sid), Some(2));
    assert!(host.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    let images = frame_images(&host.answers[&id]);
    let decoded =
        image::load_from_memory_with_format(&images[0].2, image::ImageFormat::Jpeg).unwrap();
    let px = decoded.to_rgb8().get_pixel(270, 140).0;
    assert!(
        px.iter()
            .zip([0x4C, 0x8D, 0xFF])
            .all(|(a, b)| a.abs_diff(b) <= 3),
        "the window's raster at the layer's center: {px:?}"
    );
    // One ask for the bucket (`svgRasterDimensions(64, 36, 80, 45, …)`:
    // 80 × 45), naming the document, the layer, and the SVG's bytes.
    assert_eq!(host.compat_asks.len(), 1, "{:?}", host.compat_asks);
    let ask = &host.compat_asks[0];
    assert_eq!(ask.document.as_str(), "Ptest");
    assert_eq!(
        (ask.composition.as_str(), ask.layer.as_str()),
        ("Ptest-root", "svg")
    );
    assert_eq!(ask.source, pm_protocol::snapshot::CompatSource::SvgImage);
    let raster = ask.svg.as_ref().unwrap();
    assert_eq!(
        (raster.key.width, raster.key.height, raster.asset.as_str()),
        (80, 45, "art")
    );
    let content = pm_render::vector::svg::content_hash(&host.media["art"]).unwrap();
    assert_eq!(raster.key.content, content);
    assert_eq!(content.len(), 64);

    // No window has the project open: the frames fail with the host's words.
    let mut lonely = Host::new("svg-lonely", &[]);
    let path = lonely.dir.join("art.svg");
    std::fs::write(&path, svg).unwrap();
    lonely.media.insert("art".into(), path);
    let sid = lonely.snapshot(&doc, SessionRole::Job, 2);
    let id = lonely.send_request(Request::AgentFrames(frames_request(1)), Some(sid), Some(2));
    assert!(lonely.run_until(Duration::from_secs(60), |h| h.answers.contains_key(&id)));
    let Message::Response {
        body: Response::AgentFrames(result),
        ..
    } = &lonely.answers[&id]
    else {
        panic!()
    };
    match &result.outcomes[0] {
        AgentFrameOutcome::Failed { error, .. } => {
            assert!(
                error.message.contains("open the project in a window"),
                "{error:?}"
            )
        }
        other => panic!("{other:?}"),
    }
}
