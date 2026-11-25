//! Harness adapters. Codex and Claude Code run through their own CLIs with the
//! headless host as their MCP server; Aero runs inside the host (its tools are
//! the host's `Automation` bodies), so its adapter is a JSON-RPC method on the
//! runner's host connection that lanes A and B implement (`aero/run`, the wire
//! contract in docs/native/aero-eval.md).
//!
//! One step is one tool call the model issued. Every adapter counts steps as
//! the stream arrives and stops the run the moment it passes the task's
//! limit; the wall-clock deadline is enforced the same way. A stopped CLI
//! loses its whole process group.
use super::mcp::{Deadlines, McpClient, OnNote, RpcError};
use super::sandbox::Confinement;
use crate::record::Usage;
use crate::suite::Limits;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessId {
    Aero,
    Codex,
    ClaudeCode,
    /// Does nothing: proves seeding, collection and grading on the real host
    /// with no model (every task should fail). Never a comparator.
    None,
}

impl HarnessId {
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "aero" => Self::Aero,
            "codex" => Self::Codex,
            "claude-code" => Self::ClaudeCode,
            "none" => Self::None,
            _ => {
                return Err(format!(
                    "unknown harness {s} (aero, codex, claude-code, none)"
                ));
            }
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aero => "aero",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::None => "none",
        }
    }
    /// Whether a run would reach a paid model unless it is replayed.
    pub fn uses_model(self) -> bool {
        self != Self::None
    }
}

/// Where the harness's model requests go. Always explicit, so a replay can
/// never fall through to real authentication.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum Provider {
    /// No model at all (the `none` harness).
    None,
    /// A recorded stream on loopback.
    Replay {
        #[serde(rename = "baseUrl")]
        base_url: String,
    },
    /// A real provider with an approved batch's credentials.
    Live,
}

/// How an approved real-model batch hands a harness its credentials. Not
/// wired yet: until the Chief approves how a batch supplies them, every live
/// run is refused here, before anything starts.
pub trait Credentials {
    /// Environment entries for the harness (never written to the record).
    fn env(&self, harness: HarnessId, home: &Path) -> Result<Vec<(String, String)>, String>;
}

/// The only implementation today: refuses.
pub struct NotApproved;

impl Credentials for NotApproved {
    fn env(&self, harness: HarnessId, _: &Path) -> Result<Vec<(String, String)>, String> {
        Err(format!(
            "{}: live runs need batch credentials, and how a batch supplies them is not approved yet (the Chief decides); use --replay <recording>",
            harness.as_str()
        ))
    }
}

/// Everything a harness run needs; paths are absolute.
pub struct RunContext<'a> {
    pub prompt: &'a str,
    pub model: &'a str,
    pub effort: &'a str,
    pub limits: &'a Limits,
    /// The run's working folder (the harness's cwd).
    pub workdir: &'a Path,
    /// Isolated home for the harness CLI: never the owner's sign-ins.
    pub home: &'a Path,
    pub profile: &'a Path,
    /// How a CLI harness reaches the host as its MCP server.
    pub mcp_command: &'a Path,
    pub mcp_args: Vec<String>,
    /// The harness CLI (codex / claude) to run.
    pub bin: Option<&'a Path>,
    pub provider: Provider,
    /// Raw event stream of the harness, written as it arrives.
    pub transcript: &'a Path,
    /// Aero ablations: subsystems switched off for this run.
    pub ablate: &'a [String],
    /// The OS boundary every process of the run is started under.
    pub confine: Option<&'a Confinement>,
    pub deadlines: &'a Deadlines,
    /// Aero: the run id and the project revision the request is bound to.
    pub run_id: &'a str,
    pub project_id: &'a str,
    pub revision: Option<u64>,
}

