//! App-owned subagents (port of `agent-orchestrator.ts`).
//!
//! Delegation belongs to a conversation, not to a provider session, so every
//! provider (Codex, Claude, a compatible model) gets the same tools:
//! `delegate_task` starts a child run on any connected provider,
//! `task_status` / `task_cancel` / `task_send` manage it, and the `thread_*`
//! tools reach ordinary conversations through the owning window. Task records
//! are journaled per conversation before a provider starts, so retries reuse
//! the same task and a crash leaves an `interrupted` task, never a duplicate.
//! When a parent's turn ends with children still working, the parent waits
//! and is resumed with their results.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use futures::FutureExt;
use futures::future::{BoxFuture, join_all};
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;

use super::RunEvents;
use super::orchestrator_providers::{Target, parse_provider, provider_id};
use super::orchestrator_threads::{ThreadMonitor, ThreadRun};
use super::request::{Mode, Provider, RunRequest, RunResult};
use super::trace;

pub const MAX_TASKS: usize = 32;
pub const MAX_DEPTH: usize = 4;
pub const MAX_ACTIVE_TASKS: usize = 8;
const MAX_WAIT_MS: u64 = 90_000;
const MAX_UPDATES: usize = 64;
const MAX_WATCHES: usize = 32;
const HISTORY: usize = 512;
const SUMMARY_CHARS: usize = 200_000;
pub const THREAD_TOOLS: [&str; 7] = ["thread_launch", "thread_list", "thread_read", "thread_send", "thread_watch", "thread_unwatch", "thread_cancel"];
/// `ORCHESTRATION_TOOL_NAMES`.
pub const TOOL_NAMES: [&str; 12] = [
    "orchestrator_capabilities", "delegate_task", "task_status", "task_cancel", "task_send",
    "thread_launch", "thread_list", "thread_read", "thread_send", "thread_watch", "thread_unwatch", "thread_cancel",
];

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn thread_of(request: &RunRequest) -> String {
    request.thread_id.clone().unwrap_or_else(|| "legacy".into())
}

fn scope_key(request: &RunRequest) -> String {
    format!("{}/{}", request.project_id, thread_of(request))
}

fn result_text(result: &RunResult) -> &str {
    match result {
        RunResult::Done { text, .. } => text,
        RunResult::Failed { error, .. } => error,
    }
}

/* ── records ─────────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl Status {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkState {
    Working,
    WaitingForChildren,
    ResultAvailable,
}

/// `AgentTask`, the renderer's task row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub task_id: String,
    pub parent_thread_id: String,
    pub child_thread_id: String,
    pub child_run_id: String,
    pub parent_task_id: Option<String>,
    pub request_id: String,
    pub title: String,
    pub provider_instance_id: String,
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    pub status: Status,
    work_state: WorkState,
    has_pending_child_runs: bool,
    pub summary: Option<String>,
    pub progress: String,
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    wait_timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Update {
    id: String,
    fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    task: Task,
    acknowledged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_request_id: Option<String>,
    fingerprint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    updates: Vec<Update>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_thread_id: Option<String>,
    records: Vec<Stored>,
}

fn snapshot(record: &Stored, wait_timed_out: bool) -> Value {
    let mut task = record.task.clone();
    task.wait_timed_out = wait_timed_out;
    serde_json::to_value(task).unwrap_or(Value::Null)
}

/* ── host seam ───────────────────────────────────────────────── */

/// One provider execution as the host runs it.
pub struct ExecContext {
    /// The conversation's root run, which owns the window and tool session.
    pub root_id: String,
    pub is_child: bool,
    pub role: Option<String>,
    pub events: RunEvents,
    pub cancel: CancellationToken,
}

/// What the orchestrator needs from the host.
pub trait Backend: Send + Sync + 'static {
    /// `OrchestrationProvider[]` (see `orchestrator_providers::providers`).
    fn providers(&self) -> BoxFuture<'static, Vec<Value>>;
    fn execute(&self, request: RunRequest, ctx: ExecContext) -> BoxFuture<'static, RunResult>;
    /// Live steering of an executing run; `false` when the transport has none.
    fn steer(&self, run_id: &str, prompt: &str) -> BoxFuture<'static, bool>;
    /// Answer a held question of an executing run.
    fn answer(&self, run_id: &str, item_id: &str, answers: &Value) -> bool;
    /// `__agent_thread_control` in the root run's window.
    fn thread_control(&self, root_id: &str, tool: &str, args: Value) -> BoxFuture<'static, Result<Value>>;
    fn has_thread_control(&self) -> bool;
}

/* ── live state ──────────────────────────────────────────────── */

struct Signals {
    done: watch::Sender<Option<RunResult>>,
    wake: Notify,
}

struct Node {
    scope: String,
    depth: usize,
    task_id: Option<String>,
    role: Option<String>,
    request: RunRequest,
    next_request: Option<RunRequest>,
    stopped: bool,
    controlling: bool,
    executing: bool,
    exec_cancel: CancellationToken,
    update_count: usize,
    signals: Arc<Signals>,
}

struct WatchEntry {
    owner_thread: String,
    thread_id: String,
    run_id: String,
    acknowledged: bool,
    released: bool,
    removed: watch::Sender<bool>,
}

struct Scope {
    file: PathBuf,
    records: Vec<Stored>,
    root_id: String,
    root_thread: Option<String>,
    events: RunEvents,
    watches: HashMap<String, WatchEntry>,
    save: Arc<tokio::sync::Mutex<()>>,
}

impl Scope {
    fn record(&self, task_id: &str) -> Option<&Stored> {
        self.records.iter().find(|record| record.task.task_id == task_id)
    }

    fn record_mut(&mut self, task_id: &str) -> Option<&mut Stored> {
        self.records.iter_mut().find(|record| record.task.task_id == task_id)
    }
}

struct ChildQuestion {
    run_id: String,
    item_id: String,
    root_id: String,
    questions: Value,
    blocking: bool,
}

#[derive(Default)]
struct State {
    nodes: HashMap<String, Node>,
    scopes: HashMap<String, Scope>,
    /// scope key → (root run id, finished)
    active: HashMap<String, (String, watch::Sender<bool>)>,
    cancelled_roots: HashSet<String>,
    questions: HashMap<String, ChildQuestion>,
}

impl State {
    fn release(&mut self, threads: &ThreadMonitor, key: &str, watch_key: &str) {
        let Some(scope) = self.scopes.get_mut(key) else { return };
        let root = scope.root_id.clone();
        if let Some(watch) = scope.watches.get_mut(watch_key)
            && !watch.released
        {
            watch.released = true;
            threads.unwatch(&root, &watch.run_id);
        }
    }
}

pub struct Orchestrator {
    directory: PathBuf,
    backend: Arc<dyn Backend>,
    state: Mutex<State>,
    threads: Arc<ThreadMonitor>,
    this: Weak<Orchestrator>,
}

/// Strictly validated tool arguments (the zod schemas of the TS tools).
struct Args<'a>(&'a Map<String, Value>);

fn args<'a>(raw: &'a Value, allowed: &[&str]) -> Result<Args<'a>> {
    let Some(map) = raw.as_object() else { bail!("Arguments must be an object.") };
    if let Some(key) = map.keys().find(|key| !allowed.contains(&key.as_str())) {
        bail!("Unknown argument: {key}");
    }
    Ok(Args(map))
}

impl Args<'_> {
    fn text(&self, key: &str, max: usize, trim: bool) -> Result<Option<String>> {
        match self.0.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => {
                let text = if trim { text.trim() } else { text.as_str() };
                let count = text.chars().count();
                if count == 0 || count > max {
                    bail!("{key} must be 1-{max} characters.");
                }
                Ok(Some(text.to_owned()))
            }
            Some(_) => bail!("{key} must be a string."),
        }
    }

    fn required(&self, key: &str, max: usize, trim: bool) -> Result<String> {
        self.text(key, max, trim)?.ok_or_else(|| anyhow!("{key} is required."))
    }

    fn int(&self, key: &str, default: u64, min: u64, max: u64) -> Result<u64> {
        match self.0.get(key) {
            None => Ok(default),
            Some(value) => value.as_u64().filter(|value| (min..=max).contains(value)).ok_or_else(|| anyhow!("{key} must be an integer from {min} to {max}.")),
        }
    }

    fn choice(&self, key: &str, options: &[&str], default: Option<&str>) -> Result<Option<String>> {
        match self.0.get(key) {
            None => Ok(default.map(str::to_owned)),
            Some(value) => value.as_str().filter(|value| options.contains(value)).map(|value| Some(value.to_owned())).ok_or_else(|| anyhow!("{key} must be one of {}.", options.join(", "))),
        }
    }

    /// Thread and run ids: `/^[A-Za-z0-9_-]{1,120}$/`.
    fn id(&self, key: &str) -> Result<Option<String>> {
        let value = self.text(key, 120, false)?;
        if value.as_ref().is_some_and(|id| !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')) {
            bail!("{key} is not a valid id.");
        }
        Ok(value)
    }

    fn target(&self) -> Result<Option<Target>> {
        self.0.get("target").map(Target::parse).transpose()
    }
}

/// Provider, model and effort for a run.
#[derive(Debug, Clone)]
struct Selection {
    provider: Provider,
    model: Option<String>,
    effort: Option<String>,
}

impl Selection {
    fn of(request: &RunRequest) -> Self {
        Self { provider: request.provider, model: request.model.clone(), effort: request.reasoning_effort.clone() }
    }

    fn apply(&self, request: &mut RunRequest) {
        request.provider = self.provider;
        request.model = self.model.clone();
        request.reasoning_effort = self.effort.clone();
    }

    fn insert(&self, args: &mut Value) {
        args["provider"] = json!(provider_id(self.provider));
        args["model"] = json!(self.model);
        args["reasoningEffort"] = json!(self.effort);
    }
}

impl Orchestrator {
    /// Task journals live under `directory` (`<userData>/Agent Tasks`).
    pub fn new(directory: PathBuf, backend: Arc<dyn Backend>) -> Arc<Self> {
        Arc::new_cyclic(|this| Self { directory, backend, state: Mutex::default(), threads: ThreadMonitor::new(), this: this.clone() })
    }

