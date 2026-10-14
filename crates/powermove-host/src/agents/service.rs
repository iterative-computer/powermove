//! The agent channels (port of `registerCodexIpc` in `codex/index.ts`).
//!
//! A run belongs to the window that started it: only that window may steer,
//! answer or cancel it, its tool calls go only to that window, and closing
//! the window cancels the run.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use bytes::Bytes;
use futures::FutureExt;
use parking_lot::Mutex;
use rfd::{MessageButtons, MessageDialogResult, MessageLevel};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::RunEvents;
use super::accounts::{ChatGpt, Claude, Publish};
use super::approvals::{ApprovalRequest, Approvals, Decision};
use super::autonomous::{self, Approver, Env, NativeTools, RunContext};
use super::claude_run::{ClaudeProvider, ClaudeRuns};
use super::codex_app::CodexApp;
use super::codex_exec::CodexExec;
use super::process;
use super::prompts::Prompts;
use super::request::{self, Access, Mode, Provider, RunRequest, RunResult, amend_live_result, is_request_id};
use super::runtime::Runtimes;
use super::tools::{OutsideSandbox, SessionOptions, ToolBridge};
use super::workspace;
use crate::host::{Config, Host};
use crate::ids::{is_extension_id, is_project_id};
use crate::router::{Call, Router};

#[path = "orchestrator_host.rs"]
mod orchestrator_host;

const RUN_TIMEOUT: Duration = Duration::from_secs(3_600);
const CONSENT_TTL: Duration = Duration::from_secs(5 * 60);
const FIX_PROMPT_FILES: usize = 40;
const FIX_PROMPT_FILE_BYTES: usize = 64 * 1024;
const OUTSIDE_OUTPUT_BYTES: usize = 64 * 1024;
const UNAVAILABLE: &str = "is not available in the Windows app yet.";

struct ActiveRun {
    owner: u64,
    cancel: CancellationToken,
    /// Keep edits already committed through the live tool transaction.
    preserve: AtomicBool,
}

/// Computer authority: a native confirmation mints a one-use, short-lived token.
#[derive(Default)]
struct Consent {
    tokens: Mutex<HashMap<String, SystemTime>>,
    standing: AtomicBool,
}

impl Consent {
    fn mint(&self) -> Value {
        let token = hex::encode(rand::random::<[u8; 32]>());
        let expires = SystemTime::now() + CONSENT_TTL;
        self.tokens.lock().insert(token.clone(), expires);
        let expires_at = expires.duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as u64);
        json!({ "granted": true, "token": token, "expiresAt": expires_at })
    }

    /// Consume exactly once.
    fn consume(&self, token: &str) -> bool {
        self.tokens.lock().remove(token).is_some_and(|expires| SystemTime::now() < expires)
    }
}

pub struct Agents {
    /// Shared with the account broadcasters.
    host: Arc<Mutex<Weak<Host>>>,
    user_data: PathBuf,
    env: Option<Arc<Env>>,
    env_error: String,
    bridge: Option<Arc<ToolBridge>>,
    approvals: Arc<Approvals>,
    chatgpt: Arc<ChatGpt>,
    claude: Arc<Claude>,
    codex_app: Arc<CodexApp>,
    claude_runs: ClaudeRuns,
    runs: Mutex<HashMap<String, Arc<ActiveRun>>>,
    /// A project's workspace is removed with the project, but never under a running agent.
    running_projects: Mutex<HashMap<String, usize>>,
    forget_when_idle: Mutex<HashSet<String>>,
    consent: Consent,
    /// App-owned subagents for every provider.
    orchestrator: Arc<super::orchestrator::Orchestrator>,
    compatible: super::compatible::Compatible,
}

/// Account state is app-wide, so it reaches every open window.
fn broadcaster(host: Arc<Mutex<Weak<Host>>>, channel: &'static str) -> Publish {
    Arc::new(move |status| {
        if let Some(host) = host.lock().upgrade() {
            host.broadcast(channel, status, None);
        }
    })
}

