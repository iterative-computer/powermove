//! Explicit opt-in, synthetic editor-only live check. No credentials are read,
//! copied or printed; the native CLI owns authentication in its isolated home.
use pm_agents::{
    Access, ApprovalMode, Event, RunResult,
    approvals::Authority,
    claude_launch, codex,
    transport::{self, Action, Machine, ProcessSpec},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::TryRecvError,
    thread,
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("PM_AGENT_LIVE_CHECK").as_deref() != Ok("1") {
        return Err("Opt in with PM_AGENT_LIVE_CHECK=1; this check contacts your provider using its existing isolated runtime home.".into());
    }
    let provider = std::env::var("PM_AGENT_LIVE_PROVIDER")?;
    let executable = PathBuf::from(std::env::var("PM_AGENT_LIVE_BINARY")?).canonicalize()?;
    let runtime_home =
        PathBuf::from(std::env::var("PM_AGENT_LIVE_RUNTIME_HOME")?).canonicalize()?;
    let cwd = PathBuf::from(std::env::var("PM_AGENT_LIVE_WORKSPACE")?).canonicalize()?;
    let mut env = BTreeMap::from([("PATH".into(), std::env::var("PATH")?)]);
    let home_key = match provider.as_str() {
        "claude" => "CLAUDE_CONFIG_DIR",
        "codex" => "CODEX_HOME",
        _ => return Err("Provider must be claude or codex".into()),
    };
    env.insert(
        home_key.into(),
        runtime_home.to_str().ok_or("Non-UTF8 runtime home")?.into(),
    );
    // HOME is passed through as the runtime's ordinary environment, never rewritten.
    if let Ok(value) = std::env::var("HOME") {
        env.insert("HOME".into(), value);
    }
    let spec = ProcessSpec {
        executable,
        args: vec!["app-server".into()],
        cwd: cwd.clone(),
        env,
    };
    let schema = json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false});
    let prompt = "Return {\"text\":\"native provider check\"} as the final structured answer. Do not call tools or inspect files.";
    let handle = if provider == "claude" {
        let options = claude_launch::Options {
            schema,
            prompt: prompt.into(),
            image_paths: vec![],
            model: None,
            effort: None,
            session_id: None,
            extensions_dir: None,
            instructions: "Synthetic native editor protocol check. Do not modify any files.".into(),
            additional_instructions: None,
            shell_network_instructions: String::new(),
            native_tools: None,
            tool_names: vec![],
            inspection_tool_names: vec![],
            external_mcp_servers: json!({}),
            ask_outside_sandbox: false,
        };
        options.launch(
            spec,
            "live-check".into(),
            Authority::new(Access::Editor, ApprovalMode::Supervised, false)?,
        )?
    } else {
        let handle = transport::spawn(spec, Machine::Codex(codex::Client::default()), vec![]);
        let run = serde_json::from_value(
            json!({"id":"live-check","cwd":cwd,"prompt":prompt,"schema":schema,"imagePaths":[]}),
        )?;
        handle
            .send(Action::Run(Box::new(run)))
            .map_err(|_| "Could not admit live check")?;
        handle
    };
    let end = Instant::now() + Duration::from_secs(120);
    let mut observations = 0;
    while Instant::now() < end {
        match handle.try_event() {
            Ok(Event::Completed { result, .. }) => match result {
                RunResult::Success { text, .. } => {
                    let value: serde_json::Value = serde_json::from_str(&text)?;
                    if value != json!({"text":"native provider check"}) {
                        return Err("Unexpected synthetic provider result".into());
                    }
                    println!(
                        "{provider}: live structured result passed; {observations} preceding observations; no transcript printed"
                    );
                    return Ok(());
                }
                RunResult::Failure { .. } => {
                    return Err(
                        "Live provider failed; inspect its runtime-owned diagnostics locally"
                            .into(),
                    );
                }
            },
            Ok(_) => observations += 1,
            Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(5)),
            Err(TryRecvError::Disconnected) => {
                return Err("Live provider worker disconnected".into());
            }
        }
    }
    let _ = handle.send(Action::Cancel("live-check".into()));
    Err("Live provider check timed out".into())
}