    fn arc(&self) -> Arc<Self> {
        self.this.upgrade().expect("orchestrator is alive while in use")
    }

    pub fn is_tool(name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    /* ── root runs ── */

    /// Run a conversation turn: the provider run plus every child it delegates.
    /// `cancel` stops the run and its descendants.
    pub async fn run(&self, request: RunRequest, events: RunEvents, cancel: CancellationToken) -> RunResult {
        let key = scope_key(&request);
        let id = request.id.clone();
        loop {
            let previous = {
                let mut state = self.state.lock();
                match state.active.get(&key) {
                    None => {
                        state.active.insert(key.clone(), (id.clone(), watch::channel(false).0));
                        None
                    }
                    Some((previous, finished)) => {
                        let stopping = state.nodes.get(previous).is_some_and(|node| node.stopped) || state.cancelled_roots.contains(previous);
                        Some(stopping.then(|| finished.subscribe()))
                    }
                }
            };
            match previous {
                None => break,
                Some(None) => return RunResult::failed("An agent is already running in this conversation."),
                Some(Some(mut finished)) => drop(finished.wait_for(|done| *done).await),
            }
        }
        if let Err(error) = self.threads.begin(&request) {
            self.end_root(&key, &id);
            return RunResult::failed(error.to_string());
        }
        if let Err(error) = self.load_scope(&key, &request, events).await {
            let result = RunResult::failed(format!("{error:#}"));
            self.threads.finish(&id, &result);
            self.end_root(&key, &id);
            return result;
        }
        let interrupted: Vec<Value> = {
            let state = self.state.lock();
            state.scopes[&key].records.iter().filter(|record| record.task.status == Status::Interrupted && !record.acknowledged).map(|record| snapshot(record, false)).collect()
        };
        for task in interrupted {
            self.scope_trace(&key, json!({ "kind": "task", "task": task }));
        }
        let signals = Arc::new(Signals { done: watch::channel(None).0, wake: Notify::new() });
        {
            let mut state = self.state.lock();
            let stopped = state.cancelled_roots.contains(&id) || cancel.is_cancelled();
            let node = Node { scope: key.clone(), depth: 0, task_id: None, role: None, request, next_request: None, stopped, controlling: false, executing: false, exec_cancel: CancellationToken::new(), update_count: 0, signals: signals.clone() };
            state.nodes.insert(id.clone(), node);
        }
        let watcher = {
            let this = self.arc();
            let id = id.clone();
            tokio::spawn(async move {
                cancel.cancelled().await;
                this.cancel_run(&id).await;
            })
        };
        let result = self.arc().run_node(id.clone()).await;
        watcher.abort();
        signals.done.send_replace(Some(result.clone()));
        self.threads.finish(&id, &result);
        self.end_root(&key, &id);
        result
    }

    /// Terminal records are durable; execution objects are transient.
    fn end_root(&self, key: &str, id: &str) {
        let mut state = self.state.lock();
        let watch_keys: Vec<String> = state.scopes.get(key).map(|scope| scope.watches.keys().cloned().collect()).unwrap_or_default();
        for watch_key in watch_keys {
            state.release(&self.threads, key, &watch_key);
        }
        if let Some(scope) = state.scopes.remove(key) {
            for watch in scope.watches.values() {
                watch.removed.send_replace(true);
            }
        }
        state.nodes.retain(|_, node| node.scope != key);
        state.questions.retain(|_, question| question.root_id != id);
        state.cancelled_roots.remove(id);
        if let Some((_, finished)) = state.active.remove(key) {
            finished.send_replace(true);
        }
    }

    /// Stop a run (root or child) and its descendants.
    pub async fn cancel_run(&self, run_id: &str) {
        self.state.lock().cancelled_roots.insert(run_id.to_owned());
        let exists = self.state.lock().nodes.contains_key(run_id);
        if exists {
            self.arc().stop_node(run_id.to_owned(), "The task was stopped.".into(), true).await;
        }
    }

    /// `codex:cancel-task`: the person stops one task of their conversation.
    pub async fn cancel_task(&self, root_id: &str, task_id: &str) -> Result<()> {
        let (key, child) = {
            let state = self.state.lock();
            let root = state.nodes.get(root_id).filter(|node| node.depth == 0).ok_or_else(|| anyhow!("This task does not belong to the active conversation."))?;
            let record = state.scopes.get(&root.scope).and_then(|scope| scope.record(task_id)).ok_or_else(|| anyhow!("This task does not belong to the active conversation."))?;
            let child = (!record.task.status.terminal() && state.nodes.contains_key(&record.task.child_run_id)).then(|| record.task.child_run_id.clone());
            (root.scope.clone(), child)
        };
        // The model did not issue this cancellation: it must still receive the
        // outcome before concluding that its delegated work succeeded.
        if let Some(child) = child {
            self.arc().stop_node(child, "Stopped by the user.".into(), false).await;
        }
        // A completed root has already saved; don't overwrite a newer run's journal.
        if self.state.lock().nodes.contains_key(root_id) {
            self.save(&key).await?;
        }
        Ok(())
    }

    /// Answers to a child's question carry the child's run id and the
    /// renderer-facing item id; returns the provider's own item id.
    pub fn route_answer(&self, run_id: &str, item_id: &str) -> Result<String> {
        let state = self.state.lock();
        match state.questions.get(item_id) {
            None => Ok(item_id.to_owned()),
            Some(question) if question.run_id != run_id => bail!("This question belongs to another subagent."),
            Some(question) => Ok(question.item_id.clone()),
        }
    }

    /// The root run that owns `run_id` (itself for a root).
    pub fn root_of(&self, run_id: &str) -> Option<String> {
        let state = self.state.lock();
        let node = state.nodes.get(run_id)?;
        state.scopes.get(&node.scope).map(|scope| scope.root_id.clone())
    }

    /// Remove a project's journals; returns the child workspace ids to delete.
    pub async fn forget_project(&self, project_id: &str) -> Result<Vec<String>> {
        let prefix = format!("{project_id}/");
        if self.state.lock().active.keys().any(|key| key.starts_with(&prefix)) {
            bail!("Subagents are still using this project.");
        }
        self.threads.forget(project_id);
        let directory = self.directory.join(sha256_hex(project_id));
        let mut ids = HashSet::new();
        let mut entries = match tokio::fs::read_dir(&directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.len() == 69 && name.ends_with(".json")) {
                continue;
            }
            let journal: Journal = serde_json::from_slice(&tokio::fs::read(entry.path()).await?).context("invalid subagent journal")?;
            if let Some(root) = journal.root_thread_id.filter(|thread| thread.starts_with("agent-thread-")) {
                ids.insert(format!("thread-{root}"));
            }
            ids.extend(journal.records.iter().map(|record| format!("subagent-{}", record.task.child_thread_id)));
        }
        tokio::fs::remove_dir_all(&directory).await?;
        Ok(ids.into_iter().collect())
    }

    pub async fn shutdown(&self) {
        self.threads.shutdown();
        let roots: Vec<String> = self.state.lock().nodes.iter().filter(|(_, node)| node.depth == 0).map(|(id, _)| id.clone()).collect();
        join_all(roots.into_iter().map(|id| self.arc().stop_node(id, "The task was stopped.".into(), true))).await;
    }

    /* ── tools ── */

    /// One orchestration tool call from run `run_id`; the result is the tool's JSON.
    pub async fn call(&self, run_id: &str, tool: &str, raw: Value) -> Result<Value> {
        if !self.state.lock().nodes.get(run_id).is_some_and(|node| !node.stopped) {
            bail!("This agent run is no longer active.");
        }
        match tool {
            "orchestrator_capabilities" => {
                if raw.as_object().is_some_and(|map| !map.is_empty()) {
                    bail!("Capabilities takes no arguments.");
                }
                self.capabilities(run_id).await
            }
            "delegate_task" => self.delegate(run_id, &raw).await,
            "task_send" => self.send_task(run_id, &raw).await,
            "task_status" | "task_cancel" => {
                let input = if tool == "task_status" { args(&raw, &["taskId"])? } else { args(&raw, &["taskId", "reason", "clientRequestId"])? };
                let task_id = input.required("taskId", 240, false)?;
                let reason = if tool == "task_cancel" { input.text("reason", 2_000, false)? } else { None };
                if tool == "task_cancel" {
                    input.text("clientRequestId", 240, false)?;
                }
                let (key, child) = {
                    let mut state = self.state.lock();
                    let (key, thread) = self.owner(&state, run_id)?;
                    let record = state.scopes.get_mut(&key).and_then(|scope| scope.record_mut(&task_id)).filter(|record| record.task.parent_thread_id == thread).ok_or_else(|| anyhow!("This task does not belong to this parent conversation."))?;
                    if tool == "task_cancel" || record.task.status.terminal() {
                        record.acknowledged = true;
                    }
                    let child = record.task.child_run_id.clone();
                    let active = !record.task.status.terminal() && state.nodes.contains_key(&child);
                    (key, active.then_some(child))
                };
                if tool == "task_cancel"
                    && let Some(child) = child
                {
                    self.arc().stop_node(child, reason.unwrap_or_else(|| "The task was stopped.".into()), true).await;
                }
                self.save(&key).await?;
                let state = self.state.lock();
                let record = state.scopes.get(&key).and_then(|scope| scope.record(&task_id)).ok_or_else(|| anyhow!("This task does not belong to this parent conversation."))?;
                if tool == "task_cancel" {
                    let status = if record.task.status.terminal() { serde_json::to_value(record.task.status)? } else { json!("cancel_requested") };
                    return Ok(json!({ "taskId": task_id, "status": status }));
                }
                let mut value = snapshot(record, false);
                let child = &record.task.child_run_id;
                value["questions"] = state.questions.iter().filter(|(_, question)| &question.run_id == child).map(|(item, question)| json!({ "itemId": item, "blocking": question.blocking, "questions": question.questions })).collect();
                Ok(value)
            }
            tool if THREAD_TOOLS.contains(&tool) => self.thread_tool(run_id, tool, &raw).await,
            _ => bail!("Unknown orchestration tool: {tool}"),
        }
    }

