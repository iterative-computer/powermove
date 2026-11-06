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
            "agent-composer",
            "keydown",
            EventDetail {
                key: Some(key.into()),
                shift_key: Some(shift),
                ..Default::default()
            },
        );
    }
    fn type_text(&mut self, text: &str) {
        for letter in text.chars() {
            self.key(&letter.to_lowercase().to_string(), letter.is_uppercase());
        }
    }
    fn agent(&self) -> agent_host::SharedAgent {
        let entries = self.panels.broker.entries.lock().unwrap();
        let backend = entries["agent"].backend.lock().unwrap();
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
        assert!(Instant::now() < deadline, "the carried run stopped delivering");
        thread::sleep(Duration::from_millis(5));
    }
    drop(state);
    let mut back = Agent::mount_application(&actor, environment(root.path(), "slow"), Some(carry));
    back.connected(Provider::Chatgpt);
    assert!(back.text().contains("Saw: Make a project"), "{}", back.text());
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
    assert!(!state.lock().unwrap().runs.is_empty(), "closing kept the run");
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
    panel.click("live:question-question-0-Soft");
    panel.click("live:question-answer");
    panel.wait(idle);
    assert!(panel.text().contains("Using Soft"));
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
        let entries = panel.panels.broker.entries.lock().unwrap();
        let mut backend = entries["settings"].backend.lock().unwrap();
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
