//! Harness adapters. Codex and Claude Code run through their own CLIs with the
//! headless host as their MCP server; Aero runs inside the host (its tools are
//! the host's `Automation` bodies), so its adapter is a JSON-RPC method on the
//! runner's host connection that lanes A and B implement (`aero/run`).
use crate::record::Usage;
use crate::suite::Limits;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessId {
    Aero,
    Codex,
    ClaudeCode,
    /// Does nothing: proves seeding, collection and grading on the real host
    /// with no model (every task should fail).
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
    /// Replay origin: the harness's model base URL points here.
    pub model_base_url: Option<String>,
    /// Raw event stream of the harness, written as it arrives.
    pub transcript: &'a Path,
    /// Aero ablations: subsystems switched off for this run.
    pub ablate: &'a [String],
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HarnessOutcome {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub wall_ms: u64,
    pub usage: Usage,
    pub tool_calls: u32,
    pub retries: u32,
    pub gate_continuations: Option<u32>,
    pub final_text: String,
}

/// The runner's own JSON-RPC connection to the host (object-safe).
pub trait HostLink {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String>;
    fn take_notifications(&mut self) -> Vec<Value>;
}

impl<R: std::io::Read, W: Write> HostLink for super::mcp::McpClient<R, W> {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        super::mcp::McpClient::request(self, method, params)
    }
    fn take_notifications(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.notifications)
    }
}

