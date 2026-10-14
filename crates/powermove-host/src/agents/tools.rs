//! Run-scoped editor tools for the agent CLIs (port of `agent-tools/bridge.ts`
//! and the MCP shim `mcp-server.mjs` / `mcp-stdio.mjs`).
//!
//! The renderer owns project state, rendering and Undo. Each run gets a
//! session with a random bearer token; the CLI reaches Powermove's tools over
//! a loopback streamable-HTTP MCP endpoint, and every call is forwarded to the
//! window that owns the run as `agent-tool:request` and settled by its
//! `agent-tool:response`. A token only ever reaches its own run's window.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use base64::Engine;
use bytes::Bytes;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{OnceCell, oneshot};

use super::prompts::{Prompts, ToolSpec};
use crate::router::{Call, Client};

const TOOL_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TOOL_TEXT_CHARS: usize = 2_000_000;
const MAX_TOOL_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOOL_CONTENT_ITEMS: usize = 8;
const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const OUTSIDE_SANDBOX_TOOL: &str = "run_outside_sandbox";

/// Tools whose main-process half is not ported yet: window capture and input,
/// the Store, ffmpeg media analysis, Creative Cloud inspection, and
/// dialog-free save/export. They are hidden from the agent. Subagent tools
/// are visible once an orchestrator is installed (`set_orchestrator`).
const UNAVAILABLE_TOOLS: &[&str] = &[
    "capture_panel", "computer_use_panel",
    "probe_media", "sample_media_frames", "media_contact_sheet", "media_waveform", "transcribe_media",
    "store_search", "store_extension", "store_source", "store_library", "store_install", "store_update", "store_uninstall", "store_publish_prepare", "store_publish",
    "inspect_creative_workspace", "inspect_creative_extension", "save_project", "export_video",
];

fn unavailable(tool: &str) -> bool {
    UNAVAILABLE_TOOLS.contains(&tool)
}

#[derive(Debug, Clone)]
pub enum Content {
    Text(String),
    Image { data: Bytes, mime: String },
}

#[derive(Debug, Clone)]
pub struct ToolResponse {
    pub ok: bool,
    pub content: Vec<Content>,
    pub error: Option<String>,
    pub changed: Option<bool>,
    pub history_id: Option<String>,
}

impl ToolResponse {
    fn text(text: String) -> Self {
        Self { ok: true, content: vec![Content::Text(text)], error: None, changed: None, history_id: None }
    }

    /// The MCP `CallToolResult`.
    fn mcp(&self) -> Value {
        let mut content: Vec<Value> = self
            .content
            .iter()
            .map(|item| match item {
                Content::Text(text) => json!({ "type": "text", "text": text }),
                Content::Image { data, mime } => json!({ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(data), "mimeType": mime }),
            })
            .collect();
        if !self.ok && !self.content.iter().any(|item| matches!(item, Content::Text(_))) {
            content.push(json!({ "type": "text", "text": self.error.clone().unwrap_or_else(|| "The Powermove tool failed.".into()) }));
        }
        json!({ "content": content, "isError": !self.ok })
    }
}

#[derive(Debug, Default, Clone)]
pub struct FinishResult {
    pub changed: bool,
    pub history_id: Option<String>,
    pub warning: Option<String>,
}

/// Asks the person, then runs one command outside the sandbox; returns its report.
pub type OutsideSandbox = Arc<dyn Fn(String, String, u64) -> BoxFuture<'static, Result<String>> + Send + Sync>;

/// Handles an orchestration tool call: (run id, tool, arguments) → result JSON.
pub type Orchestrate = Arc<dyn Fn(String, String, Value) -> BoxFuture<'static, Result<Value>> + Send + Sync>;

pub struct ToolSession {
    pub run_id: String,
    pub owner: Arc<Client>,
    pub base_revision: u64,
    token: String,
    app_context: bool,
    inspection_only: bool,
    pub changed: AtomicBool,
    closed: AtomicBool,
    finishing: OnceCell<FinishResult>,
    outside_sandbox: Option<OutsideSandbox>,
    bridge: Weak<ToolBridge>,
    /// A subagent's session edits inside its root run's live transaction.
    transaction: Option<Arc<ToolSession>>,
}

