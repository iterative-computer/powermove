//! Editor-mode ChatGPT runs over a shared `codex app-server` (port of
//! `codex/app-server-runner.ts`). Unlike `codex exec`, App Server exposes the
//! active thread and turn ids that `turn/steer` needs, and asks the client
//! its questions mid-turn.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::RunEvents;
use super::app_server::{AppServer, Incoming, app_server_error, method_is};
use super::autonomous::{NativeTools, RunContext};
use super::request::RunResult;
use super::runtime::Runtimes;
use super::trace::{self, clip, fragment_text, human_label, item_id, output_excerpt, progress_line, tool_detail};
use super::workspace::image_extension;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const QUESTIONS: usize = 6;
const OPTIONS: usize = 8;

#[derive(Default)]
struct TurnState {
    turn_id: Option<String>,
    final_text: String,
    delta_items: HashSet<String>,
    unscoped_delta: bool,
    /// Held `request_user_input` calls: trace item id → App Server request id.
    questions: HashMap<String, Value>,
    question_messages: HashSet<String>,
    queued_steering: Vec<(String, Vec<Bytes>)>,
    cancel_requested: bool,
    settle: Option<oneshot::Sender<RunResult>>,
}

struct Turn {
    server: Arc<AppServer>,
    request_id: String,
    thread_id: String,
    directory: PathBuf,
    events: RunEvents,
    state: Mutex<TurnState>,
}

pub struct CodexApp {
    runtimes: Runtimes,
    server: tokio::sync::Mutex<Option<Arc<AppServer>>>,
    turns: Mutex<HashMap<String, Arc<Turn>>>,
}

fn normalized_type(value: &Value) -> String {
    value.as_str().unwrap_or_default().replace('_', "").to_lowercase()
}

fn basename(value: &Value) -> String {
    let text: String = value.as_str().unwrap_or_default().chars().filter(|character| !(*character <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(character))).collect();
    text.split(['/', '\\']).filter(|part| !part.is_empty()).last().unwrap_or_default().to_owned()
}

fn changed_files(item: &Value) -> Vec<String> {
    item["changes"].as_array().into_iter().flatten().map(|change| if change.is_string() { basename(change) } else { basename(&change["path"]) }).filter(|name| !name.is_empty()).collect()
}

fn trace_item_id(item: &Value) -> Option<String> {
    item["id"].as_str().filter(|id| !id.is_empty() && id.chars().count() <= 240).map(item_id)
}

fn collab_label(tool: &str) -> &'static str {
    match tool {
        "spawnAgent" => "Start subagent",
        "sendInput" | "sendMessage" | "followupTask" => "Message subagent",
        "resumeAgent" => "Resume subagent",
        "wait" => "Wait for subagents",
        "closeAgent" => "Close subagent",
        "interruptAgent" => "Stop subagent",
        "listAgents" => "List subagents",
        _ => "Subagent",
    }
}

/// `appServerToolStart`.
fn tool_start(item: &Value) -> Option<Value> {
    let id = trace_item_id(item)?;
    Some(match normalized_type(&item["type"]).as_str() {
        "commandexecution" => trace::tool_start(&id, "bash", "Run", &tool_detail("Bash", &json!({ "command": item["command"] }), None)),
        "filechange" => trace::tool_start(&id, "edit", "Edit", &tool_detail("Edit", &json!({ "file_path": changed_files(item).join(", ") }), None)),
        "websearch" => trace::tool_start(&id, "search", "Search", &tool_detail("WebSearch", &json!({ "query": item["query"] }), None)),
        "imagegeneration" => trace::tool_start(&id, "image", "Image", ""),
        "computeruse" => trace::tool_start(&id, "computer", "Computer", ""),
        "contextcompaction" => trace::tool_start(&id, "compact", "Compacting context", ""),
        "imageview" => trace::tool_start(&id, "view_image", "View image", &basename(&item["path"])),
        "sleep" => {
            let seconds = item["durationMs"].as_f64().map_or(0, |ms| (ms / 1000.0).round() as i64);
            trace::tool_start(&id, "wait", "Wait", &if seconds > 0 { format!("{seconds}s") } else { String::new() })
        }
        "collabagenttoolcall" => trace::tool_start(&id, "agent", collab_label(item["tool"].as_str().unwrap_or_default()), &tool_detail("Task", &json!({ "description": item["prompt"] }), None)),
        "mcptoolcall" | "dynamictoolcall" => {
            let tool = item["tool"].as_str().filter(|tool| !tool.is_empty()).unwrap_or("mcp");
            trace::tool_start(&id, tool, &human_label(tool), &tool_detail(tool, &json!({ "tool": tool }), None))
        }
        _ => return None,
    })
}

