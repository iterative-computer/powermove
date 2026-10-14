//! Autonomous ChatGPT runs through `codex exec --json` (port of
//! `codex/runner.ts` and `codex/adapter.ts`).

use std::path::Path;
use std::process::Stdio;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::autonomous::{self, Attempt, Env, NativeTools, Provider, RunContext, TOOL_TOKEN_ENV};
use super::codex_events;
use super::process::{self, Exit};
use super::request::Access;
use super::runtime;
use super::workspace::{self, Workspace, valid_session_id};

pub const PROJECT_PERMISSION_PROFILE: &str = "powermove";

pub struct CodexExec;

/// `projectSandboxArgv`: the permission profile sandboxed Project shells run under.
fn project_sandbox_argv(shell_network: bool, supervised: bool) -> Vec<String> {
    let profile = format!("permissions.{PROJECT_PERMISSION_PROFILE}");
    let mut config: Vec<String> = if supervised {
        vec!["approval_policy=\"never\"".into()]
    } else {
        vec!["approvals_reviewer=\"auto_review\"".into(), "approval_policy=\"on-request\"".into()]
    };
    config.push(format!("default_permissions=\"{PROJECT_PERMISSION_PROFILE}\""));
    config.push(format!("{profile}.extends=\":workspace\""));
    if shell_network {
        config.push(format!("{profile}.network.enabled=true"));
    }
    config.into_iter().flat_map(|value| ["--config".to_owned(), value]).collect()
}

/// The run-scoped Powermove tools as a streamable-HTTP MCP server. The
/// bearer token reaches Codex through the environment, never the argv.
pub fn native_mcp_argv(tools: Option<&NativeTools>) -> Vec<String> {
    let Some(tools) = tools else { return Vec::new() };
    [
        format!("mcp_servers.powermove.url={}", serde_json::to_string(&tools.url).unwrap_or_default()),
        format!("mcp_servers.powermove.bearer_token_env_var=\"{TOOL_TOKEN_ENV}\""),
        "mcp_servers.powermove.required=true".into(),
        // Run-scoped tools carry no annotations; approve them without a review.
        "mcp_servers.powermove.default_tools_approval_mode=\"approve\"".into(),
        "mcp_servers.powermove.startup_timeout_sec=30".into(),
        format!("mcp_servers.powermove.tool_timeout_sec={}", tools.timeout_sec),
    ]
    .into_iter()
    .flat_map(|value| ["--config".to_owned(), value])
    .collect()
}

pub struct AutonomousArgv<'a> {
    pub schema_path: &'a Path,
    pub output_path: &'a Path,
    pub instructions: &'a str,
    pub prompt: &'a str,
    pub image_paths: &'a [std::path::PathBuf],
    pub model: Option<&'a str>,
    pub reasoning_effort: Option<&'a str>,
    pub access: Access,
    pub supervised: bool,
    pub extensions_dir: &'a Path,
    pub session_id: Option<&'a str>,
    pub tools: Option<&'a NativeTools>,
    pub shell_network_instructions: &'a str,
}

/// `buildAutonomousArgv`.
pub fn autonomous_argv(options: &AutonomousArgv) -> Vec<String> {
    let sandboxed = options.access != Access::Computer;
    let mut argv: Vec<String> = vec!["--search".into()];
    if !sandboxed {
        argv.push("--dangerously-bypass-approvals-and-sandbox".into());
    }
    argv.extend(["--add-dir".into(), options.extensions_dir.display().to_string(), "exec".into()]);
    let session = options.session_id.filter(|id| valid_session_id(id));
    if session.is_some() {
        argv.push("resume".into());
    }
    argv.push("--skip-git-repo-check".into());
    if sandboxed {
        argv.extend(project_sandbox_argv(options.access == Access::Project, options.supervised));
    }
    argv.extend(native_mcp_argv(options.tools));
    argv.extend(["--output-schema".into(), options.schema_path.display().to_string(), "--output-last-message".into(), options.output_path.display().to_string(), "--json".into()]);
    if let Some(model) = options.model {
        argv.extend(["--model".into(), model.to_owned()]);
    }
    if let Some(effort) = options.reasoning_effort {
        argv.extend(["--config".into(), format!("model_reasoning_effort=\"{effort}\"")]);
    }
    for image in options.image_paths {
        argv.extend(["--image".into(), image.display().to_string()]);
    }
    // Positionals follow `--`, so the session id cannot read as a flag. The
    // prompt goes to stdin (`-`): Windows caps a command line at 32,767
    // characters, which the instructions alone can exceed.
    argv.push("--".into());
    if let Some(session) = session {
        argv.push(session.to_owned());
    }
    argv.push("-".into());
    argv
}