impl ToolSession {
    /// How the CLI reaches this session's tools.
    pub fn endpoint(&self) -> McpEndpoint {
        let bridge = self.bridge.upgrade();
        McpEndpoint { url: bridge.map(|bridge| bridge.url.clone()).unwrap_or_default(), token: self.token.clone() }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn note(&self, response: &ToolResponse) {
        if let Some(changed) = response.changed {
            self.changed.store(changed, Ordering::Release);
        }
        if let Some(root) = &self.transaction {
            root.note(response);
        }
    }

    /// Seal or roll back the run's live edits (`__finish_run`), once.
    pub async fn finish(self: &Arc<Self>, commit: bool) -> FinishResult {
        self.finishing
            .get_or_init(|| async {
                let mut result = FinishResult { changed: self.changed.load(Ordering::Acquire), ..Default::default() };
                if let Some(bridge) = self.bridge.upgrade() {
                    // The root run seals a subagent's edits with its own.
                    if !self.owner.is_closed() && !self.is_closed() && self.transaction.is_none() {
                        match bridge.call_renderer(self, "__finish_run", json!({ "commit": commit })).await {
                            Ok(response) => {
                                result.changed = response.changed == Some(true);
                                result.history_id = response.history_id.clone();
                                if !response.ok {
                                    result.warning = response.error.clone();
                                }
                            }
                            Err(error) => result.warning = Some(format!("{error:#}")),
                        }
                    }
                    self.closed.store(true, Ordering::Release);
                    bridge.close(self);
                }
                self.closed.store(true, Ordering::Release);
                result
            })
            .await
            .clone()
    }
}

#[derive(Debug, Clone)]
pub struct McpEndpoint {
    pub url: String,
    pub token: String,
}

struct PendingCall {
    run_id: String,
    owner: u64,
    tool: String,
    settle: oneshot::Sender<Result<ToolResponse>>,
}

pub struct ToolBridge {
    prompts: Arc<Prompts>,
    url: String,
    sessions: Mutex<HashMap<String, Arc<ToolSession>>>,
    runs: Mutex<HashMap<String, Arc<ToolSession>>>,
    pending: Mutex<HashMap<String, PendingCall>>,
    timeout: Duration,
    orchestrate: std::sync::OnceLock<Orchestrate>,
}

pub struct SessionOptions {
    pub run_id: String,
    pub owner: Arc<Client>,
    pub base_revision: u64,
    pub app_context: bool,
    pub inspection_only: bool,
    pub outside_sandbox: Option<OutsideSandbox>,
}

impl ToolBridge {
    /// Bind the loopback MCP endpoint and return the bridge.
    pub async fn start(prompts: Arc<Prompts>, timeout: Option<Duration>) -> Result<Arc<Self>> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let address: SocketAddr = listener.local_addr()?;
        let bridge = Arc::new(Self {
            prompts,
            url: format!("http://{address}/mcp"),
            sessions: Mutex::default(),
            runs: Mutex::default(),
            pending: Mutex::default(),
            timeout: timeout.unwrap_or(TOOL_TIMEOUT),
            orchestrate: std::sync::OnceLock::new(),
        });
        let app = axum::Router::new()
            .route("/mcp", post(mcp_post).get(|| async { StatusCode::METHOD_NOT_ALLOWED }).delete(|| async { StatusCode::METHOD_NOT_ALLOWED }))
            .layer(axum::extract::DefaultBodyLimit::max(MAX_REQUEST_BYTES))
            .with_state(Arc::downgrade(&bridge));
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                tracing::error!("agent tool endpoint stopped: {error}");
            }
        });
        Ok(bridge)
    }

    pub fn open_session(self: &Arc<Self>, options: SessionOptions) -> Result<Arc<ToolSession>> {
        if self.runs.lock().contains_key(&options.run_id) {
            bail!("Agent tool session already exists.");
        }
        let token = hex::encode(rand::random::<[u8; 32]>());
        let session = Arc::new(ToolSession {
            run_id: options.run_id.clone(),
            owner: options.owner.clone(),
            base_revision: options.base_revision,
            token: token.clone(),
            app_context: options.app_context,
            inspection_only: options.inspection_only,
            changed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            finishing: OnceCell::new(),
            outside_sandbox: options.outside_sandbox,
            bridge: Arc::downgrade(self),
            transaction: None,
        });
        self.sessions.lock().insert(token, session.clone());
        self.runs.lock().insert(options.run_id, session.clone());
        // A closed window cannot answer: fail its calls now rather than at the timeout.
        let bridge = Arc::downgrade(self);
        let owner = options.owner.id;
        options.owner.on_close(move || {
            if let Some(bridge) = bridge.upgrade() {
                bridge.fail_owner(owner);
            }
        });
        Ok(session)
    }

    /// A subagent's session: its own token and tool filter, but renderer
    /// calls (and the final seal) go through `root`'s live transaction.
    pub fn open_child_session(self: &Arc<Self>, run_id: &str, root: &Arc<ToolSession>, inspection_only: bool) -> Result<Arc<ToolSession>> {
        if self.runs.lock().contains_key(run_id) {
            bail!("Agent tool session already exists.");
        }
        let token = hex::encode(rand::random::<[u8; 32]>());
        let session = Arc::new(ToolSession {
            run_id: run_id.to_owned(),
            owner: root.owner.clone(),
            base_revision: root.base_revision,
            token: token.clone(),
            app_context: root.app_context,
            inspection_only,
            changed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            finishing: OnceCell::new(),
            outside_sandbox: root.outside_sandbox.clone(),
            bridge: Arc::downgrade(self),
            transaction: Some(root.clone()),
        });
        self.sessions.lock().insert(token, session.clone());
        self.runs.lock().insert(run_id.to_owned(), session.clone());
        Ok(session)
    }

    /// Route the subagent tools to `orchestrate` and show them to agents.
    pub fn set_orchestrator(&self, orchestrate: Orchestrate) {
        let _ = self.orchestrate.set(orchestrate);
    }

    pub fn session_for_run(&self, run_id: &str) -> Option<Arc<ToolSession>> {
        self.runs.lock().get(run_id).cloned()
    }

    fn close(&self, session: &ToolSession) {
        self.sessions.lock().remove(&session.token);
        let mut runs = self.runs.lock();
        if runs.get(&session.run_id).is_some_and(|other| std::ptr::eq(other.as_ref(), session)) {
            runs.remove(&session.run_id);
        }
        drop(runs);
        let mut pending = self.pending.lock();
        let ids: Vec<String> = pending.iter().filter(|(_, call)| call.run_id == session.run_id).map(|(id, _)| id.clone()).collect();
        for id in ids {
            if let Some(call) = pending.remove(&id) {
                let _ = call.settle.send(Err(anyhow!("Agent tool session ended.")));
            }
        }
    }

    fn fail_owner(&self, owner: u64) {
        let mut pending = self.pending.lock();
        let ids: Vec<String> = pending.iter().filter(|(_, call)| call.owner == owner).map(|(id, _)| id.clone()).collect();
        for id in ids {
            if let Some(call) = pending.remove(&id) {
                let message = format!("Powermove window closed while running {}. The action outcome is unknown; inspect the project before repeating edits.", call.tool);
                let _ = call.settle.send(Err(anyhow!(message)));
            }
        }
    }

    /// The tools this session's agent sees.
    pub fn visible_tools(&self, session: &ToolSession) -> Vec<ToolSpec> {
        let prompts = &self.prompts;
        let mut tools: Vec<ToolSpec> = prompts
            .tools
            .iter()
            .filter(|tool| !unavailable(&tool.name))
            .filter(|tool| self.orchestrate.get().is_some() || !super::orchestrator::Orchestrator::is_tool(&tool.name))
            .filter(|tool| !session.app_context || prompts.app_tool_names.contains(&tool.name))
            .filter(|tool| !session.inspection_only || prompts.inspection_tool_names.contains(&tool.name))
            .cloned()
            .collect();
        if session.outside_sandbox.is_some() {
            tools.push(prompts.outside_sandbox_tool.clone());
        }
        tools
    }

    /// One `agent-tool:request` round trip to the owning window.
    pub async fn call_renderer(&self, session: &ToolSession, tool: &str, args: Value) -> Result<ToolResponse> {
        let session = session.transaction.as_deref().unwrap_or(session);
        if session.owner.is_closed() {
            bail!("Powermove window is no longer available.");
        }
        let call_id = format!("tool-{}", uuid::Uuid::new_v4());
        let (settle, receiver) = oneshot::channel();
        self.pending.lock().insert(call_id.clone(), PendingCall { run_id: session.run_id.clone(), owner: session.owner.id, tool: tool.to_owned(), settle });
        session.owner.emit("agent-tool:request", json!({
            "runId": session.run_id,
            "callId": call_id,
            "tool": tool,
            "arguments": args,
            "baseRevision": session.base_revision,
        }));
        let timeout = if tool == "__mcp_export_video" || tool == "__mcp_save_project" { Duration::from_secs(3_600) } else { self.timeout };
        let outcome = tokio::time::timeout(timeout, receiver).await;
        self.pending.lock().remove(&call_id);
        match outcome {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => bail!("Agent tool session ended."),
            Err(_) => bail!("Powermove timed out while running {tool}. Inspect get_workspace_state before retrying; an edit may already have completed."),
        }
    }

    /// `agent-tool:response` from a window. Only the window that owns the
    /// call can settle it; a malformed answer fails the call promptly.
    pub fn on_renderer_response(&self, call: &Call) {
        let value = call.arg(0);
        let Some(call_id) = value["callId"].as_str() else { return };
        let mut pending = self.pending.lock();
        let Some(entry) = pending.get(call_id) else { return };
        if entry.owner != call.client.id || value["runId"].as_str() != Some(entry.run_id.as_str()) {
            return;
        }
        let entry = pending.remove(call_id).expect("checked above");
        drop(pending);
        let parsed = parse_renderer_response(value, call).ok_or_else(|| {
            anyhow!("Powermove returned an invalid or oversized response for {}. Request a smaller state page. The action may have completed; inspect before retrying any edit.", entry.tool)
        });
        if let Ok(response) = &parsed {
            if let Some(session) = self.runs.lock().get(&entry.run_id) {
                session.note(response);
            }
        }
        let _ = entry.settle.send(parsed);
    }

    /// Dispatch one agent tool call for `session`.
    pub async fn call_tool(&self, session: &ToolSession, tool: &str, args: Value) -> Result<ToolResponse> {
        if session.is_closed() || session.owner.is_closed() {
            bail!("Powermove tool session is not active.");
        }
        if tool == OUTSIDE_SANDBOX_TOOL {
            let Some(run) = session.outside_sandbox.clone() else { bail!("{OUTSIDE_SANDBOX_TOOL} is not available in this run.") };
            let command = args["command"].as_str().filter(|text| !text.trim().is_empty() && text.len() <= 20_000);
            let reason = args["reason"].as_str().filter(|text| !text.trim().is_empty() && text.chars().count() <= 600);
            let timeout = match &args["timeoutMs"] {
                Value::Null => Some(120_000),
                value => value.as_u64().filter(|ms| (1..=600_000).contains(ms)),
            };
            let (Some(command), Some(reason), Some(timeout)) = (command, reason, timeout) else {
                bail!("{OUTSIDE_SANDBOX_TOOL} expects {{ command, reason, timeoutMs? }}.");
            };
            return Ok(ToolResponse::text(run(command.to_owned(), reason.to_owned(), timeout).await?));
        }
        if super::orchestrator::Orchestrator::is_tool(tool) {
            let Some(orchestrate) = self.orchestrate.get() else { bail!("Subagent orchestration is unavailable in this connection.") };
            let result = orchestrate(session.run_id.clone(), tool.to_owned(), args).await?;
            return Ok(ToolResponse::text(result.to_string()));
        }
        if self.prompts.tool(tool).is_none() {
            bail!("Unknown Powermove tool: {tool}");
        }
        if unavailable(tool) {
            bail!("{tool} is not available in the Windows app yet.");
        }
        if session.inspection_only && !self.prompts.inspection_tool_names.iter().any(|name| name == tool) {
            bail!("This planning run can only inspect the project.");
        }
        if session.app_context && !self.prompts.app_tool_names.iter().any(|name| name == tool) {
            bail!("This agent has no project attached. Open a project to use composition tools.");
        }
        // Fork staging runs in the host, against the run's extension stage.
        if super::changes::is_stage_tool(tool) {
            return Ok(ToolResponse::text(super::changes::call_stage_tool(&session.run_id, tool, &args).await?));
        }
        let response = self.call_renderer(session, tool, args).await?;
        session.note(&response);
        Ok(response)
    }

    fn session(&self, token: &str) -> Option<Arc<ToolSession>> {
        self.sessions.lock().get(token).cloned().filter(|session| !session.is_closed())
    }
}