/// `appServerToolEnd`.
fn tool_end(item: &Value) -> Option<Value> {
    let id = trace_item_id(item)?;
    let kind = normalized_type(&item["type"]);
    const TOOLS: [&str; 11] = ["commandexecution", "filechange", "mcptoolcall", "dynamictoolcall", "websearch", "imagegeneration", "computeruse", "contextcompaction", "imageview", "sleep", "collabagenttoolcall"];
    if !TOOLS.contains(&kind.as_str()) {
        return None;
    }
    let status = item["status"].as_str().unwrap_or_default().to_lowercase();
    let is_error = matches!(status.as_str(), "failed" | "error" | "declined");
    let raw = if is_error {
        match &item["error"] {
            Value::String(text) => json!(text),
            error if error["message"].is_string() => error["message"].clone(),
            _ => item["status"].clone(),
        }
    } else {
        match kind.as_str() {
            "commandexecution" => [&item["aggregatedOutput"], &item["aggregated_output"], &item["output"]].into_iter().find(|value| !value.is_null()).cloned().unwrap_or(Value::Null),
            "filechange" => {
                let files = changed_files(item);
                json!(if files.len() > 3 { format!("{} files changed", files.len()) } else { files.join(", ") })
            }
            "websearch" => item["query"].clone(),
            "mcptoolcall" => if item["result"].is_null() { item["output"].clone() } else { item["result"].clone() },
            "dynamictoolcall" => json!(item["contentItems"].as_array().into_iter().flatten().map(|part| part["text"].as_str().unwrap_or_default()).collect::<Vec<_>>().join("\n")),
            "contextcompaction" => json!("Earlier context summarized"),
            "imageview" | "sleep" | "collabagenttoolcall" => Value::Null,
            _ => item["output"].clone(),
        }
    };
    Some(trace::tool_end(&id, is_error, &output_excerpt(&raw)))
}

/// `item/tool/requestUserInput` questions, bounded for the renderer.
fn reply_questions(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(QUESTIONS)
        .filter_map(|entry| {
            let id = entry["id"].as_str().filter(|id| id.len() <= 200)?;
            let question = clip(entry.get("question"), 1_000);
            if question.is_empty() {
                return None;
            }
            let options: Vec<Value> = entry["options"]
                .as_array()
                .into_iter()
                .flatten()
                .take(OPTIONS)
                .filter_map(|option| {
                    let label = clip(option.get("label"), 160);
                    (!label.is_empty()).then(|| json!({ "label": label, "description": clip(option.get("description"), 400) }))
                })
                .collect();
            // With no options a typed answer is the only possible answer.
            let allow_other = entry["isOther"] == true || options.is_empty();
            Some(json!({ "id": id, "header": clip(entry.get("header"), 80), "question": question, "options": options, "allowOther": allow_other, "secret": entry["isSecret"] == true }))
        })
        .collect()
}

/// Questions posted on an async agent message (`title` + plain options).
fn message_questions(item_id: &str, value: &Value) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(QUESTIONS)
        .enumerate()
        .filter_map(|(index, entry)| {
            let question = clip(entry.get("title"), 1_000);
            if question.is_empty() {
                return None;
            }
            let options: Vec<Value> = entry["options"].as_array().into_iter().flatten().take(OPTIONS).map(|option| clip(Some(option), 160)).filter(|label| !label.is_empty()).map(|label| json!({ "label": label, "description": "" })).collect();
            Some(json!({ "id": format!("{item_id}-{index}"), "header": "", "question": question, "options": options, "allowOther": true, "secret": false }))
        })
        .collect()
}

/// The `config` that attaches the run's inspection tools. The bearer token
/// travels over the App Server's stdin, never a command line.
fn live_inspection_config(tools: Option<&NativeTools>) -> Value {
    let Some(tools) = tools else { return json!({ "mcp_servers": {} }) };
    json!({ "mcp_servers": { "powermove": {
        "url": tools.url,
        "http_headers": { "Authorization": format!("Bearer {}", tools.token) },
        "startup_timeout_sec": 30,
        "tool_timeout_sec": 120,
        "required": true,
        "enabled_tools": tools.names,
        "default_tools_approval_mode": "approve",
    } } })
}

