//! Real V8 panel, real kernel actor, real provider pipes; the provider is fake.
use super::*;
use pm_agents::{Provider, launch::Environment};
use pm_kernel::{KernelActor, KernelConfig, OpenSource};
use pm_surface::events::EventDetail;
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn wait(ticket: Ticket<Evaluation>) -> pm_runtime_v8::Result<Evaluation> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = ticket.try_take() {
            return result;
        }
        assert!(Instant::now() < deadline, "v4 ticket timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn finish(ticket: Ticket<Evaluation>) {
    wait(ticket).expect("v4 worker execution");
}
fn actor() -> KernelActor {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project = pm_model::json::parse(
        r##"{"id":"synthetic","name":"Synthetic","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[]}"##,
    )
    .unwrap();
    actor
        .handle()
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    actor
}
fn environment(profile: &Path, mode: &str) -> Environment {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/fake-provider.py");
    let bundled = profile.join("Resources/codex/bin");
    std::fs::create_dir_all(&bundled).unwrap();
    std::fs::copy(&fake, bundled.join("codex")).unwrap();
    Environment {
        profile: profile.into(),
        resources: Some(profile.join("Resources")),
        app_version: "test".into(),
        executable: PathBuf::from("/bin/false"),
        vars: BTreeMap::from([
            ("PATH".into(), std::env::var("PATH").unwrap()),
            ("PM_FAKE_PROVIDER".into(), mode.into()),
            ("CLAUDE_BINARY".into(), fake.to_string_lossy().into_owned()),
        ]),
        tools: vec!["get_project_state".into()],
        composition_tools: vec!["get_project_state".into()],
    }
}
struct Agent {
    panels: Builtins,
    latest: Option<Publication>,
    typed: String,
}
impl Agent {
    /// A browser that only remembers what it was asked to open.
    fn mount_recording(
        actor: &KernelActor,
        environment: Environment,
        opened: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> Self {
        let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
        panels.set_agent_environment(
            environment,
            std::sync::Arc::new(move |url| {
                opened.lock().unwrap().push(url.to_owned());
                Ok(())
            }),
        );
        finish(panels.mount("agent").unwrap());
        finish(panels.show("agent").unwrap());
        Self {
            panels,
            latest: None,
            typed: String::new(),
        }
    }
    fn mount(actor: &KernelActor, environment: Environment) -> Self {
        Self::mount_recording(actor, environment, Default::default())
    }
    /// The panel shown with no project open.
    fn mount_application(
        actor: &KernelActor,
        environment: Environment,
        carry: Option<AgentCarry>,
    ) -> Self {
        let panels = Builtins::application(actor.handle(), || {}, carry).unwrap();
        panels.set_agent_environment(environment, std::sync::Arc::new(|_| Ok(())));
        let mut panels = panels;
        finish(panels.mount("agent").unwrap());
        finish(panels.show("agent").unwrap());
        Self {
            panels,
            latest: None,
            typed: String::new(),
        }
    }
    /// Sign-in status arrives from a worker; a run needs it.
    fn connected(&mut self, provider: Provider) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.panels.broker.agent.available(provider) {
            assert!(
                Instant::now() < deadline,
                "{:?}",
                self.panels.broker.agent.connection(provider)
            );
            thread::sleep(Duration::from_millis(5));
        }
        self.text();
    }
    /// Acts on the publication last seen, as a click does: a streaming run may
    /// publish again before the event arrives.
    fn press(&mut self, id: &str, event: &str, detail: EventDetail) {
        self.text();
        self.press_seen(id, event, detail);
    }
    fn press_seen(&mut self, id: &str, event: &str, detail: EventDetail) {
        let target = self.node(id);
        let (generation, revision) = {
            let p = self.latest.as_ref().expect("agent publication");
            (p.generation, p.surface.revision())
        };
        let driven = self
            .panels
            .event("agent", generation, revision, target, event, detail);
        match driven.map(wait) {
            Ok(Ok(_)) => {
                self.text();
            }
            other => panic!("{id}: {other:?}"),
        }
    }
    fn revision(&self) -> u64 {
        self.panels
            .status()
            .iter()
            .find(|p| p["panel"] == "agent")
            .and_then(|p| p["surfaceRevision"].as_u64())
            .unwrap()
    }
    fn change(&mut self, id: &str, value: &str) {
        let value = Some(json!(value));
        self.press(
            id,
            "change",
            EventDetail {
                value,
                ..Default::default()
            },
        );
    }
    fn click(&mut self, id: &str) {
        self.press(id, "click", EventDetail::default());
    }
    fn send(&mut self, text: &str) {
        self.type_text(text);
        self.key("Enter", false);
        self.typed.clear();
    }
    fn surface(&mut self) -> &Publication {
        if let Some(latest) = self.panels.take_publications().pop() {
            self.latest = Some(latest);
        }
        self.latest.as_ref().expect("agent publication")
    }
    fn node(&mut self, id: &str) -> u64 {
        let surface = &self.surface().surface;
        surface
            .nodes()
            .iter()
            .find(|(_, n)| n.props.get("id").and_then(Value::as_str) == Some(id))
            .map(|(key, _)| *key)
            .unwrap_or_else(|| panic!("no node {id}"))
    }
    fn key(&mut self, key: &str, shift: bool) {
        self.press(
            "agent-composer-text",
            "keydown",
            EventDetail {
                key: Some(key.into()),
                shift_key: Some(shift),
                ..Default::default()
            },
        );
    }
    /// The native composer reports its whole text as it changes.
    fn type_text(&mut self, text: &str) {
        self.typed.push_str(text);
        let value = self.typed.clone();
        self.press(
            "agent-composer-text",
            "input",
            EventDetail {
                value: Some(json!(value)),
                ..Default::default()
            },
        );
    }
    fn agent(&self) -> agent_host::SharedAgent {
        let backend = self
            .panels
            .broker
            .entry("agent")
            .unwrap()
            .lock()
            .unwrap()
            .backend
            .clone();
        let backend = backend.lock().unwrap();
        backend.native.agent.clone().unwrap()
    }
    fn wait(&self, until: impl Fn(&agent_host::AgentState) -> bool) {
        let agent = self.agent();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !until(&agent.lock().unwrap()) {
            assert!(Instant::now() < deadline, "agent run timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn text(&mut self) -> String {
        for (_, ticket) in self.panels.publish() {
            finish(ticket.unwrap());
        }
        let errors = self.panels.take_errors();
        assert!(errors.is_empty(), "{errors:?}");
        self.surface()
            .surface
            .nodes()
            .values()
            .map(|n| n.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
fn thread_id(agent: &agent_host::SharedAgent) -> String {
    agent.lock().unwrap().panel.active_id().to_owned()
}
fn idle(state: &agent_host::AgentState) -> bool {
    state.runs.is_empty()
}
#[test]
fn typed_message_runs_a_bundled_provider_and_its_reply_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    panel.connected(Provider::Chatgpt);
    assert!(!panel.text().contains("Connect Your Agent"));
    panel.send("Hi");
    panel.wait(idle);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    {
        let state = agent.lock().unwrap();
        let session = state.panel.session(&thread).unwrap();
        assert!(session.run.is_none());
        assert_eq!(session.draft.text(), "");
    }
    let text = panel.text();
    assert!(
        text.contains("Saw: Hi"),
        "{text}\n{}",
        agent.lock().unwrap().panel.raw_archive()
    );
    let archive = actor.handle().state().settings["agentThreads.synthetic"].clone();
    drop(agent);
    panel.panels.close();
    let stored = pm_model::json::to_canonical_string(&archive);
    assert!(stored.contains("Saw: Hi"), "{stored}");
    assert!(
        stored.contains("\"lastRunStatus\":\"completed\""),
        "{stored}"
    );
    let mut reopened = Agent::mount(&actor, environment(root.path(), "reply"));
    reopened.connected(Provider::Chatgpt);
    assert!(reopened.text().contains("Saw: Hi"));
    reopened.panels.close();
}
#[test]
fn typing_lands_after_the_panel_republished_for_something_else() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    panel.connected(Provider::Chatgpt);
    panel.text();
    // A status or a streaming run republishes before the keystroke arrives.
    panel.panels.broker.agent.touch();
    panel.press_seen(
        "agent-composer-text",
        "input",
        EventDetail {
            value: Some(json!("Hi")),
            ..Default::default()
        },
    );
    let agent = panel.agent();
    let thread = thread_id(&agent);
    let text = agent
        .lock()
        .unwrap()
        .panel
        .session(&thread)
        .unwrap()
        .draft
        .text();
    assert_eq!(text, "Hi");
    drop(agent);
    panel.panels.close();
}
#[test]
fn stop_cancels_a_held_run_through_the_provider() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "hold"));
    panel.connected(Provider::Chatgpt);
    panel.send("Wait");
    panel.click("agent-stop");
    panel.wait(idle);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    let state = agent.lock().unwrap();
    assert!(state.panel.session(&thread).unwrap().run.is_none());
    assert_eq!(
        state.panel.raw_archive()["threads"][0]["lastRunStatus"],
        "cancelled"
    );
    drop(state);
    panel.panels.close();
}
#[test]
fn stop_lands_while_a_stream_republishes_the_panel() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "stream"));
    panel.connected(Provider::Chatgpt);
    panel.send("Go");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !panel.text().contains("token") {
        assert!(Instant::now() < deadline, "no stream");
        thread::sleep(Duration::from_millis(5));
    }
    // The user sees this revision; the stream publishes past it before the click.
    let seen = panel.latest.as_ref().unwrap().surface.revision();
    while panel.revision() < seen + 3 {
        assert!(Instant::now() < deadline, "stream stopped publishing");
        thread::sleep(Duration::from_millis(5));
    }
    panel.press_seen("agent-stop", "click", EventDetail::default());
    panel.wait(idle);
    let agent = panel.agent();
    let state = agent.lock().unwrap();
    assert_eq!(
        state.panel.raw_archive()["threads"][0]["lastRunStatus"],
        "cancelled"
    );
    let receipts = &state.panel.raw_archive()["threads"][0]["nativeRunEvents"];
    assert!(receipts.as_array().unwrap().len() < 10, "{receipts}");
    drop(state);
    panel.panels.close();
}
#[test]
fn an_application_run_outlives_its_window_and_shows_on_return() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut home = Agent::mount_application(&actor, environment(root.path(), "slow"), None);
    home.connected(Provider::Chatgpt);
    home.send("Make a project");
    // The run opens a project: the application window goes, its run stays.
    let carry = home.panels.carry_agent().expect("a live run");
    home.panels.close();
    drop(home);
    let state = carry.0.as_ref().unwrap().1.clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !state.lock().unwrap().runs.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the carried run stopped delivering"
        );
        thread::sleep(Duration::from_millis(5));
    }
    drop(state);
    let mut back = Agent::mount_application(&actor, environment(root.path(), "slow"), Some(carry));
    back.connected(Provider::Chatgpt);
    assert!(
        back.text().contains("Saw: Make a project"),
        "{}",
        back.text()
    );
    let agent = back.agent();
    assert_eq!(
        agent.lock().unwrap().panel.raw_archive()["threads"][0]["lastRunStatus"],
        "completed"
    );
    drop(agent);
    back.panels.close();
}
#[test]
fn dropping_a_carried_run_cancels_it() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut home = Agent::mount_application(&actor, environment(root.path(), "hold"), None);
    home.connected(Provider::Chatgpt);
    home.send("Wait");
    let state = home.agent();
    let carry = home.panels.carry_agent().expect("a live run");
    home.panels.close();
    drop(home);
    assert!(
        !state.lock().unwrap().runs.is_empty(),
        "closing kept the run"
    );
    drop(carry);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !state.lock().unwrap().runs.is_empty() {
        assert!(Instant::now() < deadline, "the run was not cancelled");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        state.lock().unwrap().panel.raw_archive()["threads"][0]["lastRunStatus"],
        "cancelled"
    );
}
#[test]
fn signing_out_during_a_run_keeps_its_conversation_and_stop() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "hold"));
    panel.connected(Provider::Chatgpt);
    panel.send("Wait");
    let agent = panel.agent();
    let thread = thread_id(&agent);
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_some());
    let link = panel.panels.broker.agent.clone();
    link.accounts().unwrap().disconnect(Provider::Chatgpt);
    let deadline = Instant::now() + Duration::from_secs(10);
    while link.connection(Provider::Chatgpt).0 != "unavailable" {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!panel.text().contains("Connect Your Agent"));
    panel.click("agent-stop");
    panel.wait(idle);
    assert_eq!(
        agent.lock().unwrap().panel.raw_archive()["threads"][0]["lastRunStatus"],
        "cancelled"
    );
    assert!(panel.text().contains("Connect Your Agent"));
    panel.panels.close();
}
#[test]
fn claude_question_is_answered_with_the_cards_own_buttons() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    panel.connected(Provider::Claude);
    panel.change("agent-provider", "claude");
    panel.connected(Provider::Claude);
    panel.send("Style");
    let agent = panel.agent();
    let thread = thread_id(&agent);
    let open =
        |state: &agent_host::AgentState| state.panel.session(&thread).unwrap().needs_answer();
    panel.wait(open);
    assert!(panel.text().contains("Choose a style?"));
    // thread_read sees the live run: its trace and the open question.
    let live = panel.panels.agent_thread_live(&thread).unwrap();
    assert_eq!(live["busy"], true, "{live}");
    assert!(live["runId"].is_string(), "{live}");
    let question = &live["questions"][0];
    assert_eq!(question["blocking"], true, "{live}");
    assert!(question["itemId"].is_string(), "{live}");
    assert!(
        question["questions"]
            .to_string()
            .contains("Choose a style?"),
        "{live}"
    );
    assert!(panel.panels.agent_thread_live("no-such-thread").is_none());
    panel.click("live:question-question-0-Soft");
    panel.click("live:question-answer");
    panel.wait(idle);
    assert!(panel.text().contains("Using Soft"));
    let settled = panel.panels.agent_thread_live(&thread).unwrap();
    assert_eq!(settled["busy"], false);
    assert_eq!(settled["activity"], "");
    assert_eq!(settled["questions"], json!([]));
    panel.panels.close();
}
#[test]
fn supervised_approval_reaches_the_process_and_deny_too() {
    for (choice, reply) in [("Allow once", "Using allow"), ("Deny", "Using deny")] {
        let root = tempfile::tempdir().unwrap();
        let actor = actor();
        let mut panel = Agent::mount(&actor, environment(root.path(), "approval"));
        panel.connected(Provider::Claude);
        panel.change("agent-provider", "claude");
        panel.change("agent-access", "supervised");
        panel.connected(Provider::Claude);
        panel.send("Build");
        let agent = panel.agent();
        let thread = thread_id(&agent);
        let open =
            |state: &agent_host::AgentState| state.panel.session(&thread).unwrap().needs_answer();
        panel.wait(open);
        assert!(panel.text().contains("Run this command?"));
        let answer = agent
            .lock()
            .unwrap()
            .panel
            .session(&thread)
            .unwrap()
            .trace
            .rows
            .values()
            .find(|row| row["kind"] == "question")
            .map(|row| row["id"].as_str().unwrap().to_owned())
            .unwrap();
        panel.click(&format!("live:{answer}-decision-{choice}"));
        panel.click(&format!("live:{answer}-answer"));
        panel.wait(idle);
        assert!(panel.text().contains(reply), "{}", panel.text());
        panel.panels.close();
    }
}
#[test]
fn composition_run_gets_session_and_edit_tools_and_the_chosen_model() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut environment = environment(root.path(), "reply");
    let record = root.path().join("argv.json");
    environment.vars.insert(
        "PM_FAKE_RECORD".into(),
        record.to_string_lossy().into_owned(),
    );
    environment.tools = pm_mcp_tools();
    environment.composition_tools = vec![
        "start_session".into(),
        "apply_commands".into(),
        "get_project_state".into(),
    ];
    let mut panel = Agent::mount(&actor, environment);
    panel.connected(Provider::Claude);
    panel.change("agent-provider", "claude");
    panel.change("agent-model", "fixture");
    panel.change("agent-effort", "high");
    panel.connected(Provider::Claude);
    panel.send("Go");
    let agent = panel.agent();
    let thread = thread_id(&agent);
    let open =
        |state: &agent_host::AgentState| state.panel.session(&thread).unwrap().needs_answer();
    panel.wait(open);
    let args: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    let value = |flag: &str| {
        let at = args.iter().position(|a| a == flag).unwrap();
        args[at + 1].clone()
    };
    assert_eq!(value("--model"), "fixture");
    assert_eq!(value("--effort"), "high");
    let allowed = value("--allowedTools");
    assert!(
        allowed.contains("mcp__powermove__start_session"),
        "{allowed}"
    );
    assert!(
        allowed.contains("mcp__powermove__apply_commands"),
        "{allowed}"
    );
    assert!(
        !allowed.contains("mcp__powermove__export_video"),
        "{allowed}"
    );
    // The tool server gets the run's grant token, never its id; the token
    // names this run while it lives.
    let config: Value = serde_json::from_str(&value("--mcp-config")).unwrap();
    let server = config["mcpServers"]["powermove"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let token = server[server.iter().position(|a| *a == "--agent-run").unwrap() + 1];
    let run = agent
        .lock()
        .unwrap()
        .panel
        .session(&thread)
        .unwrap()
        .run
        .as_ref()
        .unwrap()
        .id
        .clone();
    assert_ne!(token, run);
    assert_eq!(
        pm_agents::launch::grants::resolve(token).map(|(r, _)| r),
        Some(run)
    );
    panel.panels.close();
}
fn pm_mcp_tools() -> Vec<String> {
    vec![
        "start_session".into(),
        "apply_commands".into(),
        "export_video".into(),
    ]
}
#[test]
fn signing_in_from_settings_opens_the_browser_and_connects_the_panel() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let opened = std::sync::Arc::<std::sync::Mutex<Vec<String>>>::default();
    let mut panel = Agent::mount_recording(
        &actor,
        environment(root.path(), "signed-out"),
        opened.clone(),
    );
    finish(panel.panels.mount("settings").unwrap());
    let accounts = panel.panels.broker.agent.clone();
    let read = |accounts: &agent_host::AgentLink| accounts.connection(Provider::Chatgpt);
    let deadline = Instant::now() + Duration::from_secs(10);
    while read(&accounts).0 == "checking" {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(read(&accounts).1, "Sign in to ChatGPT in Settings.");
    assert!(panel.text().contains("Connect Your Agent"));
    let settings = |panel: &Agent, action: Value| {
        let backend = panel
            .panels
            .broker
            .entry("settings")
            .unwrap()
            .lock()
            .unwrap()
            .backend
            .clone();
        let mut backend = backend.lock().unwrap();
        let revision = backend.settings_snapshot().unwrap()["revision"].clone();
        match backend
            .native_command(
                "panel.settings",
                &json!({"revision":revision,"action":action}),
            )
            .unwrap()
        {
            models::NativeCommand::Value(value) => value,
            _ => panic!("settings value"),
        }
    };
    let row = |publication: &Value, id: &str| {
        publication["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap()
            .clone()
    };
    let before = settings(&panel, json!({"type":"extensions.retry"}));
    assert_eq!(row(&before, "account.chatgpt")["actionLabel"], "Sign In…");
    settings(
        &panel,
        json!({"type":"row","id":"account.chatgpt","action":"account.connect"}),
    );
    panel.connected(Provider::Chatgpt);
    assert_eq!(
        opened.lock().unwrap().as_slice(),
        ["https://chatgpt.com/auth"]
    );
    let after = settings(&panel, json!({"type":"extensions.retry"}));
    let chatgpt = row(&after, "account.chatgpt");
    assert_eq!(chatgpt["actionLabel"], "Sign Out");
    assert_eq!(chatgpt["detail"], "user@example.invalid · plus");
    assert!(!panel.text().contains("Connect Your Agent"));
    panel.change("agent-model", "fixture-5");
    panel.send("Hi");
    panel.wait(idle);
    assert!(panel.text().contains("Saw: Hi"));
    settings(
        &panel,
        json!({"type":"row","id":"account.chatgpt","action":"account.disconnect"}),
    );
    while read(&accounts).0 == "connected" {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(panel.text().contains("Connect Your Agent"));
    panel.panels.close();
}
#[test]
fn missing_runtime_reports_no_connection_and_keeps_the_draft() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut environment = environment(root.path(), "reply");
    environment.resources = None;
    let mut panel = Agent::mount(&actor, environment);
    assert!(panel.text().contains("runtime is missing"));
    panel.panels.close();
}
#[test]
fn an_agent_tool_launches_sends_to_and_cancels_a_background_thread() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let visible = thread_id(&agent);
    let launched = panel
        .panels
        .agent_thread_control(
            "thread_launch",
            &json!({"title":"Side task","message":"Hi from a tool"}),
        )
        .unwrap();
    let thread = launched["threadId"].as_str().unwrap().to_owned();
    assert_ne!(thread, visible, "launch never takes the visible thread");
    let finished = |state: &agent_host::AgentState| {
        state
            .panel
            .session(&thread)
            .is_ok_and(|s| s.run.is_none() && s.conversation.len() > 1)
    };
    panel.wait(finished);
    assert_eq!(thread_id(&agent), visible);
    let reply = |state: &agent_host::AgentState, text: &str| {
        state
            .panel
            .session(&thread)
            .unwrap()
            .conversation
            .values()
            .any(|m| m.to_string().contains(text))
    };
    assert!(reply(&agent.lock().unwrap(), "Saw: Hi from a tool"));
    panel
        .panels
        .agent_thread_control(
            "thread_send",
            &json!({"threadId":thread,"message":"And again"}),
        )
        .unwrap();
    panel.wait(|state| {
        reply(state, "Saw: And again") && state.panel.session(&thread).unwrap().run.is_none()
    });
    let idle_cancel = panel
        .panels
        .agent_thread_control("thread_cancel", &json!({"threadId":thread}))
        .unwrap();
    assert_eq!(idle_cancel["busy"], false);
    let missing = panel
        .panels
        .agent_thread_control("thread_send", &json!({"threadId":"none","message":"x"}))
        .unwrap_err();
    assert_eq!(
        missing,
        "This thread does not exist in the current project."
    );
    panel.panels.close();
}
#[test]
fn a_signed_out_provider_refuses_a_thread_launch_before_anything_changes() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    // Once the bundled providers have answered, the compatible one (never
    // configured here) is the signed-out provider.
    panel.connected(Provider::Chatgpt);
    assert!(!panel.panels.broker.agent.available(Provider::Compatible));
    let agent = panel.agent();
    let before = agent.lock().unwrap().panel.summaries().len();
    let refused = panel
        .panels
        .agent_thread_control(
            "thread_launch",
            &json!({"title":"t","message":"m","target":{"providerInstanceId":"compatible"},"callerAccess":"project"}),
        )
        .unwrap_err();
    assert_eq!(
        refused,
        "Sign in to your compatible provider in Settings › Agents before starting a run."
    );
    assert_eq!(agent.lock().unwrap().panel.summaries().len(), before);
    panel.panels.close();
}
#[test]
fn a_planning_caller_launches_planning_threads_and_retries_are_deduplicated() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    panel.connected(Provider::Chatgpt);
    let request = json!({"title":"Plan","message":"Look only","callerAccess":"editor",
        "callerRunId":"caller-1","clientRequestId":"req-1"});
    let first = panel
        .panels
        .agent_thread_control("thread_launch", &request)
        .unwrap();
    assert_eq!(first["access"], "editor", "{first}");
    let thread = first["threadId"].as_str().unwrap().to_owned();
    let again = panel
        .panels
        .agent_thread_control("thread_launch", &request)
        .unwrap();
    assert_eq!(
        again["threadId"], first["threadId"],
        "a retry returns the first launch"
    );
    let mut reused = request.clone();
    reused["message"] = json!("Something else");
    assert_eq!(
        panel
            .panels
            .agent_thread_control("thread_launch", &reused)
            .unwrap_err(),
        "This clientRequestId belongs to another thread launch."
    );
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_none());
    // A planning caller's new turn stays at planning access.
    let sent = panel
        .panels
        .agent_thread_control(
            "thread_send",
            &json!({"threadId":thread,"message":"More","callerAccess":"editor"}),
        )
        .unwrap();
    assert_eq!(sent["access"], "editor", "{sent}");
    assert_eq!(sent["delivery"], "started", "{sent}");
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_none());
    panel.panels.close();
}
#[test]
fn reconfiguring_a_running_thread_restarts_it_and_cancel_stops_it() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "hold"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let launched = panel
        .panels
        .agent_thread_control("thread_launch", &json!({"title":"Hold","message":"Wait"}))
        .unwrap();
    let thread = launched["threadId"].as_str().unwrap().to_owned();
    let first_run = launched["runId"].as_str().unwrap().to_owned();
    assert_eq!(launched["busy"], true, "{launched}");
    // A different effort can't steer the live turn: it stops and restarts.
    let effort = if agent
        .lock()
        .unwrap()
        .panel
        .session(&thread)
        .unwrap()
        .settings
        .effort
        == "high"
    {
        "low"
    } else {
        "high"
    };
    let sent = panel
        .panels
        .agent_thread_control(
            "thread_send",
            &json!({"threadId":thread,"message":"Go on","target":{"options":{"reasoningEffort":effort}}}),
        )
        .unwrap();
    assert_eq!(sent["delivery"], "started", "{sent}");
    assert_eq!(sent["reasoningEffort"], effort, "{sent}");
    assert_ne!(sent["runId"], json!(first_run), "{sent}");
    assert_eq!(sent["busy"], true, "{sent}");
    let cancelled = panel
        .panels
        .agent_thread_control("thread_cancel", &json!({"threadId":thread}))
        .unwrap();
    assert_eq!(cancelled["threadId"], json!(thread));
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_none());
    panel.panels.close();
}
#[test]
fn attached_files_reach_the_provider_run() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    panel.connected(Provider::Chatgpt);
    // A 1×1 PNG and a text note, as Add Attachments records them.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    panel
        .panels
        .agent_attach(vec![
            json!({"id":"a1","name":"frame.png","type":"image/png","size":68,"dataUrl":format!("data:image/png;base64,{png}")}),
            json!({"id":"a2","name":"notes.txt","type":"text/plain","size":5,"content":"hello"}),
        ])
        .unwrap();
    panel.send("Use these");
    panel.wait(idle);
    let text = panel.text();
    assert!(text.contains("notes.txt"), "{text}");
    let written: Vec<_> = walk(root.path())
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "reference-0.png"))
        .collect();
    assert_eq!(
        written.len(),
        1,
        "the image is a file the provider can open"
    );
    assert_eq!(&std::fs::read(&written[0]).unwrap()[1..4], b"PNG");
    panel.panels.close();
}
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
/// A slow fake provider: each turn answers after `delay` seconds.
fn slow_environment(profile: &Path, delay: &str) -> Environment {
    let mut environment = environment(profile, "slow");
    environment
        .vars
        .insert("PM_FAKE_DELAY".into(), delay.into());
    environment
}
fn live_run(agent: &agent_host::SharedAgent, thread: &str) -> String {
    agent
        .lock()
        .unwrap()
        .panel
        .session(thread)
        .unwrap()
        .run
        .as_ref()
        .unwrap()
        .id
        .clone()
}
fn said(state: &agent_host::AgentState, thread: &str, text: &str) -> bool {
    state
        .panel
        .session(thread)
        .unwrap()
        .conversation
        .values()
        .any(|m| m.to_string().contains(text))
}
/// The latest delegate_task row: live in the trace, or kept with the reply.
fn task_row(state: &agent_host::AgentState, thread: &str) -> Option<Value> {
    let session = state.panel.session(thread).unwrap();
    let kept = session
        .conversation
        .values()
        .flat_map(|m| m["steps"].as_array().cloned().unwrap_or_default());
    session
        .trace
        .rows
        .values()
        .cloned()
        .chain(kept)
        .filter(|row| row["toolName"] == "delegate_task")
        .last()
}
const RESUMED: &str = "Saw: Review these results, verify changes where needed";
#[test]
fn a_delegated_task_runs_and_its_result_resumes_the_calling_run() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, slow_environment(root.path(), "1.0"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    panel.send("Make a title card");
    let run = live_run(&agent, &thread);
    // What the run's own MCP session sends (`--agent-run`).
    let capabilities = panel
        .panels
        .agent_orchestrate("orchestrator_capabilities", &json!({"agentRunId":run}))
        .unwrap();
    assert_eq!(
        capabilities["parentThreadId"],
        json!(thread),
        "{capabilities}"
    );
    assert_eq!(capabilities["features"]["maxParallelTasks"], 8);
    let chatgpt = &capabilities["providers"][0];
    assert_eq!(chatgpt["canRunChildTask"], true, "{capabilities}");
    assert_eq!(chatgpt["models"][0]["id"], "fixture-5", "{capabilities}");
    assert_eq!(
        chatgpt["models"][0]["options"][0]["options"][1]["label"],
        "High"
    );
    let delegated = panel
        .panels
        .agent_orchestrate(
            "delegate_task",
            &json!({"agentRunId":run,"task":"Child brief","title":"Child","clientRequestId":"round-1"}),
        )
        .unwrap();
    assert_eq!(delegated["status"], "running", "{delegated}");
    let task = delegated["taskId"].as_str().unwrap().to_owned();
    // A retry with the same key is the same task; another brief is refused.
    let again = panel
        .panels
        .agent_orchestrate(
            "delegate_task",
            &json!({"agentRunId":run,"task":"Child brief","title":"Child","clientRequestId":"round-1"}),
        )
        .unwrap();
    assert_eq!(again["taskId"], json!(task));
    assert!(
        panel
            .panels
            .agent_orchestrate(
                "delegate_task",
                &json!({"agentRunId":run,"task":"Other","clientRequestId":"round-1"}),
            )
            .unwrap_err()
            .contains("already belongs to a different task")
    );
    // The root's turn ends before the child's: it waits, then resumes.
    panel.wait(|state| {
        state.runs.held(&run)
            && state.panel.session(&thread).unwrap().activity == "Waiting for 1 subagent to finish…"
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let agent2 = agent.clone();
    while !{
        let state = agent2.lock().unwrap();
        said(&state, &thread, RESUMED) && state.panel.session(&thread).unwrap().run.is_none()
    } {
        assert!(Instant::now() < deadline, "the run never resumed");
        thread::sleep(Duration::from_millis(20));
    }
    let state = agent.lock().unwrap();
    let row = task_row(&state, &thread).expect("task row");
    assert_eq!(row["status"], "done", "{row}");
    assert_eq!(row["output"], "Saw: Child brief", "{row}");
    assert!(state.tasks.idle());
    drop(state);
    // Main's archive layout, with the result delivered (acknowledged).
    let archives = walk(&root.path().join("Agent Tasks"));
    assert_eq!(archives.len(), 1, "{archives:?}");
    let saved: Value = serde_json::from_slice(&std::fs::read(&archives[0]).unwrap()).unwrap();
    let record = &saved["records"][0];
    assert_eq!(record["task"]["status"], "completed", "{saved}");
    assert_eq!(record["task"]["summary"], "Saw: Child brief");
    assert_eq!(record["acknowledged"], true);
    assert_eq!(record["clientRequestId"], "round-1");
    // The run is over: its tools answer as main's do.
    assert_eq!(
        panel
            .panels
            .agent_orchestrate("task_status", &json!({"agentRunId":run,"taskId":task}))
            .unwrap_err(),
        "This agent run is no longer active."
    );
    panel.panels.close();
}
#[test]
fn wait_mode_status_cancel_and_send_follow_mains_contract() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, slow_environment(root.path(), "0.6"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    // Its turn holds, as a real provider's does while it waits on a tool
    // call: the run can't settle between these calls.
    panel.send("Hold: Plan it");
    let run = live_run(&agent, &thread);
    let call = |tool: &str, args: Value| {
        let mut args = args;
        args["agentRunId"] = json!(run);
        panel.panels.agent_orchestrate(tool, &args)
    };
    // wait returns the finished result and acknowledges it.
    let waited = call(
        "delegate_task",
        json!({"task":"Quick look","mode":"wait","timeoutMs":20000}),
    )
    .unwrap();
    assert_eq!(waited["status"], "completed", "{waited}");
    assert_eq!(waited["summary"], "Saw: Quick look");
    assert_eq!(waited["waitTimedOut"], false);
    // Cancelling a finished task keeps its result.
    let kept = call("task_cancel", json!({"taskId":waited["taskId"]})).unwrap();
    assert_eq!(kept["status"], "completed", "{kept}");
    assert!(
        call("task_status", json!({"taskId":"nope"}))
            .unwrap_err()
            .contains("does not belong")
    );
    // A wait that times out never cancels: the task goes on.
    let short = call(
        "delegate_task",
        json!({"task":"Longer look","mode":"wait","timeoutMs":0}),
    )
    .unwrap();
    assert_eq!(short["waitTimedOut"], true, "{short}");
    let id = short["taskId"].as_str().unwrap().to_owned();
    let status = call("task_status", json!({"taskId":id})).unwrap();
    assert_eq!(status["workState"], "working", "{status}");
    assert_eq!(status["questions"], json!([]));
    // App Server takes new direction live; new settings interrupt and resume.
    let sent = call(
        "task_send",
        json!({"taskId":id,"message":"Also check the end","clientRequestId":"u1"}),
    )
    .unwrap();
    assert_eq!(sent["delivery"], "live", "{sent}");
    let resumed = call(
        "task_send",
        json!({"taskId":id,"target":{"options":{"reasoningEffort":"high"}}}),
    )
    .unwrap();
    assert_eq!(resumed["delivery"], "resumed", "{resumed}");
    assert_eq!(resumed["reasoningEffort"], "high", "{resumed}");
    let repeat = call(
        "task_send",
        json!({"taskId":id,"message":"Also check the end","clientRequestId":"u1"}),
    )
    .unwrap();
    assert_eq!(repeat["delivery"], "already_sent");
    assert!(
        call("task_send", json!({"taskId":id}))
            .unwrap_err()
            .contains("Supply message")
    );
    let cancelled = call("task_cancel", json!({"taskId":id,"reason":"Not needed"})).unwrap();
    assert_eq!(cancelled["status"], "cancel_requested", "{cancelled}");
    panel.wait(|state| {
        task_row(state, &thread)
            .is_some_and(|r| r["task"]["taskId"] == json!(id) && r["task"]["status"] == "cancelled")
    });
    panel.click("agent-stop");
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_none() && state.tasks.idle());
    panel.panels.close();
}
#[test]
fn a_watched_thread_run_resumes_the_watcher_with_its_result() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, slow_environment(root.path(), "1.0"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    panel.send("Coordinate");
    let run = live_run(&agent, &thread);
    let side = panel
        .panels
        .agent_thread_control(
            "thread_launch",
            &json!({"title":"Side","message":"Side work","callerAccess":"editor"}),
        )
        .unwrap();
    let side_thread = side["threadId"].as_str().unwrap().to_owned();
    assert!(
        panel
            .panels
            .agent_orchestrate("thread_watch", &json!({"agentRunId":run,"threadId":thread}))
            .unwrap_err()
            .contains("cannot watch its own run")
    );
    let watched = panel
        .panels
        .agent_orchestrate(
            "thread_watch",
            &json!({"agentRunId":run,"threadId":side_thread}),
        )
        .unwrap();
    assert_eq!(watched["watching"], true, "{watched}");
    assert_eq!(watched["status"], "running");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !{
        let state = agent.lock().unwrap();
        said(&state, &thread, RESUMED) && state.panel.session(&thread).unwrap().run.is_none()
    } {
        assert!(Instant::now() < deadline, "the watcher never resumed");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(said(&agent.lock().unwrap(), &side_thread, "Saw: Side work"));
    panel.panels.close();
}
#[test]
fn stopping_a_run_that_waits_for_its_subagent_stops_both() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, slow_environment(root.path(), "0.3"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    panel.send("Go");
    let run = live_run(&agent, &thread);
    // A child that never answers.
    panel
        .panels
        .agent_orchestrate(
            "delegate_task",
            &json!({"agentRunId":run,"task":"Hold: forever"}),
        )
        .unwrap();
    panel.wait(|state| state.runs.held(&run));
    panel.click("agent-stop");
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_none() && state.tasks.idle());
    let state = agent.lock().unwrap();
    let row = task_row(&state, &thread).expect("task row");
    assert_eq!(row["task"]["status"], "cancelled", "{row}");
    assert_eq!(
        state.panel.raw_archive()["threads"][0]["lastRunStatus"],
        "cancelled"
    );
    drop(state);
    panel.panels.close();
}
#[test]
fn a_run_publishes_a_visual_reply_that_opens_in_its_viewer() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, slow_environment(root.path(), "1.0"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    panel.send("Chart the timing");
    let run = live_run(&agent, &thread);
    let refused = panel
        .panels
        .agent_visual_reply(&json!({"agentRunId":run,"title":"x","html":"<iframe></iframe>"}))
        .unwrap_err();
    assert!(refused.contains("cannot embed pages"), "{refused}");
    let published = panel
        .panels
        .agent_visual_reply(&json!({"agentRunId":run,"title":"Timing chart",
            "html":"<html><body><svg width=10 height=10></svg></body></html>","height":420}))
        .unwrap();
    assert_eq!(published["saved"], true);
    assert_eq!(published["threadId"], json!(thread));
    let id = published["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("visual-"), "{id}");
    // Main's limit: four per run.
    for _ in 0..3 {
        panel
            .panels
            .agent_visual_reply(&json!({"agentRunId":run,"title":"More","html":"<p>x</p>"}))
            .unwrap();
    }
    assert_eq!(
        panel
            .panels
            .agent_visual_reply(&json!({"agentRunId":run,"title":"Fifth","html":"<p>x</p>"}))
            .unwrap_err(),
        "This run already published four visual replies. Continue in a follow-up message."
    );
    assert!(panel.text().contains("Timing chart"));
    // Saved with the thread, in main's shape.
    let saved = agent
        .lock()
        .unwrap()
        .panel
        .visual_reply(&thread, &id)
        .unwrap();
    assert_eq!(
        saved["html"],
        "<!doctype html><html><head></head><body><svg width=10 height=10></svg></body></html>"
    );
    assert_eq!(saved["runId"], json!(run));
    assert_eq!(saved["height"], 420);
    // Open queues the viewer for the window.
    let open = panel
        .surface()
        .surface
        .nodes()
        .values()
        .find_map(|n| {
            n.props
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| s.ends_with(&id))
                .map(str::to_owned)
        })
        .expect("Open button");
    panel.panels.take_platform_requests();
    panel.click(&open);
    let requests = panel.panels.take_platform_requests();
    let request = requests
        .iter()
        .find(|r| r.operation == "agent.visual")
        .expect("viewer request");
    assert_eq!(request.input["title"], "Timing chart");
    assert_eq!(
        request.input["id"],
        json!(id),
        "the viewer is kept per reply"
    );
    assert_eq!(request.input["height"], 420);
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_none());
    assert!(
        panel
            .panels
            .agent_visual_reply(&json!({"agentRunId":run,"title":"Late","html":"<p>x</p>"}))
            .unwrap_err()
            .contains("no longer active")
    );
    panel.panels.close();
}