impl Agents {
    /// `claude_home` replaces Powermove's private Claude config directory (live tests).
    pub(crate) async fn new(config: &Config, claude_home: Option<PathBuf>) -> Arc<Self> {
        let runtimes = Runtimes { user_data: config.user_data.clone(), resources_dir: config.resources_dir.clone(), codex_binary: None, claude_binary: None, claude_home };
        let host_slot = Arc::new(Mutex::new(Weak::new()));
        let chatgpt = ChatGpt::new(runtimes.clone(), broadcaster(host_slot.clone(), "chatgpt:changed"), Arc::new(|url: &str| crate::app::open_external(url)));
        let claude = Claude::new(runtimes.clone(), broadcaster(host_slot.clone(), "claude:changed"));
        let (env, env_error, bridge) = match Prompts::load(&config.resources_dir) {
            Ok(prompts) => {
                let prompts = Arc::new(prompts);
                let bridge = match ToolBridge::start(prompts.clone(), None).await {
                    Ok(bridge) => Some(bridge),
                    Err(error) => {
                        tracing::warn!("agent tools unavailable: {error:#}");
                        None
                    }
                };
                let env = Env { user_data: config.user_data.clone(), resources_dir: config.resources_dir.clone(), live_extensions: config.user_data.join("extensions"), runtimes: runtimes.clone(), prompts };
                (Some(Arc::new(env)), String::new(), bridge)
            }
            Err(error) => {
                tracing::warn!("agent prompts unavailable: {error:#}");
                (None, format!("Powermove's agent resources are missing ({error:#})."), None)
            }
        };
        let agents = Arc::new_cyclic(|this| Self {
            orchestrator: orchestrator_host::orchestrator(&config.user_data, this.clone()),
            compatible: super::compatible::Compatible::new(&config.user_data),
            host: host_slot,
            user_data: config.user_data.clone(),
            env,
            env_error,
            bridge,
            approvals: Arc::default(),
            chatgpt,
            claude,
            codex_app: CodexApp::new(runtimes),
            claude_runs: ClaudeRuns::default(),
            runs: Mutex::default(),
            running_projects: Mutex::default(),
            forget_when_idle: Mutex::default(),
            consent: Consent::default(),
        });
        orchestrator_host::connect(&agents);
        agents
    }

    /// Record the host so account changes can reach every window.
    fn remember(&self, call: &Call) {
        let mut host = self.host.lock();
        if host.upgrade().is_none() {
            *host = Arc::downgrade(&call.host);
        }
    }

    fn owned(&self, id: &str, client: u64) -> Option<Arc<ActiveRun>> {
        self.runs.lock().get(id).filter(|run| run.owner == client).cloned()
    }

    /// `codex:run`.
    async fn run(self: Arc<Self>, call: Call) -> Result<Value> {
        let request = RunRequest::parse(&call)?;
        self.remember(&call);
        let owner = call.client.clone();
        let run = Arc::new(ActiveRun { owner: owner.id, cancel: CancellationToken::new(), preserve: AtomicBool::new(false) });
        {
            let mut runs = self.runs.lock();
            if runs.contains_key(&request.id) {
                bail!("request id is already active");
            }
            runs.insert(request.id.clone(), run.clone());
        }
        *self.running_projects.lock().entry(request.project_id.clone()).or_default() += 1;
        let cancel = run.cancel.clone();
        owner.on_close(move || cancel.cancel());
        let id = request.id.clone();
        let project = request.project_id.clone();
        let events = RunEvents::window(owner.clone(), id.clone());
        let result = self.run_root(request, &call, &run, events).await;

        self.approvals.close_run(&id);
        self.runs.lock().remove(&id);
        let idle = {
            let mut projects = self.running_projects.lock();
            let remaining = projects.get(&project).copied().unwrap_or(1).saturating_sub(1);
            if remaining == 0 {
                projects.remove(&project);
            } else {
                projects.insert(project.clone(), remaining);
            }
            remaining == 0 && self.forget_when_idle.lock().remove(&project)
        };
        if idle {
            self.forget_project(&project).await;
        }
        Ok(result.to_json())
    }