impl CodexApp {
    pub fn new(runtimes: Runtimes) -> Arc<Self> {
        Arc::new(Self { runtimes, server: tokio::sync::Mutex::default(), turns: Mutex::default() })
    }

    async fn server(self: &Arc<Self>) -> anyhow::Result<Arc<AppServer>> {
        let mut slot = self.server.lock().await;
        if let Some(server) = slot.as_ref().filter(|server| server.is_alive()) {
            return Ok(server.clone());
        }
        let binary = self.runtimes.codex()?;
        let env = self.runtimes.codex_env(&binary).await?;
        // Agent questions are experimental App Server surface, withheld without this opt-in.
        let (server, events) = AppServer::start(&binary, &env, "powermove_agent", true, REQUEST_TIMEOUT).await?;
        *slot = Some(server.clone());
        tokio::spawn(dispatch(Arc::downgrade(self), server.clone(), events));
        Ok(server)
    }

    pub async fn run(self: &Arc<Self>, ctx: &RunContext) -> RunResult {
        let request = &ctx.request;
        if self.turns.lock().contains_key(&request.id) {
            return RunResult::failed(format!("A Codex run with id {} is already active.", request.id));
        }
        let cancelled = || RunResult::cancelled("The Codex run was cancelled.");
        let server = match self.server().await {
            Ok(server) => server,
            Err(error) => return RunResult::failed(format!("{error:#}")),
        };
        if ctx.cancel.is_cancelled() {
            return cancelled();
        }
        let directory = std::env::temp_dir().join(format!("powermove-codex-app-{}", uuid::Uuid::new_v4()));
        let result = self.run_in(ctx, &server, &directory).await;
        let _ = tokio::fs::remove_dir_all(&directory).await;
        result
    }

    async fn run_in(self: &Arc<Self>, ctx: &RunContext, server: &Arc<AppServer>, directory: &PathBuf) -> RunResult {
        let request = &ctx.request;
        if let Err(error) = tokio::fs::create_dir_all(directory).await {
            return RunResult::failed(error.to_string());
        }
        let input = match write_input(directory, "frame", &request.prompt, &request.images).await {
            Ok(input) => input,
            Err(error) => return RunResult::failed(error.to_string()),
        };
        /* The renderer prompt already carries Powermove's bounded conversation
           history; a fresh ephemeral thread keeps it from being duplicated. */
        let mut params = json!({
            "cwd": directory, "approvalPolicy": "never", "sandbox": "read-only", "ephemeral": true,
            "config": live_inspection_config(ctx.tools.as_ref()), "serviceName": "powermove",
        });
        if let Some(model) = &request.model {
            params["model"] = json!(model);
        }
        let thread_id = match server.request("thread/start", params).await {
            Ok(started) => match started["thread"]["id"].as_str().filter(|id| id.len() <= 200) {
                Some(id) => id.to_owned(),
                None => return RunResult::failed("Codex App Server did not return a thread id."),
            },
            Err(error) => return RunResult::failed(format!("{error:#}")),
        };
        if ctx.cancel.is_cancelled() {
            return RunResult::cancelled("The Codex run was cancelled.");
        }
        let (settle, settled) = oneshot::channel();
        let turn = Arc::new(Turn {
            server: server.clone(),
            request_id: request.id.clone(),
            thread_id: thread_id.clone(),
            directory: directory.clone(),
            events: ctx.events.clone(),
            state: Mutex::new(TurnState { settle: Some(settle), ..TurnState::default() }),
        });
        self.turns.lock().insert(request.id.clone(), turn.clone());
        let mut params = json!({
            "threadId": thread_id, "input": input, "cwd": directory, "approvalPolicy": "never",
            "sandboxPolicy": { "type": "readOnly" }, "outputSchema": request.schema.clone().unwrap_or_else(|| json!({ "type": "object" })),
        });
        if let Some(model) = &request.model {
            params["model"] = json!(model);
        }
        if let Some(effort) = &request.reasoning_effort {
            params["effort"] = json!(effort);
        }
        {
            let this = self.clone();
            let server = server.clone();
            let turn = turn.clone();
            tokio::spawn(async move {
                match server.request("turn/start", params).await {
                    Ok(started) => match started["turn"]["id"].as_str().filter(|id| id.len() <= 200) {
                        Some(id) => {
                            let (cancel, queued) = {
                                let mut state = turn.state.lock();
                                state.turn_id = Some(id.to_owned());
                                (state.cancel_requested, std::mem::take(&mut state.queued_steering))
                            };
                            if cancel {
                                let _ = server.request("turn/interrupt", json!({ "threadId": turn.thread_id, "turnId": id })).await;
                                return;
                            }
                            for (prompt, images) in queued {
                                let _ = send_steering(&server, &turn, &prompt, &images).await;
                            }
                        }
                        None => this.finish(&turn, RunResult::failed("Codex App Server did not return a turn id.")),
                    },
                    Err(error) => this.finish(&turn, RunResult::failed(format!("{error:#}"))),
                }
            });
        }
        tokio::select! {
            result = settled => result.unwrap_or_else(|_| RunResult::failed("Codex App Server stopped.")),
            () = ctx.cancel.cancelled() => {
                self.interrupt(server, &turn).await;
                self.finish(&turn, RunResult::cancelled("The Codex run was cancelled."));
                RunResult::cancelled("The Codex run was cancelled.")
            }
            () = tokio::time::sleep(ctx.timeout) => {
                self.interrupt(server, &turn).await;
                self.finish(&turn, RunResult::failed("The coding agent took too long to respond."));
                RunResult::failed("The coding agent took too long to respond.")
            }
        }
    }