impl RunContext<'_> {
    fn base_url(&self) -> Option<&str> {
        match &self.provider {
            Provider::Replay { base_url } => Some(base_url),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HarnessOutcome {
    pub exit_code: Option<i32>,
    /// The harness ended its turn by itself, successfully.
    pub completed: bool,
    pub stop_reason: String,
    pub timed_out: bool,
    pub over_step_limit: bool,
    /// None when the harness never started.
    pub wall_ms: Option<u64>,
    /// None when the stream carried no usage.
    pub usage: Option<Usage>,
    pub tool_calls: u32,
    pub retries: u32,
    pub gate_continuations: Option<u32>,
    pub final_text: String,
    /// The host stopped answering and must be stopped (no collection).
    pub host_hung: bool,
    /// Why the run failed to run properly (not a grader failure).
    pub error: Option<String>,
}

impl HarnessOutcome {
    fn failed(e: impl Into<String>) -> Self {
        let e = e.into();
        Self {
            stop_reason: e.clone(),
            error: Some(e),
            ..Default::default()
        }
    }
}

/// The runner's own JSON-RPC connection to the host (object-safe).
pub trait HostLink {
    fn request_until(
        &mut self,
        method: &str,
        params: Value,
        end: Instant,
        on_note: OnNote,
    ) -> Result<Value, RpcError>;
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String>;
    /// Keep waiting for the request whose deadline passed.
    fn resume(&mut self, end: Instant, on_note: OnNote) -> Result<Value, RpcError>;
}

impl<W: Write> HostLink for McpClient<W> {
    fn request_until(
        &mut self,
        method: &str,
        params: Value,
        end: Instant,
        on_note: OnNote,
    ) -> Result<Value, RpcError> {
        McpClient::request_until(self, method, params, end, on_note)
    }
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        McpClient::notify(self, method, params)
    }
    fn resume(&mut self, end: Instant, on_note: OnNote) -> Result<Value, RpcError> {
        McpClient::resume(self, end, on_note)
    }
}

pub trait Harness {
    fn id(&self) -> HarnessId;
    fn run(&self, cx: &RunContext, host: &mut dyn HostLink) -> HarnessOutcome;
}

pub fn for_id(id: HarnessId) -> Box<dyn Harness> {
    match id {
        HarnessId::Aero => Box::new(AeroHarness),
        HarnessId::Codex => Box::new(Cli(HarnessId::Codex)),
        HarnessId::ClaudeCode => Box::new(Cli(HarnessId::ClaudeCode)),
        HarnessId::None => Box::new(NoHarness),
    }
}

pub struct NoHarness;
impl Harness for NoHarness {
    fn id(&self) -> HarnessId {
        HarnessId::None
    }
    fn run(&self, _: &RunContext, _: &mut dyn HostLink) -> HarnessOutcome {
        HarnessOutcome {
            exit_code: Some(0),
            completed: true,
            stop_reason: "did nothing".into(),
            wall_ms: Some(0),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------- CLIs

pub struct Cli(pub HarnessId);

/// The MCP server entry both CLIs get: the host's stdio proxy.
fn mcp_entry(cx: &RunContext) -> Value {
    json!({"command": cx.mcp_command, "args": cx.mcp_args})
}

pub fn codex_argv(cx: &RunContext) -> Vec<String> {
    let q = |s: &str| serde_json::to_string(s).unwrap();
    let mut a = vec![
        "exec".into(),
        "--json".into(),
        "--skip-git-repo-check".into(),
        "--cd".into(),
        cx.workdir.display().to_string(),
        "--sandbox".into(),
        "workspace-write".into(),
        "--add-dir".into(),
        cx.profile.display().to_string(),
        "--model".into(),
        cx.model.into(),
        "-c".into(),
        format!("model_reasoning_effort={}", q(cx.effort)),
        "-c".into(),
        "approval_policy=\"never\"".into(),
        "-c".into(),
        format!(
            "mcp_servers.powermove.command={}",
            q(&cx.mcp_command.display().to_string())
        ),
        "-c".into(),
        format!(
            "mcp_servers.powermove.args={}",
            serde_json::to_string(&cx.mcp_args).unwrap()
        ),
    ];
    if let Some(url) = cx.base_url() {
        a.extend([
            "-c".into(),
            format!(
                "model_providers.replay={{name=\"replay\",base_url={},wire_api=\"responses\",env_key=\"OPENAI_API_KEY\"}}",
                q(&format!("{url}/v1"))
            ),
            "-c".into(),
            "model_provider=\"replay\"".into(),
        ]);
    }
    a.extend(["--".into(), cx.prompt.into()]);
    a
}

/// No `--max-turns`: a turn is not a step. Steps are counted from the stream
/// and enforced by the runner.
pub fn claude_argv(cx: &RunContext) -> Vec<String> {
    let mcp = json!({"mcpServers": {"powermove": mcp_entry(cx)}});
    vec![
        "--print".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--model".into(),
        cx.model.into(),
        "--effort".into(),
        cx.effort.into(),
        "--mcp-config".into(),
        mcp.to_string(),
        "--strict-mcp-config".into(),
        "--add-dir".into(),
        cx.profile.display().to_string(),
        "--allowedTools".into(),
        "mcp__powermove__*,Read,Write,Edit,Glob,Grep".into(),
        "--".into(),
        cx.prompt.into(),
    ]
}

/// The base every process of a run gets: nothing inherited but PATH, the
/// locale and a home and temp folder inside the run.
pub fn base_env(home: &Path, tmp: &Path) -> Vec<(String, String)> {
    vec![
        ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
        ("LANG".into(), "en_US.UTF-8".into()),
        ("HOME".into(), home.display().to_string()),
        ("TMPDIR".into(), tmp.display().to_string()),
        ("PM_NATIVE_NO_ORIGINAL".into(), "1".into()),
    ]
}

/// The CLI's environment: [`base_env`], the CLIs' config folders inside the
/// isolated home, and for replays a model endpoint on loopback with a dummy
/// key. Live credentials come only through [`Credentials`].
pub fn cli_env(id: HarnessId, cx: &RunContext) -> Vec<(String, String)> {
    let home = cx.home.display().to_string();
    let mut env = base_env(cx.home, &cx.home.join("tmp"));
    env.push(("CODEX_HOME".into(), format!("{home}/codex")));
    env.push(("CLAUDE_CONFIG_DIR".into(), format!("{home}/claude")));
    if let Some(url) = cx.base_url() {
        match id {
            HarnessId::Codex => env.push(("OPENAI_API_KEY".into(), "replay-no-key".into())),
            HarnessId::ClaudeCode => {
                env.push(("ANTHROPIC_BASE_URL".into(), url.to_owned()));
                env.push(("ANTHROPIC_API_KEY".into(), "replay-no-key".into()));
            }
            _ => {}
        }
    }
    env
}

impl Harness for Cli {
    fn id(&self) -> HarnessId {
        self.0
    }
    fn run(&self, cx: &RunContext, _: &mut dyn HostLink) -> HarnessOutcome {
        match self.run_cli(cx) {
            Ok(o) => o,
            Err(e) => HarnessOutcome::failed(e),
        }
    }
}

impl Cli {
    fn run_cli(&self, cx: &RunContext) -> Result<HarnessOutcome, String> {
        let bin = cx
            .bin
            .ok_or_else(|| format!("{}: pass --harness-bin", self.0.as_str()))?;
        let mut env = cli_env(self.0, cx);
        if cx.provider == Provider::Live {
            env.extend(NotApproved.env(self.0, cx.home)?);
        }
        let argv = match self.0 {
            HarnessId::Codex => codex_argv(cx),
            _ => claude_argv(cx),
        };
        for d in ["tmp", "codex", "claude"] {
            std::fs::create_dir_all(cx.home.join(d)).map_err(|e| e.to_string())?;
        }
        let mut cmd = super::sandbox::command(bin, cx.confine);
        cmd.args(&argv)
            .current_dir(cx.workdir)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(
                std::fs::File::create(cx.transcript.with_extension("stderr"))
                    .map_err(|e| e.to_string())?,
            ));
        let parser: Arc<Mutex<Box<dyn StreamParser + Send>>> = Arc::new(Mutex::new(match self.0 {
            HarnessId::Codex => Box::new(CodexParser::default()),
            _ => Box::new(ClaudeParser::default()),
        }));
        let mut transcript = std::fs::File::create(cx.transcript).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let mut child =
            super::proc::spawn_group(&mut cmd).map_err(|e| format!("{}: {e}", bin.display()))?;
        let stdout = child.stdout.take().unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let p = parser.clone();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let result = (|| {
                loop {
                    let mut buf = Vec::new();
                    if r.read_until(b'\n', &mut buf).map_err(|e| e.to_string())? == 0 {
                        return Ok(());
                    }
                    transcript.write_all(&buf).map_err(|e| e.to_string())?;
                    if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                        p.lock().unwrap().event(&v);
                    }
                }
            })();
            let _ = done_tx.send(result);
        });
        let deadline = started + Duration::from_secs(cx.limits.wall_secs);
        let mut o = HarnessOutcome::default();
        let natural = loop {
            if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
                break Some(s);
            }
            if parser.lock().unwrap().steps() > cx.limits.steps {
                o.over_step_limit = true;
                o.stop_reason = format!("stopped after {} steps", cx.limits.steps);
                break None;
            }
            if Instant::now() >= deadline {
                o.timed_out = true;
                o.stop_reason = format!("stopped at the {} s limit", cx.limits.wall_secs);
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // Always stop the group: nothing the harness started outlives it.
        let stopped = super::proc::stop_group(&mut child, cx.deadlines.grace);
        o.wall_ms = Some(started.elapsed().as_millis() as u64);
        let mut errors = Vec::new();
        if stopped.stragglers {
            errors.push("processes of the harness survived the stop".to_string());
        }
        match done_rx.recv_timeout(cx.deadlines.grace) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(format!("transcript: {e}")),
            Err(_) => errors
                .push("a process outside the harness's group kept its output open".to_string()),
        }
        let status = natural.or(stopped.status);
        let p = parser.lock().unwrap().outcome();
        o.usage = p.usage;
        o.tool_calls = p.tool_calls;
        o.retries = p.retries;
        o.final_text = p.final_text;
        o.exit_code = status.and_then(|s| s.code());
        let failed_turn = parser.lock().unwrap().failed();
        if let Some(s) = natural {
            o.stop_reason = match (s.code(), &failed_turn) {
                (Some(0), None) => "exited 0".into(),
                (Some(0), Some(f)) => format!("exited 0 after a failed turn: {f}"),
                (Some(c), _) => format!("exited {c}"),
                (None, _) => "killed by a signal".into(),
            };
        }
        o.completed = natural.is_some_and(|s| s.code() == Some(0))
            && failed_turn.is_none()
            && errors.is_empty();
        o.error = (!errors.is_empty()).then(|| errors.join("; "));
        Ok(o)
    }
}