    /// The root run: one tool session spans its turns and subagents, and the
    /// orchestrator runs the provider (see `execute`).
    async fn run_root(&self, request: RunRequest, call: &Call, run: &ActiveRun, events: RunEvents) -> RunResult {
        if self.env.is_none() {
            return RunResult::failed(self.env_error.clone());
        }
        if request.access == Access::Computer && !request.consent_token.as_deref().is_some_and(|token| self.consent.consume(token)) {
            return RunResult::failed("Computer access requires fresh approval for this run.");
        }
        let approve = self.approver(&request.id, &events, &run.cancel);
        // `codex exec` and API models cannot ask the person themselves, so a
        // supervised Project run asks through Powermove's run_outside_sandbox tool.
        let asks = request.provider == Provider::Compatible || request.provider == Provider::ChatGpt && request.supervised;
        let outside_sandbox = (request.mode == Mode::Autonomous && request.access == Access::Project && asks)
            .then(|| outside_sandbox_runner(workspace::workspace_root(&self.user_data, &request.workspace_id()), approve.clone(), run.cancel.clone()));
        let session = match &self.bridge {
            Some(bridge) => match bridge.open_session(SessionOptions {
                run_id: request.id.clone(),
                owner: call.client.clone(),
                base_revision: request.base_revision(),
                app_context: request.app_context,
                inspection_only: request.mode == Mode::Editor,
                outside_sandbox,
            }) {
                Ok(session) => Some((bridge.clone(), session)),
                Err(error) => return RunResult::failed(format!("{error:#}")),
            },
            None => None,
        };
        let mode = request.mode;
        let mut result = self.orchestrator.run(request, events, run.cancel.clone()).await;
        if let Some((_, session)) = session {
            let changed_before = session.changed.load(Ordering::Acquire);
            let finish = session.finish(result.is_ok() || run.preserve.load(Ordering::Acquire)).await;
            if let RunResult::Done { text, live_edits, live_history_id, .. } = &mut result {
                if mode == Mode::Autonomous {
                    *text = amend_live_result(text, changed_before || finish.changed, finish.warning.as_deref());
                    if finish.changed {
                        *live_edits = true;
                        *live_history_id = finish.history_id.clone();
                    }
                }
            }
            if let Some(warning) = &finish.warning {
                tracing::warn!("[agent-tools] {warning}");
            }
        }
        result
    }

    /// One provider execution: a turn of a root run, or a subagent.
    async fn execute(&self, mut request: RunRequest, exec: super::orchestrator::ExecContext) -> RunResult {
        let Some(env) = self.env.clone() else { return RunResult::failed(self.env_error.clone()) };
        let approve = self.approver(&request.id, &exec.events, &exec.cancel);
        let root = self.bridge.as_ref().and_then(|bridge| bridge.session_for_run(&exec.root_id).map(|session| (bridge.clone(), session)));
        // A subagent edits inside its root's live transaction.
        let child = match (&root, exec.is_child) {
            (Some((bridge, root)), true) => match bridge.open_child_session(&request.id, root, request.mode == Mode::Editor) {
                Ok(session) => Some(session),
                Err(error) => return RunResult::failed(format!("{error:#}")),
            },
            _ => None,
        };
        let tools = root.as_ref().map(|(bridge, root)| {
            let session = child.as_ref().unwrap_or(root);
            let endpoint = session.endpoint();
            let names: Vec<String> = bridge.visible_tools(session).into_iter().map(|tool| tool.name).collect();
            let has_outside = names.iter().any(|name| name == super::tools::OUTSIDE_SANDBOX_TOOL);
            NativeTools {
                url: endpoint.url,
                token: endpoint.token,
                names,
                timeout_sec: (if has_outside { env.prompts.outside_sandbox_call_ms } else { 0 }).max(3_600_000).div_ceil(1000),
                outside_sandbox: has_outside,
            }
        });
        let extra = orchestrator_host::instructions(&env, tools.as_ref(), exec.role.as_deref());
        // Editor runs have no instruction slot of their own.
        if let (Some(extra), Mode::Editor, false) = (&extra, request.mode, request.provider == Provider::Compatible) {
            request.prompt = format!("{}\n\n{extra}", request.prompt);
        }
        let (mode, provider, is_child, run_id) = (request.mode, request.provider, exec.is_child, request.id.clone());
        let workspace_id = request.workspace_id();
        let project_id = request.project_id.clone();
        let ctx = RunContext { request, events: exec.events, cancel: exec.cancel, tools, approve: Some(approve), timeout: RUN_TIMEOUT, extra_instructions: extra };
        let discovered = || ctx.request.model.as_ref().and_then(|model| self.claude.efforts.lock().get(model).cloned());
        let result = match (provider, mode) {
            (Provider::ChatGpt, Mode::Editor) => self.codex_app.run(&ctx).await,
            (Provider::ChatGpt, Mode::Autonomous) => autonomous::run(&CodexExec, &env, &ctx).await,
            (Provider::Claude, Mode::Editor) => self.claude_runs.run_editor(&env, &ctx, discovered().as_ref()).await,
            (Provider::Claude, Mode::Autonomous) => autonomous::run(&ClaudeProvider { runs: &self.claude_runs, discovered: discovered() }, &env, &ctx).await,
            (Provider::Compatible, _) => self.compatible.run(&env, &ctx).await,
        };
        if is_child {
            if let Some(session) = child {
                session.finish(false).await;
            }
            self.approvals.close_run(&run_id);
            if let (RunResult::Done { text, .. }, Mode::Autonomous) = (&result, mode) {
                if let Err(error) = orchestrator_host::copy_child_artifacts(&self.user_data, &workspace_id, &project_id, text).await {
                    tracing::warn!("subagent artifacts were not copied: {error:#}");
                }
            }
        }
        result
    }

