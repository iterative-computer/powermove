//! Exercise the real launch boundary with an in-memory model and synthetic MCP.
mod support;
use pm_agents::{
    Access, ApprovalMode, Event, Provider, RunResult,
    compatible::{ChatHost, Saved},
    launch::{self, Environment, Request},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};
#[derive(Debug, Default)]
struct RecordingModel(Mutex<Option<Value>>);
impl ChatHost for RecordingModel {
    fn saved(&self) -> Option<Saved> {
        Some(Saved {
            base_url: "http://fixture.invalid/v1".into(),
            model: "fixture".into(),
            ..Saved::default()
        })
    }
    fn key(&self, _: &str) -> Option<String> {
        None
    }
    fn stream(
        &self,
        _: &str,
        _: Option<&str>,
        body: Vec<u8>,
        _: &AtomicBool,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<u16, String> {
        *self.0.lock().unwrap() = Some(serde_json::from_slice(&body).unwrap());
        sink(b"data: {\"choices\":[{\"delta\":{\"content\":\"Verified\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")?;
        Ok(200)
    }
}
#[test]
fn ordinary_provider_launch_receives_task_creative_response_and_authoring_guidance() {
    let temp = support::Temp::new();
    let model = Arc::new(RecordingModel::default());
    let env = Environment {
        profile: temp.0.join("isolated-profile"),
        executable: Path::new(env!("CARGO_MANIFEST_DIR")).join("../builtins/fixtures/fake-mcp.py"),
        vars: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        compatible: Some(model.clone()),
        ..Environment::default()
    };
    let handle = launch::launch(
        &env,
        Request {
            run: "context-launch-fixture".into(),
            prompt: "Create an effect panel and theme".into(),
            provider: Provider::Compatible,
            model: Some("fixture".into()),
            effort: None,
            access: Access::Editor,
            approval: ApprovalMode::Auto,
            computer_consent: false,
            workspace: temp.0.clone(),
            images: vec![],
            role: None,
            files: vec![],
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Event::Completed { result, .. }) = handle.try_event() {
            assert!(
                matches!(result, RunResult::Success { ok: true, .. }),
                "{result:?}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "synthetic run did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    let body = model.0.lock().unwrap();
    let user_prompt = body.as_ref().unwrap()["messages"][1]["content"]
        .as_str()
        .unwrap();
    for section in [
        "TASK FOCUS",
        "CREATIVE JUDGMENT",
        "RESPONSE STYLE",
        "BACKGROUND TESTING",
        "TOOL RELIABILITY",
        "EFFECT REQUESTS",
        "NATIVE PANEL DESIGN",
        "REVERSIBLE THEMES",
    ] {
        assert!(
            user_prompt.contains(section),
            "ordinary launch omitted {section}"
        );
    }
    assert!(user_prompt.ends_with("Create an effect panel and theme"));
}