pub trait Harness {
    fn id(&self) -> HarnessId;
    fn run(&self, cx: &RunContext, host: &mut dyn HostLink) -> Result<HarnessOutcome, String>;
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
    fn run(&self, _: &RunContext, _: &mut dyn HostLink) -> Result<HarnessOutcome, String> {
        Ok(HarnessOutcome {
            exit_code: Some(0),
            ..Default::default()
        })
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
    if let Some(url) = &cx.model_base_url {
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
        "--max-turns".into(),
        cx.limits.steps.to_string(),
        "--add-dir".into(),
        cx.profile.display().to_string(),
        "--allowedTools".into(),
        "mcp__powermove__*,Read,Write,Edit,Glob,Grep".into(),
        "--".into(),
        cx.prompt.into(),
    ]
}

/// The CLI's environment: nothing inherited but PATH and the locale, an
/// isolated home, and for replays a model endpoint on loopback with a dummy
/// key. A real run's credentials come only from an approved batch's prepared
/// home, never from the owner's sign-ins.
pub fn cli_env(id: HarnessId, cx: &RunContext) -> Vec<(String, String)> {
    let home = cx.home.display().to_string();
    let mut env = vec![
        ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
        ("LANG".into(), "en_US.UTF-8".into()),
        ("HOME".into(), home.clone()),
        ("TMPDIR".into(), format!("{home}/tmp")),
        ("CODEX_HOME".into(), format!("{home}/codex")),
        ("CLAUDE_CONFIG_DIR".into(), format!("{home}/claude")),
        ("PM_NATIVE_NO_ORIGINAL".into(), "1".into()),
    ];
    if let Some(url) = &cx.model_base_url {
        match id {
            HarnessId::Codex => env.push(("OPENAI_API_KEY".into(), "replay-no-key".into())),
            HarnessId::ClaudeCode => {
                env.push(("ANTHROPIC_BASE_URL".into(), url.clone()));
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
    fn run(&self, cx: &RunContext, _: &mut dyn HostLink) -> Result<HarnessOutcome, String> {
        let bin = cx
            .bin
            .ok_or_else(|| format!("{}: pass --harness-bin", self.0.as_str()))?;
        let argv = match self.0 {
            HarnessId::Codex => codex_argv(cx),
            _ => claude_argv(cx),
        };
        for d in ["tmp", "codex", "claude"] {
            std::fs::create_dir_all(cx.home.join(d)).map_err(|e| e.to_string())?;
        }
        let mut cmd = Command::new(bin);
        cmd.args(&argv)
            .current_dir(cx.workdir)
            .env_clear()
            .envs(cli_env(self.0, cx))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(
                std::fs::File::create(cx.transcript.with_extension("stderr"))
                    .map_err(|e| e.to_string())?,
            ));
        let mut parser: Box<dyn StreamParser + Send> = match self.0 {
            HarnessId::Codex => Box::new(CodexParser::default()),
            _ => Box::new(ClaudeParser::default()),
        };
        let started = Instant::now();
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", bin.display()))?;
        let stdout = child.stdout.take().unwrap();
        let transcript = cx.transcript.to_path_buf();
        let reader = std::thread::spawn(move || -> Result<Box<dyn StreamParser + Send>, String> {
            let mut out = std::fs::File::create(&transcript).map_err(|e| e.to_string())?;
            for line in BufReader::new(stdout).lines() {
                let line = line.map_err(|e| e.to_string())?;
                writeln!(out, "{line}").map_err(|e| e.to_string())?;
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    parser.event(&v);
                }
            }
            Ok(parser)
        });
        let deadline = started + Duration::from_secs(cx.limits.wall_secs);
        let (exit_code, timed_out) = loop {
            if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
                break (s.code(), false);
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let s = child.wait().ok();
                break (s.and_then(|s| s.code()), true);
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let wall_ms = started.elapsed().as_millis() as u64;
        parser = reader
            .join()
            .map_err(|_| "transcript reader panicked".to_string())??;
        let mut o = parser.outcome();
        o.exit_code = exit_code;
        o.timed_out = timed_out;
        o.wall_ms = wall_ms;
        Ok(o)
    }
}

/// Turns a harness's event stream into the run's counters.
pub trait StreamParser {
    fn event(&mut self, v: &Value);
    fn outcome(&self) -> HarnessOutcome;
}

/// `codex exec --json`: thread/turn/item events; usage on `turn.completed`.
#[derive(Default)]
pub struct CodexParser {
    o: HarnessOutcome,
}

impl StreamParser for CodexParser {
    fn event(&mut self, v: &Value) {
        let ty = v["type"].as_str().unwrap_or("");
        match ty {
            "turn.completed" => {
                let u = &v["usage"];
                self.o.usage.add(Usage {
                    input: u["input_tokens"].as_u64().unwrap_or(0),
                    cached: u["cached_input_tokens"].as_u64().unwrap_or(0),
                    output: u["output_tokens"].as_u64().unwrap_or(0),
                });
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
}

/// `claude --print --output-format stream-json`: assistant messages carry
/// tool_use blocks; the final `result` carries usage.
#[derive(Default)]
pub struct ClaudeParser {
    o: HarnessOutcome,
    seen: std::collections::BTreeSet<String>,
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
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let cached = n("cache_read_input_tokens");
                self.o.usage.add(Usage {
                    input: n("input_tokens") + n("cache_creation_input_tokens") + cached,
                    cached,
                    output: n("output_tokens"),
                });
                self.o.final_text = v["result"].as_str().unwrap_or("").to_owned();
            }
            _ => {}
        }
    }
    fn outcome(&self) -> HarnessOutcome {
        self.o.clone()
    }
}

// ---------------------------------------------------------------- Aero

/// `aero/run` params: one Aero session, run to completion inside the host the
/// runner holds, on the project the runner opened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AeroRequest {
    pub prompt: String,
    pub model: String,
    pub effort: String,
    pub wall_secs: u64,
    pub max_steps: u32,
    /// Completion-gate continuations allowed (DESIGN.md 4.3; default 3).
    pub max_gate_continuations: u32,
    /// Replay: the host-fetch bridge sends model requests here instead.
    pub model_base_url: Option<String>,
    /// Subsystems switched off for an ablation run: `topics`,
    /// `completion-gate`, `self-check`, `evaluator`.
    pub ablate: Vec<String>,
}

/// `aero/event` notifications the host sends while the session runs.
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

/// The `aero/run` result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AeroResult {
    pub final_text: String,
    /// The completion gate let the turn end (false: "not done: …").
    pub completed: bool,
}

pub struct AeroHarness;

impl Harness for AeroHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Aero
    }
    fn run(&self, cx: &RunContext, host: &mut dyn HostLink) -> Result<HarnessOutcome, String> {
        let req = AeroRequest {
            prompt: cx.prompt.into(),
            model: cx.model.into(),
            effort: cx.effort.into(),
            wall_secs: cx.limits.wall_secs,
            max_steps: cx.limits.steps,
            max_gate_continuations: 3,
            model_base_url: cx.model_base_url.clone(),
            ablate: cx.ablate.to_vec(),
        };
        let started = Instant::now();
        let result = host
            .request("aero/run", serde_json::to_value(&req).unwrap())
            .map_err(|e| {
                if e.contains("-32601") {
                    "this host has no Aero yet: aero/run arrives with lanes A and B".to_string()
                } else {
                    e
                }
            })?;
        let wall_ms = started.elapsed().as_millis() as u64;
        let mut o = HarnessOutcome {
            exit_code: Some(0),
            wall_ms,
            gate_continuations: Some(0),
            ..Default::default()
        };
        let mut log = std::fs::File::create(cx.transcript).map_err(|e| e.to_string())?;
        for n in host.take_notifications() {
            if n["method"] != "aero/event" {
                continue;
            }
            let _ = writeln!(log, "{}", n["params"]);
            match serde_json::from_value::<AeroEvent>(n["params"].clone()) {
                Ok(AeroEvent::Usage {
                    input,
                    cached,
                    output,
                }) => o.usage.add(Usage {
                    input,
                    cached,
                    output,
                }),
                Ok(AeroEvent::ToolCall { .. }) => o.tool_calls += 1,
                Ok(AeroEvent::Retry { .. }) => o.retries += 1,
                Ok(AeroEvent::GateContinuation { .. }) => {
                    *o.gate_continuations.as_mut().unwrap() += 1
                }
                Err(e) => return Err(format!("aero/event the runner cannot read: {e}")),
            }
        }
        let r: AeroResult = serde_json::from_value(result)
            .map_err(|e| format!("aero/run result the runner cannot read: {e}"))?;
        o.final_text = r.final_text;
        o.timed_out = wall_ms > cx.limits.wall_secs * 1000;
        if !r.completed {
            o.exit_code = Some(1);
        }
        Ok(o)
    }
}

/// Where a harness's isolated home lives inside a run folder.
pub fn home_in(run_dir: &Path) -> PathBuf {
    run_dir.join("home")
}