    fn approver(&self, run_id: &str, events: &RunEvents, cancel: &CancellationToken) -> Approver {
        let approvals = self.approvals.clone();
        let run_id = run_id.to_owned();
        let events = events.clone();
        let cancel = cancel.clone();
        Arc::new(move |request: ApprovalRequest| {
            let approvals = approvals.clone();
            let run_id = run_id.clone();
            let events = events.clone();
            let cancel = cancel.clone();
            async move { approvals.request(&run_id, &events, request, &cancel).await }.boxed()
        })
    }

    /// `codex:cancel`.
    async fn cancel(self: Arc<Self>, call: Call) -> Result<Value> {
        let value = call.arg(0);
        let Some(id) = value["id"].as_str().filter(|id| is_request_id(id)) else { bail!("invalid request id") };
        let preserve = match &value["preserveChanges"] {
            Value::Null => false,
            Value::Bool(flag) => *flag,
            _ => bail!("invalid request id"),
        };
        if let Some(run) = self.owned(id, call.client.id) {
            // A provider without live steering is replaced by a continuation
            // run: seal its completed edits before interrupting it.
            if preserve {
                run.preserve.store(true, Ordering::Release);
                if let Some(session) = self.bridge.as_ref().and_then(|bridge| bridge.session_for_run(id)) {
                    session.finish(true).await;
                }
            }
            run.cancel.cancel();
        }
        Ok(Value::Null)
    }

    /// `codex:steer`: live steering exists only on the App Server transport.
    async fn steer(self: Arc<Self>, call: Call) -> Result<Value> {
        let value = call.arg(0);
        let id = value["id"].as_str().filter(|id| is_request_id(id));
        let prompt = value["prompt"].as_str().filter(|prompt| !prompt.is_empty() && prompt.chars().count() <= request::PROMPT_CHARS);
        let images = value["images"].as_array().filter(|images| images.len() <= request::IMAGES);
        let (Some(id), Some(prompt), Some(images)) = (id, prompt, images) else { bail!("invalid steering request") };
        let images = images.iter().map(|image| call.bytes(image)).collect::<Result<Vec<Bytes>>>()?;
        if images.iter().any(|image| image.len() > request::IMAGE_BYTES) {
            bail!("invalid steering request");
        }
        if self.owned(id, call.client.id).is_none() {
            return Ok(json!({ "accepted": false }));
        }
        Ok(json!({ "accepted": self.codex_app.steer(id, prompt.to_owned(), images).await }))
    }