    async fn interrupt(&self, server: &AppServer, turn: &Turn) {
        let turn_id = {
            let mut state = turn.state.lock();
            state.cancel_requested = true;
            state.turn_id.clone()
        };
        if let Some(turn_id) = turn_id {
            let _ = server.request("turn/interrupt", json!({ "threadId": turn.thread_id, "turnId": turn_id })).await;
        }
    }

    /// Settle the turn once; an unanswered question must not keep App Server waiting.
    fn finish(&self, turn: &Arc<Turn>, result: RunResult) {
        {
            let mut turns = self.turns.lock();
            if !turns.get(&turn.request_id).is_some_and(|current| Arc::ptr_eq(current, turn)) {
                return;
            }
            turns.remove(&turn.request_id);
        }
        let (settle, questions) = {
            let mut state = turn.state.lock();
            (state.settle.take(), std::mem::take(&mut state.questions))
        };
        for rpc_id in questions.values() {
            turn.server.respond(rpc_id, json!({ "answers": {} }));
        }
        if let Some(settle) = settle {
            let _ = settle.send(result);
        }
    }

    pub async fn steer(self: &Arc<Self>, id: &str, prompt: String, images: Vec<Bytes>) -> bool {
        let Some(turn) = self.turns.lock().get(id).cloned() else { return false };
        {
            let mut state = turn.state.lock();
            if state.turn_id.is_none() {
                state.queued_steering.push((prompt, images));
                return true;
            }
        }
        send_steering(&turn.server, &turn, &prompt, &images).await.is_ok()
    }

    /// Settle a held `request_user_input` call with the person's answers.
    pub fn answer(&self, id: &str, item_id: &str, answers: &Value) -> bool {
        let Some(turn) = self.turns.lock().get(id).cloned() else { return false };
        let Some(rpc_id) = turn.state.lock().questions.remove(item_id) else { return false };
        let answers: serde_json::Map<String, Value> = answers.as_object().into_iter().flatten().map(|(id, list)| (id.clone(), json!({ "answers": list }))).collect();
        turn.server.respond(&rpc_id, json!({ "answers": answers }));
        true
    }

    pub async fn shutdown(&self) {
        for turn in self.turns.lock().drain().map(|(_, turn)| turn).collect::<Vec<_>>() {
            if let Some(settle) = turn.state.lock().settle.take() {
                let _ = settle.send(RunResult::cancelled("The Codex run was cancelled."));
            }
        }
        if let Some(server) = self.server.lock().await.take() {
            server.shutdown().await;
        }
    }

    fn turn_for(&self, thread_id: &str, turn_id: Option<&str>) -> Option<Arc<Turn>> {
        self.turns
            .lock()
            .values()
            .find(|turn| turn.thread_id == thread_id && (turn_id.is_none() || turn.state.lock().turn_id.as_deref().is_none_or(|current| Some(current) == turn_id)))
            .cloned()
    }