/// Turns a harness's event stream into the run's counters.
pub trait StreamParser {
    fn event(&mut self, v: &Value);
    fn outcome(&self) -> HarnessOutcome;
    /// Steps so far.
    fn steps(&self) -> u32 {
        self.outcome().tool_calls
    }
    /// The turn the stream reports as failed, if any.
    fn failed(&self) -> Option<String>;
}

fn add_usage(o: &mut HarnessOutcome, u: Usage) {
    o.usage.get_or_insert_with(Usage::default).add(u);
}

/// `codex exec --json`: thread/turn/item events; usage on `turn.completed`.
#[derive(Default)]
pub struct CodexParser {
    o: HarnessOutcome,
    failed: Option<String>,
}

impl StreamParser for CodexParser {
    fn event(&mut self, v: &Value) {
        let ty = v["type"].as_str().unwrap_or("");
        match ty {
            "turn.completed" => {
                let u = &v["usage"];
                if u.is_object() {
                    add_usage(
                        &mut self.o,
                        Usage {
                            input: u["input_tokens"].as_u64().unwrap_or(0),
                            cached: u["cached_input_tokens"].as_u64().unwrap_or(0),
                            output: u["output_tokens"].as_u64().unwrap_or(0),
                        },
                    );
                }
            }
            "turn.failed" => {
                self.failed = Some(
                    v["error"]["message"]
                        .as_str()
                        .unwrap_or("turn failed")
                        .to_owned(),
                )
            }
            "item.completed" => {
                let item = &v["item"];
                match item["type"].as_str().unwrap_or("") {
                    "agent_message" => {
                        self.o.final_text = item["text"].as_str().unwrap_or("").to_owned()
                    }
                    "reasoning" | "todo_list" | "error" => {}
                    _ => self.o.tool_calls += 1,
                }
            }
            "error"
                if v["message"]
                    .as_str()
                    .is_some_and(|m| m.to_lowercase().starts_with("reconnecting")) =>
            {
                self.o.retries += 1
            }
            _ => {}
        }
    }
    fn outcome(&self) -> HarnessOutcome {
        self.o.clone()
    }
    fn steps(&self) -> u32 {
        self.o.tool_calls
    }
    fn failed(&self) -> Option<String> {
        self.failed.clone()
    }
}