    /// `codex:answer`.
    async fn answer(self: Arc<Self>, call: Call) -> Result<Value> {
        let value = call.arg(0);
        let id = value["id"].as_str().filter(|id| is_request_id(id));
        let item = value["itemId"].as_str().filter(|item| !item.is_empty() && item.chars().count() <= 240);
        let answers = value["answers"].as_object().filter(|answers| {
            answers.len() <= 12
                && answers.iter().all(|(key, list)| key.chars().count() <= 200 && list.as_array().is_some_and(|list| list.len() <= 12 && list.iter().all(|answer| answer.as_str().is_some_and(|text| text.chars().count() <= 8_000))))
        });
        let (Some(id), Some(item), Some(_)) = (id, item, answers) else { bail!("invalid answer") };
        // A subagent's question carries its own run id; its root owns the window.
        if self.owned(&self.orchestrator.root_of(id).unwrap_or_else(|| id.to_owned()), call.client.id).is_none() {
            return Ok(json!({ "accepted": false }));
        }
        let item = &self.orchestrator.route_answer(id, item)?;
        let answers = &value["answers"];
        let accepted = self.approvals.answer(id, item, answers) || self.codex_app.answer(id, item, answers) || self.claude_runs.answer(id, item, answers);
        Ok(json!({ "accepted": accepted }))
    }

    /// `consent:computer`.
    async fn consent(self: Arc<Self>, call: Call) -> Result<Value> {
        let value = call.arg(0);
        let (Some(project), Some(summary)) = (value["projectName"].as_str(), value["summary"].as_str()) else { bail!("invalid request") };
        if value["revoke"] == true {
            self.consent.standing.store(false, Ordering::Release);
            return Ok(json!({ "granted": false }));
        }
        let standing = value["standing"] == true;
        if !(standing && self.consent.standing.load(Ordering::Acquire)) {
            let (title, description, allow) = if standing {
                ("Give Powermove agents full access to your PC?".to_owned(), "Agents will run commands and change files anywhere on this PC without asking, until you choose another access level or quit Powermove.".to_owned(), "Allow Full Access")
            } else {
                (format!("Allow computer access for “{}”?", project.chars().take(200).collect::<String>()), summary.chars().take(2_000).collect(), "Allow this run")
            };
            let result = crate::dialog::message()
                .set_level(MessageLevel::Warning)
                .set_title(&title)
                .set_description(description)
                .set_buttons(MessageButtons::OkCancelCustom(allow.into(), "Cancel".into()))
                .show()
                .await;
            let granted = matches!(&result, MessageDialogResult::Custom(label) if label == allow) || matches!(result, MessageDialogResult::Ok);
            if !granted {
                return Ok(json!({ "granted": false }));
            }
            if standing {
                self.consent.standing.store(true, Ordering::Release);
            }
        }
        Ok(self.consent.mint())
    }

    /// `agent:artifacts:forget`: waits for the project's last run.
    async fn forget(self: Arc<Self>, call: Call) -> Result<Value> {
        let Some(project) = call.arg(0).as_str().filter(|id| is_project_id(id)).map(str::to_owned) else { bail!("invalid project id") };
        if self.running_projects.lock().contains_key(&project) {
            self.forget_when_idle.lock().insert(project);
            return Ok(Value::Null);
        }
        self.forget_when_idle.lock().remove(&project);
        self.forget_project(&project).await;
        Ok(Value::Null)
    }

    /// `agent:artifacts:sweep`.
    async fn sweep(self: Arc<Self>, call: Call) -> Result<Value> {
        let Some(live) = call.arg(0).as_array().filter(|ids| ids.len() <= 100_000) else { bail!("invalid project ids") };
        let live: Vec<String> = live.iter().filter_map(Value::as_str).filter(|id| is_project_id(id)).map(str::to_owned).collect();
        if live.len() != call.arg(0).as_array().map_or(0, Vec::len) {
            bail!("invalid project ids");
        }
        // An empty library is more likely unreadable storage than no projects.
        if live.is_empty() {
            return Ok(json!([]));
        }
        let mut busy: Vec<String> = self.running_projects.lock().keys().cloned().collect();
        busy.extend(self.forget_when_idle.lock().iter().cloned());
        Ok(json!(workspace::sweep(&self.user_data, &live, &busy).await))
    }