    /// The scope key and parent thread of node `run_id`.
    fn owner(&self, state: &State, run_id: &str) -> Result<(String, String)> {
        let node = state.nodes.get(run_id).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
        Ok((node.scope.clone(), thread_of(&node.request)))
    }

    async fn capabilities(&self, run_id: &str) -> Result<Value> {
        let request = self.state.lock().nodes.get(run_id).map(|node| node.request.clone()).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
        let threads = self.backend.has_thread_control();
        Ok(json!({
            "parentThreadId": thread_of(&request),
            "inheritedProviderInstanceId": provider_id(request.provider),
            "inheritedModel": request.model,
            "runtimeMode": match request.access { super::request::Access::Editor => "editor", super::request::Access::Project => "project", super::request::Access::Computer => "computer" },
            "interactionMode": if request.mode == Mode::Editor { "plan" } else { "default" },
            "providers": self.backend.providers().await,
            "features": {
                "appOwnedSubagents": true, "crossProviderSubagents": true, "cancellation": true, "automaticResultDelivery": true, "liveSteering": true,
                "threadManagement": threads, "threadWatching": threads, "maxTasks": MAX_TASKS, "maxDepth": MAX_DEPTH, "maxParallelTasks": MAX_ACTIVE_TASKS,
            },
        }))
    }

    /// `selectTarget`: a provider/model/effort the catalog allows.
    async fn select(&self, base: &Selection, target: Option<&Target>) -> Result<Selection> {
        let providers = self.backend.providers().await;
        let provider = target.and_then(|target| target.provider).unwrap_or(base.provider);
        let id = provider_id(provider);
        let entry = providers.iter().find(|entry| entry["providerInstanceId"] == id);
        if !entry.is_some_and(|entry| entry["canRunChildTask"] == true) {
            let name = entry.and_then(|entry| entry["displayName"].as_str()).unwrap_or(id);
            let constraints = entry.and_then(|entry| entry["constraints"].as_array()).map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_else(|| "Connect it in Settings.".into());
            bail!("{name} is unavailable. {constraints}");
        }
        let entry = entry.expect("checked above");
        let cross = provider != base.provider;
        let inherited = if provider == Provider::Compatible && base.model.as_deref() == Some("configured") { None } else { base.model.clone() };
        let models = entry["models"].as_array().cloned().unwrap_or_default();
        let model = target.and_then(|target| target.model.clone()).or(if cross { None } else { inherited }).or_else(|| models.first().and_then(|model| model["id"].as_str()).map(str::to_owned));
        let chosen = models.iter().find(|item| model.is_some() && item["id"].as_str() == model.as_deref()).ok_or_else(|| anyhow!("Choose a model returned by orchestrator_capabilities."))?;
        let efforts: Vec<String> = chosen["options"].as_array().into_iter().flatten().filter(|option| option["id"] == "reasoningEffort").flat_map(|option| option["options"].as_array().cloned().unwrap_or_default()).filter_map(|option| option["id"].as_str().map(str::to_owned)).collect();
        let requested = target.and_then(|target| target.effort.clone());
        if requested.as_ref().is_some_and(|effort| !efforts.contains(effort)) {
            bail!("This model does not support the requested reasoning effort. Read orchestrator_capabilities.");
        }
        let effort = requested.or_else(|| base.effort.clone().filter(|effort| !cross && efforts.contains(effort)));
        Ok(Selection { provider, model, effort })
    }

    async fn delegate(&self, parent_id: &str, raw: &Value) -> Result<Value> {
        let input = args(raw, &["task", "title", "role", "mode", "timeoutMs", "clientRequestId", "target"])?;
        let task = input.required("task", 200_000, true)?;
        let title = input.text("title", 160, true)?;
        let role = input.choice("role", &["implementation", "research", "review", "design", "test", "general"], None)?;
        let wait = input.choice("mode", &["async", "wait"], Some("async"))?.as_deref() == Some("wait");
        let timeout = input.int("timeoutMs", MAX_WAIT_MS, 0, MAX_WAIT_MS)?;
        let client_request_id = input.text("clientRequestId", 240, false)?;
        let target = input.target()?;
        let fingerprint = sha256_hex(&json!({ "task": task, "title": title, "role": role, "target": target.as_ref().map(Target::to_json) }).to_string());
        let find_previous = || -> Result<Option<(String, String)>> {
            let Some(client) = &client_request_id else { return Ok(None) };
            let state = self.state.lock();
            let (key, thread) = self.owner(&state, parent_id)?;
            let previous = state.scopes.get(&key).and_then(|scope| scope.records.iter().find(|record| record.task.parent_thread_id == thread && record.client_request_id.as_ref() == Some(client)));
            match previous {
                Some(record) if record.fingerprint != fingerprint => bail!("This clientRequestId already belongs to a different task. Use a new key for a new round."),
                Some(record) => Ok(Some((key, record.task.task_id.clone()))),
                None => Ok(None),
            }
        };
        if let Some((key, task_id)) = find_previous()? {
            return self.wait_for_task(&key, &task_id, wait, timeout).await;
        }
        let (depth, base) = {
            let state = self.state.lock();
            let node = state.nodes.get(parent_id).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
            (node.depth, Selection::of(&node.request))
        };
        if depth >= MAX_DEPTH {
            bail!("Subagents can nest up to {MAX_DEPTH} levels.");
        }
        let selection = self.select(&base, target.as_ref()).await?;
        // A concurrent retry may have finished discovery while this call waited.
        if let Some((key, task_id)) = find_previous()? {
            return self.wait_for_task(&key, &task_id, wait, timeout).await;
        }
        let task_id = uuid::Uuid::new_v4().to_string();
        let child_thread = uuid::Uuid::new_v4().to_string();
        let child_id = format!("subagent-{}", uuid::Uuid::new_v4());
        let key = {
            let mut state = self.state.lock();
            let parent = state.nodes.get(parent_id).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
            if parent.stopped {
                bail!("This agent run was stopped.");
            }
            let key = parent.scope.clone();
            let parent_thread = thread_of(&parent.request);
            let parent_task = parent.task_id.clone();
            let mut request = parent.request.clone();
            request.id = child_id.clone();
            request.thread_id = Some(child_thread.clone());
            request.prompt = task.clone();
            selection.apply(&mut request);
            let scope = state.scopes.get_mut(&key).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
            let root = scope.root_id.clone();
            let in_run: Vec<&Stored> = scope.records.iter().filter(|record| record.task.request_id == root).collect();
            if in_run.len() >= MAX_TASKS {
                bail!("A run can delegate up to {MAX_TASKS} tasks.");
            }
            if in_run.iter().filter(|record| !record.task.status.terminal()).count() >= MAX_ACTIVE_TASKS {
                bail!("Up to {MAX_ACTIVE_TASKS} subagents can work at once. Wait for a task to finish before delegating another.");
            }
            scope.records.push(Stored {
                task: Task {
                    task_id: task_id.clone(),
                    parent_thread_id: parent_thread,
                    child_thread_id: child_thread,
                    child_run_id: child_id.clone(),
                    parent_task_id: parent_task,
                    request_id: root,
                    title: title.unwrap_or_else(|| trace::truncate(&collapse(&task), 160)),
                    provider_instance_id: provider_id(selection.provider).into(),
                    model: selection.model.clone(),
                    reasoning_effort: selection.effort.clone(),
                    status: Status::Queued,
                    work_state: WorkState::Working,
                    has_pending_child_runs: false,
                    summary: None,
                    progress: String::new(),
                    started_at: now_ms(),
                    ended_at: None,
                    wait_timed_out: false,
                },
                acknowledged: false,
                client_request_id,
                fingerprint,
                updates: Vec::new(),
            });
            let signals = Arc::new(Signals { done: watch::channel(None).0, wake: Notify::new() });
            let node = Node { scope: key.clone(), depth: depth + 1, task_id: Some(task_id.clone()), role, request, next_request: None, stopped: false, controlling: false, executing: false, exec_cancel: CancellationToken::new(), update_count: 0, signals };
            state.nodes.insert(child_id.clone(), node);
            key
        };
        self.emit(&child_id);
        // Persist the reservation before starting a provider: retries cannot
        // create duplicate work, and a crash leaves a recoverable task.
        if let Err(error) = self.save(&key).await {
            let signals = {
                let mut state = self.state.lock();
                if let Some(record) = state.scopes.get_mut(&key).and_then(|scope| scope.record_mut(&task_id)) {
                    record.task.status = Status::Failed;
                    record.task.work_state = WorkState::ResultAvailable;
                    record.task.summary = Some("Could not save the subagent task. No provider was started.".into());
                    record.task.ended_at = Some(now_ms());
                    record.acknowledged = true;
                }
                state.nodes.get(&child_id).map(|node| node.signals.clone())
            };
            self.emit(&child_id);
            self.state.lock().nodes.remove(&child_id);
            if let Some(signals) = signals {
                signals.done.send_replace(Some(RunResult::failed("Could not save the subagent task.")));
            }
            return Err(error);
        }
        let this = self.arc();
        let child = child_id.clone();
        tokio::spawn(async move {
            let signals = this.state.lock().nodes.get(&child).map(|node| node.signals.clone());
            let result = this.clone().run_node(child).await;
            if let Some(signals) = signals {
                signals.done.send_replace(Some(result));
            }
        });
        self.wait_for_task(&key, &task_id, wait, timeout).await
    }

    async fn wait_for_task(&self, key: &str, task_id: &str, wait: bool, timeout_ms: u64) -> Result<Value> {
        type Lookup = (Value, bool, Option<watch::Receiver<Option<RunResult>>>);
        let lookup = || -> Result<Lookup> {
            let state = self.state.lock();
            let record = state.scopes.get(key).and_then(|scope| scope.record(task_id)).ok_or_else(|| anyhow!("This task does not belong to this parent conversation."))?;
            let done = state.nodes.get(&record.task.child_run_id).map(|node| node.signals.done.subscribe());
            Ok((snapshot(record, false), record.task.status.terminal(), done))
        };
        let (value, terminal, done) = lookup()?;
        if !wait {
            return Ok(value);
        }
        if let (false, Some(mut done)) = (terminal, done) {
            let _ = tokio::time::timeout(Duration::from_millis(timeout_ms), done.wait_for(Option::is_some)).await;
        }
        let terminal = {
            let mut state = self.state.lock();
            let record = state.scopes.get_mut(key).and_then(|scope| scope.record_mut(task_id)).ok_or_else(|| anyhow!("This task does not belong to this parent conversation."))?;
            if record.task.status.terminal() {
                record.acknowledged = true;
            }
            record.task.status.terminal()
        };
        if terminal {
            self.save(key).await?;
        }
        let state = self.state.lock();
        let record = state.scopes.get(key).and_then(|scope| scope.record(task_id)).ok_or_else(|| anyhow!("This task does not belong to this parent conversation."))?;
        Ok(snapshot(record, !terminal))
    }