    /// App Server asks the client things mid-turn; every request gets a reply.
    fn server_request(&self, server: &AppServer, id: Value, method: &str, params: &Value) {
        let turn = params["threadId"].as_str().and_then(|thread| self.turn_for(thread, params["turnId"].as_str()));
        if method_is(method, "item/tool/requestUserInput") {
            let questions = reply_questions(&params["questions"]);
            let item = params["itemId"].as_str().filter(|item| item.chars().count() <= 240).map(item_id);
            match (turn, item) {
                (Some(turn), Some(item)) if !questions.is_empty() => {
                    turn.state.lock().questions.insert(item.clone(), id);
                    turn.events.trace(json!({ "kind": "question", "itemId": item, "questions": questions, "transport": "reply", "blocking": params["isBlocking"] != false }));
                }
                _ => server.respond(&id, json!({ "answers": {} })),
            }
            return;
        }
        if method_is(method, "mcpServer/elicitation/request") {
            server.respond(&id, json!({ "action": "decline", "content": null, "_meta": null }));
            return;
        }
        // Approvals cannot arrive under `approvalPolicy: never`.
        server.respond_error(&id, &format!("Powermove does not handle {method}."));
    }

    /// `request_user_input_async` arrives as an agent message carrying `questions`.
    fn question_message(turn: &Turn, item: &Value) -> bool {
        if normalized_type(&item["type"]) != "agentmessage" {
            return false;
        }
        let Some(id) = trace_item_id(item) else { return false };
        let mut state = turn.state.lock();
        if state.question_messages.contains(&id) {
            return true;
        }
        let questions = message_questions(&id, &item["questions"]);
        if questions.is_empty() {
            return false;
        }
        state.question_messages.insert(id.clone());
        drop(state);
        turn.events.trace(json!({ "kind": "question", "itemId": id, "questions": questions, "transport": "message", "blocking": false }));
        true
    }

    fn notification(&self, method: &str, params: &Value) {
        let thread_id = params["threadId"].as_str();
        if method_is(method, "warning") {
            let text = params["message"].as_str().map(progress_line).unwrap_or_default();
            for turn in self.turns.lock().values() {
                if !text.is_empty() && thread_id.is_none_or(|thread| thread == turn.thread_id) {
                    turn.events.progress(&text);
                }
            }
            return;
        }
        if method_is(method, "serverRequest/resolved") {
            for turn in self.turns.lock().values().filter(|turn| Some(turn.thread_id.as_str()) == thread_id) {
                let mut state = turn.state.lock();
                let closed: Vec<String> = state.questions.iter().filter(|(_, rpc)| **rpc == params["requestId"]).map(|(item, _)| item.clone()).collect();
                for item in closed {
                    state.questions.remove(&item);
                    turn.events.trace(json!({ "kind": "question-closed", "itemId": item }));
                }
            }
            return;
        }
        let turn_id = params["turnId"].as_str().or_else(|| if method_is(method, "turn/completed") { params["turn"]["id"].as_str() } else { None });
        let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) else { return };
        let Some(turn) = self.turn_for(thread_id, Some(turn_id)) else { return };
        let item = &params["item"];
        if method_is(method, "item/commandExecution/outputDelta") {
        } else if method_is(method, "error") && params["error"].is_object() {
            if params["willRetry"] == true {
                turn.events.progress("Connection interrupted. Retrying…");
            }
        } else if method_is(method, "model/rerouted") {
            if let Some(model) = params["toModel"].as_str().filter(|model| model.len() <= 120) {
                turn.events.progress(&format!("Continuing on {model}"));
            }
        } else if method_is(method, "item/started") && item.is_object() {
            if !Self::question_message(&turn, item) {
                if let Some(step) = tool_start(item) {
                    turn.events.trace(step);
                }
            }
        } else if method_is(method, "item/agentMessage/delta") {
            let Some(delta) = params["delta"].as_str() else { return };
            let item = params["itemId"].as_str().filter(|item| item.chars().count() <= 240).map(item_id);
            {
                let mut state = turn.state.lock();
                // A question's text is shown by its card, not repeated as prose.
                if item.as_ref().is_some_and(|item| state.question_messages.contains(item)) {
                    return;
                }
                match &item {
                    Some(item) => drop(state.delta_items.insert(item.clone())),
                    None => state.unscoped_delta = true,
                }
            }
            let text = fragment_text(delta);
            if !text.is_empty() {
                turn.events.trace(trace::text_step("answer", &text));
            }
        } else if method_is(method, "item/reasoning/textDelta") || method_is(method, "item/reasoning/summaryTextDelta") {
            let Some(delta) = params["delta"].as_str() else { return };
            let text = fragment_text(delta);
            if !text.is_empty() {
                turn.events.trace(trace::text_step("thought", &text));
            }
            if method_is(method, "item/reasoning/summaryTextDelta") {
                turn.events.progress(&progress_line(delta));
            }
        } else if method_is(method, "item/completed") && item.is_object() {
            if let Some(step) = tool_end(item) {
                turn.events.trace(step);
                return;
            }
            if Self::question_message(&turn, item) || normalized_type(&item["type"]) != "agentmessage" {
                return;
            }
            let Some(text) = item["text"].as_str() else { return };
            let streamed = {
                let mut state = turn.state.lock();
                // An async message (a mid-run update) is never the structured final answer.
                if item["delivery"] != "async" {
                    state.final_text = text.to_owned();
                }
                state.unscoped_delta || trace_item_id(item).is_some_and(|id| state.delta_items.contains(&id))
            };
            if !streamed {
                let text = fragment_text(text);
                if !text.is_empty() {
                    turn.events.trace(trace::text_step("answer", &text));
                }
            }
        } else if method_is(method, "turn/completed") {
            let final_text = turn.state.lock().final_text.clone();
            let result = match params["turn"]["status"].as_str() {
                Some("completed") if !final_text.is_empty() => RunResult::done(final_text, "editor"),
                Some("completed") => RunResult::failed("ChatGPT generation completed without a result."),
                Some("interrupted") => RunResult::cancelled("The Codex run was cancelled."),
                _ => RunResult::failed(app_server_error(&params["turn"], "ChatGPT generation failed.")),
            };
            self.finish(&turn, result);
        }
    }

    fn exited(&self, message: &str) {
        let turns: Vec<Arc<Turn>> = self.turns.lock().values().cloned().collect();
        for turn in turns {
            self.finish(&turn, RunResult::failed(message.to_owned()));
        }
    }
}

