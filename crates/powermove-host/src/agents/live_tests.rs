//! Real CLI runs through the host with a fake renderer window. They spend a
//! little subscription quota, so they only run on request:
//!
//!   cargo test -p powermove-host --lib agents::live_tests -- --ignored --nocapture
//!
//! Requires `scripts/windows/stage-api-pack.ps1` and a signed-in CLI.

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use serde_json::{Value, json};

use super::service::{Agents, install};
use super::tools::tests::{decode_event, fake_window};
use crate::host::{Config, Host};
use crate::router::{Call, Router};

async fn host(claude_home: Option<PathBuf>) -> (Arc<Host>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config = Config {
        user_data: directory.path().join("data"),
        renderer_dir: directory.path().into(),
        builtin_extensions: directory.path().into(),
        resources_dir: repo.join("target/powermove-resources"),
        ffmpeg: directory.path().into(),
        temp_dir: directory.path().into(),
        version: "test".into(),
        port: 0,
    };
    let mut router = Router::default();
    install(&mut router, &Agents::new(&config, claude_home).await);
    let host = Host::new(config, "token".into());
    host.install_router(router);
    (host, directory)
}

async fn invoke(host: &Arc<Host>, client: &Arc<crate::router::Client>, channel: &str, arg: Value) -> anyhow::Result<Value> {
    let handler = host.router().get(channel).unwrap();
    let payload = handler(Call { host: host.clone(), client: client.clone(), args: vec![arg], blobs: Arc::new(Vec::<Bytes>::new()) }).await?;
    Ok(payload.json)
}

/// Run one request while a fake renderer answers every editor tool call.
async fn run(host: Arc<Host>, request: Value) -> (Value, Vec<String>) {
    let (window, mut frames) = fake_window(1);
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let renderer = {
        let host = host.clone();
        let window = window.clone();
        let seen = seen.clone();
        tokio::spawn(async move {
            while let Some(frame) = frames.recv().await {
                let (channel, event) = decode_event(frame);
                if channel == "codex:event" {
                    if event["kind"] == "trace" && event["step"]["kind"] == "tool-start" {
                        seen.lock().push(format!("tool {}", event["step"]["label"]));
                    }
                    println!("{event}");
                    continue;
                }
                if channel != "agent-tool:request" {
                    continue;
                }
                let tool = event["tool"].as_str().unwrap_or_default().to_owned();
                seen.lock().push(format!("call {tool}"));
                let text = if tool == "__finish_run" { "{}".to_owned() } else { json!({ "revision": 1, "name": "Live test", "layers": [{ "id": "title", "type": "text", "text": "Hello" }] }).to_string() };
                let response = json!({ "runId": event["runId"], "callId": event["callId"], "ok": true, "changed": false, "content": [{ "type": "text", "text": text }] });
                invoke(&host, &window, "agent-tool:response", response).await.unwrap();
            }
        })
    };
    let result = invoke(&host, &window, "codex:run", request).await.unwrap();
    // The host keeps the window's sender alive, so the fake renderer is stopped here.
    renderer.abort();
    window.close();
    let seen = seen.lock().clone();
    (result, seen)
}

fn request(provider: &str, mode: &str, model: Value) -> Value {
    json!({
        "id": format!("live-{}", uuid::Uuid::new_v4()),
        "provider": provider,
        "mode": mode,
        "prompt": "Call get_project_state once to read the project, then finish. Report the title layer's text in the summary. Make no edits and no files.",
        "schema": if mode == "editor" { json!({ "type": "object", "properties": { "summary": { "type": "string" } }, "required": ["summary"], "additionalProperties": false }) } else { Value::Null },
        "images": [],
        "model": model,
        "reasoningEffort": "low",
        "access": if mode == "editor" { "editor" } else { "project" },
        "projectId": "live-test",
        "projectName": "Live test",
        "projectJSON": "{\"revision\":1}",
        "attachments": [],
        "consentToken": null,
        "approval": "auto"
    })
}

#[tokio::test]
#[ignore = "spends ChatGPT quota"]
async fn chatgpt_autonomous_run_reads_the_project() {
    let (host, _directory) = host(None).await;
    let (result, seen) = run(host, request("chatgpt", "autonomous", json!("gpt-6-luna"))).await;
    println!("result: {result}\nseen: {seen:?}");
    assert_eq!(result["ok"], true, "{result}");
    assert!(seen.iter().any(|item| item == "call get_project_state"), "{seen:?}");
}

#[tokio::test]
#[ignore = "spends ChatGPT quota"]
async fn chatgpt_editor_run_reads_the_project() {
    let (host, _directory) = host(None).await;
    let (result, seen) = run(host, request("chatgpt", "editor", json!("gpt-6-luna"))).await;
    println!("result: {result}\nseen: {seen:?}");
    assert_eq!(result["ok"], true, "{result}");
    assert!(seen.iter().any(|item| item == "call get_project_state"), "{seen:?}");
}