    fn artifact_path(&self, channel: &str, value: &Value) -> Result<PathBuf> {
        let (Some(project), Some(path)) = (value["projectId"].as_str().filter(|id| is_project_id(id)), value["path"].as_str()) else { bail!("{channel}: invalid artifact reference") };
        workspace::validated_artifact_path(&workspace::workspace_root(&self.user_data, project).join("artifacts"), path)
    }

    pub async fn shutdown(&self) {
        self.orchestrator.shutdown().await;
        for run in self.runs.lock().values() {
            run.cancel.cancel();
        }
        self.codex_app.shutdown().await;
        self.chatgpt.shutdown().await;
        self.claude.shutdown().await;
    }
}

/// Asks the person, then runs one command in the run's workspace without the sandbox.
fn outside_sandbox_runner(root: PathBuf, approve: Approver, cancel: CancellationToken) -> OutsideSandbox {
    Arc::new(move |command: String, reason: String, timeout_ms: u64| {
        let root = root.clone();
        let approve = approve.clone();
        let cancel = cancel.clone();
        async move {
            match approve(ApprovalRequest { title: "Run a command outside the project sandbox?".into(), detail: command.clone(), reason: Some(reason) }).await {
                Decision::Allowed => {}
                Decision::Denied(message) => bail!(message),
            }
            let args = vec!["-NoProfile".to_owned(), "-NonInteractive".to_owned(), "-Command".to_owned(), command];
            let mut process = super::runtime::command(std::path::Path::new("powershell"), &args, Some(&root), &[], &[]);
            process.stdin(std::process::Stdio::null());
            let mut output = String::new();
            let exit = process::run(process, &cancel, Duration::from_millis(timeout_ms), |_| {}, |line| {
                if output.len() < OUTSIDE_OUTPUT_BYTES {
                    output.push_str(line);
                    output.push('\n');
                }
            })
            .await;
            if let Some(error) = exit.spawn_error {
                bail!("The command could not start: {error}");
            }
            let truncated = output.len() >= OUTSIDE_OUTPUT_BYTES;
            output.push_str(&super::trace::truncate(exit.stderr.trim(), OUTSIDE_OUTPUT_BYTES / 4));
            let code = exit.code.map_or_else(|| if exit.timed_out { "none (timed out)".to_owned() } else { "none".to_owned() }, |code| code.to_string());
            Ok(format!("Exit code: {code}{}\n{}", if truncated { " (output truncated)" } else { "" }, output.trim_end()))
        }
        .boxed()
    })
}

/// `buildFixPrompt`.
fn fix_prompt(value: &Value) -> Result<String> {
    let (Some(id), Some(error), Some(files)) = (value["id"].as_str().filter(|id| is_extension_id(id)), value["error"].as_str().filter(|error| error.chars().count() <= 4_000), value["files"].as_array()) else {
        bail!("codex:fix-prompt: invalid request");
    };
    let mut rendered = Vec::new();
    for file in files.iter().take(FIX_PROMPT_FILES) {
        let (Some(path), Some(text)) = (file["path"].as_str().filter(|path| !path.is_empty() && path.chars().count() <= 1_000), file["text"].as_str()) else {
            bail!("codex:fix-prompt: invalid extension file");
        };
        let text = if text.len() > FIX_PROMPT_FILE_BYTES { format!("{}\n/* …truncated… */", super::trace::truncate(text, FIX_PROMPT_FILE_BYTES)) } else { text.to_owned() };
        rendered.push(format!("<file path={}>\n{text}\n</file>", serde_json::to_string(path)?));
    }
    let files = if rendered.is_empty() { "(none)".to_owned() } else { rendered.join("\n\n") };
    let prompt = format!("The extension `{id}` fails: {error}. Files:\n\n{files}");
    // Must fit the codex:run prompt limit or the follow-up run is rejected.
    let budget = request::PROMPT_CHARS - 200;
    Ok(if prompt.chars().count() > budget { format!("{}\n/* …truncated… */", prompt.chars().take(budget).collect::<String>()) } else { prompt })
}

pub async fn register(router: &mut Router, config: &Config) -> Result<Arc<Agents>> {
    let agents = Agents::new(config, None).await;
    install(router, &agents);
    Ok(agents)
}