/// `claude --print --output-format stream-json`: assistant messages carry
/// tool_use blocks; the final `result` carries usage and whether it failed.
#[derive(Default)]
pub struct ClaudeParser {
    o: HarnessOutcome,
    seen: std::collections::BTreeSet<String>,
    failed: Option<String>,
}

impl StreamParser for ClaudeParser {
    fn event(&mut self, v: &Value) {
        match v["type"].as_str().unwrap_or("") {
            "assistant" => {
                for b in v["message"]["content"].as_array().into_iter().flatten() {
                    if b["type"] == "tool_use" {
                        let id = b["id"].as_str().unwrap_or("").to_owned();
                        if id.is_empty() || self.seen.insert(id) {
                            self.o.tool_calls += 1;
                        }
                    }
                }
            }
            "system" if v["subtype"] == "api_retry" => self.o.retries += 1,
            "result" => {
                let u = &v["usage"];
                if u.is_object() {
                    let n = |k: &str| u[k].as_u64().unwrap_or(0);
                    let cached = n("cache_read_input_tokens");
                    add_usage(
                        &mut self.o,
                        Usage {
                            input: n("input_tokens") + n("cache_creation_input_tokens") + cached,
                            cached,
                            output: n("output_tokens"),
                        },
                    );
                }
                if v["is_error"] == true || v["subtype"].as_str().is_some_and(|s| s != "success") {
                    self.failed = Some(
                        v["subtype"]
                            .as_str()
                            .unwrap_or("the result is an error")
                            .to_owned(),
                    );
                }
                self.o.final_text = v["result"].as_str().unwrap_or("").to_owned();
            }
            _ => {}
        }
    }
    fn outcome(&self) -> HarnessOutcome {
        self.o.clone()
    }
    fn steps(&self) -> u32 {
        self.o.tool_calls
    }
    fn failed(&self) -> Option<String> {
        self.failed.clone()
    }
}