/// The run's full prompt (instructions, then the request), sent on stdin.
pub fn autonomous_input(options: &AutonomousArgv) -> String {
    let sandboxed = options.access != Access::Computer;
    let network = if sandboxed && options.access == Access::Project { format!("\n\nSHELL NETWORK\n{}", options.shell_network_instructions) } else { String::new() };
    let approvals = if sandboxed && options.supervised && options.tools.is_some_and(|tools| tools.outside_sandbox) {
        format!("\n\nAPPROVALS\nThe sandbox confines writes to this workspace and you cannot escalate a command yourself. When one genuinely needs to run outside it, such as installing a font or writing to the user's home folder, call {} with that one command and why. Powermove asks the user to approve it; if they decline, do not retry it.", super::tools::OUTSIDE_SANDBOX_TOOL)
    } else {
        String::new()
    };
    format!("{}{network}{approvals}\n\nUSER REQUEST\n{}", options.instructions, options.prompt)
}

fn meaningful_stderr(stderr: &str) -> String {
    stderr.lines().filter(|line| !line.trim().to_lowercase().starts_with("reading additional input from stdin")).collect::<Vec<_>>().join("\n").trim().to_owned()
}

/// Remove bearer tokens and API keys before a diagnostic reaches the log.
pub fn redact(text: &str) -> String {
    static SECRET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(authorization:\s*(?:bearer\s+)?|bearer\s+)\S+|\bsk-[A-Za-z0-9_-]{8,}|\b[0-9a-f]{64}\b").unwrap());
    SECRET.replace_all(text, "$1[redacted]").into_owned()
}

/// `humanizeCodexFailure`: raw CLI stderr never reaches the conversation.
pub fn humanize_failure(diagnostic: &str, fallback: &str) -> String {
    let text = diagnostic.trim();
    if text.is_empty() {
        return fallback.to_owned();
    }
    tracing::error!("[codex] run failed: {}", redact(&super::trace::truncate(text, 4_000)));
    let matches = |pattern: &str| Regex::new(pattern).expect("static pattern").is_match(text);
    if matches(r"(?i)AuthRequired|www_authenticate|Unauthorized|\b401\b") && matches(r"(?i)rmcp|mcp") {
        return "One of your Codex integrations (an MCP server) needs to be signed in again. Run `codex` in a terminal, re-authenticate it, then retry.".into();
    }
    if matches(r"(?i)not logged in|login required|please run codex login|invalid api key|missing (?:authentication )?credentials|authentication credentials (?:were|are) not provided") {
        return "ChatGPT is not connected to Powermove. Connect your ChatGPT account and retry.".into();
    }
    if matches(r"(?i)ENOENT|command not found|No such file|program not found|cannot find the file") {
        return "The Codex CLI could not be launched. Check that `codex` is installed.".into();
    }
    if matches(r"(?i)already has an active writer") {
        return "The saved agent session couldn’t reopen. Try again to reconnect the agent; if it repeats, restart Powermove.".into();
    }
    if matches(r"(?i)ECONNRESET|ENOTFOUND|fetch failed|network|connection (?:closed|refused)|stream disconnected") {
        return "The connection to the agent was interrupted. Check your internet connection, then retry.".into();
    }
    if matches(r"(?i)rate.?limit|\b429\b|overloaded") {
        return "The model is rate-limited right now. Wait a moment and retry.".into();
    }
    if matches(r"(?i)invalid_json_schema|Invalid schema for response_format") {
        return "Powermove's agent response format was rejected. Restart Powermove and retry; if it persists, update the app.".into();
    }
    static LOG_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\d{4}-\d{2}-\d{2}T\S+\s+(ERROR|WARN|INFO)\s+\S*:?\s*").unwrap());
    let last = text.lines().map(|line| LOG_PREFIX.replace(line, "").trim().to_owned()).filter(|line| !line.is_empty()).last().unwrap_or_else(|| fallback.to_owned());
    format!("The agent failed: {}", redact(&super::trace::truncate(&last, 240)))
}