fn parse_renderer_response(value: &Value, call: &Call) -> Option<ToolResponse> {
    let items = value["content"].as_array()?;
    if items.len() > MAX_TOOL_CONTENT_ITEMS || !value["ok"].is_boolean() {
        return None;
    }
    let mut content = Vec::new();
    for item in items {
        match item["type"].as_str()? {
            "text" => {
                let text = item["text"].as_str()?;
                if text.chars().count() > MAX_TOOL_TEXT_CHARS {
                    return None;
                }
                content.push(Content::Text(text.to_owned()));
            }
            "image" => {
                let mime = item["mimeType"].as_str().filter(|mime| *mime == "image/png" || *mime == "image/jpeg")?;
                let data = call.bytes(&item["data"]).ok()?;
                if data.len() > MAX_TOOL_IMAGE_BYTES {
                    return None;
                }
                content.push(Content::Image { data, mime: mime.to_owned() });
            }
            _ => return None,
        }
    }
    Some(ToolResponse {
        ok: value["ok"] == true,
        content,
        error: value["error"].as_str().filter(|error| error.len() <= 2_000).map(str::to_owned),
        changed: value["changed"].as_bool(),
        history_id: value["historyId"].as_str().filter(|id| id.len() <= 200).map(str::to_owned),
    })
}

