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
    CapabilityHandle, CapabilityKind, HandleAccess, HandleGrant, HandleResolved, OutputChoice,
    OutputCommitted, OutputDestination, PortId,
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
