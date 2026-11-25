//! Exercise the same in-process Handle and kernel tool session the Agent panel uses.
use pm_agents::{
    Access, Event, Provider, RunResult,
    aero::{self, Model, Options, Reply},
    launch::Request,
    transport::{Action, Handle},
};
use pm_mcp::{Automation, McpSession, session::aero_tools::Tools};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

struct Fixture {
    profile: tempfile::TempDir,
    host: Arc<Mutex<Automation>>,
    project: String,
    revision: u64,
}

impl Fixture {
    fn new() -> Self {
        let profile = tempfile::Builder::new()
            .prefix("pm-aero-")
            .tempdir()
            .unwrap();
        let root = profile.path();
        let mut host = Automation::new(root.into(), root.join("exports")).unwrap();
        let project = host
            .create_project(&json!({"name":"Synthetic cut","duration":4}))
            .unwrap()["projectId"]
            .as_str()
            .unwrap()
            .to_owned();
        let host = Arc::new(Mutex::new(host));
        {
            let mut seed = McpSession::new(host.clone());
            seed.dispatch("start_session", &json!({"prompt":"Seed synthetic layers"}))
                .unwrap();
            seed.dispatch(
                "apply_commands",
                &json!({"commands":[
                    {"type":"add_layer","id":"keeper","name":"Keep","layerType":"solid",
                        "from":0,"duration":4},
                    {"type":"add_layer","id":"flub","name":"Remove","layerType":"solid",
                        "from":1,"duration":1}
                ]}),
            )
            .unwrap();
            seed.finish(true).unwrap();
        }
        let revision = host.lock().unwrap().revision(&project).unwrap();
        Self {
            profile,
            host,
            project,
            revision,
        }
    }

    fn start(
        &self,
        model: Model,
        build: impl FnOnce(Tools) -> Box<dyn aero::Session> + Send + 'static,
    ) -> (Handle, mpsc::Receiver<Value>) {
        self.start_at(
            "cut",
            self.profile.path().join("aero/sessions/cut"),
            model,
            build,
        )
    }

    fn start_at(
        &self,
        run: &str,
        scope: std::path::PathBuf,
        model: Model,
        build: impl FnOnce(Tools) -> Box<dyn aero::Session> + Send + 'static,
    ) -> (Handle, mpsc::Receiver<Value>) {
        self.start_prompt(run, scope, "Cut the flubbed take.", model, build)
    }

    fn start_prompt(
        &self,
        run: &str,
        scope: std::path::PathBuf,
        prompt: &str,
        model: Model,
        build: impl FnOnce(Tools) -> Box<dyn aero::Session> + Send + 'static,
    ) -> (Handle, mpsc::Receiver<Value>) {
        self.start_choice(run, scope, prompt, (None, None), model, build)
    }

    /// A turn on the picker's `(model, effort)`, as the panel sends it.
    fn start_choice(
        &self,
        run: &str,
        scope: std::path::PathBuf,
        prompt: &str,
        (model_id, effort): (Option<&str>, Option<&str>),
        model: Model,
        build: impl FnOnce(Tools) -> Box<dyn aero::Session> + Send + 'static,
    ) -> (Handle, mpsc::Receiver<Value>) {
        let host = self.host.clone();
        let project = self.project.clone();
        let revision = self.revision;
        let (report, notes) = mpsc::channel();
        let handle = aero::start(
            Request {
                run: run.into(),
                prompt: prompt.into(),
                provider: Provider::Aero,
                model: model_id.map(str::to_owned),
                effort: effort.map(str::to_owned),
                access: Access::Project,
                approval: Default::default(),
                computer_consent: false,
                workspace: scope.clone(),
                images: vec![],
                role: None,
                files: vec![],
                project: Some(project.clone()),
            },
            run.into(),
            scope,
            Options {
                model,
                report: Some(Arc::new(move |v| {
                    let _ = report.send(v);
                })),
                wall: Duration::from_secs(10),
                ..Options::development()
            },
            move |scope| {
                Tools::open(host, &project, Some(revision), scope, Access::Project, None)
                    .map(build)
                    .map_err(|e| e.to_string())
            },
        );
        (handle, notes)
    }

    fn tools_at(&self, scope: &std::path::Path, access: Access) -> Tools {
        std::fs::create_dir_all(scope).unwrap();
        Tools::open(self.host.clone(), &self.project, None, scope, access, None).unwrap()
    }

    /// Extension staging over a fake installed folder whose compiler refuses
    /// any source containing "broken".
    fn with_extensions(self) -> Self {
        let root = self.profile.path().join("installed");
        std::fs::create_dir_all(root.join("live")).unwrap();
        self.host.lock().unwrap().extensions = Some(Arc::new(Extensions(root)));
        self
    }

    fn tools(&self, access: Access) -> Tools {
        let scope = self.profile.path().join("direct");
        std::fs::create_dir(&scope).unwrap();
        Tools::open(
            self.host.clone(),
            &self.project,
            Some(self.revision),
            &scope,
            access,
            None,
        )
        .unwrap()
    }
}

fn complete(handle: &Handle) -> RunResult {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match handle.try_event() {
            Ok(Event::Completed { result, .. }) => return result,
            Ok(Event::Trace { value, .. }) if value["kind"] == "tool-end" => eprintln!("{value}"),
            Ok(_) | Err(mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(3))
            }
            other => panic!("Aero did not finish: {other:?}"),
        }
    }
}

fn call(name: &str, op: &str, revision: Option<u64>, args: Value) -> Reply {
    Reply::Tool {
        name: name.into(),
        args: json!({"op":op, "revision":revision,"args":args}),
    }
}

fn scripted(f: impl Fn(usize, &[u8]) -> Reply + Send + Sync + 'static) -> Model {
    let next = AtomicUsize::new(0);
    Model::fake(move |input| f(next.fetch_add(1, Ordering::Relaxed), input))
}

#[test]
fn fake_model_cuts_through_the_real_kernel_and_reports_usage() {
    let fixture = Fixture::new();
    let rev = fixture.revision;
    let (turn, notes) = fixture.start(
        scripted(move |step, _| match step {
            0 => call(
                "edit",
                "commands",
                Some(rev),
                json!({"commands":[
                    {"type":"delete_layers","targets":["flub"]}
                ]}),
            ),
            _ => Reply::Text("Cut the flubbed take.".into()),
        }),
        |tools| Box::new(tools),
    );
    assert_eq!(
        complete(&turn),
        RunResult::success("Cut the flubbed take.".into(), Access::Project)
    );
    let project = fixture
        .host
        .lock()
        .unwrap()
        .read_document(&fixture.project)
        .unwrap();
    assert_eq!(project["layers"].as_array().unwrap().len(), 1);
    assert_eq!(project["layers"][0]["id"], "keeper");
    let notes: Vec<_> = notes.try_iter().collect();
    assert!(notes.iter().any(|note| note["type"] == "tool-started"));
    assert!(notes.iter().any(|note| note["type"] == "usage"));
}

/// A model that narrates before each tool call (as real models do) ends with
/// its last answer, not every step's text run together: the Agent panel
/// showed "Reading the layer first.Renaming it…" after the reply (2026-11-25).
#[test]
fn a_turn_that_narrates_before_its_tool_calls_ends_with_its_last_answer_alone() {
    let fixture = Fixture::new();
    let rev = fixture.revision;
    let (turn, _notes) = fixture.start(
        scripted(move |step, _| match step {
            0 => Reply::Said(
                "Reading the project first.".into(),
                Box::new(call("project", "summary", None, json!({}))),
            ),
            1 => Reply::Said(
                "Cutting the flubbed take.".into(),
                Box::new(call(
                    "edit",
                    "commands",
                    Some(rev),
                    json!({"commands":[{"type":"delete_layers","targets":["flub"]}]}),
                )),
            ),
            _ => Reply::Text("Cut the flubbed take.".into()),
        }),
        |tools| Box::new(tools),
    );
    assert_eq!(
        complete(&turn),
        RunResult::success("Cut the flubbed take.".into(), Access::Project)
    );
}