async fn write_input(directory: &std::path::Path, prefix: &str, prompt: &str, images: &[Bytes]) -> std::io::Result<Vec<Value>> {
    let mut input = vec![json!({ "type": "text", "text": prompt })];
    for (index, image) in images.iter().enumerate() {
        let path = directory.join(format!("{prefix}-{index}.{}", image_extension(image)));
        tokio::fs::write(&path, image).await?;
        input.push(json!({ "type": "localImage", "path": path }));
    }
    Ok(input)
}

async fn send_steering(server: &AppServer, turn: &Turn, prompt: &str, images: &[Bytes]) -> anyhow::Result<()> {
    let Some(turn_id) = turn.state.lock().turn_id.clone() else { return Ok(()) };
    let prefix = format!("steer-{}", uuid::Uuid::new_v4().simple());
    let input = write_input(&turn.directory, &prefix, prompt, images).await?;
    server.request("turn/steer", json!({ "threadId": turn.thread_id, "expectedTurnId": turn_id, "input": input })).await?;
    Ok(())
}

async fn dispatch(runner: Weak<CodexApp>, server: Arc<AppServer>, mut events: mpsc::UnboundedReceiver<Incoming>) {
    while let Some(event) = events.recv().await {
        let Some(runner) = runner.upgrade() else { return };
        match event {
            Incoming::Request { id, method, params } => runner.server_request(&server, id, &method, &params),
            Incoming::Notification { method, params } => runner.notification(&method, &params),
            Incoming::Exited(message) => {
                runner.exited(&message);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_items_and_questions() {
        let start = tool_start(&json!({ "id": "i1", "type": "commandExecution", "command": "dir" })).unwrap();
        assert_eq!(start, json!({ "kind": "tool-start", "itemId": "i1", "toolName": "bash", "label": "Run", "detail": "dir" }));
        let end = tool_end(&json!({ "id": "i1", "type": "mcpToolCall", "status": "failed", "error": { "message": "nope" } })).unwrap();
        assert_eq!(end, json!({ "kind": "tool-end", "itemId": "i1", "isError": true, "output": "nope" }));
        let questions = reply_questions(&json!([{ "id": "q", "question": "Pick", "options": [], "isSecret": true }]));
        assert_eq!(questions[0]["allowOther"], true);
        assert_eq!(questions[0]["secret"], true);
        assert_eq!(message_questions("m", &json!([{ "title": "Why?", "options": ["A"] }]))[0]["id"], "m-0");
        let config = live_inspection_config(Some(&NativeTools { url: "http://x/mcp".into(), token: "t".into(), names: vec!["get_project_state".into()], timeout_sec: 120, outside_sandbox: false }));
        assert_eq!(config["mcp_servers"]["powermove"]["enabled_tools"], json!(["get_project_state"]));
    }
}