fn diagnostic(exit: &Exit) -> Option<String> {
    codex_events::error_from_stdout(&exit.stdout).or_else(|| Some(meaningful_stderr(&exit.stderr)).filter(|text| !text.is_empty())).or_else(|| exit.spawn_error.clone())
}

fn unknown_session(text: &str) -> bool {
    static UNKNOWN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:unknown|invalid|missing|not found|does not exist|could not find).{0,80}(?:session|thread|rollout)|(?:session|thread|rollout).{0,80}(?:unknown|invalid|missing|not found|does not exist|could not find)|no rollout found").unwrap()
    });
    UNKNOWN.is_match(text)
}

impl Provider for CodexExec {
    fn name(&self) -> &'static str {
        "Codex"
    }

    async fn attempt(&self, env: &Env, ctx: &RunContext, layout: &Workspace, prompt: &str, session: Option<String>) -> Attempt {
        let failed = |message: String| Attempt { exit: Exit::default(), output: None, failure: Some(message), retry_fresh: false };
        let binary = match env.runtimes.codex() {
            Ok(binary) => binary,
            Err(error) => return failed(error.to_string()),
        };
        let mut process_env = match env.runtimes.codex_env(&binary).await {
            Ok(process_env) => process_env,
            Err(error) => return failed(format!("Could not prepare Codex: {error:#}")),
        };
        if let Some(tools) = &ctx.tools {
            process_env.push((TOOL_TOKEN_ENV, tools.token.clone().into()));
        }
        // A repair must produce a new report, never reuse the rejected output.
        let _ = tokio::fs::remove_file(&layout.output_path).await;
        let request = &ctx.request;
        let instructions = autonomous::instructions(env, ctx, layout, prompt);
        let options = AutonomousArgv {
            schema_path: &layout.schema_path,
            output_path: &layout.output_path,
            instructions: &instructions,
            prompt,
            image_paths: &layout.image_paths,
            model: request.model.as_deref(),
            reasoning_effort: request.reasoning_effort.as_deref(),
            access: request.access,
            supervised: request.supervised,
            extensions_dir: &layout.extensions_dir,
            session_id: session.as_deref(),
            tools: ctx.tools.as_ref(),
            shell_network_instructions: &env.prompts.shell_network_instructions,
        };
        let input = autonomous_input(&options);
        let mut command = runtime::command(&binary, &autonomous_argv(&options), Some(&layout.root), &process_env, &[]);
        command.stdin(Stdio::piped());
        let mut thread = None;
        let exit = process::run(command, &ctx.cancel, ctx.timeout, |stdin| {
            stdin.write(input);
            stdin.close();
        }, |line| {
            let Ok(event) = serde_json::from_str::<Value>(line) else { return };
            if let Some(id) = codex_events::thread_id(&event) {
                thread = Some(id);
            }
            if let Some(text) = codex_events::progress(&event) {
                ctx.events.progress(&text);
            }
            if let Some(step) = codex_events::trace_step(&event) {
                ctx.events.trace(step);
            }
        })
        .await;
        if let Some(thread) = &thread {
            workspace::write_session(&layout.session_path, thread).await;
        }
        if exit.success() {
            let output = tokio::fs::read_to_string(&layout.output_path).await.ok().and_then(|text| serde_json::from_str(&text).ok());
            return Attempt { exit, output, failure: None, retry_fresh: false };
        }
        let started = codex_events::turn_started(&exit.stdout);
        let detail = diagnostic(&exit);
        let raw = format!("{}\n{}", exit.stderr, exit.stdout);
        let resumable = session.is_some() && (unknown_session(&raw) || raw.to_lowercase().contains("already has an active writer"));
        let retry_fresh = !started && (resumable || detail.is_none());
        let failure = humanize_failure(detail.as_deref().unwrap_or_default(), "The autonomous agent failed.");
        Attempt { exit, output: None, failure: Some(failure), retry_fresh }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn tools() -> NativeTools {
        NativeTools { url: "http://127.0.0.1:1/mcp".into(), token: "secret-token".into(), names: vec![], timeout_sec: 3_600, outside_sandbox: true }
    }

    #[test]
    fn builds_a_resumed_sandboxed_run() {
        let tools = tools();
        let options = AutonomousArgv {
            schema_path: Path::new("s.json"),
            output_path: Path::new("o.json"),
            instructions: "INSTR",
            prompt: "--do it",
            image_paths: &[PathBuf::from("a.png")],
            model: Some("gpt-x"),
            reasoning_effort: Some("high"),
            access: Access::Project,
            supervised: true,
            extensions_dir: Path::new("ext"),
            session_id: Some("019a-abc"),
            tools: Some(&tools),
            shell_network_instructions: "NET",
        };
        let argv = autonomous_argv(&options);
        let joined = argv.join(" ");
        assert_eq!(&argv[..5], ["--search", "--add-dir", "ext", "exec", "resume"]);
        assert!(joined.contains("approval_policy=\"never\""));
        assert!(joined.contains("permissions.powermove.network.enabled=true"));
        assert!(joined.contains("mcp_servers.powermove.url=\"http://127.0.0.1:1/mcp\""));
        assert!(joined.contains("bearer_token_env_var=\"POWERMOVE_AGENT_TOOL_TOKEN\""));
        assert!(!joined.contains("secret-token"));
        let dash = argv.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(argv[dash + 1..], ["019a-abc", "-"]);
        assert!(!joined.contains("INSTR"));
        let last = autonomous_input(&options);
        assert!(last.starts_with("INSTR\n\nSHELL NETWORK\nNET\n\nAPPROVALS") && last.ends_with("USER REQUEST\n--do it"));
    }

    #[test]
    fn computer_runs_bypass_the_sandbox_and_drop_bad_sessions() {
        let argv = autonomous_argv(&AutonomousArgv {
            schema_path: Path::new("s"),
            output_path: Path::new("o"),
            instructions: "I",
            prompt: "p",
            image_paths: &[],
            model: None,
            reasoning_effort: None,
            access: Access::Computer,
            supervised: false,
            extensions_dir: Path::new("e"),
            session_id: Some("--evil"),
            tools: None,
            shell_network_instructions: "NET",
        });
        assert_eq!(argv[1], "--dangerously-bypass-approvals-and-sandbox");
        assert!(!argv.contains(&"resume".to_owned()) && !argv.iter().any(|arg| arg.contains("permissions.")));
    }

    #[test]
    fn humanizes_and_redacts_failures() {
        assert!(humanize_failure("Error: not logged in", "x").contains("not connected"));
        assert_eq!(humanize_failure("", "fallback"), "fallback");
        assert_eq!(redact("Authorization: Bearer abc123"), "Authorization: Bearer [redacted]");
        assert!(humanize_failure("2026-01-01T00:00:00Z ERROR codex: boom happened", "x").ends_with("boom happened"));
    }
}