#[test]
fn a_private_answer_reaches_the_subagent_but_never_its_prompt() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    panel.send("Hold: Coordinate");
    let run = live_run(&agent, &thread);
    let call = |tool: &str, args: Value| {
        let mut args = args;
        args["agentRunId"] = json!(run);
        panel.panels.agent_orchestrate(tool, &args)
    };
    let task = call("delegate_task", json!({"task":"Secret: unlock the asset"})).unwrap();
    let id = task["taskId"].as_str().unwrap().to_owned();
    let child = task["childRunId"].as_str().unwrap().to_owned();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        let status = call("task_status", json!({"taskId":id})).unwrap();
        if status["questions"]
            .as_array()
            .is_some_and(|q| !q.is_empty())
        {
            break status;
        }
        assert!(Instant::now() < deadline, "{status}");
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        status["questions"][0]["questions"][0]["secret"], true,
        "{status}"
    );
    let sent = call("task_send", json!({"taskId":id,"message":"4921"})).unwrap();
    assert_eq!(sent["delivery"], "answered", "{sent}");
    let prompt = agent
        .lock()
        .unwrap()
        .tasks
        .child_request(&child)
        .unwrap()
        .prompt
        .clone();
    assert!(!prompt.contains("4921"), "{prompt}");
    assert!(
        prompt.ends_with("A private answer was delivered to your held question."),
        "{prompt}"
    );
    // The subagent itself got the answer.
    panel
        .wait(|state| task_row(state, &thread).is_some_and(|r| r["task"]["status"] == "completed"));
    let row = task_row(&agent.lock().unwrap(), &thread).unwrap();
    assert!(row["output"].as_str().unwrap().contains("4921"), "{row}");
    panel.click("agent-stop");
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_none() && state.tasks.idle());
    panel.panels.close();
}
#[test]
fn subagents_inherit_the_runs_attachments() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, slow_environment(root.path(), "1.0"));
    panel.connected(Provider::Chatgpt);
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    panel
        .panels
        .agent_attach(vec![
            json!({"id":"a1","name":"frame.png","type":"image/png","size":68,"dataUrl":format!("data:image/png;base64,{png}")}),
            json!({"id":"a2","name":"notes.txt","type":"text/plain","size":5,"content":"hello"}),
        ])
        .unwrap();
    let agent = panel.agent();
    let thread = thread_id(&agent);
    panel.send("Look at these");
    let run = live_run(&agent, &thread);
    let task = panel
        .panels
        .agent_orchestrate(
            "delegate_task",
            &json!({"agentRunId":run,"task":"Check the frame"}),
        )
        .unwrap();
    let child = task["childRunId"].as_str().unwrap().to_owned();
    let request = agent
        .lock()
        .unwrap()
        .tasks
        .child_request(&child)
        .unwrap()
        .clone();
    assert_eq!(request.images.len(), 1, "{request:?}");
    assert!(
        request.images[0].ends_with("reference-0.png"),
        "{request:?}"
    );
    assert_eq!(request.files.len(), 1, "{request:?}");
    assert_eq!(request.files[0]["name"], "notes.txt");
    // The provider was told: its prompt ends with main's ATTACHED FILES.
    panel
        .wait(|state| task_row(state, &thread).is_some_and(|r| r["task"]["status"] == "completed"));
    let row = task_row(&agent.lock().unwrap(), &thread).unwrap();
    assert!(
        row["output"].as_str().unwrap().contains("notes.txt"),
        "{row}"
    );
    panel.wait(|state| state.panel.session(&thread).unwrap().run.is_none() && state.tasks.idle());
    panel.panels.close();
}
#[test]
fn a_result_that_cant_be_saved_is_kept_until_it_is() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, slow_environment(root.path(), "1.0"));
    panel.connected(Provider::Chatgpt);
    let agent = panel.agent();
    let thread = thread_id(&agent);
    panel.send("Make a card");
    let run = live_run(&agent, &thread);
    panel
        .panels
        .agent_orchestrate(
            "delegate_task",
            &json!({"agentRunId":run,"task":"Child brief"}),
        )
        .unwrap();
    // The task archive's folder stops taking writes before the child ends.
    let folder = walk(&root.path().join("Agent Tasks"))[0]
        .parent()
        .unwrap()
        .to_owned();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500)).unwrap();
    panel.wait(|state| {
        task_row(state, &thread).is_some_and(|r| {
            r["task"]["progress"]
                == "The task result could not be saved yet. Powermove keeps trying."
        })
    });
    thread::sleep(Duration::from_millis(1500));
    assert!(
        agent.lock().unwrap().runs.held(&run),
        "the parent still waits"
    );
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !{
        let state = agent.lock().unwrap();
        said(&state, &thread, RESUMED) && state.panel.session(&thread).unwrap().run.is_none()
    } {
        assert!(Instant::now() < deadline, "the run never resumed");
        thread::sleep(Duration::from_millis(20));
    }
    let saved: Value = serde_json::from_slice(&std::fs::read(&walk(&folder)[0]).unwrap()).unwrap();
    assert_eq!(
        saved["records"][0]["task"]["status"], "completed",
        "{saved}"
    );
    panel.panels.close();
}