// ---------------------------------------------------------------- Aero

/// `aero/run` params: one Aero session, run to completion inside the host the
/// runner holds, bound to the project and revision the runner opened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AeroRequest {
    pub run_id: String,
    pub project_id: String,
    /// The revision the runner saw after opening the project; the host refuses
    /// a run whose project or revision differ.
    pub revision: Option<u64>,
    pub prompt: String,
    pub model: String,
    pub effort: String,
    pub wall_secs: u64,
    /// Steps (tool calls) allowed.
    pub max_steps: u32,
    /// Completion-gate continuations allowed (DESIGN.md 4.3; default 3).
    pub max_gate_continuations: u32,
    /// Where model requests go; a replay never falls back to real auth.
    pub provider: Provider,
    /// Subsystems switched off for an ablation run: `topics`,
    /// `completion-gate`, `self-check`, `evaluator`.
    pub ablate: Vec<String>,
}

/// One `aero/event` notification: run-scoped and ordered (`seq` counts from
/// 0 with no gaps).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AeroNotice {
    pub run_id: String,
    pub seq: u64,
    pub event: AeroEvent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AeroEvent {
    Usage {
        input: u64,
        cached: u64,
        output: u64,
    },
    ToolCall {
        name: String,
        ok: bool,
    },
    Retry {
        reason: String,
    },
    GateContinuation {
        failures: Vec<String>,
    },
}

/// Why an Aero run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AeroReason {
    /// The completion gate let the turn end.
    Completed,
    /// The gate still failed after the allowed continuations.
    GateExhausted,
    StepLimit,
    Deadline,
    /// The runner sent `aero/cancel`.
    Cancelled,
    Error,
}

/// The `aero/run` result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AeroResult {
    pub run_id: String,
    pub final_text: String,
    pub completed: bool,
    pub reason: AeroReason,
}

pub struct AeroHarness;

/// Counts and checks `aero/event`s as they arrive.
struct AeroStream {
    run_id: String,
    max_steps: u32,
    next_seq: u64,
    o: HarnessOutcome,
    log: Option<std::fs::File>,
    error: Option<String>,
    cancelled: bool,
}

