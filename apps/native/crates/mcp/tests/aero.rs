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
        let host = self.host.clone();
        let project = self.project.clone();
        let revision = self.revision;
        let scope = self.profile.path().join("aero/sessions/cut");
        let (report, notes) = mpsc::channel();
        let handle = aero::start(
            Request {
                run: "cut".into(),
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
            "cut".into(),
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
