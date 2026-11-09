//! A provider run's tool server names its run (`--agent-run`): orchestration
//! calls reach the app's surfaces as that run, over the real loopback bridge.
use pm_mcp::{Automation, McpSession, automation::SurfaceHook, transport::LoopbackServer};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
fn scratch() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-agent-run"));
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}
#[derive(Default)]
struct Recorded(Mutex<Vec<(String, Value)>>);
impl SurfaceHook for Recorded {
    fn call(&self, tool: &str, _: Option<&str>, args: &Value) -> pm_mcp::Result<Value> {
        self.0.lock().unwrap().push((tool.into(), args.clone()));
        Ok(json!({"taskId":"t1","status":"running"}))
    }
}
#[test]
fn orchestration_calls_carry_the_launching_run() {
    let profile = scratch();
    let mut host = Automation::new(profile.clone(), profile.join("exports")).unwrap();
    host.create_project(&json!({"name":"Synthetic orchestration"}))
        .unwrap();
    let recorded = Arc::new(Recorded::default());
    host.surfaces = Some(recorded.clone());
    let host = Arc::new(Mutex::new(host));
    let mut server = LoopbackServer::start_with(&profile, {
        let host = host.clone();
        move || McpSession::new(host.clone())
    })
    .unwrap();
    let mut process = Command::new(env!("CARGO_BIN_EXE_powermove-mcp"))
        .args(["--powermove-mcp", "--profile"])
        .arg(&profile)
        .args(["--agent-run", "run-7"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = process.stdin.take().unwrap();
    let mut output = BufReader::new(process.stdout.take().unwrap());
    let mut call = |id: u64, name: &str, arguments: Value| -> Value {
        let request = json!({"jsonrpc":"2.0","id":id,"method":if name.is_empty() { "initialize" } else { "tools/call" },
            "params":{"name":name,"arguments":arguments,"protocolVersion":"2025-11-25","capabilities":{},
            "clientInfo":{"name":"test","version":"1"}}});
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0);
        serde_json::from_str(&line).unwrap()
    };
    call(1, "", json!({}));
    call(2, "start_session", json!({"prompt":"Synthetic"}));
    let delegated = call(3, "delegate_task", json!({"task":"Child brief"}));
    assert_eq!(delegated["result"]["isError"], false, "{delegated}");
    // Schema validation precedes the window: an unknown key never reaches it.
    let invalid = call(4, "task_status", json!({"taskId":"t1","extra":1}));
    assert_eq!(invalid["result"]["isError"], true, "{invalid}");
    drop(input);
    assert!(process.wait().unwrap().success());
    server.shutdown();
    let seen = recorded.0.lock().unwrap();
    let (tool, args) = seen
        .iter()
        .find(|(tool, _)| tool == "delegate_task")
        .expect("delegate_task reached the app");
    assert_eq!(tool, "delegate_task");
    assert_eq!(args["agentRunId"], "run-7", "{args}");
    assert_eq!(args["task"], "Child brief");
    assert!(!seen.iter().any(|(tool, _)| tool == "task_status"));
}
#[test]
fn a_session_without_a_run_explains_where_subagents_come_from() {
    let profile = scratch();
    let mut host = Automation::new(profile.clone(), profile.join("exports")).unwrap();
    host.create_project(&json!({"name":"Synthetic orchestration"}))
        .unwrap();
    host.surfaces = Some(Arc::new(Recorded::default()));
    let mut session = McpSession::new(Arc::new(Mutex::new(host)));
    session
        .dispatch("start_session", &json!({"prompt":"x"}))
        .unwrap();
    let refused = session
        .dispatch("delegate_task", &json!({"task":"Child brief"}))
        .unwrap_err();
    assert!(
        refused.message.contains("belong to a Powermove agent run"),
        "{}",
        refused.message
    );
}