/// A provider stream that drops mid-reply, or goes silent, ends the turn
/// with words a person understands: the panel said "IncompleteStream" (the
/// core's error code), and a stalled stream kept the turn working for an
/// hour (2026-11-25, emulate's disconnect and stall faults).
#[test]
fn a_dropped_or_silent_model_stream_ends_the_turn_with_a_plain_error() {
    use pm_agents::edge::{Chunk, Idle, Net, NetRequest};
    use std::sync::atomic::AtomicBool;
    /// How the stream breaks after its first chunk: the connection errors,
    /// the body just ends (emulate's disconnect), or nothing more arrives.
    #[derive(Debug, Clone, Copy)]
    enum Broken {
        Reset,
        Ends,
        Silent,
    }
    impl Net for Broken {
        fn stream(
            &self,
            _: &NetRequest<'_>,
            stop: &AtomicBool,
            on: &mut dyn FnMut(Chunk<'_>) -> Result<(), String>,
        ) -> Result<(), String> {
            on(Chunk::Status(200))?;
            on(Chunk::Data(
                br#"data: {"id":"c","object":"chat.completion.chunk","created":0,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"Thinking"},"finish_reason":null}]}

"#,
            ))?;
            match self {
                Broken::Reset => return Err("connection reset by peer".into()),
                Broken::Ends => return Ok(()),
                Broken::Silent => {}
            }
            while !stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err("Stopped.".into())
        }
    }
    let dropped = "The connection to the model dropped before its reply finished. Try again.";
    for (broken, expected) in [
        (Broken::Reset, dropped),
        (Broken::Ends, dropped),
        (
            Broken::Silent,
            "The model stopped responding: nothing arrived for 0.3 seconds. Try again.",
        ),
    ] {
        let fixture = Fixture::new();
        let configured = json!({"custom":{"protocol":"openai-chat-completions",
            "base_url":"http://127.0.0.1:9/v1","auth":{"type":"none"},"tool_choice_mode":"send",
            "model_metadata":{"m":{"supports_tool_use":true}}}});
        let live = aero::Live {
            identity: None,
            provider: "custom".into(),
            configured: configured.to_string(),
            model: "m".into(),
            catalog: None,
            net: Arc::new(Idle {
                inner: Arc::new(broken),
                after: Duration::from_millis(300),
            }),
            credentials: Arc::new(|_| Ok(("http://127.0.0.1:9".into(), vec![]))),
        };
        let (turn, _notes) = fixture.start(Model::live(live), |tools| Box::new(tools));
        assert_eq!(complete(&turn), RunResult::failure(expected, false));
    }
}

#[test]
fn stale_revisions_and_read_access_leave_the_project_untouched() {
    let fixture = Fixture::new();
    let data = json!({"op":"commands","revision":fixture.revision + 1,"args":{"commands":[
        {"type":"set_layer","target":"keeper","patch":{"name":"Wrong"}}
    ]}})
    .to_string();
    let mut tools = fixture.tools(Access::Project);
    assert!(
        tools
            .call("edit", &data, &mut |_| false)
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    tools.finish().unwrap();
    let path = fixture.profile.path().join("direct");
    std::fs::remove_dir_all(path).unwrap();
    let mut read = fixture.tools(Access::Editor);
    assert!(
        read.call("edit", &data, &mut |_| true)
            .unwrap_err()
            .to_string()
            .contains("Access")
    );
    assert_eq!(
        fixture
            .host
            .lock()
            .unwrap()
            .revision(&fixture.project)
            .unwrap(),
        fixture.revision
    );
}

#[test]
fn a_regressed_plan_check_continues_the_fake_model_turn() {
    let fixture = Fixture::new();
    let revision = fixture.revision;
    let state = fixture
        .host
        .lock()
        .unwrap()
        .read_document(&fixture.project)
        .unwrap();
    let index = state["layers"]
        .as_array()
        .unwrap()
        .iter()
        .position(|layer| layer["id"] == "keeper")
        .unwrap();
    let pointer = format!("/layers/{index}/name");
    let (turn, notes) = fixture.start(scripted(move |step, input| match step {
        0 => call("plan","add",None,json!({
            "id":"first","behavior":"Keep the original name","check":{"type":"state","pointer":pointer,"equals":"Keep"}
        })),
        1 => call("plan","verify",None,json!({"id":"first"})),
        2 => call("edit","commands",Some(revision),json!({"commands":[
            {"type":"set_layer","target":"keeper","patch":{"name":"Temporary label"}}
        ]})),
        3 => Reply::Text("I think the edit is done.".into()),
        4 => {
            assert!(String::from_utf8_lossy(input).contains("Plan item first no longer passes"));
            call("edit","commands",Some(revision+1),json!({"commands":[
                {"type":"set_layer","target":"keeper","patch":{"name":"Keep"}}
            ]}))
        }
        _ => Reply::Text("Restored the original name.".into()),
    }), |tools| Box::new(tools));
    assert_eq!(
        complete(&turn),
        RunResult::success("Restored the original name.".into(), Access::Project)
    );
    assert_eq!(
        notes
            .try_iter()
            .filter(|v| v["type"] == "gate-continuation")
            .count(),
        1
    );
}

struct Held {
    tools: Tools,
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl aero::Session for Held {
    fn context(&mut self) -> Result<Value, String> {
        aero::Session::context(&mut self.tools)
    }
    fn call(
        &mut self,
        _: &str,
        _: &str,
        _: &mut dyn FnMut(&str) -> bool,
    ) -> Result<aero::ToolResult, String> {
        let _ = self.started.send(());
        let _ = self.release.recv();
        Ok(aero::ToolResult::text("retired"))
    }
    fn failures(&mut self) -> Vec<aero::Failure> {
        Vec::new()
    }
    fn finish(&mut self) -> Result<(), String> {
        aero::Session::finish(&mut self.tools)
    }
}

#[test]
fn stop_retires_a_turn_while_an_in_flight_tool_holds_its_lease() {
    let fixture = Fixture::new();
    let (started, ready) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (turn, _) = fixture.start(
        Model::fake(|_| call("project", "summary", None, json!({}))),
        move |tools| {
            Box::new(Held {
                tools,
                started,
                release: held,
            })
        },
    );
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    turn.send(Action::Cancel("cut".into())).unwrap();
    // Stop settles the admitted call before it counts as complete.
    release.send(()).unwrap();
    assert_eq!(complete(&turn), RunResult::failure("Stopped.", true));
}

/// A loopback replay server: each POST gets the next scripted SSE body. A
/// `None` body holds the request open until the test releases it.
struct ReplayServer {
    url: String,
    release: mpsc::Sender<()>,
}

fn sse(events: &[Value]) -> String {
    let mut out = String::new();
    for event in events {
        out.push_str(&format!("data: {event}\n\n"));
    }
    out + "data: [DONE]\n\n"
}

fn finish(reason: &str) -> Value {
    json!({"type":"finish","finishReason":{"unified":reason,"raw":reason},
        "usage":{"inputTokens":{"total":7},"outputTokens":{"total":3}}})
}

fn tool_body(id: &str, name: &str, input: Value) -> Option<String> {
    let input = input.to_string();
    Some(sse(&[
        json!({"type":"tool-input-start","id":id,"toolName":name}),
        json!({"type":"tool-input-delta","id":id,"delta":input}),
        json!({"type":"tool-call","toolCallId":id,"toolName":name,"input":input}),
        finish("tool-calls"),
    ]))
}

fn text_body(text: &str) -> Option<String> {
    Some(sse(&[
        json!({"type":"text-delta","id":"0","delta":text}),
        finish("stop"),
    ]))
}

impl ReplayServer {
    fn start(bodies: Vec<Option<String>>) -> Self {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/v1/stream",
            listener.local_addr().unwrap().port()
        );
        let (release, released) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            for body in bodies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut request = vec![0; length];
                reader.read_exact(&mut request).unwrap();
                let Some(body) = body else {
                    let _ = released.recv_timeout(Duration::from_secs(20));
                    return;
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
                );
            }
        });
        Self { url, release }
    }
}

/// The eval runner's side of `aero_rpc::serve`: newline JSON both ways.
struct Wire {
    input: std::io::PipeWriter,
    output: mpsc::Receiver<Value>,
    served: std::thread::JoinHandle<pm_mcp::Result<()>>,
}

struct Lines(Vec<u8>, mpsc::Sender<Value>);
impl std::io::Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(bytes);
        while let Some(end) = self.0.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.0.drain(..=end).collect();
            let _ = self.1.send(serde_json::from_slice(&line).unwrap());
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Wire {
    fn open(fixture: &Fixture) -> Self {
        let (reader, input) = std::io::pipe().unwrap();
        let (sender, output) = mpsc::channel();
        let host = fixture.host.clone();
        let profile = fixture.profile.path().to_owned();
        let served = std::thread::spawn(move || {
            pm_mcp::aero_rpc::serve(
                std::io::BufReader::new(reader),
                Lines(Vec::new(), sender),
                host,
                profile,
            )
        });
        let mut wire = Self {
            input,
            output,
            served,
        };
        wire.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}));
        let init = wire.next();
        let flags = &init["result"]["capabilities"]["experimental"]["powermoveEval"];
        assert_eq!(flags["evaluator"], false, "{init}");
        wire.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"open_project","arguments":{"projectId":fixture.project}}}));
        assert_eq!(wire.next()["result"]["isError"], false);
        wire
    }

    fn send(&mut self, message: Value) {
        use std::io::Write;
        writeln!(self.input, "{message}").unwrap();
    }

    fn next(&self) -> Value {
        self.output
            .recv_timeout(Duration::from_secs(20))
            .expect("eval host answered")
    }

    fn run(&mut self, fixture: &Fixture, id: &str, url: &str) {
        self.send(
            json!({"jsonrpc":"2.0","id":id,"method":"aero/run","params":{
            "runId":id,"projectId":fixture.project,"revision":fixture.revision,
            "prompt":"Cut the flubbed take.","model":"fake/model","effort":"high","wallSecs":30,
            "maxSteps":8,"maxGateContinuations":2,"provider":{"mode":"replay","baseUrl":url}}}),
        );
    }

    /// Notifications until the run's answer, which is returned last.
    fn collect(&self, id: &str) -> (Vec<Value>, Value) {
        let mut events = Vec::new();
        loop {
            let message = self.next();
            if message["id"] == id {
                return (events, message);
            }
            assert_eq!(message["method"], "aero/event", "{message}");
            assert_eq!(message["params"]["runId"], id);
            events.push(message["params"].clone());
        }
    }

    fn close(self) {
        drop(self.input);
        self.served.join().unwrap().unwrap();
    }
}

#[test]
fn aero_run_streams_sequenced_events_and_answers_the_eval_runner() {
    let fixture = Fixture::new();
    let replay = ReplayServer::start(vec![
        tool_body(
            "call-1",
            "edit",
            json!({"op":"commands","revision":fixture.revision,
                "args":{"commands":[{"type":"delete_layers","targets":["flub"]}]}}),
        ),
        text_body("Cut the flubbed take."),
    ]);
    let mut wire = Wire::open(&fixture);
    wire.run(&fixture, "eval-cut", &replay.url);
    let (events, answer) = wire.collect("eval-cut");
    assert_eq!(
        answer["result"],
        json!({"runId":"eval-cut","finalText":"Cut the flubbed take.","completed":true,"reason":"completed"})
    );
    for (seq, event) in events.iter().enumerate() {
        assert_eq!(event["seq"], seq as u64, "{events:?}");
    }
    let kinds: Vec<_> = events
        .iter()
        .map(|event| event["event"]["type"].clone())
        .collect();
    let started = events
        .iter()
        .find(|e| e["event"]["type"] == "tool-started")
        .unwrap();
    assert_eq!(started["event"]["name"], "edit");
    let call = &started["event"]["callId"];
    assert!(
        events.iter().any(|e| e["event"]["type"] == "tool-finished"
            && e["event"]["callId"] == *call
            && e["event"]["ok"] == true),
        "{kinds:?}"
    );
    let usage = events
        .iter()
        .find(|e| e["event"]["type"] == "usage")
        .unwrap();
    for key in ["input", "cached", "output"] {
        assert!(usage["event"][key].is_u64(), "{usage}");
    }
    let project = fixture
        .host
        .lock()
        .unwrap()
        .read_document(&fixture.project)
        .unwrap();
    assert_eq!(project["layers"].as_array().unwrap().len(), 1);
    // A second run on the same connection starts from the new revision only.
    wire.run(&fixture, "eval-stale", &replay.url);
    assert!(
        wire.next()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("revision")
    );
    wire.close();
}

#[test]
fn aero_cancel_answers_the_pending_run_while_the_model_is_streaming() {
    let fixture = Fixture::new();
    let replay = ReplayServer::start(vec![None]);
    let mut wire = Wire::open(&fixture);
    wire.run(&fixture, "eval-hold", &replay.url);
    // Another request while the run is in flight is refused, not queued.
    wire.send(json!({"jsonrpc":"2.0","id":9,"method":"tools/list"}));
    assert_eq!(wire.next()["error"]["code"], -32000);
    // A cancel for another run is ignored.
    wire.send(json!({"jsonrpc":"2.0","method":"aero/cancel","params":{"runId":"other","reason":"deadline"}}));
    wire.send(json!({"jsonrpc":"2.0","method":"aero/cancel","params":{"runId":"eval-hold","reason":"deadline"}}));
    let started = Instant::now();
    let (_, answer) = wire.collect("eval-hold");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(answer["result"]["reason"], "cancelled");
    assert_eq!(answer["result"]["completed"], false);
    assert_eq!(
        fixture
            .host
            .lock()
            .unwrap()
            .revision(&fixture.project)
            .unwrap(),
        fixture.revision
    );
    let _ = replay.release.send(());
    wire.close();
}

