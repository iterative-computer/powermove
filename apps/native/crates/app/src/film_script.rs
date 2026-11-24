//! The film's requests, as the agent makes them: each change is the MCP
//! tool calls an agent run made for that request, replayed through the
//! agent's own connection on the film's project (`demo_editor`). Recorded
//! edits, not a live agent; nothing leaves the film's temporary profile.
use pm_mcp::transport::ToolConnection;
use pm_shell::stage::Change;
use serde_json::Value;
use std::path::Path;

/// Calls one of the agent's tools; its JSON answer.
fn call(tools: &mut crate::mcp::Connection, name: &str, args: Value) -> anyhow::Result<Value> {
    let result = tools
        .invoke(name, args)
        .map_err(|e| anyhow::anyhow!("{name}: {e:?}"))?;
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    anyhow::ensure!(result["isError"] != true, "{name}: {text}");
    Ok(serde_json::from_str(text).unwrap_or(Value::String(text.to_owned())))
}

pub fn apply(
    tools: &mut crate::mcp::Connection,
    profile: &Path,
    change: Change,
) -> anyhow::Result<()> {
    let _ = (
        profile,
        call as fn(&mut crate::mcp::Connection, &str, Value) -> anyhow::Result<Value>,
    );
    let _ = tools;
    anyhow::bail!("{change:?} is not recorded yet")
}
