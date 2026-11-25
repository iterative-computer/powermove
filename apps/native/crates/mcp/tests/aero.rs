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
        let host = self.host.clone();
        let project = self.project.clone();
        let revision = self.revision;
        let (report, notes) = mpsc::channel();
        let handle = aero::start(
            Request {
                run: run.into(),
                prompt: "Cut the flubbed take.".into(),
                provider: Provider::Aero,
                model: None,
                effort: None,
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
    fn failures(&mut self) -> Vec<String> {
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
    assert_eq!(complete(&turn), RunResult::failure("Stopped.", true));
    release.send(()).unwrap();
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
        assert_eq!(
            init["result"]["capabilities"]["experimental"]["powermoveEval"].is_object(),
            true
        );
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

#[test]
fn stop_during_a_real_edit_leaves_one_atomic_undoable_change() {
    struct Delayed {
        tools: Tools,
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        done: mpsc::Sender<()>,
    }
    impl aero::Session for Delayed {
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
            let result = aero::Session::call(&mut self.tools, tool, input, approve);
            let _ = self.done.send(());
            result
        }
        fn failures(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn finish(&mut self) -> Result<(), String> {
            aero::Session::finish(&mut self.tools)
        }
    }
    let fixture = Fixture::new();
    let revision = fixture.revision;
    let (started, ready) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (done, finished) = mpsc::channel();
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
            Box::new(Delayed {
                tools,
                started,
                release: held,
                done,
            })
        },
    );
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    turn.send(Action::Cancel("cut".into())).unwrap();
    assert_eq!(complete(&turn), RunResult::failure("Stopped.", true));
    // The tool already holds its lease: it finishes as one transaction.
    release.send(()).unwrap();
    // The edit's lease commits when the call returns (reads may see its
    // changes earlier, before its history entry exists).
    finished.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(keeper_name(&fixture), "Stopped edit");
    let layers = |fixture: &Fixture| {
        fixture
            .host
            .lock()
            .unwrap()
            .read_document(&fixture.project)
            .unwrap()["layers"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(layers(&fixture), 1);
    drop(turn);
    undo_once(&fixture);
    assert_eq!(keeper_name(&fixture), "Keep");
    assert_eq!(layers(&fixture), 2, "both commands undo as one step");
}

#[test]
fn review_grades_in_a_fresh_session_that_never_sees_the_generator() {
    let fixture = Fixture::new();
    let evaluated = Arc::new(AtomicUsize::new(0));
    let count = evaluated.clone();
    let step = AtomicUsize::new(0);
    let (turn, _) = fixture.start(
        Model::fake(move |input| {
            let text = String::from_utf8_lossy(input);
            if text.contains("You are an independent evaluator") {
                count.fetch_add(1, Ordering::Relaxed);
                // Only the evidence reaches the evaluator: no generator
                // instructions, prompt or tool history.
                assert!(!text.contains("You are Aero"), "{text}");
                assert!(!text.contains("Cut the flubbed take"), "{text}");
                assert!(text.contains("Two layers remain"), "{text}");
                return Reply::Text(
                    json!({"evaluator":"fresh","rubric":"cut","scores":{"clean":{"score":4,"evidence":"Two layers remain"}}})
                        .to_string(),
                );
            }
            match step.fetch_add(1, Ordering::Relaxed) {
                0 => call(
                    "review",
                    "evaluate",
                    None,
                    json!({"result":{"summary":"Two layers remain"},"rubric":{"id":"cut","criteria":["clean"]}}),
                ),
                _ => {
                    assert!(text.contains(r#"\"evaluator\":\"fresh\""#), "{text}");
                    Reply::Text("Reviewed.".into())
                }
            }
        }),
        |tools| Box::new(tools),
    );
    assert_eq!(
        complete(&turn),
        RunResult::success("Reviewed.".into(), Access::Project)
    );
    assert_eq!(evaluated.load(Ordering::Relaxed), 1);
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
    fn failures(&mut self) -> Vec<String> {
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