fn keeper_name(fixture: &Fixture) -> Value {
    let project = fixture
        .host
        .lock()
        .unwrap()
        .read_document(&fixture.project)
        .unwrap();
    project["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["id"] == "keeper")
        .unwrap()["name"]
        .clone()
}

fn undo_once(fixture: &Fixture) {
    fixture
        .host
        .lock()
        .unwrap()
        .undo(&fixture.project, pm_kernel::Origin::Interface, 0)
        .unwrap();
}

#[test]
fn a_failed_edit_rolls_back_without_leaving_an_undo_entry() {
    let fixture = Fixture::new();
    let mut tools = fixture.tools(Access::Project);
    let rename = |revision: u64, target: &str, name: &str| {
        json!({"op":"commands","revision":revision,"args":{"commands":[
            {"type":"set_layer","target":target,"patch":{"name":name}}
        ]}})
        .to_string()
    };
    let renamed = tools
        .call(
            "edit",
            &rename(fixture.revision, "keeper", "Renamed"),
            &mut |_| false,
        )
        .unwrap();
    let revision = renamed["revision"].as_u64().unwrap();
    // A batch whose second command fails applies nothing.
    let failing = json!({"op":"commands","revision":revision,"args":{"commands":[
        {"type":"set_layer","target":"keeper","patch":{"name":"Half applied"}},
        {"type":"set_layer","target":"no-such-layer","patch":{"name":"Missing"}}
    ]}})
    .to_string();
    let error = tools
        .call("edit", &failing, &mut |_| false)
        .unwrap_err()
        .to_string();
    assert!(!error.is_empty());
    assert_eq!(keeper_name(&fixture), "Renamed");
    assert_eq!(
        fixture
            .host
            .lock()
            .unwrap()
            .revision(&fixture.project)
            .unwrap(),
        revision,
        "a rolled-back edit keeps the revision"
    );
    tools.finish().unwrap();
    // One Undo, as a person presses it, reverts the successful edit: the
    // failed one left no empty entry in front of it.
    undo_once(&fixture);
    assert_eq!(keeper_name(&fixture), "Keep");
}

const KILL_CHILD: &str = "PM_AERO_KILL_CHILD";

/// Counts the calls that reach the project tools.
struct Counted {
    tools: Tools,
    calls: Arc<AtomicUsize>,
    hold: bool,
}
impl aero::Session for Counted {
    fn context(&mut self) -> Result<Value, String> {
        aero::Session::context(&mut self.tools)
    }
    fn call(
        &mut self,
        tool: &str,
        input: &str,
        approve: &mut dyn FnMut(&str) -> bool,
    ) -> Result<aero::ToolResult, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.hold {
            // The child's host is killed while this call runs.
            loop {
                std::thread::park();
            }
        }
        aero::Session::call(&mut self.tools, tool, input, approve)
    }
    fn failures(&mut self) -> Vec<aero::Failure> {
        Vec::new()
    }
    fn finish(&mut self) -> Result<(), String> {
        aero::Session::finish(&mut self.tools)
    }
}

#[test]
fn a_turn_resumes_after_its_host_is_killed_mid_edit_without_rerunning_it() {
    if let Ok(dir) = std::env::var(KILL_CHILD) {
        let dir = std::path::PathBuf::from(dir);
        let fixture = Fixture::new();
        let revision = fixture.revision;
        let calls = Arc::new(AtomicUsize::new(0));
        let held = calls.clone();
        let (_turn, _) = fixture.start_at(
            "before-crash",
            dir.join("session"),
            Model::fake(move |_| {
                call(
                    "edit",
                    "commands",
                    Some(revision),
                    json!({"commands":[{"type":"delete_layers","targets":["flub"]}]}),
                )
            }),
            move |tools| {
                Box::new(Counted {
                    tools,
                    calls: held,
                    hold: true,
                })
            },
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        while calls.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the edit never started");
            std::thread::sleep(Duration::from_millis(5));
        }
        // Let the journal store the running call before saying READY.
        std::thread::sleep(Duration::from_millis(500));
        std::fs::write(dir.join("READY"), b"").unwrap();
        loop {
            std::thread::park();
        }
    }
    let dir = tempfile::Builder::new()
        .prefix("pm-aero-kill-")
        .tempdir()
        .unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_turn_resumes_after_its_host_is_killed_mid_edit_without_rerunning_it",
            "--test-threads=1",
        ])
        .env(KILL_CHILD, dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !dir.path().join("READY").exists() {
        assert!(
            Instant::now() < deadline && child.try_wait().unwrap().is_none(),
            "the child host never reached its edit"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // SAFETY: the pid is the child this test started.
    unsafe { libc::kill(child.id() as i32, libc::SIGKILL) };
    let _ = child.wait();

    let fixture = Fixture::new();
    let bodies = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = bodies.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (turn, _) = fixture.start_at(
        "after-crash",
        dir.path().join("session"),
        Model::fake(move |input| {
            let mut bodies = seen.lock().unwrap();
            bodies.push(String::from_utf8_lossy(input).into_owned());
            Reply::Text(match bodies.len() {
                1 => "The cut may have run before the restart; checking first.".into(),
                _ => "Done.".into(),
            })
        }),
        move |tools| {
            Box::new(Counted {
                tools,
                calls: counted,
                hold: false,
            })
        },
    );
    let mut progress = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = loop {
        match turn.try_event() {
            Ok(Event::Completed { result, .. }) => break result,
            Ok(Event::Progress { text, .. }) => progress.push(text),
            Ok(_) | Err(mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(3))
            }
            other => panic!("Aero did not finish: {other:?}"),
        }
    };
    assert_eq!(result, RunResult::success("Done.".into(), Access::Project));
    assert!(
        progress.iter().any(|text| text.contains("Resuming")),
        "{progress:?}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the half-run edit was re-run"
    );
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "{bodies:#?}");
    // The resumed turn carries the interrupted edit call; the new prompt
    // runs after it.
    assert!(bodies[0].contains("delete_layers"), "{}", bodies[0]);
    assert!(bodies[1].contains("Cut the flubbed take."));
}

// ---- Sol's handback review, P1-1 to P1-6 -----------------------------------