impl AeroStream {
    fn note(&mut self, m: &Value) -> Option<(String, Value)> {
        if m["method"] != "aero/event" {
            return None;
        }
        if let Some(log) = self.log.as_mut() {
            let _ = writeln!(log, "{}", m["params"]);
        }
        let n = match serde_json::from_value::<AeroNotice>(m["params"].clone()) {
            Ok(n) => n,
            Err(e) => {
                self.error
                    .get_or_insert(format!("aero/event the runner cannot read: {e}"));
                return None;
            }
        };
        if n.run_id != self.run_id {
            self.error.get_or_insert(format!(
                "aero/event for run {} during run {}",
                n.run_id, self.run_id
            ));
            return None;
        }
        if n.seq != self.next_seq {
            self.error.get_or_insert(format!(
                "aero/event {} arrived when {} was due (events must be ordered, no gaps)",
                n.seq, self.next_seq
            ));
        }
        self.next_seq = n.seq + 1;
        match n.event {
            AeroEvent::Usage {
                input,
                cached,
                output,
            } => add_usage(
                &mut self.o,
                Usage {
                    input,
                    cached,
                    output,
                },
            ),
            AeroEvent::ToolCall { .. } => self.o.tool_calls += 1,
            AeroEvent::Retry { .. } => self.o.retries += 1,
            AeroEvent::GateContinuation { .. } => *self.o.gate_continuations.get_or_insert(0) += 1,
        }
        if self.o.tool_calls > self.max_steps && !self.cancelled {
            self.cancelled = true;
            self.o.over_step_limit = true;
            return Some((
                "aero/cancel".into(),
                json!({"runId": self.run_id, "reason": "step-limit"}),
            ));
        }
        None
    }
}

impl Harness for AeroHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Aero
    }
    fn run(&self, cx: &RunContext, host: &mut dyn HostLink) -> HarnessOutcome {
        if cx.provider == Provider::Live
            && let Err(e) = NotApproved.env(HarnessId::Aero, cx.home)
        {
            return HarnessOutcome::failed(e);
        }
        let req = AeroRequest {
            run_id: cx.run_id.into(),
            project_id: cx.project_id.into(),
            revision: cx.revision,
            prompt: cx.prompt.into(),
            model: cx.model.into(),
            effort: cx.effort.into(),
            wall_secs: cx.limits.wall_secs,
            max_steps: cx.limits.steps,
            max_gate_continuations: 3,
            provider: cx.provider.clone(),
            ablate: cx.ablate.to_vec(),
        };
        let mut s = AeroStream {
            run_id: cx.run_id.into(),
            max_steps: cx.limits.steps,
            next_seq: 0,
            o: HarnessOutcome {
                gate_continuations: Some(0),
                ..Default::default()
            },
            log: std::fs::File::create(cx.transcript).ok(),
            error: None,
            cancelled: false,
        };
        let started = Instant::now();
        let end = started + Duration::from_secs(cx.limits.wall_secs);
        let params = serde_json::to_value(&req).unwrap();
        let mut answer = host.request_until("aero/run", params, end, &mut |m| s.note(m));
        if let Err(RpcError::Timeout(_)) = answer {
            // The deadline: cancel through the harness's own action, then give
            // the host a grace period to end the run and answer.
            s.o.timed_out = true;
            let _ = host.notify(
                "aero/cancel",
                json!({"runId": cx.run_id, "reason": "deadline"}),
            );
            answer = host.resume(Instant::now() + cx.deadlines.grace, &mut |m| s.note(m));
        }
        let mut o = std::mem::take(&mut s.o);
        o.wall_ms = Some(started.elapsed().as_millis() as u64);
        let result = match answer {
            Ok(v) => serde_json::from_value::<AeroResult>(v)
                .map_err(|e| format!("aero/run result the runner cannot read: {e}")),
            Err(RpcError::Remote(-32601, _)) if !o.timed_out => {
                Err("this host has no Aero yet: aero/run arrives with lanes A and B".into())
            }
            Err(e @ (RpcError::Timeout(_) | RpcError::Closed(_))) => {
                o.host_hung = true;
                Err(format!("the host did not end the run: {e}"))
            }
            Err(e) => Err(e.to_string()),
        };
        match result {
            Ok(r) if r.run_id != cx.run_id => {
                o.error = Some(format!(
                    "aero/run answered for run {} instead of {}",
                    r.run_id, cx.run_id
                ));
            }
            Ok(r) => {
                o.final_text = r.final_text;
                o.stop_reason = serde_json::to_value(r.reason)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                o.completed = r.completed
                    && r.reason == AeroReason::Completed
                    && !o.timed_out
                    && !o.over_step_limit;
                o.exit_code = Some(if o.completed { 0 } else { 1 });
            }
            Err(e) => {
                o.stop_reason = e.clone();
                o.error = Some(e);
            }
        }
        if let Some(e) = s.error {
            o.completed = false;
            o.error.get_or_insert(e);
        }
        o
    }
}

/// Where a harness's isolated home lives inside a run folder.
pub fn home_in(run_dir: &Path) -> PathBuf {
    run_dir.join("home")
}