pub(crate) fn install(router: &mut Router, agents: &Arc<Agents>) {
    let agents = agents.clone();

    router.json_with(&agents, "chatgpt:status", |agents, call| async move {
        agents.remember(&call);
        Ok(agents.chatgpt.status().await)
    });
    router.json_with(&agents, "chatgpt:models", |agents, call| async move {
        agents.remember(&call);
        Ok(json!(agents.chatgpt.models().await.unwrap_or_default()))
    });
    router.json_with(&agents, "chatgpt:connect", |agents, call| async move {
        agents.remember(&call);
        Ok(agents.chatgpt.connect().await)
    });
    router.json_with(&agents, "chatgpt:disconnect", |agents, call| async move {
        agents.remember(&call);
        Ok(agents.chatgpt.disconnect().await)
    });
    router.json_with(&agents, "claude:status", |agents, call| async move {
        agents.remember(&call);
        Ok(agents.claude.status().await)
    });
    router.json_with(&agents, "claude:models", |agents, call| async move {
        agents.remember(&call);
        Ok(json!(agents.claude.models().await.unwrap_or_default()))
    });
    router.json_with(&agents, "claude:connect", |agents, call| async move {
        agents.remember(&call);
        Ok(agents.claude.connect().await)
    });
    router.json_with(&agents, "claude:disconnect", |agents, call| async move {
        agents.remember(&call);
        Ok(agents.claude.disconnect().await)
    });
    router.json("agent-runtime:update", |_| async move { bail!("Updating agent runtimes {UNAVAILABLE}") });
    router.json_with(&agents, "compatible:status", |agents, _| async move { agents.compatible.status().await });
    router.json_with(&agents, "compatible:configure", |agents, call| async move { agents.compatible.configure(call.arg(0)).await });

    router.json_with(&agents, "codex:run", |agents, call| agents.run(call));
    router.json_with(&agents, "codex:steer", |agents, call| agents.steer(call));
    router.json_with(&agents, "codex:answer", |agents, call| agents.answer(call));
    router.json_with(&agents, "codex:cancel", |agents, call| agents.cancel(call));
    router.json_with(&agents, "codex:cancel-task", |agents, call| agents.cancel_task(call));
    router.json("codex:fix-prompt", |call| async move { Ok(json!(fix_prompt(call.arg(0))?)) });
    super::changes::install(router);
    router.json_with(&agents, "consent:computer", |agents, call| agents.consent(call));
    router.json_with(&agents, "agent-tool:response", |agents, call| async move {
        if let Some(bridge) = &agents.bridge {
            bridge.on_renderer_response(&call);
        }
        Ok(Value::Null)
    });

    router.on_with(&agents, "agent:artifact:read", |agents, call| async move {
        let path = agents.artifact_path("agent:artifact:read", call.arg(0))?;
        let data = Bytes::from(tokio::fs::read(&path).await?);
        let mut payload = crate::wire::Payload::default();
        let data = payload.attach(data);
        payload.json = json!({ "name": path.file_name().unwrap_or_default().to_string_lossy(), "mime": workspace::mime_type(&path), "data": data });
        Ok(payload)
    });
    router.json_with(&agents, "agent:reveal", |agents, call| async move {
        crate::app::reveal(&agents.artifact_path("agent:reveal", call.arg(0))?)?;
        Ok(Value::Null)
    });
    router.json_with(&agents, "agent:artifacts:forget", |agents, call| agents.forget(call));
    router.json_with(&agents, "agent:artifacts:sweep", |agents, call| agents.sweep(call));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_tokens_are_single_use() {
        let consent = Consent::default();
        let token = consent.mint()["token"].as_str().unwrap().to_owned();
        assert!(consent.consume(&token));
        assert!(!consent.consume(&token));
    }

    #[test]
    fn fix_prompts_render_files() {
        let prompt = fix_prompt(&json!({ "id": "glow", "error": "boom", "files": [{ "path": "index.ts", "text": "x" }] })).unwrap();
        assert_eq!(prompt, "The extension `glow` fails: boom. Files:\n\n<file path=\"index.ts\">\nx\n</file>");
        assert!(fix_prompt(&json!({ "id": "Bad", "error": "", "files": [] })).is_err());
    }
}