struct Extensions(std::path::PathBuf);
impl pm_mcp::extensions::ExtensionHost for Extensions {
    fn roots(&self) -> Vec<std::path::PathBuf> {
        vec![self.0.join("live")]
    }
    fn history(&self) -> std::path::PathBuf {
        self.0.join("history")
    }
    fn builtins(&self) -> Option<std::path::PathBuf> {
        None
    }
    fn manifest(&self, raw: &Value) -> Result<Value, String> {
        Ok(raw.clone())
    }
    fn check(&self, dir: &std::path::Path, _: &Value) -> Result<(), String> {
        match std::fs::read_to_string(dir.join("index.ts")) {
            Ok(source) if source.contains("broken") => Err("index.ts:1:1: broken source".into()),
            _ => Ok(()),
        }
    }
    fn reload(&self, _: &[String]) {}
    fn health(&self) -> pm_mcp::Result<Vec<Value>> {
        Ok(Vec::new())
    }
    fn api_pack(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

fn args(op: &str, revision: Option<u64>, args: Value) -> String {
    json!({"op":op,"revision":revision,"args":args}).to_string()
}

fn write_extension(tools: &mut Tools, id: &str, source: &str) {
    for (path, text) in [
        (
            format!("{id}/manifest.json"),
            json!({"id":id,"name":id,"version":"1.0.0"}).to_string(),
        ),
        (format!("{id}/index.ts"), source.to_owned()),
    ] {
        tools
            .call(
                "files",
                &args("write", None, json!({"path":path,"text":text})),
                &mut |_| true,
            )
            .unwrap();
    }
}

fn read_staged(tools: &mut Tools, path: &str) -> pm_mcp::Result<Value> {
    tools.call(
        "files",
        &args("read", None, json!({"path":path})),
        &mut |_| true,
    )
}

/// Every event until the run completes: (result, progress lines).
fn complete_with_progress(handle: &Handle) -> (RunResult, Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut progress = Vec::new();
    loop {
        match handle.try_event() {
            Ok(Event::Completed { result, .. }) => return (result, progress),
            Ok(Event::Progress { text, .. }) => progress.push(text),
            Ok(_) | Err(mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(3))
            }
            other => panic!("Aero did not finish: {other:?}"),
        }
    }
}

/// Holds each project tool call until the test releases it.
struct Gated {
    tools: Tools,
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl aero::Session for Gated {
    fn context(&mut self) -> Result<Value, String> {
        aero::Session::context(&mut self.tools)
    }
    fn call(
        &mut self,
        tool: &str,
        input: &str,
        approve: &mut dyn FnMut(&str) -> bool,
    ) -> Result<aero::ToolResult, String> {
        let _ = self.started.send(());
        let _ = self.release.recv();
        aero::Session::call(&mut self.tools, tool, input, approve)
    }
    fn failures(&mut self) -> Vec<aero::Failure> {
        aero::Session::failures(&mut self.tools)
    }
    fn finish(&mut self) -> Result<(), String> {
        aero::Session::finish(&mut self.tools)
    }
    fn halt(&mut self, flag: Arc<std::sync::atomic::AtomicBool>) {
        aero::Session::halt(&mut self.tools, flag);
    }
}

/// P1-1: an edit dispatched to the tool worker when Stop lands never starts,
/// and Stop does not count as complete until the dispatched call settled.
#[test]
fn stop_during_a_dispatched_edit_settles_first_and_never_edits() {
    let fixture = Fixture::new();
    let revision = fixture.revision;
    let (started, ready) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (turn, _) = fixture.start(
        Model::fake(move |_| {
            call(
                "edit",
                "commands",
                Some(revision),
                json!({"commands":[
                    {"type":"set_layer","target":"keeper","patch":{"name":"Stopped edit"}},
                    {"type":"delete_layers","targets":["flub"]}
                ]}),
            )
        }),
        move |tools| {
            Box::new(Gated {
                tools,
                started,
                release: held,
            })
        },
    );
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    turn.send(Action::Cancel("cut".into())).unwrap();
    // Still settling: the dispatched call has not answered.
    let quiet = Instant::now() + Duration::from_millis(400);
    while Instant::now() < quiet {
        if let Ok(Event::Completed { .. }) = turn.try_event() {
            panic!("Stop completed before the dispatched edit settled");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    release.send(()).unwrap();
    assert_eq!(complete(&turn), RunResult::failure("Stopped.", true));
    assert_eq!(keeper_name(&fixture), "Keep", "an edit ran after Stop");
    let layers = fixture
        .host
        .lock()
        .unwrap()
        .read_document(&fixture.project)
        .unwrap()["layers"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(layers, 2);
}

/// P1-1: Send right after Stop runs the new prompt; the stopped turn is
/// never resumed.
#[test]
fn stop_then_immediate_send_runs_the_new_prompt_and_never_resumes() {
    let fixture = Fixture::new();
    let scope = fixture.profile.path().join("aero/sessions/thread");
    let (asked, waiting) = mpsc::channel();
    let asked = Mutex::new(asked);
    let (first, _) = fixture.start_at(
        "first",
        scope.clone(),
        Model::fake(move |_| {
            let _ = asked.lock().unwrap().send(());
            Reply::Hold
        }),
        |tools| Box::new(tools),
    );
    waiting.recv_timeout(Duration::from_secs(10)).unwrap();
    first.send(Action::Cancel("first".into())).unwrap();
    assert_eq!(complete(&first), RunResult::failure("Stopped.", true));
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = bodies.clone();
    let (second, _) = fixture.start_at(
        "second",
        scope,
        Model::fake(move |input| {
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(input).into());
            Reply::Text("Fresh start.".into())
        }),
        |tools| Box::new(tools),
    );
    let (result, progress) = complete_with_progress(&second);
    assert_eq!(
        result,
        RunResult::success("Fresh start.".into(), Access::Project)
    );
    assert!(
        !progress.iter().any(|line| line.contains("Resuming")),
        "the stopped turn was resumed: {progress:?}"
    );
    assert_eq!(bodies.lock().unwrap().len(), 1);
}

/// P1-1: Stop while a tool runs, then reopen the thread: the reopened run
/// never resumes the stopped turn and never re-runs its tool.
#[test]
fn stop_then_reopen_never_resumes_the_stopped_turn() {
    let fixture = Fixture::new();
    let scope = fixture.profile.path().join("aero/sessions/thread");
    let (started, ready) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (first, _) = fixture.start_at(
        "first",
        scope.clone(),
        Model::fake(|_| call("project", "summary", None, json!({}))),
        move |tools| {
            Box::new(Gated {
                tools,
                started,
                release: held,
            })
        },
    );
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    first.send(Action::Cancel("first".into())).unwrap();
    release.send(()).unwrap();
    assert_eq!(complete(&first), RunResult::failure("Stopped.", true));
    drop(first);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (reopened, _) = fixture.start_at(
        "reopened",
        scope,
        Model::fake(|_| Reply::Text("Reopened.".into())),
        move |tools| {
            Box::new(Counted {
                tools,
                calls: counted,
                hold: false,
            })
        },
    );
    let (result, progress) = complete_with_progress(&reopened);
    assert_eq!(
        result,
        RunResult::success("Reopened.".into(), Access::Project)
    );
    assert!(
        !progress.iter().any(|line| line.contains("Resuming")),
        "{progress:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// P1-2: a draft that failed to compile survives Stop and the next Send.
#[test]
fn a_draft_survives_stop_after_a_compile_error_and_the_next_send() {
    let fixture = Fixture::new().with_extensions();
    let scope = fixture.profile.path().join("aero/sessions/thread");
    let step = AtomicUsize::new(0);
    let (holding, held) = mpsc::channel();
    let holding = Mutex::new(holding);
    let (first, _) = fixture.start_at(
        "first",
        scope.clone(),
        Model::fake(move |_| match step.fetch_add(1, Ordering::SeqCst) {
            0 => call(
                "files",
                "write",
                None,
                json!({"path":"tint/manifest.json","text":"{\"id\":\"tint\",\"name\":\"tint\",\"version\":\"1.0.0\"}"}),
            ),
            1 => call(
                "files",
                "write",
                None,
                json!({"path":"tint/index.ts","text":"export const draft = 'broken until fixed';"}),
            ),
            2 => call("extension", "check", None, json!({"id":"tint"})),
            _ => {
                let _ = holding.lock().unwrap().send(());
                Reply::Hold
            }
        }),
        |tools| Box::new(tools),
    );
    held.recv_timeout(Duration::from_secs(10)).unwrap();
    first.send(Action::Cancel("first".into())).unwrap();
    assert_eq!(complete(&first), RunResult::failure("Stopped.", true));
    let (second, _) = fixture.start_at(
        "second",
        scope,
        Model::fake(|input| {
            if String::from_utf8_lossy(input).contains("broken until fixed") {
                Reply::Text("The draft is still there.".into())
            } else {
                call("files", "read", None, json!({"path":"tint/index.ts"}))
            }
        }),
        |tools| Box::new(tools),
    );
    // The gate fails on the draft (unpublished staging), so the turn ends
    // "not done"; what matters is that the model read the draft back.
    let (result, _) = complete_with_progress(&second);
    match result {
        RunResult::Success { text, .. } => assert_eq!(text, "The draft is still there."),
        RunResult::Failure { error, .. } => assert_eq!(
            error,
            "Powermove stopped before its checks passed: 1 problem remains (1 unpublished draft). Review the result before you continue."
        ),
    }
}

/// P1-2: a draft written before the host was killed is there on reopen.
#[test]
fn a_draft_survives_a_killed_host_and_reopen() {
    let fixture = Fixture::new().with_extensions();
    let scope = fixture.profile.path().join("aero/sessions/thread");
    let mut tools = fixture.tools_at(&scope, Access::Project);
    write_extension(&mut tools, "grain", "export const grain = 'kept';");
    // A killed host runs no cleanup.
    std::mem::forget(tools);
    let mut reopened = fixture.tools_at(&scope, Access::Project);
    let read = read_staged(&mut reopened, "grain/index.ts").expect("the draft survived");
    assert!(read.to_string().contains("kept"), "{read}");
}

/// P1-2: a turn that ends with its gate exhausted by unpublished staging
/// leaves the drafts for the next Send.
#[test]
fn a_draft_survives_an_exhausted_gate_and_the_next_send() {
    let fixture = Fixture::new().with_extensions();
    let scope = fixture.profile.path().join("aero/sessions/thread");
    let step = AtomicUsize::new(0);
    let (first, _) = fixture.start_at(
        "first",
        scope.clone(),
        Model::fake(move |_| match step.fetch_add(1, Ordering::SeqCst) {
            0 => call(
                "files",
                "write",
                None,
                json!({"path":"glow/index.ts","text":"export const glow = 'kept for later';"}),
            ),
            _ => Reply::Text("Done, I think.".into()),
        }),
        |tools| Box::new(tools),
    );
    let (result, _) = complete_with_progress(&first);
    assert!(
        matches!(&result, RunResult::Failure { error, .. } if error.starts_with("Powermove stopped before its checks passed")),
        "{result:?}"
    );
    let mut reopened = fixture.tools_at(&scope, Access::Project);
    let read = read_staged(&mut reopened, "glow/index.ts").expect("the draft survived");
    assert!(read.to_string().contains("kept for later"), "{read}");
}

/// P1-2: undoing one publication keeps the session's other drafts.
#[test]
fn undoing_a_publication_keeps_unrelated_drafts() {
    let fixture = Fixture::new().with_extensions();
    let scope = fixture.profile.path().join("aero/sessions/thread");
    let mut tools = fixture.tools_at(&scope, Access::Project);
    write_extension(&mut tools, "one", "export const one = 1;");
    let published = tools
        .call(
            "extension",
            &args(
                "publish",
                None,
                json!({"extensions":[{"id":"one","action":"created","summary":"One"}]}),
            ),
            &mut |_| true,
        )
        .unwrap();
    let change_set = published["changeSetId"].as_str().unwrap().to_owned();
    write_extension(&mut tools, "two", "export const two = 'draft';");
    tools
        .call(
            "undo",
            &args("extension", None, json!({"changeSetId":change_set})),
            &mut |_| true,
        )
        .unwrap();
    let read = read_staged(&mut tools, "two/index.ts").expect("the unrelated draft stays");
    assert!(read.to_string().contains("draft"), "{read}");
    assert!(
        read_staged(&mut tools, "one/index.ts").is_err(),
        "the undone extension left the stage with its installed tree"
    );
}

/// P1-4: Sol's sequence: a good edit, a rejected batch, then session Undo.
#[test]
fn session_undo_works_after_a_rejected_batch() {
    let fixture = Fixture::new();
    let mut tools = fixture.tools(Access::Project);
    let renamed = tools
        .call(
            "edit",
            &args(
                "commands",
                Some(fixture.revision),
                json!({"commands":[{"type":"set_layer","target":"keeper","patch":{"name":"Renamed"}}]}),
            ),
            &mut |_| false,
        )
        .unwrap();
    let revision = renamed["revision"].as_u64().unwrap();
    assert!(
        tools
            .call(
                "edit",
                &args(
                    "commands",
                    Some(revision),
                    json!({"commands":[{"type":"set_layer","target":"no-such-layer","patch":{"name":"x"}}]}),
                ),
                &mut |_| false,
            )
            .is_err()
    );
    tools
        .call("undo", &args("session", None, json!({})), &mut |_| false)
        .expect("session Undo stays available after a clean abort");
    assert_eq!(keeper_name(&fixture), "Keep");
}

/// P1-4: session Undo restores authored data byte for byte, lone UTF-16
/// surrogates included.
#[test]
fn session_undo_keeps_lone_surrogates_byte_for_byte() {
    use pm_kernel::{Command as KernelCommand, MediaCommand, Origin};
    let fixture = Fixture::new();
    {
        let mut host = fixture.host.lock().unwrap();
        let revision = host.revision(&fixture.project).unwrap();
        let asset = pm_model::json::parse(
            r#"{"id":"a","name":"Take \ud800 one","kind":"video","dur":3,"w":640,"h":360,"persisted":true,"sourcePath":"/synthetic/take.mp4"}"#,
        )
        .unwrap();
        let command: KernelCommand = MediaCommand::AddAsset {
            asset_id: "a".into(),
            asset,
        }
        .into();
        host.apply_commands(
            &fixture.project,
            vec![command],
            "Synthetic asset",
            Origin::Mcp,
            revision,
            1,
        )
        .unwrap();
    }
    let native = |fixture: &Fixture| {
        let host = fixture.host.lock().unwrap();
        let view = host.view(&fixture.project).unwrap();
        pm_model::json::to_canonical_string(&view.snapshot.document.to_value().unwrap())
    };
    let before = native(&fixture);
    assert!(before.contains(r"\ud800"), "{before}");
    let revision = fixture
        .host
        .lock()
        .unwrap()
        .revision(&fixture.project)
        .unwrap();
    let scope = fixture.profile.path().join("surrogates");
    let mut tools = fixture.tools_at(&scope, Access::Project);
    tools
        .call(
            "edit",
            &args(
                "commands",
                Some(revision),
                json!({"commands":[{"type":"set_layer","target":"keeper","patch":{"name":"Renamed"}}]}),
            ),
            &mut |_| false,
        )
        .unwrap();
    tools
        .call("undo", &args("session", None, json!({})), &mut |_| false)
        .unwrap();
    let after = native(&fixture);
    assert!(
        after.contains(r"\ud800"),
        "the surrogate was rewritten: {after}"
    );
    assert!(after.contains("\"Keep\""));
}

/// P1-5: closing the project during a run gives the next edit a terminal
/// conflict, never a panic.
#[test]
fn closing_the_project_during_a_run_gives_a_conflict_not_a_panic() {
    let fixture = Fixture::new();
    let mut tools = fixture.tools(Access::Project);
    fixture.host.lock().unwrap().close_headless().unwrap();
    let _ = tools.call("project", &args("summary", None, json!({})), &mut |_| false);
    let error = tools
        .call(
            "edit",
            &args(
                "commands",
                Some(fixture.revision),
                json!({"commands":[{"type":"set_layer","target":"keeper","patch":{"name":"x"}}]}),
            ),
            &mut |_| false,
        )
        .unwrap_err();
    assert_eq!(error.code, "revision_conflict", "{}", error.message);
}

/// Holds the completion check until the test releases it.
struct HeldGate {
    tools: Tools,
    checking: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    held: bool,
}
impl aero::Session for HeldGate {
    fn context(&mut self) -> Result<Value, String> {
        aero::Session::context(&mut self.tools)
    }
    fn call(
        &mut self,
        tool: &str,
        input: &str,
        approve: &mut dyn FnMut(&str) -> bool,
    ) -> Result<aero::ToolResult, String> {
        aero::Session::call(&mut self.tools, tool, input, approve)
    }
    fn failures(&mut self) -> Vec<aero::Failure> {
        if !self.held {
            self.held = true;
            let _ = self.checking.send(());
            let _ = self.release.recv();
        }
        Vec::new()
    }
    fn finish(&mut self) -> Result<(), String> {
        aero::Session::finish(&mut self.tools)
    }
}

fn steer(handle: &Handle, run: &str, prompt: &str) {
    let accepted = handle
        .send_checked(Action::Steer {
            run: run.into(),
            prompt: prompt.into(),
            images: Vec::new(),
        })
        .unwrap()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert!(accepted, "{prompt} was refused");
}

/// The order in which the steers first reach the model.
fn steer_order(bodies: &[String], steers: &[&str]) -> Vec<String> {
    let all = bodies.join("\n");
    let mut found: Vec<(usize, String)> = steers
        .iter()
        .filter_map(|steer| all.find(steer).map(|at| (at, (*steer).to_owned())))
        .collect();
    found.sort();
    found.into_iter().map(|(_, steer)| steer).collect()
}

/// P1-6: steers accepted while the turn is finishing all come back and run,
/// in order (none lost to a single slot or a stale request).
#[test]
fn steers_accepted_at_the_end_of_a_turn_all_run_in_order() {
    let fixture = Fixture::new();
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = bodies.clone();
    let (checking, at_gate) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (turn, _) = fixture.start(
        Model::fake(move |input| {
            let mut bodies = seen.lock().unwrap();
            bodies.push(String::from_utf8_lossy(input).into());
            Reply::Text(format!("Answer {}.", bodies.len()))
        }),
        move |tools| {
            Box::new(HeldGate {
                tools,
                checking,
                release: held,
                held: false,
            })
        },
    );
    at_gate.recv_timeout(Duration::from_secs(10)).unwrap();
    let steers = [
        "Steer alpha please",
        "Steer bravo please",
        "Steer charlie please",
    ];
    for text in steers {
        steer(&turn, "cut", text);
    }
    release.send(()).unwrap();
    let (result, _) = complete_with_progress(&turn);
    assert!(matches!(result, RunResult::Success { .. }), "{result:?}");
    let bodies = bodies.lock().unwrap();
    assert_eq!(
        steer_order(&bodies, &steers),
        steers,
        "{} requests",
        bodies.len()
    );
}

/// P1-6: steers accepted before Stop are kept and run first, in order, when
/// the thread is sent to again.
#[test]
fn steers_kept_across_stop_run_in_order_on_the_next_send() {
    let fixture = Fixture::new();
    let scope = fixture.profile.path().join("aero/sessions/thread");
    let (started, ready) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (first, _) = fixture.start_at(
        "first",
        scope.clone(),
        Model::fake(|_| call("project", "summary", None, json!({}))),
        move |tools| {
            Box::new(Gated {
                tools,
                started,
                release: held,
            })
        },
    );
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    let steers = ["Then make it blue", "Then make it red"];
    for text in steers {
        steer(&first, "first", text);
    }
    first.send(Action::Cancel("first".into())).unwrap();
    release.send(()).unwrap();
    assert_eq!(complete(&first), RunResult::failure("Stopped.", true));
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = bodies.clone();
    let (second, _) = fixture.start_prompt(
        "second",
        scope,
        "Now tidy the timeline.",
        Model::fake(move |input| {
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(input).into());
            Reply::Text("Done.".into())
        }),
        |tools| Box::new(tools),
    );
    let (result, _) = complete_with_progress(&second);
    assert!(matches!(result, RunResult::Success { .. }), "{result:?}");
    let bodies = bodies.lock().unwrap();
    let order = steer_order(&bodies, &[steers[0], steers[1], "Now tidy the timeline."]);
    assert_eq!(order, [steers[0], steers[1], "Now tidy the timeline."]);
}

/// P1-6: a steer the core admits but then answers with a REQUEST_DONE that
/// is not FX_OK (FX_STALE, or an error: here the turn's steer queue is full)
/// comes back and runs as a prompt. Every steer the panel was told was
/// accepted reaches the model.
#[test]
fn a_steer_answered_not_ok_comes_back_and_reaches_the_model() {
    let fixture = Fixture::new();
    let bodies = Arc::new(Mutex::new(String::new()));
    let seen = bodies.clone();
    let (turn, _) = fixture.start(
        Model::fake(move |input| {
            seen.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(input));
            Reply::Text("ok".into())
        }),
        |tools| Box::new(tools),
    );
    let mut replies = Vec::new();
    for index in 0..80 {
        let text = format!("Steer number {index:03} now");
        match turn.send_checked(Action::Steer {
            run: "cut".into(),
            prompt: text.clone(),
            images: Vec::new(),
        }) {
            Ok(reply) => replies.push((text, reply)),
            Err(_) => break,
        }
    }
    let accepted: Vec<String> = replies
        .into_iter()
        .filter_map(|(text, reply)| {
            reply
                .recv_timeout(Duration::from_secs(20))
                .ok()
                .filter(|ok| *ok)
                .map(|_| text)
        })
        .collect();
    let (result, _) = complete_with_progress(&turn);
    assert!(matches!(result, RunResult::Success { .. }), "{result:?}");
    assert!(
        accepted.len() > 10,
        "only {} steers were accepted",
        accepted.len()
    );
    let bodies = bodies.lock().unwrap();
    let lost: Vec<_> = accepted
        .iter()
        .filter(|text| !bodies.contains(text.as_str()))
        .collect();
    assert!(
        lost.is_empty(),
        "accepted steers never reached the model: {lost:?}"
    );
}

/// A copy of the fixture with layers hidden on purpose (opacity 0, as a
/// real project's loading veils are): the project check reports each one
/// as never visible before any turn starts.
fn with_hidden_layers(fixture: Fixture) -> Fixture {
    {
        let mut seed = McpSession::new(fixture.host.clone());
        seed.dispatch("start_session", &json!({"prompt":"Seed hidden layers"}))
            .unwrap();
        let commands: Vec<_> = (0..3)
            .flat_map(|n| {
                let id = format!("veil{n}");
                [
                    json!({"type":"add_layer","id":id,"name":format!("Loading veil {n}"),
                        "layerType":"solid","from":0,"duration":4}),
                    json!({"type":"set_property","target":id,"path":"opacity","value":0}),
                ]
            })
            .collect();
        seed.dispatch("apply_commands", &json!({"commands":commands}))
            .unwrap();
        let check = seed.dispatch("check_project", &json!({})).unwrap();
        assert_eq!(
            check["issues"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|issue| issue["code"] == "never-visible")
                .count(),
            3,
            "{check}"
        );
        seed.finish(true).unwrap();
    }
    let revision = fixture
        .host
        .lock()
        .unwrap()
        .revision(&fixture.project)
        .unwrap();
    Fixture {
        revision,
        ..fixture
    }
}

/// Every request body the fake model received, in order.
fn recorded(
    f: impl Fn(usize, &str) -> Reply + Send + Sync + 'static,
) -> (Model, Arc<Mutex<Vec<String>>>) {
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = bodies.clone();
    let model = scripted(move |step, input| {
        let body = String::from_utf8_lossy(input).into_owned();
        seen.lock().unwrap().push(body.clone());
        f(step, &body)
    });
    (model, bodies)
}

/// PM-730: on a real project the gate sent the project's own hidden layers
/// to the model as problems to fix, three rounds, then failed the turn.
#[test]
fn a_rename_on_a_project_with_hidden_layers_passes_the_gate_in_one_round() {
    let fixture = with_hidden_layers(Fixture::new());
    let rev = fixture.revision;
    let (model, bodies) = recorded(move |step, _| match step {
        0 => call(
            "edit",
            "commands",
            Some(rev),
            json!({"commands":[{"type":"set_layer","target":"keeper","patch":{"name":"Harness Title"}}]}),
        ),
        _ => Reply::Text("Renamed Keep to Harness Title.".into()),
    });
    let (turn, notes) = fixture.start(model, |tools| Box::new(tools));
    assert_eq!(
        complete(&turn),
        RunResult::success("Renamed Keep to Harness Title.".into(), Access::Project)
    );
    assert_eq!(
        notes
            .try_iter()
            .filter(|v| v["type"] == "gate-continuation")
            .count(),
        0
    );
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "one edit, one answer, no repair round");
    // Their names reach the model only in its own reads of the project,
    // never as findings to fix.
    for body in bodies.iter() {
        assert!(!body.contains("never shows"), "{body}");
        assert!(!body.contains("Completion checks failed"), "{body}");
    }
    let project = fixture
        .host
        .lock()
        .unwrap()
        .read_document(&fixture.project)
        .unwrap();
    let layers = project["layers"].as_array().unwrap();
    assert!(layers.iter().any(|l| l["name"] == "Harness Title"));
    assert_eq!(
        layers
            .iter()
            .filter(|l| l["name"].as_str().unwrap().starts_with("Loading veil"))
            .count(),
        3
    );
}

/// PM-730: a problem the turn adds still fails the gate, and the repair
/// prompt names that one problem, not the project's older findings.
#[test]
fn a_problem_the_turn_adds_is_the_only_one_the_gate_names() {
    let fixture = with_hidden_layers(Fixture::new());
    let rev = fixture.revision;
    let (model, bodies) = recorded(move |step, _| match step {
        0 => call(
            "edit",
            "commands",
            Some(rev),
            json!({"commands":[{"type":"set_property","target":"flub","path":"opacity","value":0}]}),
        ),
        1 => Reply::Text("Hid the flubbed take.".into()),
        2 => call(
            "edit",
            "commands",
            Some(rev + 1),
            json!({"commands":[{"type":"set_property","target":"flub","path":"opacity","value":100}]}),
        ),
        _ => Reply::Text("Showed the take again.".into()),
    });
    let (turn, notes) = fixture.start(model, |tools| Box::new(tools));
    assert_eq!(
        complete(&turn),
        RunResult::success("Showed the take again.".into(), Access::Project)
    );
    let continued: Vec<_> = notes
        .try_iter()
        .filter(|v| v["type"] == "gate-continuation")
        .collect();
    assert_eq!(continued.len(), 1, "{continued:?}");
    let failures = continued[0]["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].as_str().unwrap().starts_with(
            "“Remove” is fully transparent wherever it plays in 0s–4s (its opacity or its group's opacity is 0), so it never shows. (layer flub"
        ),
        "{failures:?}"
    );
    let bodies = bodies.lock().unwrap();
    // The edit's own result warns about the new problem only.
    assert!(
        bodies[1].contains("“Remove” is fully transparent"),
        "{}",
        bodies[1]
    );
    let repair = &bodies[2];
    assert!(repair.contains("Completion checks failed"), "{repair}");
    for body in bodies.iter() {
        assert!(!body.contains("“Loading veil"), "{body}");
    }
}

/// PM-730: a gate still failing after its last round ends the turn in one
/// plain sentence (the panel showed "couldn't finish" and Details the raw
/// JSON list).
#[test]
fn an_exhausted_gate_ends_the_turn_in_plain_words() {
    let fixture = with_hidden_layers(Fixture::new());
    let rev = fixture.revision;
    let (model, _) = recorded(move |step, _| match step {
        0 => call(
            "edit",
            "commands",
            Some(rev),
            json!({"commands":[{"type":"set_property","target":"flub","path":"opacity","value":0}]}),
        ),
        _ => Reply::Text("Done.".into()),
    });
    let (turn, notes) = fixture.start(model, |tools| Box::new(tools));
    let RunResult::Failure { error, .. } = complete(&turn) else {
        panic!("an unrepaired problem fails the turn");
    };
    assert_eq!(
        notes
            .try_iter()
            .filter(|v| v["type"] == "gate-continuation")
            .count(),
        3
    );
    assert_eq!(
        error,
        "Powermove stopped before its checks passed: 1 problem remains (1 hidden layer). Review the result before you continue."
    );
    assert!(
        !error.contains(['{', '[', '"']) && error.len() <= 400,
        "{error}"
    );
}

/// The person's own edit through another session (the editor's writer).
fn person_edits(fixture: &Fixture, commands: Value) {
    let mut person = McpSession::new(fixture.host.clone());
    person
        .dispatch("start_session", &json!({"prompt":"The person edits"}))
        .unwrap();
    person
        .dispatch("apply_commands", &json!({ "commands": commands }))
        .unwrap();
    person.finish(true).unwrap();
}

fn revision_now(host: &Arc<Mutex<Automation>>, project: &str) -> u64 {
    host.lock().unwrap().revision(project).unwrap()
}

fn layer_value(fixture: &Fixture, id: &str, path: &str) -> Value {
    let project = fixture
        .host
        .lock()
        .unwrap()
        .read_document(&fixture.project)
        .unwrap();
    let layer = project["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["id"] == id)
        .unwrap()
        .clone();
    path.split('/').fold(layer, |v, key| v[key].clone())
}

/// PM-730 P1-6: the turn's final words are a fixed sentence with counts.
/// A plan item's wording (here JSON and a credential) never reaches them.
#[test]
fn an_exhausted_gate_never_quotes_a_plan_or_the_project() {
    const SECRET: &str = "sk-live-PM730-never-shown";
    let fixture = Fixture::new();
    let revision = fixture.revision;
    let index = fixture
        .host
        .lock()
        .unwrap()
        .read_document(&fixture.project)
        .unwrap()["layers"]
        .as_array()
        .unwrap()
        .iter()
        .position(|layer| layer["id"] == "keeper")
        .unwrap();
    let pointer = format!("/layers/{index}/name");
    let behavior = format!("Keep {{\"token\":\"{SECRET}\",\"list\":[1]}} \"Keep\"");
    let (model, _) = recorded(move |step, _| match step {
        0 => call(
            "plan",
            "add",
            None,
            json!({
                "id":"first","behavior":behavior,"check":{"type":"state","pointer":pointer,"equals":"Keep"}
            }),
        ),
        1 => call("plan", "verify", None, json!({"id":"first"})),
        2 => call(
            "edit",
            "commands",
            Some(revision),
            json!({"commands":[{"type":"set_layer","target":"keeper","patch":{"name":"Other"}}]}),
        ),
        _ => Reply::Text("Done.".into()),
    });
    let (turn, _) = fixture.start(model, |tools| Box::new(tools));
    let result = complete(&turn);
    let serialized = serde_json::to_string(&result).unwrap();
    let RunResult::Failure { error, .. } = result else {
        panic!("the regressed plan fails the turn");
    };
    assert_eq!(
        error,
        "Powermove stopped before its checks passed: 1 problem remains (1 plan check). Review the result before you continue."
    );
    assert!(
        error.len() <= 200 && !error.contains(['{', '[', '"']),
        "{error}"
    );
    assert!(!serialized.contains(SECRET), "{serialized}");
}

/// PM-730 P1-2: a layer the person hides while the turn runs is their
/// choice: no tool result and no gate round asks the model to change it.
#[test]
fn a_layer_the_person_hides_mid_turn_is_never_the_models_to_fix() {
    let fixture = Arc::new(Fixture::new());
    let person = fixture.clone();
    let (model, bodies) = recorded(move |step, _| match step {
        0 => {
            person_edits(
                &person,
                json!([{"type":"set_property","target":"flub","path":"opacity","value":0}]),
            );
            let revision = revision_now(&person.host, &person.project);
            call(
                "edit",
                "commands",
                Some(revision),
                json!({"commands":[{"type":"set_layer","target":"keeper","patch":{"name":"Harness Title"}}]}),
            )
        }
        _ => Reply::Text("Renamed.".into()),
    });
    let (turn, notes) = fixture.start(model, |tools| Box::new(tools));
    assert_eq!(
        complete(&turn),
        RunResult::success("Renamed.".into(), Access::Project)
    );
    assert_eq!(
        notes
            .try_iter()
            .filter(|v| v["type"] == "gate-continuation")
            .count(),
        0
    );
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "one edit, one answer, no repair round");
    assert!(!bodies[1].contains("“Remove”"), "{}", bodies[1]);
    assert_eq!(
        layer_value(&fixture, "flub", "p/opacity/v").as_f64(),
        Some(0.),
        "the person's choice stays"
    );
}

/// Holds the first gate check until released (so a steer queues a second
/// prompt behind the turn), and otherwise is the real tools.
struct FirstGateHeld {
    tools: Tools,
    checking: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    held: bool,
}
impl aero::Session for FirstGateHeld {
    fn context(&mut self) -> Result<Value, String> {
        aero::Session::context(&mut self.tools)
    }
    fn call(
        &mut self,
        tool: &str,
        input: &str,
        approve: &mut dyn FnMut(&str) -> bool,
    ) -> Result<aero::ToolResult, String> {
        aero::Session::call(&mut self.tools, tool, input, approve)
    }
    fn failures(&mut self) -> Vec<aero::Failure> {
        if !self.held {
            self.held = true;
            let _ = self.checking.send(());
            let _ = self.release.recv();
        }
        aero::Session::failures(&mut self.tools)
    }
    fn finish(&mut self) -> Result<(), String> {
        aero::Session::finish(&mut self.tools)
    }
    fn begin(&mut self) {
        aero::Session::begin(&mut self.tools)
    }
}

/// PM-730 P1-3: the person's next request, sent as the first one ends,
/// owes nothing the first one fixed. The turn hid a layer and showed it
/// again; the person then hides it and asks for a rename: no repair round.
/// (The core joins a steer sent at the gate into the running turn; a fresh
/// prompt's reset is a_fresh_prompt_forgets_what_the_last_turn_introduced.)
#[test]
fn a_request_sent_as_the_turn_ends_owes_nothing_the_turn_fixed() {
    let fixture = Arc::new(Fixture::new());
    let rev = fixture.revision;
    let person = fixture.clone();
    let (model, _) = recorded(move |step, _| match step {
        0 => call(
            "edit",
            "commands",
            Some(rev),
            json!({"commands":[{"type":"set_property","target":"flub","path":"opacity","value":0}]}),
        ),
        1 => call(
            "edit",
            "commands",
            Some(rev + 1),
            json!({"commands":[{"type":"set_property","target":"flub","path":"opacity","value":100}]}),
        ),
        2 => Reply::Text("Prompt one done.".into()),
        3 => {
            person_edits(
                &person,
                json!([{"type":"set_property","target":"flub","path":"opacity","value":0}]),
            );
            let revision = revision_now(&person.host, &person.project);
            call(
                "edit",
                "commands",
                Some(revision),
                json!({"commands":[{"type":"set_layer","target":"keeper","patch":{"name":"Harness Title"}}]}),
            )
        }
        _ => Reply::Text("Renamed.".into()),
    });
    let (checking, at_gate) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (turn, notes) = fixture.start(model, move |tools| {
        Box::new(FirstGateHeld {
            tools,
            checking,
            release: held,
            held: false,
        })
    });
    at_gate.recv_timeout(Duration::from_secs(10)).unwrap();
    steer(&turn, "cut", "Steer: now rename Keep please");
    release.send(()).unwrap();
    let (result, _) = complete_with_progress(&turn);
    assert_eq!(
        result,
        RunResult::success("Renamed.".into(), Access::Project)
    );
    assert_eq!(
        notes
            .try_iter()
            .filter(|v| v["type"] == "gate-continuation")
            .count(),
        0
    );
    assert_eq!(
        layer_value(&fixture, "flub", "p/opacity/v").as_f64(),
        Some(0.)
    );
}

/// The fixture with a composition `comp` the editor doesn't have open.
fn with_composition(
    fixture: Fixture,
    comp: &str,
    fps: f64,
    duration: f64,
    commands: Value,
) -> Fixture {
    {
        let mut host = fixture.host.lock().unwrap();
        let revision = host.revision(&fixture.project).unwrap();
        host.apply_commands(
            &fixture.project,
            vec![
                pm_kernel::CompCommand::Create {
                    comp_id: comp.into(),
                    settings: pm_kernel::commands::comps::NewComp {
                        name: Some(comp.into()),
                        fps: Some(fps),
                        duration: Some(duration),
                        ..Default::default()
                    },
                }
                .into(),
            ],
            "Fixture",
            pm_kernel::Origin::Mcp,
            revision,
            1,
        )
        .unwrap();
    }
    let mut seed = McpSession::new(fixture.host.clone());
    seed.dispatch("start_session", &json!({"prompt":"Seed a composition"}))
        .unwrap();
    seed.dispatch(
        "apply_commands",
        &json!({"compositionId":comp,"commands":commands}),
    )
    .unwrap();
    seed.finish(true).unwrap();
    let revision = revision_now(&fixture.host, &fixture.project);
    Fixture {
        revision,
        ..fixture
    }
}

fn comp_layers(fixture: &Fixture, comp: &str) -> Vec<Value> {
    let mut reader = McpSession::new(fixture.host.clone());
    reader
        .dispatch("start_session", &json!({"prompt":"Read"}))
        .unwrap();
    let state = reader
        .dispatch(
            "get_project_state",
            &json!({"compositionId":comp,"indexOnly":true}),
        )
        .unwrap();
    reader.finish(false).unwrap();
    state["layers"].as_array().unwrap().clone()
}

/// PM-730: an edit is checked in the composition it lands in, and an edit
/// the kernel can't take back (one aimed at a composition by id, PM-734) is
/// never refused: it is applied, the model is told what it broke, and one
/// Undo takes it back. The project and the undo history agree.
#[test]
fn an_edit_in_a_closed_composition_is_kept_and_warned_never_refused_half_applied() {
    let fixture = with_composition(
        Fixture::new(),
        "pre",
        30.,
        4.,
        json!([{"type":"add_layer","id":"inside","name":"Inside","layerType":"solid","from":0,"duration":4}]),
    );
    let rev = fixture.revision;
    let (model, bodies) = recorded(move |step, _| match step {
        0 => call(
            "edit",
            "commands",
            Some(rev),
            json!({"compositionId":"pre","commands":[{"type":"delete_layers","targets":["inside"]}]}),
        ),
        _ => Reply::Text("Done.".into()),
    });
    let (turn, _) = fixture.start(model, |tools| Box::new(tools));
    let result = complete(&turn);
    assert!(
        matches!(&result, RunResult::Failure { error, .. } if error.contains("1 blank range")),
        "{result:?}"
    );
    let bodies = bodies.lock().unwrap();
    assert!(!bodies[1].contains("Project check failed"), "{}", bodies[1]);
    assert!(
        bodies[1].contains("It is applied (one Undo takes it back)"),
        "{}",
        bodies[1]
    );
    assert!(
        bodies[1].contains("newly blank span totaling 4s"),
        "{}",
        bodies[1]
    );
    assert!(
        comp_layers(&fixture, "pre").is_empty(),
        "the edit is applied"
    );
    undo_once(&fixture);
    assert!(
        comp_layers(&fixture, "pre")
            .iter()
            .any(|layer| layer["id"] == "inside"),
        "one Undo takes it back"
    );
}

/// PM-730 P1-1: the person opening another composition mid-turn shows the
/// model none of that composition's older findings.
#[test]
fn opening_another_composition_mid_turn_surfaces_nothing_from_it() {
    let fixture = Arc::new(with_composition(
        Fixture::new(),
        "pre",
        30.,
        4.,
        json!([
            {"type":"add_layer","id":"inside","name":"Inside","layerType":"solid","from":0,"duration":4},
            {"type":"add_layer","id":"preveil","name":"Pre veil","layerType":"solid","from":0,"duration":4},
            {"type":"set_property","target":"preveil","path":"opacity","value":0}
        ]),
    ));
    let rev = fixture.revision;
    let person = fixture.clone();
    let (model, bodies) = recorded(move |step, _| match step {
        0 => call(
            "edit",
            "commands",
            Some(rev),
            json!({"commands":[{"type":"set_layer","target":"keeper","patch":{"name":"Harness Title"}}]}),
        ),
        1 => {
            let mut host = person.host.lock().unwrap();
            let revision = host.revision(&person.project).unwrap();
            host.apply_commands(
                &person.project,
                vec![
                    pm_kernel::CompCommand::Open {
                        comp_id: "pre".into(),
                    }
                    .into(),
                ],
                "Open",
                pm_kernel::Origin::Mcp,
                revision,
                1,
            )
            .unwrap();
            Reply::Text("Renamed.".into())
        }
        _ => Reply::Text("Renamed again.".into()),
    });
    let (turn, notes) = fixture.start(model, |tools| Box::new(tools));
    assert_eq!(
        complete(&turn),
        RunResult::success("Renamed.".into(), Access::Project)
    );
    assert_eq!(
        notes
            .try_iter()
            .filter(|v| v["type"] == "gate-continuation")
            .count(),
        0
    );
    for body in bodies.lock().unwrap().iter() {
        assert!(!body.contains("“Pre veil”"), "{body}");
    }
}

/// The open composition: 50 one-second layers two seconds apart, so 50
/// blank ranges, more than the public check lists.
fn strip() -> Fixture {
    let layers: Vec<_> = (0..50)
        .map(|k| {
            json!({"type":"add_layer","id":format!("s{k}"),"name":format!("Strip {k}"),
                "layerType":"solid","from":2 * k,"duration":1})
        })
        .collect();
    let fixture = with_composition(Fixture::new(), "strip", 10., 100., json!(layers));
    let mut reader = McpSession::new(fixture.host.clone());
    reader
        .dispatch("start_session", &json!({"prompt":"Read"}))
        .unwrap();
    let check = reader
        .dispatch("check_project", &json!({"compositionId":"strip"}))
        .unwrap();
    reader.finish(false).unwrap();
    let blank = check["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|issue| issue["code"] == "empty-frames")
        .unwrap()
        .clone();
    assert!(
        blank["message"].as_str().unwrap().contains("in 50 spans"),
        "{blank}"
    );
    {
        let mut host = fixture.host.lock().unwrap();
        let revision = host.revision(&fixture.project).unwrap();
        host.apply_commands(
            &fixture.project,
            vec![
                pm_kernel::CompCommand::Open {
                    comp_id: "strip".into(),
                }
                .into(),
            ],
            "Open",
            pm_kernel::Origin::Mcp,
            revision,
            1,
        )
        .unwrap();
    }
    let revision = revision_now(&fixture.host, &fixture.project);
    Fixture {
        revision,
        ..fixture
    }
}

fn strip_edit(command: Value) -> (Fixture, String) {
    let fixture = strip();
    let rev = fixture.revision;
    let (model, bodies) = recorded(move |step, _| match step {
        0 => call(
            "edit",
            "commands",
            Some(rev),
            json!({ "commands": [command] }),
        ),
        _ => Reply::Text("Done.".into()),
    });
    let (turn, _) = fixture.start(model, |tools| Box::new(tools));
    complete(&turn);
    let edit = bodies.lock().unwrap()[1].clone();
    (fixture, edit)
}

/// PM-730 P1-4: closing the first blank range is no failure, though the
/// project's 41st blank range then moves into the first forty.
#[test]
fn closing_a_gap_never_surfaces_the_projects_forty_first() {
    let (fixture, edit) =
        strip_edit(json!({"type":"set_layer","target":"s0","patch":{"duration":2}}));
    assert!(!edit.contains("Project check failed"), "{edit}");
    let s0 = comp_layers(&fixture, "strip")
        .into_iter()
        .find(|layer| layer["id"] == "s0")
        .unwrap();
    assert_eq!(s0["duration"].as_f64(), Some(2.), "{s0}");
}

/// PM-730 P1-4: a new blank range past the first forty is still caught.
#[test]
fn a_new_gap_past_the_first_forty_is_caught() {
    let (fixture, edit) = strip_edit(json!({"type":"delete_layers","targets":["s47"]}));
    assert!(edit.contains("Project check failed"), "{edit}");
    assert!(edit.contains("[[94.0,95.0]]"), "only the new range: {edit}");
    drop(fixture);
}

/// PM-730 P1-3: a fresh prompt in the same session starts owing nothing:
/// what the last turn's own edits introduced is no longer the model's.
#[test]
fn a_fresh_prompt_forgets_what_the_last_turn_introduced() {
    let fixture = Fixture::new();
    let mut tools = fixture.tools(Access::Project);
    tools
        .call(
            "edit",
            &json!({"op":"commands","revision":fixture.revision,"args":{"commands":[
                {"type":"set_property","target":"flub","path":"opacity","value":0}
            ]}})
            .to_string(),
            &mut |_| false,
        )
        .unwrap();
    let owed = aero::Session::failures(&mut tools);
    assert_eq!(owed.len(), 1, "{owed:?}");
    assert_eq!(owed[0].kind, aero::Failing::HiddenLayer);
    aero::Session::begin(&mut tools);
    assert_eq!(aero::Session::failures(&mut tools), Vec::new());
    assert_eq!(
        layer_value(&fixture, "flub", "p/opacity/v").as_f64(),
        Some(0.)
    );
}

/// Media states for the offline-media test: "live" plays, anything else
/// is offline.
struct Library;
impl pm_mcp::media::MediaRegistry for Library {
    fn state(&self, _: &str, id: &str) -> pm_mcp::media::MediaState {
        pm_mcp::media::MediaState {
            available: id == "live",
            duration: (id == "live").then_some(30.),
            has_audio: id == "live",
            ..Default::default()
        }
    }
}

/// PM-730: switching a layer to media that is offline is the edit's doing:
/// it is caught, though media that goes offline on its own never is.
#[test]
fn switching_a_track_to_offline_media_is_caught() {
    let fixture = Fixture::new();
    {
        let mut host = fixture.host.lock().unwrap();
        let commands: Vec<pm_kernel::Command> = ["live", "gone"]
            .into_iter()
            .map(|id| {
                pm_kernel::MediaCommand::AddAsset {
                    asset_id: id.into(),
                    asset: pm_model::json::parse(
                        &json!({"id":id,"name":id,"kind":"audio","dur":30,"persisted":true})
                            .to_string(),
                    )
                    .unwrap(),
                }
                .into()
            })
            .collect();
        let revision = host.revision(&fixture.project).unwrap();
        host.apply_commands(
            &fixture.project,
            commands,
            "Synthetic media",
            pm_kernel::Origin::Mcp,
            revision,
            1,
        )
        .unwrap();
        host.media_registry = Some(Arc::new(Library));
    }
    person_edits(
        &fixture,
        json!([{"type":"add_layer","id":"track","name":"Track","layerType":"audio",
            "from":0,"duration":4,"content":{"asset":"live"}}]),
    );
    let fixture = Fixture {
        revision: revision_now(&fixture.host, &fixture.project),
        ..fixture
    };
    let rev = fixture.revision;
    let (model, bodies) = recorded(move |step, _| match step {
        0 => call(
            "edit",
            "commands",
            Some(rev),
            json!({"commands":[{"type":"set_content","target":"track","patch":{"asset":"gone"}}]}),
        ),
        _ => Reply::Text("Switched.".into()),
    });
    let (turn, _) = fixture.start(model, |tools| Box::new(tools));
    complete(&turn);
    let bodies = bodies.lock().unwrap();
    assert!(bodies[1].contains("media-offline"), "{}", bodies[1]);
    assert_eq!(
        layer_value(&fixture, "track", "d/asset"),
        "live",
        "the refused switch is taken back"
    );
}

/// The picker's level on every route the Powermove harness runs, as the
/// request leaves Powermove (PM-746): emulate (`PM_EMULATE`, its executable)
/// answers OpenAI Responses, Chat Completions and Anthropic Messages on a
/// loopback port of its own. Fake credentials only, in memory.
mod routes {
    use super::*;
    use pm_agents::edge::{Chunk, Loopback, MemorySecrets, Net, NetRequest};
    use pm_agents::launch::Environment;
    use pm_agents::{codex_auth, compatible, sources};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::AtomicBool;

    struct Emulate {
        child: Child,
        base: String,
        _dir: tempfile::TempDir,
    }
    impl Drop for Emulate {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    /// emulate with `rules`, or `None` (the test is skipped, and says so)
    /// when `PM_EMULATE` is unset.
    fn emulate(rules: Value) -> Option<Emulate> {
        let Some(bin) = std::env::var_os("PM_EMULATE") else {
            eprintln!("skipped: PM_EMULATE (emulate's executable) is not set");
            return None;
        };
        let dir = tempfile::tempdir().unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let models = [
            "claude-opus-5-5",
            "claude-opus-4-6",
            "claude-haiku-4-5-20251001",
            "claude-3-7-sonnet-20250219",
        ];
        let seed = json!({"ai":{"strict":false,"models":models,"rules":rules}});
        let path = dir.path().join("seed.yaml");
        std::fs::write(&path, seed.to_string()).unwrap();
        let child = Command::new(bin)
            .args(["start", "--service", "ai", "--host", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .arg("--seed")
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let emulate = Emulate {
            child,
            base: format!("http://127.0.0.1:{port}"),
            _dir: dir,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let rules = format!("{}/_ai/rules", emulate.base);
        let get = NetRequest {
            method: "GET",
            url: &rules,
            headers: &[],
            body: b"",
        };
        while pm_agents::edge::fetch(&Loopback, &get).map_or(true, |(code, _)| code != 200) {
            assert!(Instant::now() < deadline, "emulate did not start");
            std::thread::sleep(Duration::from_millis(100));
        }
        Some(emulate)
    }

    /// One request as it left Powermove's fetch edge (credential headers
    /// included), and the reply's bytes.
    #[derive(Clone, Debug)]
    struct Sent {
        url: String,
        headers: Vec<(String, String)>,
        body: Value,
        reply: String,
    }
    impl Sent {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }
    /// The app's network as the routes see it: provider origins go to emulate.
    #[derive(Debug)]
    struct Wire {
        base: String,
        sent: Mutex<Vec<Sent>>,
    }
    impl Net for Wire {
        fn stream(
            &self,
            request: &NetRequest<'_>,
            stop: &AtomicBool,
            on: &mut dyn FnMut(Chunk<'_>) -> Result<(), String>,
        ) -> Result<(), String> {
            let url = [
                "https://chatgpt.com",
                "https://api.openai.com",
                "https://api.anthropic.com",
            ]
            .iter()
            .find_map(|origin| request.url.strip_prefix(origin))
            .map_or_else(
                || request.url.to_owned(),
                |rest| format!("{}{rest}", self.base),
            );
            let mut reply = Vec::new();
            let streamed = Loopback.stream(
                &NetRequest {
                    url: &url,
                    ..*request
                },
                stop,
                &mut |chunk| {
                    if let Chunk::Data(bytes) = &chunk {
                        reply.extend_from_slice(bytes);
                    }
                    on(chunk)
                },
            );
            self.sent.lock().unwrap().push(Sent {
                url: request.url.into(),
                headers: request.headers.to_vec(),
                body: serde_json::from_slice(request.body).unwrap_or_default(),
                reply: String::from_utf8_lossy(&reply).into(),
            });
            streamed
        }
    }

    /// The saved custom endpoint (Settings › Providers › Custom).
    #[derive(Debug)]
    struct Custom(String);
    impl compatible::ChatHost for Custom {
        fn saved(&self) -> Option<compatible::Saved> {
            Some(compatible::Saved {
                base_url: self.0.clone(),
                model: "gpt-5.5".into(),
                vision: false,
                models: vec![],
            })
        }
        fn key(&self, _: &str) -> Option<String> {
            Some("custom-fake-key".into())
        }
        fn stream(
            &self,
            _: &str,
            _: Option<&str>,
            _: Vec<u8>,
            _: &AtomicBool,
            _: &mut dyn FnMut(&[u8]) -> Result<(), String>,
        ) -> Result<u16, String> {
            Err("unused".into())
        }
    }

    struct Routes {
        fixture: Fixture,
        env: Environment,
        wire: Arc<Wire>,
        _emulate: Emulate,
    }
    fn routes(rules: Value) -> Option<Routes> {
        let emulate = emulate(rules)?;
        let fixture = Fixture::new();
        let wire = Arc::new(Wire {
            base: emulate.base.clone(),
            sent: Mutex::default(),
        });
        let env = Environment {
            profile: fixture.profile.path().join("app"),
            keychain_prefix: "pm-effort-routes".into(),
            secrets: Some(Arc::new(MemorySecrets::default())),
            net: Some(wire.clone()),
            compatible: Some(Arc::new(Custom(format!("{}/custom/v1", emulate.base)))),
            vars: [(
                "PM_CODEX_KEYRING_SERVICE".to_owned(),
                "pm-effort-routes Codex Auth".to_owned(),
            )]
            .into(),
            ..Environment::default()
        };
        // Codex's login item is keyed by its home's canonical path.
        std::fs::create_dir_all(codex_auth::codex_home(&env)).unwrap();
        Some(Routes {
            fixture,
            env,
            wire,
            _emulate: emulate,
        })
    }
    impl Routes {
        fn secret(&self, service: &str, account: &str, value: &[u8]) {
            self.env
                .secrets
                .as_ref()
                .unwrap()
                .write(service, account, value)
                .unwrap();
        }
        /// Codex's shared login item: an OpenAI API key, or a ChatGPT sign-in.
        fn codex_login(&self, item: Value) {
            let account = codex_auth::store_key(&codex_auth::codex_home(&self.env));
            self.secret(
                &codex_auth::service(&self.env),
                &account,
                item.to_string().as_bytes(),
            );
        }
        /// One turn on the picker's `id` at `effort`; the model request it sent.
        fn turn(&self, id: &str, effort: &str, prompt: &str) -> Vec<Sent> {
            let before = self.wire.sent.lock().unwrap().len();
            let live = sources::live(&self.env, id, effort).unwrap();
            let run = format!("{}-{effort}", id.replace(['/', '.'], "-"));
            let scope = self.fixture.profile.path().join("aero/sessions").join(&run);
            let (turn, _notes) = self.fixture.start_choice(
                &run,
                scope,
                prompt,
                (Some(id), Some(effort)),
                Model::live(live),
                |tools| Box::new(tools),
            );
            let result = complete(&turn);
            let sent = self.wire.sent.lock().unwrap()[before..].to_vec();
            assert!(
                matches!(result, RunResult::Success { .. }),
                "{id} at {effort}: {result:?}; sent {sent:#?}"
            );
            assert!(!sent.is_empty(), "{id} at {effort} sent nothing");
            sent
        }
    }

    fn hello() -> Value {
        json!([{"id":"hello","reply":[{"type":"reasoning","text":"Considering the request."},
            {"type":"text","text":"Hello from emulate."}]}])
    }

    #[test]
    fn the_openai_key_route_sends_each_picker_level_as_reasoning_effort() {
        let Some(r) = routes(hello()) else { return };
        r.codex_login(json!({"auth_mode":"apikey","OPENAI_API_KEY":"sk-fake-openai-1234"}));
        for level in compatible::efforts("gpt-6-sol") {
            let sent = r.turn("openai/gpt-6-sol", &level, "Say hello.");
            for request in &sent {
                assert_eq!(request.url, "https://api.openai.com/v1/chat/completions");
                assert_eq!(request.body["reasoning_effort"], level.as_str(), "{level}");
                assert_eq!(
                    request.header("authorization"),
                    Some("Bearer sk-fake-openai-1234")
                );
            }
        }
    }

    #[test]
    fn the_chatgpt_sign_in_route_sends_each_picker_level_as_the_reasoning_effort() {
        use base64::Engine as _;
        let Some(r) = routes(hello()) else { return };
        let claims =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":4102444800}"#);
        r.codex_login(json!({"auth_mode":"chatgpt","tokens":{"id_token":"x",
            "access_token":format!("h.{claims}.s"),"refresh_token":"r","account_id":"acct-fake"}}));
        for level in compatible::efforts("gpt-6-sol") {
            let sent = r.turn("chatgpt/gpt-6-sol", &level, "Say hello.");
            for request in &sent {
                assert_eq!(
                    request.url,
                    "https://chatgpt.com/backend-api/codex/responses"
                );
                assert_eq!(
                    request.body["reasoning"]["effort"],
                    level.as_str(),
                    "{level}"
                );
                assert_eq!(request.header("chatgpt-account-id"), Some("acct-fake"));
            }
        }
    }

    #[test]
    fn the_custom_route_sends_each_picker_level_as_reasoning_effort() {
        let Some(r) = routes(hello()) else { return };
        let levels = compatible::efforts("gpt-5.5");
        assert!(!levels.is_empty());
        for level in levels {
            let sent = r.turn("custom/gpt-5.5", &level, "Say hello.");
            for request in &sent {
                assert_eq!(
                    request.url,
                    format!("{}/custom/v1/chat/completions", r._emulate.base)
                );
                assert_eq!(request.body["reasoning_effort"], level.as_str(), "{level}");
                assert_eq!(
                    request.header("authorization"),
                    Some("Bearer custom-fake-key")
                );
            }
        }
    }

    const ANTHROPIC_KEY: &str = "sk-ant-fake-abcdef12";

    /// Each Claude family's own control: adaptive thinking with the effort, or
    /// a thinking budget below the output limit; the key as `x-api-key`
    /// with the API version, never a bearer; prompt caching on.
    #[test]
    fn the_anthropic_key_route_sends_each_picker_level_as_that_models_thinking_fields() {
        let Some(r) = routes(hello()) else { return };
        r.secret(
            &sources::anthropic_service(&r.env),
            "api-key",
            ANTHROPIC_KEY.as_bytes(),
        );
        for model in [
            "claude-opus-5-5",
            "claude-opus-4-6",
            "claude-haiku-4-5-20251001",
        ] {
            let levels = sources::claude_efforts(model);
            assert!(!levels.is_empty(), "{model}");
            for level in levels {
                let fields = sources::claude_effort_fields(model, &level).unwrap();
                let sent = r.turn(&format!("anthropic/{model}"), &level, "Say hello.");
                for request in &sent {
                    let body = &request.body;
                    assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
                    assert_eq!(body["thinking"], fields["thinking"], "{model} {level}");
                    assert_eq!(
                        body["output_config"], fields["output_config"],
                        "{model} {level}"
                    );
                    assert_eq!(body["max_tokens"], sources::CLAUDE_MAX_TOKENS);
                    let budget = body["thinking"]["budget_tokens"].as_u64().unwrap_or(0);
                    assert!(budget < u64::from(sources::CLAUDE_MAX_TOKENS));
                    assert_eq!(body["cache_control"], json!({"type":"ephemeral"}));
                    assert!(body.get("tool_choice").is_none(), "{body}");
                    assert_eq!(request.header("x-api-key"), Some(ANTHROPIC_KEY));
                    assert_eq!(request.header("anthropic-version"), Some("2023-06-01"));
                    assert_eq!(request.header("authorization"), None);
                }
            }
        }
        // A level the chosen model doesn't take runs as the nearest one below.
        let sent = r.turn("anthropic/claude-opus-4-6", "xhigh", "Say hello.");
        assert_eq!(sent[0].body["output_config"]["effort"], "high");
    }

    /// The picker offers each Claude model the levels it takes, from the
    /// key's own model list, and none for a model this table doesn't know.
    #[test]
    fn the_anthropic_model_list_offers_each_models_own_levels() {
        let Some(r) = routes(hello()) else { return };
        let models =
            sources::key_models(&*r.wire, sources::Source::AnthropicKey, ANTHROPIC_KEY).unwrap();
        let levels = |id: &str| {
            let model = models.iter().find(|m| m.id == id).unwrap();
            (model.efforts.join(" "), model.default_effort.clone())
        };
        let high = Some("high".to_owned());
        assert_eq!(
            levels("claude-opus-5-5"),
            ("low medium high xhigh max".into(), high.clone())
        );
        assert_eq!(
            levels("claude-opus-4-6"),
            ("low medium high max".into(), high.clone())
        );
        assert_eq!(
            levels("claude-haiku-4-5-20251001"),
            ("low medium high".into(), high)
        );
        assert_eq!(levels("claude-3-7-sonnet-20250219"), (String::new(), None));
    }

    /// After a tool call, the next request sends the thinking block back
    /// first in that assistant turn, with the signature the model gave it,
    /// and keeps the cached prefix byte for byte.
    #[test]
    fn the_anthropic_route_sends_a_thinking_signature_back_after_a_tool_call() {
        let rules = json!([
            {"id":"look","match":{"hasToolResult":false},"times":1,"reply":[
                {"type":"reasoning","text":"Reading the layer before answering."},
                {"type":"tool_call","name":"project","arguments":{"op":"layer","args":{"layerId":"keeper"}}}]},
            {"id":"answer","match":{"hasToolResult":true},"reply":[
                {"type":"reasoning","text":"The layer is there."},
                {"type":"text","text":"The layer is called Keep."}]}
        ]);
        let Some(r) = routes(rules) else { return };
        r.secret(
            &sources::anthropic_service(&r.env),
            "api-key",
            ANTHROPIC_KEY.as_bytes(),
        );
        let sent = r.turn(
            "anthropic/claude-opus-5-5",
            "high",
            "What is the first layer called?",
        );
        assert_eq!(sent.len(), 2, "{sent:#?}");
        let (first, second) = (&sent[0], &sent[1]);
        // The signature the first reply streamed.
        let signature = first
            .reply
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .find_map(|event| event["delta"]["signature"].as_str().map(str::to_owned))
            .expect("a signature in the first reply");
        assert!(!signature.is_empty());
        let messages = second.body["messages"].as_array().unwrap();
        let assistant = messages
            .iter()
            .find(|m| m["role"] == "assistant")
            .expect("the tool turn's assistant message");
        let content = assistant["content"].as_array().unwrap();
        assert_eq!(
            content[0],
            json!({"type":"thinking","thinking":"Reading the layer before answering.","signature":signature})
        );
        assert!(content.iter().any(|block| block["type"] == "tool_use"));
        // The cache's prefix: the same tools and system prompt, and the
        // first request's messages unchanged at the start of the second.
        assert_eq!(second.body["tools"], first.body["tools"]);
        assert_eq!(second.body["system"], first.body["system"]);
        let opened = first.body["messages"].as_array().unwrap();
        assert_eq!(&messages[..opened.len()], &opened[..]);
        assert_eq!(second.body["cache_control"], json!({"type":"ephemeral"}));
        assert_eq!(second.body["output_config"]["effort"], "high");
    }
}