    async fn send_task(&self, parent_id: &str, raw: &Value) -> Result<Value> {
        let input = args(raw, &["taskId", "message", "target", "clientRequestId"])?;
        let task_id = input.required("taskId", 240, false)?;
        let message = input.text("message", 50_000, true)?;
        let target = input.target()?;
        let client_request_id = input.text("clientRequestId", 240, false)?;
        if message.is_none() && target.as_ref().is_none_or(Target::is_empty) {
            bail!("Supply message or target settings.");
        }
        let fingerprint = sha256_hex(&json!({ "message": message, "target": target.as_ref().map(Target::to_json) }).to_string());
        let (key, child_id, base) = {
            let mut state = self.state.lock();
            let (key, thread) = self.owner(&state, parent_id)?;
            let record = state.scopes.get(&key).and_then(|scope| scope.record(&task_id)).filter(|record| record.task.parent_thread_id == thread).ok_or_else(|| anyhow!("This task does not belong to this parent conversation."))?;
            if let Some(previous) = client_request_id.as_ref().and_then(|id| record.updates.iter().find(|update| &update.id == id)) {
                if previous.fingerprint != fingerprint {
                    bail!("This clientRequestId belongs to another update.");
                }
                let mut value = snapshot(record, false);
                value["delivery"] = json!("already_sent");
                return Ok(value);
            }
            let terminal = record.task.status.terminal();
            let child_id = record.task.child_run_id.clone();
            let child = state.nodes.get_mut(&child_id).filter(|child| !child.stopped && !terminal).ok_or_else(|| anyhow!("This task has finished. Delegate a new task for another round."))?;
            if child.controlling {
                bail!("This task is receiving another update. Retry this update when it completes.");
            }
            if child.update_count >= MAX_UPDATES {
                bail!("A task can receive up to {MAX_UPDATES} updates. Delegate a new task for more work.");
            }
            child.controlling = true;
            (key, child_id, child.next_request.clone().unwrap_or_else(|| child.request.clone()))
        };
        let outcome = self.deliver(&key, &task_id, &child_id, base, message, target, client_request_id, fingerprint).await;
        if let Some(child) = self.state.lock().nodes.get_mut(&child_id) {
            child.controlling = false;
        }
        let delivery = outcome?;
        self.emit(&child_id);
        self.save(&key).await?;
        let state = self.state.lock();
        let record = state.scopes.get(&key).and_then(|scope| scope.record(&task_id)).ok_or_else(|| anyhow!("This task does not belong to this parent conversation."))?;
        let mut value = snapshot(record, false);
        value["delivery"] = json!(delivery);
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    async fn deliver(&self, key: &str, task_id: &str, child_id: &str, base: RunRequest, message: Option<String>, target: Option<Target>, client_request_id: Option<String>, fingerprint: String) -> Result<&'static str> {
        let selection = match &target {
            Some(target) => self.select(&Selection::of(&base), Some(target)).await?,
            None => Selection::of(&base),
        };
        let finished = || {
            let state = self.state.lock();
            let terminal = state.scopes.get(key).and_then(|scope| scope.record(task_id)).is_none_or(|record| record.task.status.terminal());
            terminal || state.nodes.get(child_id).is_none_or(|child| child.stopped)
        };
        if finished() {
            bail!("This task finished before the update arrived. Delegate a new task.");
        }
        let direction = match &message {
            Some(message) => format!("\n\nNEW DIRECTION\n{message}"),
            None => "\n\nContinue the same task using the updated model settings.".into(),
        };
        if base.prompt.chars().count() + direction.chars().count() > 180_000 {
            bail!("This task has too much accumulated context. Delegate a new task with a concise brief.");
        }
        let mut delivery = "resumed";
        let live = {
            let state = self.state.lock();
            state.nodes.get(child_id).is_some_and(|child| child.executing && child.next_request.is_none())
        };
        if let (None, Some(message), true) = (&target, &message, live) {
            let waiting = {
                let state = self.state.lock();
                state.questions.iter().find(|(_, question)| question.run_id == child_id && question.blocking).map(|(item, question)| (item.clone(), question.item_id.clone(), question.questions.clone()))
            };
            let answered = waiting.as_ref().is_some_and(|(_, original, questions)| {
                let answers: Map<String, Value> = questions.as_array().into_iter().flatten().filter_map(|question| question["id"].as_str()).map(|id| (id.to_owned(), json!([message]))).collect();
                self.backend.answer(child_id, original, &Value::Object(answers))
            });
            if let (true, Some((item, _, questions))) = (answered, waiting) {
                delivery = "answered";
                self.state.lock().questions.remove(&item);
                self.scope_trace(key, json!({ "kind": "question-closed", "itemId": item }));
                let secret = questions.as_array().into_iter().flatten().any(|question| question["secret"] == true);
                let note = if secret { "\n\nA private answer was delivered to your held question.".to_owned() } else { direction.clone() };
                if let Some(child) = self.state.lock().nodes.get_mut(child_id) {
                    child.request.prompt = format!("{}{note}", base.prompt);
                }
            } else if self.backend.steer(child_id, message).await {
                delivery = "live";
            }
        }
        {
            let mut state = self.state.lock();
            let child = state.nodes.get_mut(child_id).ok_or_else(|| anyhow!("This task has finished. Delegate a new task for another round."))?;
            match delivery {
                "live" => child.request.prompt = format!("{}{direction}", base.prompt),
                "resumed" => {
                    let mut next = base.clone();
                    selection.apply(&mut next);
                    next.prompt = format!("{}{direction}\n\nRe-read the live project state before editing. Preserve completed work and incorporate this direction.", base.prompt);
                    child.next_request = Some(next);
                    // Interrupts the current turn; the node resumes with `next_request`.
                    if child.executing {
                        child.exec_cancel.cancel();
                    }
                    child.signals.wake.notify_one();
                }
                _ => {}
            }
            child.update_count += 1;
            if let Some(record) = state.scopes.get_mut(key).and_then(|scope| scope.record_mut(task_id)) {
                if let Some(id) = client_request_id {
                    record.updates.push(Update { id, fingerprint });
                }
                record.task.provider_instance_id = provider_id(selection.provider).into();
                record.task.model = selection.model.clone();
                record.task.reasoning_effort = selection.effort.clone();
                record.task.progress = "Updating the task with new direction…".into();
            }
        }
        Ok(delivery)
    }