/* ── streamable-HTTP MCP endpoint ───────────────────────────── */

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").map(str::trim)
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn json_response(value: Value) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], value.to_string()).into_response()
}

async fn mcp_post(State(bridge): State<Weak<ToolBridge>>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(bridge) = bridge.upgrade() else { return StatusCode::SERVICE_UNAVAILABLE.into_response() };
    // The token is the run's identity; anything else is refused before parsing.
    let Some(session) = bearer(&headers).and_then(|token| bridge.session(token)) else {
        return (StatusCode::UNAUTHORIZED, "Powermove tool session is not active.").into_response();
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return json_response(rpc_error(Value::Null, -32700, "Parse error"));
    };
    match handle_rpc(&bridge, &session, message).await {
        Some(reply) => json_response(reply),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// One JSON-RPC message; `None` for notifications and responses.
pub async fn handle_rpc(bridge: &ToolBridge, session: &ToolSession, message: Value) -> Option<Value> {
    if message.get("jsonrpc") != Some(&json!("2.0")) || !message["method"].is_string() {
        // Client responses (no method) need no reply; anything else is invalid.
        return message.get("method").is_some().then(|| rpc_error(message["id"].clone(), -32600, "Invalid Request"));
    }
    let id = message.get("id")?.clone();
    if !(id.is_string() || id.is_number()) {
        return Some(rpc_error(Value::Null, -32600, "Invalid request id"));
    }
    let params = &message["params"];
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let requested = params["protocolVersion"].as_str().unwrap_or_default();
            let version = PROTOCOL_VERSIONS.iter().find(|version| **version == requested).unwrap_or(&PROTOCOL_VERSIONS[0]);
            json!({ "protocolVersion": version, "capabilities": { "tools": { "listChanged": false } }, "serverInfo": { "name": "powermove", "version": "1.0.0" } })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": bridge.visible_tools(session) }),
        "tools/call" => {
            let (Some(name), arguments) = (params["name"].as_str(), params.get("arguments").cloned().unwrap_or_else(|| json!({}))) else {
                return Some(rpc_error(id, -32602, "Invalid tool arguments"));
            };
            if !arguments.is_object() {
                return Some(rpc_error(id, -32602, "Invalid tool arguments"));
            }
            match bridge.call_tool(session, name, arguments).await {
                Ok(response) => response.mcp(),
                Err(error) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("{error:#}") }] }),
            }
        }
        _ => return Some(rpc_error(id, -32601, "Method not found")),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::wire::{self, Payload};

    pub fn prompts() -> Arc<Prompts> {
        Arc::new(serde_json::from_value(json!({
            "instructions": { "project": {}, "app": "app" },
            "resultSchema": { "type": "object" },
            "testingInstructions": "",
            "shellNetworkInstructions": "",
            "tools": [
                { "name": "get_project_state", "description": "Read", "inputSchema": { "type": "object" } },
                { "name": "apply_commands", "description": "Edit", "inputSchema": { "type": "object" } },
                { "name": "render_frames", "description": "Frames", "inputSchema": { "type": "object" } },
                { "name": "capture_panel", "description": "Capture", "inputSchema": { "type": "object" } }
            ],
            "appToolNames": ["get_project_state"],
            "inspectionToolNames": ["get_project_state", "render_frames"],
            "outsideSandboxTool": { "name": "run_outside_sandbox", "description": "Ask", "inputSchema": { "type": "object" } },
            "outsideSandboxCallMs": 4200000
        })).unwrap())
    }

    /// A window whose outgoing frames can be read back.
    pub fn fake_window(id: u64) -> (Arc<Client>, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let (outbox, frames) = tokio::sync::mpsc::unbounded_channel();
        (Arc::new(Client::new(id, outbox, None)), frames)
    }

    pub fn decode_event(frame: Vec<u8>) -> (String, Value) {
        let payload = wire::decode(Bytes::from(frame)).unwrap();
        (payload.json["ch"].as_str().unwrap().to_owned(), payload.json["args"][0].clone())
    }

    pub fn response_call(host: Arc<crate::host::Host>, client: Arc<Client>, value: Value, blobs: Vec<Bytes>) -> Call {
        Call { host, client, args: vec![value], blobs: Arc::new(blobs) }
    }

    pub fn host() -> Arc<crate::host::Host> {
        let directory = std::env::temp_dir();
        crate::host::Host::new(
            crate::host::Config {
                user_data: directory.clone(),
                renderer_dir: directory.clone(),
                builtin_extensions: directory.clone(),
                resources_dir: directory.clone(),
                ffmpeg: directory.clone(),
                temp_dir: directory,
                version: "test".into(),
                port: 0,
            },
            "token".into(),
        )
    }

    #[tokio::test]
    async fn round_trips_a_tool_call_over_mcp_to_the_owning_window() {
        let bridge = ToolBridge::start(prompts(), Some(Duration::from_secs(5))).await.unwrap();
        let (window, mut frames) = fake_window(1);
        let (stranger, _) = fake_window(2);
        let session = bridge.open_session(SessionOptions { run_id: "run-abcdefgh".into(), owner: window.clone(), base_revision: 4, app_context: false, inspection_only: false, outside_sandbox: None }).unwrap();
        let endpoint = session.endpoint();
        let host = host();

        // The fake renderer answers each request with text and a PNG.
        let responder = {
            let bridge = bridge.clone();
            let window = window.clone();
            let stranger = stranger.clone();
            let host = host.clone();
            tokio::spawn(async move {
                let (channel, request) = decode_event(frames.recv().await.unwrap());
                assert_eq!(channel, "agent-tool:request");
                assert_eq!(request["tool"], "apply_commands");
                assert_eq!(request["baseRevision"], 4);
                let mut payload = Payload::default();
                let image = payload.attach(Bytes::from_static(b"\x89PNG"));
                let answer = json!({ "runId": request["runId"], "callId": request["callId"], "ok": true, "changed": true, "content": [{ "type": "text", "text": "done" }, { "type": "image", "data": image, "mimeType": "image/png" }] });
                // Another window cannot settle the call.
                bridge.on_renderer_response(&response_call(host.clone(), stranger, answer.clone(), payload.blobs.clone()));
                bridge.on_renderer_response(&response_call(host, window, answer, payload.blobs));
            })
        };

        let http = reqwest_post(&endpoint.url, &endpoint.token, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } })).await;
        assert_eq!(http["result"]["protocolVersion"], "2025-06-18");
        let listed = reqwest_post(&endpoint.url, &endpoint.token, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await;
        let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["get_project_state", "apply_commands", "render_frames"]);
        let called = reqwest_post(&endpoint.url, &endpoint.token, json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "apply_commands", "arguments": { "commands": [] } } })).await;
        responder.await.unwrap();
        assert_eq!(called["result"]["isError"], false);
        assert_eq!(called["result"]["content"][0]["text"], "done");
        assert_eq!(called["result"]["content"][1]["data"], base64::engine::general_purpose::STANDARD.encode(b"\x89PNG"));
        assert!(session.changed.load(Ordering::Acquire));

        // A wrong token is refused before anything reaches the window.
        let refused = raw_post(&endpoint.url, "nope", json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" })).await;
        assert!(refused.starts_with("HTTP/1.1 401"));
    }

    #[tokio::test]
    async fn inspection_runs_cannot_edit() {
        let bridge = ToolBridge::start(prompts(), None).await.unwrap();
        let (window, _frames) = fake_window(1);
        let session = bridge.open_session(SessionOptions { run_id: "run-inspect1".into(), owner: window, base_revision: 0, app_context: false, inspection_only: true, outside_sandbox: None }).unwrap();
        let error = bridge.call_tool(&session, "apply_commands", json!({})).await.unwrap_err();
        assert!(error.to_string().contains("only inspect"));
        let error = bridge.call_tool(&session, "capture_panel", json!({})).await.unwrap_err();
        assert!(error.to_string().contains("not available"));
    }

    #[tokio::test]
    async fn a_closed_window_fails_its_pending_calls() {
        let bridge = ToolBridge::start(prompts(), Some(Duration::from_secs(30))).await.unwrap();
        let (window, mut frames) = fake_window(1);
        let session = bridge.open_session(SessionOptions { run_id: "run-closing1".into(), owner: window.clone(), base_revision: 0, app_context: false, inspection_only: false, outside_sandbox: None }).unwrap();
        let call = {
            let bridge = bridge.clone();
            let session = session.clone();
            tokio::spawn(async move { bridge.call_tool(&session, "get_project_state", json!({})).await })
        };
        frames.recv().await.unwrap();
        window.close();
        assert!(call.await.unwrap().unwrap_err().to_string().contains("window closed"));
    }

    async fn raw_post(url: &str, token: &str, body: Value) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let address = url.trim_start_matches("http://").trim_end_matches("/mcp");
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let body = body.to_string();
        let request = format!("POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    async fn reqwest_post(url: &str, token: &str, body: Value) -> Value {
        let response = raw_post(url, token, body).await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let body = response.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}
