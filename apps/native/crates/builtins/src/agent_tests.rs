//! Real V8 panel, real kernel actor, real provider pipes; the provider is fake.
use super::*;
use pm_agents::{Provider, launch::Environment};
use pm_kernel::{KernelActor, KernelConfig, OpenSource};
use pm_surface::events::EventDetail;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn finish(ticket: Ticket<Evaluation>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = ticket.try_take() {
            result.expect("v4 worker execution");
            return;
        }
        assert!(Instant::now() < deadline, "v4 ticket timed out");
        thread::sleep(Duration::from_millis(1));
    }
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
        inspection_tools: vec!["get_project_state".into()],
    }
}
fn catalog() -> pm_panel_agentpanel::input::Catalog {
    pm_panel_agentpanel::input::Catalog {
        providers: vec![("claude".into(), "Claude".into())],
        models: vec![("fixture".into(), "Fixture".into())],
        ..Default::default()
    }
}
struct Agent {
    panels: Builtins,
    latest: Option<Publication>,
}
impl Agent {
    fn mount(actor: &KernelActor, environment: Environment) -> Self {
        let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
        panels.set_agent_environment(environment);
        finish(panels.mount("agent").unwrap());
        finish(panels.show("agent").unwrap());
        Self {
            panels,
            latest: None,
        }
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
    fn fire(&mut self, id: &str, event: &str, detail: EventDetail) {
        let target = self.node(id);
        let (generation, revision) = {
            let p = self.surface();
            (p.generation, p.surface.revision())
        };
        finish(
            self.panels
                .event("agent", generation, revision, target, event, detail)
                .unwrap(),
        );
        for (_, ticket) in self.panels.publish() {
            finish(ticket.unwrap());
        }
    }
    fn key(&mut self, key: &str, shift: bool) {
        self.fire(
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
    assert!(!panel.text().contains("Connect Your Agent"));
    panel.type_text("Hi");
    panel.key("Enter", false);
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
    assert!(reopened.text().contains("Saw: Hi"));
    reopened.panels.close();
}
#[test]
fn stop_cancels_a_held_run_through_the_provider() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "hold"));
    panel.type_text("Wait");
    panel.key("Enter", false);
    panel.fire("agent-stop", "click", EventDetail::default());
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
fn claude_question_answer_reaches_the_process_and_its_structured_reply_shows() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut panel = Agent::mount(&actor, environment(root.path(), "reply"));
    let agent = panel.agent();
    let thread = thread_id(&agent);
    agent
        .lock()
        .unwrap()
        .panel
        .set_provider(&thread, Provider::Claude, "fixture".into(), catalog())
        .unwrap();
    panel.type_text("Style");
    panel.key("Enter", false);
    panel.wait(|state| {
        state
            .panel
            .session(&thread)
            .unwrap()
            .question("question")
            .is_some()
    });
    assert!(panel.text().contains("Choose a style?"));
    {
        let mut state = agent.lock().unwrap();
        state
            .panel
            .choose_answer(&thread, "question", "question-0", "Soft")
            .unwrap();
    }
    panel.panels.realms["agent"]
        .eval(format!(
            "__pmBuiltin.command('panel.agent.dispatch',{})",
            json!({"sequence":panel.panels.broker.agent.sequence(),"target":{"project":"synthetic","thread":thread,"run":agent.lock().unwrap().panel.session(&thread).unwrap().run.as_ref().unwrap().id},"action":{"type":"answer","step":"question","skip":false}})
        ))
        .map(finish)
        .unwrap();
    panel.wait(idle);
    assert!(panel.text().contains("Using Soft"));
    panel.panels.close();
}
#[test]
fn missing_runtime_reports_no_connection_and_keeps_the_draft() {
    let root = tempfile::tempdir().unwrap();
    let actor = actor();
    let mut environment = environment(root.path(), "reply");
    environment.resources = None;
    let mut panel = Agent::mount(&actor, environment);
    assert!(panel.text().contains("Connect Your Agent"));
    panel.panels.close();
}