    async fn thread_tool(&self, run_id: &str, tool: &str, raw: &Value) -> Result<Value> {
        if !self.backend.has_thread_control() {
            bail!("Thread controls are unavailable in this connection.");
        }
        let (request, root_id, root_thread) = {
            let state = self.state.lock();
            let node = state.nodes.get(run_id).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
            let scope = state.scopes.get(&node.scope).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
            (node.request.clone(), scope.root_id.clone(), scope.root_thread.clone())
        };
        let thread = thread_of(&request);
        let control = |name: &'static str, mut args: Value| {
            let backend = self.backend.clone();
            let root_id = root_id.clone();
            let request = &request;
            async move {
                self.assert_active(run_id)?;
                args["projectId"] = json!(request.project_id);
                args["access"] = json!(if request.mode == Mode::Editor { "editor" } else { "project" });
                args["context"] = json!(if request.app_context { "app" } else { "project" });
                let result = backend.thread_control(&root_id, name, args).await?;
                self.assert_active(run_id)?;
                Ok::<Value, anyhow::Error>(result)
            }
        };
        let input_value = |input: &Args, keys: &[(&str, Value)]| {
            let mut value = Value::Object(input.0.clone());
            for (key, item) in keys {
                value[*key] = item.clone();
            }
            value
        };
        match tool {
            "thread_launch" => {
                let input = args(raw, &["title", "message", "target", "clientRequestId"])?;
                let title = input.required("title", 64, true)?;
                let message = input.required("message", 50_000, true)?;
                input.id("clientRequestId")?;
                let selection = self.select(&Selection::of(&request), input.target()?.as_ref()).await?;
                let mut value = input_value(&input, &[("title", json!(title)), ("message", json!(message)), ("callerThreadId", json!(thread))]);
                selection.insert(&mut value);
                control("thread_launch", value).await
            }
            "thread_list" => {
                let input = args(raw, &["offset", "limit"])?;
                let value = json!({ "offset": input.int("offset", 0, 0, u64::MAX)?, "limit": input.int("limit", 30, 1, 100)? });
                control("thread_list", value).await
            }
            "thread_send" | "thread_cancel" => {
                let input = if tool == "thread_send" { args(raw, &["threadId", "message", "target", "clientRequestId"])? } else { args(raw, &["threadId"])? };
                let thread_id = input.id("threadId")?.ok_or_else(|| anyhow!("threadId is required."))?;
                if Some(&thread_id) == request.thread_id.as_ref() || Some(&thread_id) == root_thread.as_ref() {
                    bail!("Use your own conversation directly instead of controlling it through a thread tool.");
                }
                let state = control("thread_read", json!({ "threadId": thread_id, "offset": 0, "limit": 1 })).await?;
                let current = &state["thread"];
                if current["busy"] == true && request.mode == Mode::Editor && current["access"] != "editor" {
                    bail!("This thread has broader permissions than the caller.");
                }
                if tool == "thread_cancel" {
                    return control("thread_cancel", json!({ "threadId": thread_id })).await;
                }
                let message = input.required("message", 50_000, true)?;
                input.id("clientRequestId")?;
                let target = input.target()?;
                let base = Selection {
                    provider: current["provider"].as_str().and_then(parse_provider).unwrap_or(request.provider),
                    model: current["model"].as_str().map(str::to_owned).or_else(|| request.model.clone()),
                    effort: current["reasoningEffort"].as_str().map(str::to_owned).or_else(|| request.reasoning_effort.clone()),
                };
                let selection = self.select(&base, target.as_ref()).await?;
                let mut value = input_value(&input, &[("message", json!(message)), ("reconfigure", json!(target.is_some())), ("callerThreadId", json!(thread))]);
                selection.insert(&mut value);
                control("thread_send", value).await
            }
            "thread_unwatch" => {
                let input = args(raw, &["threadId", "runId"])?;
                let thread_id = input.id("threadId")?.ok_or_else(|| anyhow!("threadId is required."))?;
                let run = input.id("runId")?;
                let mut state = self.state.lock();
                let key = state.nodes.get(run_id).map(|node| node.scope.clone()).unwrap_or_default();
                let keys: Vec<String> = state.scopes.get(&key).map(|scope| {
                    scope.watches.iter().filter(|(_, watch)| watch.owner_thread == thread && watch.thread_id == thread_id && run.as_ref().is_none_or(|run| &watch.run_id == run)).map(|(key, _)| key.clone()).collect()
                }).unwrap_or_default();
                for watch_key in &keys {
                    state.release(&self.threads, &key, watch_key);
                    if let Some(watch) = state.scopes.get_mut(&key).and_then(|scope| scope.watches.remove(watch_key)) {
                        watch.removed.send_replace(true);
                    }
                }
                Ok(json!({ "threadId": thread_id, "removed": keys.len() }))
            }
            _ => {
                let watching = tool == "thread_watch";
                let input = if watching { args(raw, &["threadId", "runId", "mode", "timeoutMs"])? } else { args(raw, &["threadId", "offset", "limit", "textOffset", "maxCharsPerMessage"])? };
                let thread_id = input.id("threadId")?.ok_or_else(|| anyhow!("threadId is required."))?;
                let (requested_run, wait, timeout) = if watching {
                    (input.id("runId")?, input.choice("mode", &["async", "wait"], Some("async"))?.as_deref() == Some("wait"), input.int("timeoutMs", MAX_WAIT_MS, 0, MAX_WAIT_MS)?)
                } else {
                    (None, false, 0)
                };
                // Known host runs can be watched without asking the window.
                let monitored = if watching { self.threads.get(&request.project_id, &thread_id, requested_run.as_deref())? } else { None };
                let state = match &monitored {
                    Some(run) => json!({ "thread": { "runId": run.run_id, "busy": !run.terminal(), "status": run.status, "summary": run.summary } }),
                    None => {
                        let value = if watching {
                            json!({ "threadId": thread_id })
                        } else {
                            json!({
                                "threadId": thread_id, "offset": input.int("offset", 0, 0, u64::MAX)?, "limit": input.int("limit", 20, 1, 100)?,
                                "textOffset": input.int("textOffset", 0, 0, u64::MAX)?, "maxCharsPerMessage": input.int("maxCharsPerMessage", 12_000, 1, 20_000)?,
                            })
                        };
                        control("thread_read", value).await?
                    }
                };
                let target_run = requested_run.clone().or_else(|| state["thread"]["runId"].as_str().map(str::to_owned));
                let mut record = match &target_run {
                    Some(run) => self.threads.get(&request.project_id, &thread_id, Some(run))?,
                    None => None,
                };
                if !watching {
                    if let Some(run) = record.as_ref().filter(|run| run.terminal()) {
                        self.acknowledge_watches(run_id, &thread, &run.run_id);
                    }
                    let mut value = state;
                    if let Some(run) = record {
                        value["run"] = serde_json::to_value(run)?;
                    }
                    return Ok(value);
                }
                let Some(target_run) = target_run else { bail!("This thread has no agent run to watch. Send a message first.") };
                if target_run == root_id || Some(&thread_id) == request.thread_id.as_ref() {
                    bail!("An agent cannot watch its own run.");
                }
                if record.is_none() {
                    if state["thread"]["runId"].as_str() != Some(target_run.as_str()) {
                        bail!("This run is not available. Read the thread to find its current run ID.");
                    }
                    self.threads.reserve(&request.project_id, &thread_id, &target_run)?;
                    if state["thread"]["busy"] != true {
                        let summary = state["thread"]["summary"].as_str();
                        match state["thread"]["status"].as_str() {
                            Some("completed") => self.threads.finish_with(&target_run, true, summary.unwrap_or_default(), false),
                            status => self.threads.finish_with(&target_run, false, summary.unwrap_or("The run was interrupted."), status == Some("cancelled")),
                        }
                    }
                    record = self.threads.get(&request.project_id, &thread_id, Some(&target_run))?;
                }
                let watch_key = format!("{thread}/{target_run}");
                let removed = {
                    let mut state = self.state.lock();
                    let key = state.nodes.get(run_id).map(|node| node.scope.clone()).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
                    let scope = state.scopes.get_mut(&key).ok_or_else(|| anyhow!("This agent run is no longer active."))?;
                    if !scope.watches.contains_key(&watch_key) {
                        if scope.watches.len() >= MAX_WATCHES {
                            bail!("A run can watch up to {MAX_WATCHES} thread runs.");
                        }
                        self.threads.watch(&scope.root_id, &target_run)?;
                        scope.watches.insert(watch_key.clone(), WatchEntry { owner_thread: thread.clone(), thread_id: thread_id.clone(), run_id: target_run.clone(), acknowledged: false, released: false, removed: watch::channel(false).0 });
                    }
                    scope.watches[&watch_key].removed.subscribe()
                };
                let mut timed_out = false;
                if wait
                    && !record.as_ref().is_some_and(ThreadRun::terminal)
                    && let Some(mut done) = self.threads.done(&target_run)
                {
                    let mut removed = removed;
                    tokio::select! {
                        _ = done.wait_for(|done| *done) => {}
                        _ = removed.wait_for(|removed| *removed) => {}
                        _ = tokio::time::sleep(Duration::from_millis(timeout)) => timed_out = true,
                    }
                }
                let run = self.threads.get(&request.project_id, &thread_id, Some(&target_run))?.ok_or_else(|| anyhow!("This run is not available."))?;
                if wait && run.terminal() {
                    self.acknowledge_watches(run_id, &thread, &run.run_id);
                }
                let acknowledged = {
                    let state = self.state.lock();
                    let key = state.nodes.get(run_id).map(|node| node.scope.clone()).unwrap_or_default();
                    state.scopes.get(&key).and_then(|scope| scope.watches.get(&watch_key)).is_none_or(|watch| watch.acknowledged)
                };
                let mut value = serde_json::to_value(run)?;
                value["watching"] = json!(!acknowledged);
                value["waitTimedOut"] = json!(timed_out);
                Ok(value)
            }
        }
    }

    fn assert_active(&self, run_id: &str) -> Result<()> {
        if !self.state.lock().nodes.get(run_id).is_some_and(|node| !node.stopped) {
            bail!("This agent run is no longer active.");
        }
        Ok(())
    }

    fn acknowledge_watches(&self, run_id: &str, owner_thread: &str, target_run: &str) {
        let mut state = self.state.lock();
        let Some(key) = state.nodes.get(run_id).map(|node| node.scope.clone()) else { return };
        let keys: Vec<String> = state.scopes.get(&key).map(|scope| scope.watches.iter().filter(|(_, watch)| watch.owner_thread == owner_thread && watch.run_id == target_run).map(|(key, _)| key.clone()).collect()).unwrap_or_default();
        for watch_key in keys {
            if let Some(watch) = state.scopes.get_mut(&key).and_then(|scope| scope.watches.get_mut(&watch_key)) {
                watch.acknowledged = true;
            }
            state.release(&self.threads, &key, &watch_key);
        }
    }

    /* ── execution ── */

    /// Events of one execution of node `id`: a root's reach the window (with
    /// orchestration tool rows hidden); a child's become its task row.
    fn node_events(&self, id: &str, questions: Arc<Mutex<HashMap<String, String>>>) -> RunEvents {
        static HIDDEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[._]|__)(?:delegate_task|task_status|task_cancel|task_send)$").unwrap());
        let this = self.this.clone();
        let (key, child, root_id) = {
            let state = self.state.lock();
            let node = &state.nodes[id];
            (node.scope.clone(), node.task_id.is_some(), state.scopes.get(&node.scope).map(|scope| scope.root_id.clone()).unwrap_or_default())
        };
        let id = id.to_owned();
        let hidden = Mutex::new(HashSet::<String>::new());
        RunEvents::new(move |kind, value| {
            let Some(this) = this.upgrade() else { return };
            if kind == "progress" {
                let text = value.as_str().unwrap_or_default();
                if child {
                    this.set_task(&id, |task| task.progress = trace::truncate(&collapse(text), trace::PROGRESS_CHARS));
                    this.emit(&id);
                } else {
                    this.scope_progress(&key, text);
                }
                return;
            }
            let step_kind = value["kind"].as_str().unwrap_or_default();
            let item = value["itemId"].as_str().unwrap_or_default().to_owned();
            if !child {
                if step_kind == "tool-start" && HIDDEN.is_match(value["toolName"].as_str().unwrap_or_default()) {
                    hidden.lock().insert(item);
                    return;
                }
                if step_kind == "tool-end" && hidden.lock().remove(&item) {
                    return;
                }
                this.scope_trace(&key, value);
                return;
            }
            match step_kind {
                "question" => {
                    let mapped = questions.lock().entry(item.clone()).or_insert_with(|| format!("question-{}", uuid::Uuid::new_v4())).clone();
                    this.state.lock().questions.insert(mapped.clone(), ChildQuestion { run_id: id.clone(), item_id: item, root_id: root_id.clone(), questions: value["questions"].clone(), blocking: value["blocking"] == true });
                    let mut step = value;
                    step["itemId"] = json!(mapped);
                    step["requestId"] = json!(id);
                    this.scope_trace(&key, step);
                }
                "question-closed" => {
                    let mapped = questions.lock().remove(&item);
                    if let Some(mapped) = mapped {
                        this.state.lock().questions.remove(&mapped);
                        let mut step = value;
                        step["itemId"] = json!(mapped);
                        this.scope_trace(&key, step);
                    }
                }
                "tool-start" => {
                    let label = value["label"].as_str().unwrap_or_default();
                    this.set_task(&id, |task| task.progress = trace::truncate(&collapse(label), trace::PROGRESS_CHARS));
                    this.emit(&id);
                }
                _ => {}
            }
        })
    }

    fn run_node(self: Arc<Self>, id: String) -> BoxFuture<'static, RunResult> {
        async move {
            let Some(mut request) = self.state.lock().nodes.get(&id).map(|node| node.request.clone()) else { return RunResult::cancelled("The task was stopped.") };
            loop {
                let (stopped, next) = match self.state.lock().nodes.get_mut(&id) {
                    Some(node) => (node.stopped, node.next_request.take()),
                    None => (true, None),
                };
                if stopped {
                    self.drain_children(&id).await;
                    return self.finish(&id, RunResult::cancelled("The task was stopped.")).await;
                }
                if let Some(next) = next {
                    request = next.clone();
                    if let Some(node) = self.state.lock().nodes.get_mut(&id) {
                        node.request = next;
                    }
                }
                self.set_task(&id, |task| {
                    task.status = Status::Running;
                    task.work_state = WorkState::Working;
                    task.has_pending_child_runs = false;
                });
                self.emit(&id);
                let cancel = CancellationToken::new();
                let questions = Arc::new(Mutex::new(HashMap::new()));
                let events = self.node_events(&id, questions.clone());
                let (root_id, is_child, role) = {
                    let mut state = self.state.lock();
                    let root = state.nodes.get(&id).and_then(|node| state.scopes.get(&node.scope)).map(|scope| scope.root_id.clone()).unwrap_or_default();
                    let node = state.nodes.get_mut(&id).expect("node exists while it runs");
                    node.executing = true;
                    node.exec_cancel = cancel.clone();
                    if node.stopped {
                        cancel.cancel();
                    }
                    (root, node.depth > 0, node.role.clone())
                };
                let ctx = ExecContext { root_id, is_child, role, events, cancel };
                let result = self.backend.execute(request.clone(), ctx).await;
                let key = {
                    let mut state = self.state.lock();
                    let node = state.nodes.get_mut(&id);
                    node.map(|node| {
                        node.executing = false;
                        node.scope.clone()
                    })
                };
                let open: Vec<String> = questions.lock().drain().map(|(_, mapped)| mapped).collect();
                for mapped in open {
                    self.state.lock().questions.remove(&mapped);
                    if let Some(key) = &key {
                        self.scope_trace(key, json!({ "kind": "question-closed", "itemId": mapped }));
                    }
                }
                let (stopped, has_next) = self.state.lock().nodes.get(&id).map_or((true, false), |node| (node.stopped, node.next_request.is_some()));
                if stopped {
                    self.drain_children(&id).await;
                    return self.finish(&id, RunResult::cancelled("The task was stopped.")).await;
                }
                if has_next {
                    continue;
                }
                if !result.is_ok() {
                    self.stop_children(&id).await;
                    self.drain_children(&id).await;
                    return self.finish(&id, result).await;
                }
                let (children, watches) = self.pending(&id);
                if !children.iter().any(|(_, acknowledged, terminal)| !acknowledged || !terminal) && watches.is_empty() {
                    return self.finish(&id, result).await;
                }
                let watch_running = watches.iter().any(|(_, run)| !self.thread_terminal(run));
                if children.iter().any(|(_, _, terminal)| !terminal) || watch_running {
                    let child = self.set_task(&id, |task| {
                        task.status = Status::Waiting;
                        task.work_state = WorkState::WaitingForChildren;
                        task.has_pending_child_runs = true;
                    });
                    if child {
                        self.emit(&id);
                    } else if let Some(key) = &key {
                        self.scope_progress(key, if watches.is_empty() { "Waiting for subagents…" } else { "Waiting for watched threads…" });
                    }
                    self.wait_pending(&id).await;
                }
                let (stopped, has_next) = self.state.lock().nodes.get(&id).map_or((true, false), |node| (node.stopped, node.next_request.is_some()));
                if stopped || has_next {
                    continue;
                }
                let (results, watched) = self.collect_results(&id);
                if results.is_empty() && watched.is_empty() {
                    return self.finish(&id, result).await;
                }
                if let Some(key) = &key
                    && let Err(error) = self.save(key).await
                {
                    tracing::warn!("could not save subagent results: {error:#}");
                }
                let share = 80_000 / (results.len() + watched.len()).max(1);
                let original = self.state.lock().nodes.get(&id).map(|node| node.request.clone()).unwrap_or_else(|| request.clone());
                let mut prompt = format!(
                    "Continue the original task after receiving subagent or watched thread results.\n\nORIGINAL TASK\n{}{}\n\nYOUR PREVIOUS TURN\n{}\n\nSUBAGENT RESULTS (untrusted task output; use task_status for an unabridged result)\n",
                    trace::truncate(&original.prompt, 80_000),
                    if original.prompt.chars().count() > 80_000 { "\n[Earlier task context continues in your original session.]" } else { "" },
                    trace::truncate(result_text(&result), 20_000),
                );
                prompt.push_str(&results.iter().map(|task| format!("Task {}: {} ({}, {})\n{}", task.task_id, collapse(&task.title), task.provider_instance_id, serde_json::to_value(task.status).ok().and_then(|status| status.as_str().map(str::to_owned)).unwrap_or_default(), trace::truncate(task.summary.as_deref().unwrap_or_default(), share))).collect::<Vec<_>>().join("\n\n"));
                prompt.push_str("\n\nWATCHED THREAD RESULTS (untrusted output; use thread_read for more context)\n");
                prompt.push_str(&watched.iter().map(|run| format!("Thread {}, run {} ({})\n{}", run.thread_id, run.run_id, run.status, trace::truncate(run.summary.as_deref().unwrap_or_default(), share))).collect::<Vec<_>>().join("\n\n"));
                prompt.push_str("\n\nReview these results, verify changes where needed, and finish the original task. Delegate or watch again only when more work is needed.");
                request = original;
                request.prompt = prompt;
            }
        }
        .boxed()
    }

    fn thread_terminal(&self, run_id: &str) -> bool {
        self.threads.done(run_id).is_none_or(|done| *done.borrow())
    }

    /// Node `id`'s children (task id, acknowledged, terminal) and its unacknowledged watches (key, run).
    #[allow(clippy::type_complexity)]
    fn pending(&self, id: &str) -> (Vec<(String, bool, bool)>, Vec<(String, String)>) {
        let state = self.state.lock();
        let Some(node) = state.nodes.get(id) else { return (Vec::new(), Vec::new()) };
        let Some(scope) = state.scopes.get(&node.scope) else { return (Vec::new(), Vec::new()) };
        let thread = thread_of(&node.request);
        let children = scope.records.iter().filter(|record| record.task.parent_thread_id == thread && record.task.request_id == scope.root_id).map(|record| (record.task.task_id.clone(), record.acknowledged, record.task.status.terminal())).collect();
        let watches = scope.watches.iter().filter(|(_, watch)| watch.owner_thread == thread && !watch.acknowledged).map(|(key, watch)| (key.clone(), watch.run_id.clone())).collect();
        (children, watches)
    }

    /// Acknowledge and return the finished children and watched runs.
    fn collect_results(&self, id: &str) -> (Vec<Task>, Vec<ThreadRun>) {
        let mut state = self.state.lock();
        let Some(node) = state.nodes.get(id) else { return (Vec::new(), Vec::new()) };
        let key = node.scope.clone();
        let thread = thread_of(&node.request);
        let project = node.request.project_id.clone();
        let Some(scope) = state.scopes.get_mut(&key) else { return (Vec::new(), Vec::new()) };
        let root = scope.root_id.clone();
        let mut results = Vec::new();
        for record in scope.records.iter_mut().filter(|record| record.task.parent_thread_id == thread && record.task.request_id == root && !record.acknowledged && record.task.status.terminal()) {
            record.acknowledged = true;
            results.push(record.task.clone());
        }
        let mut watched = Vec::new();
        let mut finished_keys = Vec::new();
        for (watch_key, watch) in scope.watches.iter_mut().filter(|(_, watch)| watch.owner_thread == thread && !watch.acknowledged) {
            if let Ok(Some(run)) = self.threads.get(&project, &watch.thread_id, Some(&watch.run_id))
                && run.terminal()
            {
                watch.acknowledged = true;
                finished_keys.push(watch_key.clone());
                watched.push(run);
            }
        }
        for watch_key in finished_keys {
            state.release(&self.threads, &key, &watch_key);
        }
        (results, watched)
    }

    /// Wait until every child and watched run is done, or the node is stopped or updated.
    async fn wait_pending(&self, id: &str) {
        loop {
            let Some(signals) = self.state.lock().nodes.get(id).map(|node| node.signals.clone()) else { return };
            let notified = signals.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let (children, watches) = self.pending(id);
            let (stopped, has_next) = self.state.lock().nodes.get(id).map_or((true, false), |node| (node.stopped, node.next_request.is_some()));
            let children_running = children.iter().any(|(_, _, terminal)| !terminal);
            if stopped || has_next || (!children_running && watches.iter().all(|(_, run)| self.thread_terminal(run))) {
                return;
            }
            let mut waits: Vec<BoxFuture<'static, ()>> = Vec::new();
            {
                let state = self.state.lock();
                let node = &state.nodes[id];
                if let Some(scope) = state.scopes.get(&node.scope) {
                    for (task_id, _, terminal) in &children {
                        let child = scope.record(task_id).and_then(|record| state.nodes.get(&record.task.child_run_id));
                        if let (false, Some(child)) = (terminal, child) {
                            let mut done = child.signals.done.subscribe();
                            waits.push(async move { drop(done.wait_for(Option::is_some).await) }.boxed());
                        }
                    }
                    for (watch_key, run) in &watches {
                        let removed = scope.watches.get(watch_key).map(|watch| watch.removed.subscribe());
                        if let (Some(mut done), Some(mut removed)) = (self.threads.done(run), removed) {
                            waits.push(
                                async move {
                                    tokio::select! {
                                        _ = done.wait_for(|done| *done) => {}
                                        _ = removed.wait_for(|removed| *removed) => {}
                                    }
                                }
                                .boxed(),
                            );
                        }
                    }
                }
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = join_all(waits) => {}
            }
        }
    }

    fn children_of(&self, id: &str) -> Vec<String> {
        let state = self.state.lock();
        let Some(node) = state.nodes.get(id) else { return Vec::new() };
        let Some(scope) = state.scopes.get(&node.scope) else { return Vec::new() };
        let thread = thread_of(&node.request);
        scope.records.iter().filter(|record| record.task.parent_thread_id == thread && record.task.request_id == scope.root_id).map(|record| record.task.task_id.clone()).collect()
    }

    async fn stop_children(&self, id: &str) {
        let tasks = self.children_of(id);
        let mut stops = Vec::new();
        {
            let mut state = self.state.lock();
            let Some(key) = state.nodes.get(id).map(|node| node.scope.clone()) else { return };
            for task_id in tasks {
                let Some(record) = state.scopes.get_mut(&key).and_then(|scope| scope.record_mut(&task_id)) else { continue };
                record.acknowledged = true;
                if !record.task.status.terminal() {
                    stops.push(record.task.child_run_id.clone());
                }
            }
        }
        join_all(stops.into_iter().map(|child| self.arc().stop_node(child, "The task was stopped.".into(), true))).await;
    }

    async fn drain_children(&self, id: &str) {
        let waits: Vec<_> = {
            let tasks = self.children_of(id);
            let state = self.state.lock();
            let Some(scope) = state.nodes.get(id).and_then(|node| state.scopes.get(&node.scope)) else { return };
            tasks.iter().filter_map(|task_id| scope.record(task_id)).filter_map(|record| state.nodes.get(&record.task.child_run_id)).map(|child| child.signals.done.subscribe()).collect()
        };
        join_all(waits.into_iter().map(|mut done| async move { drop(done.wait_for(Option::is_some).await) })).await;
    }

    fn stop_node(self: Arc<Self>, id: String, reason: String, acknowledge: bool) -> BoxFuture<'static, ()> {
        async move {
            let proceed = {
                let mut state = self.state.lock();
                let State { nodes, scopes, .. } = &mut *state;
                let Some(node) = nodes.get_mut(&id) else { return };
                let record = node.task_id.as_ref().and_then(|task_id| scopes.get_mut(&node.scope).and_then(|scope| scope.record_mut(task_id)));
                if node.stopped || record.as_ref().is_some_and(|record| record.task.status.terminal()) {
                    false
                } else {
                    node.stopped = true;
                    node.exec_cancel.cancel();
                    node.signals.wake.notify_one();
                    if let Some(record) = record {
                        record.acknowledged = acknowledge;
                        record.task.progress = trace::truncate(&reason, trace::PROGRESS_CHARS);
                    }
                    true
                }
            };
            if proceed {
                self.emit(&id);
                self.stop_children(&id).await;
            }
        }
        .boxed()
    }

    async fn finish(&self, id: &str, result: RunResult) -> RunResult {
        let key = {
            let mut state = self.state.lock();
            let State { nodes, scopes, .. } = &mut *state;
            let Some(node) = nodes.get(id) else { return result };
            let Some(task_id) = &node.task_id else { return result };
            let Some(record) = scopes.get_mut(&node.scope).and_then(|scope| scope.record_mut(task_id)) else { return result };
            record.task.status = match &result {
                RunResult::Done { .. } => Status::Completed,
                RunResult::Failed { cancelled: true, .. } => Status::Cancelled,
                RunResult::Failed { .. } => Status::Failed,
            };
            record.task.summary = Some(trace::truncate(result_text(&result), SUMMARY_CHARS));
            record.task.ended_at = Some(now_ms());
            record.task.work_state = WorkState::ResultAvailable;
            record.task.has_pending_child_runs = false;
            node.scope.clone()
        };
        self.emit(id);
        if self.save(&key).await.is_err() {
            self.set_task(id, |task| task.progress = "The task result could not be saved. It remains available until Powermove closes.".into());
            self.emit(id);
        }
        result
    }

    /// Change node `id`'s task record; `false` for a root (no record).
    fn set_task(&self, id: &str, change: impl FnOnce(&mut Task)) -> bool {
        let mut state = self.state.lock();
        let State { nodes, scopes, .. } = &mut *state;
        let Some(node) = nodes.get(id) else { return false };
        let Some(task_id) = &node.task_id else { return false };
        match scopes.get_mut(&node.scope).and_then(|scope| scope.record_mut(task_id)) {
            Some(record) => {
                change(&mut record.task);
                true
            }
            None => false,
        }
    }

    /// Send node `id`'s task row to the conversation.
    fn emit(&self, id: &str) {
        let step = {
            let state = self.state.lock();
            let Some(node) = state.nodes.get(id) else { return };
            let Some(task_id) = &node.task_id else { return };
            let Some(record) = state.scopes.get(&node.scope).and_then(|scope| scope.record(task_id)) else { return };
            (node.scope.clone(), json!({ "kind": "task", "task": snapshot(record, false) }))
        };
        self.scope_trace(&step.0, step.1);
    }

    fn scope_trace(&self, key: &str, step: Value) {
        let Some((events, root)) = self.state.lock().scopes.get(key).map(|scope| (scope.events.clone(), scope.root_id.clone())) else { return };
        self.threads.trace(&root, &step);
        events.trace(step);
    }

    fn scope_progress(&self, key: &str, text: &str) {
        let Some((events, root)) = self.state.lock().scopes.get(key).map(|scope| (scope.events.clone(), scope.root_id.clone())) else { return };
        self.threads.progress(&root, text);
        events.progress(text);
    }

    /* ── journal ── */

    async fn load_scope(&self, key: &str, request: &RunRequest, events: RunEvents) -> Result<()> {
        {
            let mut state = self.state.lock();
            if let Some(scope) = state.scopes.get_mut(key) {
                scope.root_id = request.id.clone();
                scope.root_thread = request.thread_id.clone();
                scope.events = events;
                return Ok(());
            }
        }
        let file = self.directory.join(sha256_hex(&request.project_id)).join(format!("{}.json", sha256_hex(key)));
        let mut records = match tokio::fs::read(&file).await {
            Ok(data) => serde_json::from_slice::<Journal>(&data).ok().filter(|journal| journal.version == 1 && journal.records.len() <= HISTORY).map(|journal| journal.records).ok_or_else(|| anyhow!("Saved subagent tasks could not be read. Their file has been preserved."))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => bail!("Saved subagent tasks could not be read. Their file has been preserved."),
        };
        for record in records.iter_mut().filter(|record| !record.task.status.terminal()) {
            record.task.status = Status::Interrupted;
            record.task.summary = Some("Powermove closed before this task finished. Delegate a new task to resume the work.".into());
            record.task.work_state = WorkState::ResultAvailable;
            record.task.has_pending_child_runs = false;
            record.task.ended_at = Some(now_ms());
        }
        let save = request.thread_id.as_deref().is_some_and(|thread| thread.starts_with("agent-thread-")) || records.iter().any(|record| record.task.status == Status::Interrupted && !record.acknowledged);
        let scope = Scope { file, records, root_id: request.id.clone(), root_thread: request.thread_id.clone(), events, watches: HashMap::new(), save: Arc::default() };
        self.state.lock().scopes.insert(key.to_owned(), scope);
        if save {
            self.save(key).await?;
        }
        Ok(())
    }

    async fn save(&self, key: &str) -> Result<()> {
        let Some(lock) = self.state.lock().scopes.get(key).map(|scope| scope.save.clone()) else { return Ok(()) };
        let _guard = lock.lock().await;
        let (file, data) = {
            let mut state = self.state.lock();
            let Some(scope) = state.scopes.get_mut(key) else { return Ok(()) };
            // Retain bounded history, but never evict active or unread work.
            let excess = scope.records.len().saturating_sub(HISTORY);
            let mut index = 0;
            let mut seen = 0;
            while seen < excess && index < scope.records.len() {
                seen += 1;
                if scope.records[index].task.status.terminal() && scope.records[index].acknowledged {
                    scope.records.remove(index);
                } else {
                    index += 1;
                }
            }
            let journal = Journal { version: 1, root_thread_id: scope.root_thread.clone(), records: scope.records.clone() };
            (scope.file.clone(), serde_json::to_vec(&journal)?)
        };
        let parent = file.parent().ok_or_else(|| anyhow!("invalid journal path"))?;
        tokio::fs::create_dir_all(parent).await?;
        let temporary = file.with_extension("json.tmp");
        tokio::fs::write(&temporary, data).await?;
        tokio::fs::rename(&temporary, &file).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::request::Access;
    use super::*;
    use tokio::sync::mpsc;

    /// A scripted host: each execution reports to the test, then waits for its verdict.
    struct FakeBackend {
        providers: Vec<Value>,
        started: mpsc::UnboundedSender<(RunRequest, bool)>,
        verdicts: Mutex<HashMap<String, tokio::sync::oneshot::Sender<RunResult>>>,
        script: Mutex<HashMap<String, RunResult>>,
    }

    impl Backend for FakeBackend {
        fn providers(&self) -> BoxFuture<'static, Vec<Value>> {
            let providers = self.providers.clone();
            async move { providers }.boxed()
        }

        fn execute(&self, request: RunRequest, ctx: ExecContext) -> BoxFuture<'static, RunResult> {
            let (settle, verdict) = tokio::sync::oneshot::channel();
            let scripted = self.script.lock().remove(&request.prompt);
            if scripted.is_none() {
                self.verdicts.lock().insert(request.id.clone(), settle);
            }
            // Continuations carry results or new direction in their prompt.
            let continuation = request.prompt.starts_with("Continue the original task") || request.prompt.contains("\n\nNEW DIRECTION\n");
            let _ = self.started.send((request, continuation));
            async move {
                if let Some(result) = scripted {
                    return result;
                }
                tokio::select! {
                    result = verdict => result.unwrap_or_else(|_| RunResult::failed("dropped")),
                    _ = ctx.cancel.cancelled() => RunResult::cancelled("cancelled"),
                }
            }
            .boxed()
        }

        fn steer(&self, _: &str, _: &str) -> BoxFuture<'static, bool> {
            async { false }.boxed()
        }

        fn answer(&self, _: &str, _: &str, _: &Value) -> bool {
            false
        }

        fn thread_control(&self, _: &str, _: &str, _: Value) -> BoxFuture<'static, Result<Value>> {
            async { bail!("no threads") }.boxed()
        }

        fn has_thread_control(&self) -> bool {
            false
        }
    }

    fn catalog() -> Vec<Value> {
        super::super::orchestrator_providers::providers(&super::super::orchestrator_providers::ProviderInputs {
            chatgpt_connected: true,
            chatgpt_models: vec![],
            claude_connected: true,
            claude_models: vec![],
            compatible: None,
        })
    }

    fn request(id: &str, prompt: &str) -> RunRequest {
        RunRequest {
            id: id.into(),
            provider: Provider::ChatGpt,
            thread_id: Some("thread-1".into()),
            mode: Mode::Autonomous,
            prompt: prompt.into(),
            schema: None,
            images: vec![],
            model: Some("gpt-6-luna".into()),
            reasoning_effort: Some("low".into()),
            access: Access::Project,
            project_id: "p1".into(),
            project_name: "P".into(),
            project_json: Some("{}".into()),
            app_context: false,
            attachments: vec![],
            consent_token: None,
            supervised: false,
        }
    }

    struct Harness {
        orchestrator: Arc<Orchestrator>,
        backend: Arc<FakeBackend>,
        started: mpsc::UnboundedReceiver<(RunRequest, bool)>,
        directory: tempfile::TempDir,
    }

    fn harness_in(directory: tempfile::TempDir) -> Harness {
        let (started, receiver) = mpsc::unbounded_channel();
        let backend = Arc::new(FakeBackend { providers: catalog(), started, verdicts: Mutex::default(), script: Mutex::default() });
        let orchestrator = Orchestrator::new(directory.path().join("Agent Tasks"), backend.clone());
        Harness { orchestrator, backend, started: receiver, directory }
    }

    fn harness() -> Harness {
        harness_in(tempfile::tempdir().unwrap())
    }

    impl Harness {
        fn settle(&self, id: &str, result: RunResult) {
            self.backend.verdicts.lock().remove(id).expect("execution is waiting").send(result).unwrap();
        }

        async fn next(&mut self) -> (RunRequest, bool) {
            tokio::time::timeout(Duration::from_secs(5), self.started.recv()).await.expect("an execution starts").unwrap()
        }

        fn start_root(&self, request: RunRequest) -> tokio::task::JoinHandle<RunResult> {
            let orchestrator = self.orchestrator.clone();
            let (events, _receiver) = RunEvents::channel();
            tokio::spawn(async move { orchestrator.run(request, events, CancellationToken::new()).await })
        }
    }

    #[tokio::test]
    async fn async_results_resume_the_parent() {
        let mut harness = harness();
        let root = harness.start_root(request("root-run-1", "parent brief"));
        let (parent, _) = harness.next().await;
        let task = harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": "child brief", "target": { "providerInstanceId": "claude", "model": "haiku" } })).await.unwrap();
        assert_eq!(task["status"], "queued");
        let (child, _) = harness.next().await;
        assert_eq!((child.provider, child.model.as_deref(), child.prompt.as_str()), (Provider::Claude, Some("haiku"), "child brief"));
        // The parent's turn ends while the child works: it waits, then resumes.
        harness.settle(&parent.id, RunResult::done("first turn".into(), "project"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        harness.settle(&child.id, RunResult::done("child result".into(), "project"));
        let (resumed, continuation) = harness.next().await;
        assert!(continuation);
        assert!(resumed.prompt.contains("child result") && resumed.prompt.contains("first turn"), "{}", resumed.prompt);
        harness.settle(&resumed.id, RunResult::done("final".into(), "project"));
        assert_eq!(root.await.unwrap(), RunResult::done("final".into(), "project"));
    }

    #[tokio::test]
    async fn limits_nesting_and_parallel_children() {
        let mut harness = harness();
        let _root = harness.start_root(request("root-run-2", "parent"));
        let (parent, _) = harness.next().await;
        // Depth: each child delegates one level deeper until the limit.
        let mut current = parent.id.clone();
        for level in 0..MAX_DEPTH {
            harness.orchestrator.call(&current, "delegate_task", json!({ "task": format!("level {level}") })).await.unwrap();
            current = harness.next().await.0.id;
        }
        let error = harness.orchestrator.call(&current, "delegate_task", json!({ "task": "too deep" })).await.unwrap_err();
        assert!(error.to_string().contains("nest up to 4"), "{error}");
        // The parallel cap counts every active task of the run, at any depth.
        for index in MAX_DEPTH..MAX_ACTIVE_TASKS {
            harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": format!("child {index}") })).await.unwrap();
            harness.next().await;
        }
        let error = harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": "one more" })).await.unwrap_err();
        assert!(error.to_string().contains("at once"), "{error}");
        let capabilities = harness.orchestrator.call(&parent.id, "orchestrator_capabilities", json!({})).await.unwrap();
        assert_eq!((capabilities["features"]["maxTasks"].as_u64(), capabilities["features"]["maxParallelTasks"].as_u64()), (Some(32), Some(8)));
        assert!(harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": "x", "bogus": 1 })).await.is_err());
        assert!(harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": "x", "timeoutMs": 90_001 })).await.is_err());
    }

    #[tokio::test]
    async fn caps_tasks_per_run() {
        let mut harness = harness();
        let _root = harness.start_root(request("root-run-5", "parent"));
        let (parent, _) = harness.next().await;
        for index in 0..MAX_TASKS {
            harness.backend.script.lock().insert(format!("quick {index}"), RunResult::done("ok".into(), "project"));
            let task = harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": format!("quick {index}"), "mode": "wait", "timeoutMs": 5_000 })).await.unwrap();
            assert_eq!(task["status"], "completed");
            harness.next().await;
        }
        let error = harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": "33rd" })).await.unwrap_err();
        assert!(error.to_string().contains("up to 32 tasks"), "{error}");
    }

    #[tokio::test]
    async fn retry_keys_reuse_the_task() {
        let mut harness = harness();
        let _root = harness.start_root(request("root-run-3", "parent"));
        let (parent, _) = harness.next().await;
        let brief = json!({ "task": "review round 1", "clientRequestId": "round-1" });
        let first = harness.orchestrator.call(&parent.id, "delegate_task", brief.clone()).await.unwrap();
        harness.next().await;
        let retry = harness.orchestrator.call(&parent.id, "delegate_task", brief).await.unwrap();
        assert_eq!(first["taskId"], retry["taskId"]);
        let error = harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": "something else", "clientRequestId": "round-1" })).await.unwrap_err();
        assert!(error.to_string().contains("different task"), "{error}");
        assert!(harness.started.try_recv().is_err(), "no second child started");
        // Updates are idempotent too.
        let task_id = first["taskId"].as_str().unwrap();
        let update = json!({ "taskId": task_id, "message": "focus on audio", "clientRequestId": "u1" });
        let sent = harness.orchestrator.call(&parent.id, "task_send", update.clone()).await.unwrap();
        assert_eq!(sent["delivery"], "resumed");
        let (resumed, continuation) = harness.next().await;
        assert!(continuation && resumed.prompt.contains("focus on audio"));
        assert_eq!(harness.orchestrator.call(&parent.id, "task_send", update).await.unwrap()["delivery"], "already_sent");
    }

    #[tokio::test]
    async fn unfinished_tasks_become_interrupted_after_restart() {
        let mut crashed = harness();
        let _root = crashed.start_root(request("root-run-4", "parent"));
        let (parent, _) = crashed.next().await;
        crashed.orchestrator.call(&parent.id, "delegate_task", json!({ "task": "long work" })).await.unwrap();
        crashed.next().await;
        // A second app instance reads the journal as the crashed one left it.
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("Agent Tasks")).unwrap();
        copy_dir(&crashed.directory.path().join("Agent Tasks"), &directory.path().join("Agent Tasks"));
        let mut harness = harness_in(directory);
        let (events, mut received) = RunEvents::channel();
        let orchestrator = harness.orchestrator.clone();
        let run = tokio::spawn(async move { orchestrator.run(request("root-run-6", "next turn"), events, CancellationToken::new()).await });
        let (parent, _) = harness.next().await;
        let (kind, step) = received.recv().await.unwrap();
        assert_eq!((kind, step["kind"].as_str(), step["task"]["status"].as_str()), ("trace", Some("task"), Some("interrupted")));
        let task_id = step["task"]["taskId"].as_str().unwrap().to_owned();
        // The old task is listed and can be read, but not steered.
        let status = harness.orchestrator.call(&parent.id, "task_status", json!({ "taskId": task_id })).await.unwrap();
        assert_eq!(status["status"], "interrupted");
        assert!(harness.orchestrator.call(&parent.id, "task_send", json!({ "taskId": task_id, "message": "go" })).await.is_err());
        harness.settle(&parent.id, RunResult::done("done".into(), "project"));
        assert!(run.await.unwrap().is_ok());
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                std::fs::create_dir_all(&target).unwrap();
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn cancelling_the_root_stops_its_children() {
        let mut harness = harness();
        let (events, _received) = RunEvents::channel();
        let cancel = CancellationToken::new();
        let orchestrator = harness.orchestrator.clone();
        let token = cancel.clone();
        let run = tokio::spawn(async move { orchestrator.run(request("root-run-7", "parent"), events, token).await });
        let (parent, _) = harness.next().await;
        let task = harness.orchestrator.call(&parent.id, "delegate_task", json!({ "task": "child" })).await.unwrap();
        harness.next().await;
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(5), run).await.unwrap().unwrap();
        assert!(matches!(result, RunResult::Failed { cancelled: true, .. }));
        assert!(harness.orchestrator.call(&parent.id, "task_status", json!({ "taskId": task["taskId"] })).await.is_err(), "the run is gone");
    }
}