/// A Codex parent delegates to a Codex child through the app-owned
/// orchestrator; only the child reads the project.
#[tokio::test]
#[ignore = "spends ChatGPT quota"]
async fn chatgpt_delegates_to_a_chatgpt_child() {
    let (host, _directory) = host(None).await;
    let mut request = request("chatgpt", "editor", json!("gpt-6-luna"));
    request["prompt"] = json!("Call the delegate_task tool exactly once with mode \"wait\" and task \"Call get_project_state once, then report the title layer's text.\" Do not call get_project_state yourself. Then finish, putting the child's answer in the summary.");
    let (result, seen) = run(host, request).await;
    println!("result: {result}\nseen: {seen:?}");
    assert_eq!(result["ok"], true, "{result}");
    assert!(seen.iter().any(|item| item == "call get_project_state"), "the child read the project: {seen:?}");
    assert!(result["text"].as_str().unwrap_or_default().contains("Hello"), "{result}");
}

#[tokio::test]
#[ignore = "spends Claude quota; uses the terminal login in ~/.claude"]
async fn claude_autonomous_run_reads_the_project() {
    let (host, _directory) = host(dirs::home_dir().map(|home| home.join(".claude"))).await;
    let (result, seen) = run(host, request("claude", "autonomous", json!("haiku"))).await;
    println!("result: {result}\nseen: {seen:?}");
    assert_eq!(result["ok"], true, "{result}");
    assert!(seen.iter().any(|item| item == "call get_project_state"), "{seen:?}");
}

/// Claude Code connects to the run's MCP endpoint with the token from its
/// environment. Stops at the `init` event, before any model request.
#[tokio::test]
#[ignore = "launches the real Claude Code CLI"]
async fn claude_connects_to_the_tool_endpoint() {
    use super::autonomous::{NativeTools, TOOL_TOKEN_ENV};
    use super::claude_run::{ClaudeArgv, claude_argv, user_message};
    use super::request::Access;
    use super::tools::{SessionOptions, ToolBridge};

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let prompts = Arc::new(super::prompts::Prompts::load(&repo.join("target/powermove-resources")).unwrap());
    let bridge = ToolBridge::start(prompts.clone(), None).await.unwrap();
    let (window, _frames) = fake_window(1);
    let session = bridge.open_session(SessionOptions { run_id: "live-claude-mcp".into(), owner: window, base_revision: 0, app_context: false, inspection_only: true, outside_sandbox: None }).unwrap();
    let endpoint = session.endpoint();
    let tools = NativeTools { url: endpoint.url, token: endpoint.token.clone(), names: bridge.visible_tools(&session).into_iter().map(|tool| tool.name).collect(), timeout_sec: 120, outside_sandbox: false };
    let schema = json!({ "type": "object" });
    let argv = claude_argv(&ClaudeArgv {
        schema: &schema,
        model: Some("haiku"),
        effort: None,
        session_id: None,
        access: Access::Editor,
        extensions_dir: None,
        instructions: None,
        testing_instructions: &prompts.testing_instructions,
        shell_network_instructions: "",
        tools: Some(&tools),
        asking: false,
    });
    let binary = super::runtime::Runtimes { user_data: std::env::temp_dir(), resources_dir: repo.join("target/powermove-resources"), codex_binary: None, claude_binary: None, claude_home: None }.claude().unwrap();
    let mut command = super::runtime::command(&binary, &argv, Some(&std::env::temp_dir()), &[(TOOL_TOKEN_ENV, endpoint.token.into())], &["CLAUDE_CODE_SAFE_MODE"]);
    command.stdin(std::process::Stdio::piped());
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut init = None;
    super::process::run(command, &cancel, std::time::Duration::from_secs(60), |stdin| stdin.write(user_message("hi", &[])), |line| {
        if line.contains("\"subtype\":\"init\"") {
            init = serde_json::from_str::<Value>(line).ok();
            cancel.cancel();
        }
    })
    .await;
    let init = init.expect("Claude Code sent no init event");
    println!("mcp_servers: {}", init["mcp_servers"]);
    assert!(init["mcp_servers"].as_array().unwrap().iter().any(|server| server["name"] == "powermove" && server["status"] == "connected"), "{}", init["mcp_servers"]);
    assert!(init["tools"].as_array().unwrap().iter().any(|tool| tool == "mcp__powermove__get_project_state"));
}

#[tokio::test]
#[ignore = "reads the real account state"]
async fn reports_account_status() {
    let (host, _directory) = host(dirs::home_dir().map(|home| home.join(".claude"))).await;
    let (window, _frames) = fake_window(1);
    println!("chatgpt: {}", invoke(&host, &window, "chatgpt:status", Value::Null).await.unwrap());
    println!("chatgpt models: {}", invoke(&host, &window, "chatgpt:models", Value::Null).await.unwrap());
    println!("claude: {}", invoke(&host, &window, "claude:status", Value::Null).await.unwrap());
    println!("claude models: {}", invoke(&host, &window, "claude:models", Value::Null).await.unwrap());
}
