//! Claude Code runs (port of `claude/runner.ts` and `claude/adapter.ts`).
//!
//! The prompt goes to stdin as one stream-json user message, which keeps a
//! channel open for the CLI's stdio permission prompts: AskUserQuestion
//! becomes a question card, and in a Project run every other prompt becomes
//! an approval card (or is denied when nobody can be asked).
//!
//! Windows has no Claude Code sandbox, so Project runs never pre-approve a
//! shell tool: each Bash or PowerShell call goes to the permission prompt.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use regex::Regex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::approvals::{ApprovalRequest, Decision};
use super::autonomous::{self, Attempt, Approver, Env, NativeTools, Provider, RunContext, TOOL_TOKEN_ENV};
use super::claude_events::{Event, Parser};
use super::process::{self, Exit, Stdin};
use super::request::{Access, EFFORTS, RunResult};
use super::runtime;
use super::trace::clip;
use super::workspace::{self, Workspace};

const PROJECT_TOOLS: [&str; 8] = ["Read", "Glob", "Grep", "WebSearch", "WebFetch", "Skill", "Agent", "Task"];
const EDITOR_TOOLS: [&str; 6] = ["Read", "Glob", "Grep", "Skill", "Agent", "Task"];
const EDITOR_DISALLOWED_TOOLS: &str = "Bash,Monitor,PowerShell,Write,Edit,NotebookEdit";
const SANDBOX_OFF: &str = r#"{"sandbox":{"enabled":false}}"#;
const ASKING_INSTRUCTIONS: &str = "APPROVALS\nShell commands are not sandboxed on Windows, so Powermove asks the user before each Bash or PowerShell command. Prefer the Read, Write and Edit tools and the Powermove tools; run a command only when it is genuinely needed, with a clear description. If the user declines, do not retry it.";
const QUESTIONS: usize = 6;
const OPTIONS: usize = 8;
const STANDARD_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub struct ClaudeArgv<'a> {
    pub schema: &'a Value,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub access: Access,
    pub extensions_dir: Option<&'a std::path::Path>,
    /// Autonomous runs: `agentInstructions`; editor runs use the testing instructions.
    pub instructions: Option<&'a str>,
    pub testing_instructions: &'a str,
    pub shell_network_instructions: &'a str,
    pub tools: Option<&'a NativeTools>,
    /// Project access with someone to ask before a shell command runs.
    pub asking: bool,
}

/// `buildClaudeArgv`, for Windows.
pub fn claude_argv(options: &ClaudeArgv) -> Vec<String> {
    let mut servers = serde_json::Map::new();
    if let Some(tools) = options.tools {
        // Claude Code expands ${VAR} in MCP headers, so the token stays out of the argv.
        servers.insert("powermove".into(), json!({ "type": "http", "url": tools.url, "headers": { "Authorization": format!("Bearer ${{{TOOL_TOKEN_ENV}}}") } }));
    }
    let mcp_tools: Vec<String> = options.tools.map(|tools| tools.names.iter().map(|name| format!("mcp__powermove__{name}")).collect()).unwrap_or_default();
    let mut argv: Vec<String> = [
        "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--permission-prompt-tool", "stdio", "--include-partial-messages", "--verbose",
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect();
    argv.extend(["--mcp-config".into(), json!({ "mcpServers": servers }).to_string(), "--strict-mcp-config".into(), "--json-schema".into(), options.schema.to_string()]);
    if let Some(model) = options.model {
        argv.extend(["--model".into(), model.to_owned()]);
    }
    if let Some(effort) = options.effort {
        argv.extend(["--effort".into(), effort.to_owned()]);
    }
    if options.access == Access::Editor {
        argv.push("--no-session-persistence".into());
    }
    if let Some(session) = options.session_id.filter(|id| workspace::valid_session_id(id)) {
        argv.extend(["--resume".into(), session.to_owned()]);
    }
    if options.access == Access::Computer {
        argv.extend(["--dangerously-skip-permissions".into(), "--tools".into(), "default".into()]);
    } else {
        let base: &[&str] = if options.access == Access::Editor { &EDITOR_TOOLS } else { &PROJECT_TOOLS };
        let allowed: Vec<String> = base.iter().map(|tool| (*tool).to_owned()).chain(mcp_tools).collect();
        argv.extend([
            "--permission-mode".into(),
            if options.access == Access::Editor { "default".into() } else { "acceptEdits".into() },
            "--settings".into(),
            SANDBOX_OFF.into(),
            "--tools".into(),
            "default".into(),
            "--allowedTools".into(),
            allowed.join(","),
        ]);
        if options.access == Access::Editor {
            argv.extend(["--disallowedTools".into(), EDITOR_DISALLOWED_TOOLS.into()]);
        }
    }
    if let Some(directory) = options.extensions_dir {
        argv.extend(["--add-dir".into(), directory.display().to_string()]);
    }
    let network = if options.access == Access::Project {
        format!("\n\nSHELL NETWORK\n{}{}", options.shell_network_instructions, if options.asking { format!("\n\n{ASKING_INSTRUCTIONS}") } else { String::new() })
    } else {
        String::new()
    };
    let system = match options.instructions {
        Some(instructions) => format!("{instructions}{network}\n\nReturn the final answer only through the requested JSON schema."),
        None => format!("{}\n\nUse the supplied reference files as read-only context. Return only a value matching the requested JSON schema.", options.testing_instructions),
    };
    argv.extend(["--system-prompt".into(), system]);
    argv
}

/// Windows caps a command line at 32,767 characters. Move the values that can
/// grow (system prompt, MCP config) into files under `directory` and point
/// the CLI at them; everything else stays on the command line.
pub async fn spill_long_arguments(argv: Vec<String>, directory: &std::path::Path) -> std::io::Result<Vec<String>> {
    tokio::fs::create_dir_all(directory).await?;
    let mut argv = argv;
    for index in 0..argv.len().saturating_sub(1) {
        let (flag, name) = match argv[index].as_str() {
            "--system-prompt" => ("--system-prompt-file", "system-prompt.md"),
            "--mcp-config" => ("--mcp-config", "mcp-config.json"),
            _ => continue,
        };
        let path = directory.join(name);
        tokio::fs::write(&path, &argv[index + 1]).await?;
        argv[index] = flag.to_owned();
        argv[index + 1] = path.display().to_string();
    }
    Ok(argv)
}

/// `claudeUserMessage`: the one stream-json user message on stdin.
pub fn user_message(prompt: &str, image_paths: &[PathBuf]) -> String {
    let text = if image_paths.is_empty() {
        prompt.to_owned()
    } else {
        let list: Vec<String> = image_paths.iter().map(|path| format!("- {}", path.display())).collect();
        format!("{prompt}\n\nREFERENCE IMAGES\nInspect these files with the Read tool:\n{}", list.join("\n"))
    };
    format!("{}\n", json!({ "type": "user", "session_id": "", "parent_tool_use_id": null, "message": { "role": "user", "content": [{ "type": "text", "text": text }] } }))
}

/// `modelEffort('claude', …)`: the closest supported effort at or below the request.
pub fn effort(model: Option<&str>, requested: Option<&str>, discovered: Option<&Vec<String>>) -> Option<String> {
    let requested = requested?;
    let supported: Vec<String> = match discovered {
        Some(list) => list.clone(),
        None if model.is_some_and(|model| model.contains("haiku")) => vec![],
        None if matches!(model, Some("claude-opus-4-6" | "claude-sonnet-4-6")) => ["low", "medium", "high", "max"].map(str::to_owned).to_vec(),
        None => STANDARD_EFFORTS.map(str::to_owned).to_vec(),
    };
    if supported.is_empty() {
        return None;
    }
    if supported.iter().any(|item| item == requested) {
        return Some(requested.to_owned());
    }
    let rank = EFFORTS.iter().position(|item| *item == requested).unwrap_or(0);
    EFFORTS[..=rank].iter().rev().find(|item| supported.iter().any(|supported| supported == *item)).map(|item| (*item).to_owned()).or_else(|| supported.first().cloned())
}

/// A Claude permission prompt, as the person will read it.
fn describe_permission(request: &Value, input: &Value) -> ApprovalRequest {
    let tool = request["display_name"].as_str().or_else(|| request["tool_name"].as_str()).filter(|name| name.chars().count() <= 80).unwrap_or("a tool");
    if matches!(request["tool_name"].as_str(), Some("Bash" | "PowerShell")) {
        if let Some(command) = input["command"].as_str() {
            let reason = clip(input.get("description"), 600);
            return ApprovalRequest { title: "Run this command?".into(), detail: command.to_owned(), reason: (!reason.is_empty()).then_some(reason) };
        }
    }
    if let Some(file) = input["file_path"].as_str().or_else(|| input["notebook_path"].as_str()) {
        return ApprovalRequest { title: format!("Allow {tool} to change a file outside the project?"), detail: file.to_owned(), reason: None };
    }
    ApprovalRequest { title: format!("Allow {tool}?"), detail: serde_json::to_string_pretty(input).unwrap_or_default(), reason: None }
}

/// AskUserQuestion input, bounded for the renderer. Claude always accepts a typed "Other".
fn ask_user_questions(item_id: &str, input: &Value) -> (Vec<Value>, Vec<(String, String)>) {
    let mut texts = Vec::new();
    let mut questions = Vec::new();
    for (index, entry) in input["questions"].as_array().into_iter().flatten().take(QUESTIONS).enumerate() {
        let Some(raw) = entry["question"].as_str() else { continue };
        let question = clip(entry.get("question"), 1_000);
        if question.is_empty() {
            continue;
        }
        let id = format!("{item_id}-{index}");
        texts.push((id.clone(), raw.to_owned()));
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
        let mut card = json!({ "id": id, "header": clip(entry.get("header"), 80), "question": question, "options": options, "allowOther": true, "secret": false });
        if entry["multiSelect"] == true {
            card["multiSelect"] = json!(true);
        }
        questions.push(card);
    }
    (questions, texts)
}

/// An AskUserQuestion call held open on the CLI's permission prompt.
struct Held {
    control_id: String,
    input: Value,
    /// Question id → the question text Claude keys its answers by.
    texts: Vec<(String, String)>,
}

#[derive(Default)]
struct Live {
    stdin: Option<Stdin>,
    questions: HashMap<String, Held>,
    approvals: HashMap<String, CancellationToken>,
}

/// Live Claude processes by run id, so `codex:answer` can reach a held question.
#[derive(Default)]
pub struct ClaudeRuns {
    live: Mutex<HashMap<String, Arc<Mutex<Live>>>>,
}

fn control_response(stdin: &Stdin, request_id: &str, response: Value) {
    stdin.write(format!("{}\n", json!({ "type": "control_response", "response": { "subtype": "success", "request_id": request_id, "response": response } })));
}

impl ClaudeRuns {
    /// Settle a held AskUserQuestion; no chosen answer means skipped.
    pub fn answer(&self, run_id: &str, item_id: &str, answers: &Value) -> bool {
        let Some(live) = self.live.lock().get(run_id).cloned() else { return false };
        let mut live = live.lock();
        let Some(held) = live.questions.remove(item_id) else { return false };
        let mut chosen = serde_json::Map::new();
        for (id, text) in &held.texts {
            let picked: Vec<String> = answers[id].as_array().into_iter().flatten().filter_map(Value::as_str).map(|answer| answer.trim().to_owned()).filter(|answer| !answer.is_empty()).collect();
            if !picked.is_empty() {
                chosen.insert(text.clone(), json!(picked.join(", ")));
            }
        }
        if let Some(stdin) = &live.stdin {
            let response = if chosen.is_empty() {
                json!({ "behavior": "deny", "message": "The user skipped the question. Continue with your best judgement." })
            } else {
                let mut input = held.input.clone();
                input["answers"] = Value::Object(chosen);
                json!({ "behavior": "allow", "updatedInput": input })
            };
            control_response(stdin, &held.control_id, response);
        }
        true
    }

    /// The stdio permission prompt (`ClaudeRunner.control`).
    fn control(live: &Arc<Mutex<Live>>, event: &Value, ctx: &RunContext, approve: Option<&Approver>) {
        let Some(request_id) = event["request_id"].as_str().filter(|id| id.len() <= 200).map(str::to_owned) else { return };
        let mut state = live.lock();
        let Some(stdin) = state.stdin.clone() else { return };
        if event["type"] == "control_cancel_request" {
            if let Some(token) = state.approvals.remove(&request_id) {
                token.cancel();
            }
            let closed: Vec<String> = state.questions.iter().filter(|(_, held)| held.control_id == request_id).map(|(id, _)| id.clone()).collect();
            for item_id in closed {
                state.questions.remove(&item_id);
                ctx.events.trace(json!({ "kind": "question-closed", "itemId": item_id }));
            }
            return;
        }
        let request = &event["request"];
        if request["subtype"] != "can_use_tool" {
            stdin.write(format!("{}\n", json!({ "type": "control_response", "response": { "subtype": "error", "request_id": request_id, "error": "Unsupported by Powermove." } })));
            return;
        }
        let input = if request["input"].is_object() { request["input"].clone() } else { json!({}) };
        let item_id = request["tool_use_id"].as_str().filter(|id| id.chars().count() <= 120).map(str::to_owned);
        if let (Some("AskUserQuestion"), Some(item_id)) = (request["tool_name"].as_str(), item_id) {
            if !state.questions.contains_key(&item_id) {
                let (questions, texts) = ask_user_questions(&item_id, &input);
                if !questions.is_empty() {
                    state.questions.insert(item_id.clone(), Held { control_id: request_id, input, texts });
                    ctx.events.trace(json!({ "kind": "question", "itemId": item_id, "questions": questions, "transport": "reply", "blocking": true }));
                    return;
                }
            }
        }
        if let Some(approve) = approve.filter(|_| !state.approvals.contains_key(&request_id)) {
            let token = ctx.cancel.child_token();
            state.approvals.insert(request_id.clone(), token.clone());
            let asking = approve(describe_permission(request, &input));
            let live = live.clone();
            tokio::spawn(async move {
                let decision = tokio::select! {
                    decision = asking => decision,
                    () = token.cancelled() => return,
                };
                live.lock().approvals.remove(&request_id);
                control_response(&stdin, &request_id, match decision {
                    Decision::Allowed => json!({ "behavior": "allow", "updatedInput": input }),
                    Decision::Denied(message) => json!({ "behavior": "deny", "message": message }),
                });
            });
            return;
        }
        let tool = request["tool_name"].as_str().filter(|name| name.chars().count() <= 80).unwrap_or("this tool");
        control_response(&stdin, &request_id, json!({ "behavior": "deny", "message": format!("Powermove does not allow {tool} in this run.") }));
    }

    /// One CLI process to completion.
    async fn execute(&self, env: &Env, ctx: &RunContext, argv: Vec<String>, message: String, cwd: &std::path::Path, approve: Option<&Approver>) -> (Exit, Parser, Option<String>) {
        let mut parser = Parser::new(Some(cwd.to_path_buf()));
        let binary = match env.runtimes.claude() {
            Ok(binary) => binary,
            Err(error) => return (Exit { spawn_error: Some(error.to_string()), ..Exit::default() }, parser, None),
        };
        let home = match env.runtimes.claude_home().await {
            Ok(home) => home,
            Err(error) => return (Exit { spawn_error: Some(format!("{error:#}")), ..Exit::default() }, parser, None),
        };
        let mut process_env = vec![("CLAUDE_CONFIG_DIR", home.into_os_string())];
        if let Some(tools) = &ctx.tools {
            process_env.push((TOOL_TOKEN_ENV, tools.token.clone().into()));
        }
        // Long arguments live beside the run, outside the agent's workspace, and go with it.
        let spill = std::env::temp_dir().join(format!("powermove-claude-args-{}", uuid::Uuid::new_v4()));
        let argv = match spill_long_arguments(argv, &spill).await {
            Ok(argv) => argv,
            Err(error) => return (Exit { spawn_error: Some(format!("Could not prepare the Claude run: {error}")), ..Exit::default() }, parser, None),
        };
        let mut command = runtime::command(&binary, &argv, Some(cwd), &process_env, &["CLAUDE_CODE_SAFE_MODE"]);
        command.stdin(Stdio::piped());
        let live = Arc::new(Mutex::new(Live::default()));
        self.live.lock().insert(ctx.request.id.clone(), live.clone());
        let mut session = None;
        let exit = process::run(
            command,
            &ctx.cancel,
            ctx.timeout,
            |stdin| {
                stdin.write(message);
                live.lock().stdin = Some(stdin);
            },
            |line| {
                for event in parser.line(line) {
                    match event {
                        Event::Progress(text) => ctx.events.progress(&text),
                        Event::Trace(step) => ctx.events.trace(step),
                        Event::Session(id) => session = Some(id),
                        Event::Control(event) => Self::control(&live, &event, ctx, approve),
                        // The CLI reads more turns until stdin closes; one prompt is one run.
                        Event::Result => {
                            if let Some(stdin) = &live.lock().stdin {
                                stdin.close();
                            }
                        }
                    }
                }
            },
        )
        .await;
        let _ = tokio::fs::remove_dir_all(&spill).await;
        // Held questions and approvals die with the process; the renderer closes their cards.
        self.live.lock().remove(&ctx.request.id);
        for token in live.lock().approvals.values() {
            token.cancel();
        }
        (exit, parser, session)
    }

    /// An editor-mode run: read-only, with the renderer's schema.
    pub async fn run_editor(&self, env: &Env, ctx: &RunContext, discovered: Option<&Vec<String>>) -> RunResult {
        let request = &ctx.request;
        let directory = match tempfile_dir("powermove-claude-").await {
            Ok(directory) => directory,
            Err(error) => return RunResult::failed(format!("Could not prepare the Claude run: {error:#}")),
        };
        let mut images = Vec::new();
        for (index, image) in request.images.iter().enumerate() {
            let path = directory.join(format!("reference-{index}.{}", workspace::image_extension(image)));
            if tokio::fs::write(&path, image).await.is_ok() {
                images.push(path);
            }
        }
        let schema = request.schema.clone().unwrap_or_else(|| json!({ "type": "object" }));
        let effort = effort(request.model.as_deref(), request.reasoning_effort.as_deref(), discovered);
        let argv = claude_argv(&ClaudeArgv {
            schema: &schema,
            model: request.model.as_deref(),
            effort: effort.as_deref(),
            session_id: None,
            access: Access::Editor,
            extensions_dir: None,
            instructions: None,
            testing_instructions: &env.prompts.testing_instructions,
            shell_network_instructions: &env.prompts.shell_network_instructions,
            tools: ctx.tools.as_ref(),
            asking: false,
        });
        let (exit, parser, _) = self.execute(env, ctx, argv, user_message(&request.prompt, &images), &directory, None).await;
        let _ = tokio::fs::remove_dir_all(&directory).await;
        if ctx.cancel.is_cancelled() {
            return RunResult::cancelled("The Claude run was cancelled.");
        }
        if exit.timed_out {
            return RunResult::failed("The coding agent took too long to respond. Try sending your request again.");
        }
        if !exit.success() || parser.error.is_some() {
            return RunResult::failed(humanize_failure(&exit, &parser, "Claude generation failed."));
        }
        match parser.output.clone().or_else(|| serde_json::from_str(&parser.text).ok()) {
            Some(output) => RunResult::done(output.to_string(), "editor"),
            None => RunResult::failed("Claude completed without a structured result."),
        }
    }
}

async fn tempfile_dir(prefix: &str) -> std::io::Result<PathBuf> {
    let directory = std::env::temp_dir().join(format!("{prefix}{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await?;
    Ok(directory)
}

fn humanize_failure(exit: &Exit, parser: &Parser, fallback: &str) -> String {
    let detail = parser.error.clone().filter(|text| !text.is_empty()).or_else(|| Some(exit.stderr.trim().to_owned()).filter(|text| !text.is_empty())).or_else(|| exit.spawn_error.clone()).unwrap_or_default();
    let detail = detail.trim();
    if detail.is_empty() {
        return fallback.to_owned();
    }
    tracing::error!("[claude] run failed: {}", super::codex_exec::redact(&super::trace::truncate(detail, 4_000)));
    let matches = |pattern: &str| Regex::new(pattern).expect("static pattern").is_match(detail);
    if matches(r"(?i)not logged in|login required|authentication|oauth|unauthorized|\b401\b") {
        return "Claude is not connected to Powermove. Connect your Claude subscription and retry.".into();
    }
    if matches(r"(?i)ENOENT|command not found|No such file|program not found|cannot find the file") {
        return super::runtime::CLAUDE_NOT_FOUND.into();
    }
    if matches(r"(?i)rate.?limit|\b429\b|overloaded") {
        return "Claude is rate-limited right now. Wait a moment and retry.".into();
    }
    let last = detail.lines().filter(|line| !line.trim().is_empty()).last().unwrap_or(fallback);
    format!("Claude failed: {}", super::codex_exec::redact(&super::trace::truncate(last, 240)))
}

fn unknown_session(text: &str) -> bool {
    static UNKNOWN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:unknown|invalid|missing|not found|does not exist|could not find).{0,80}(?:session|conversation)|(?:session|conversation).{0,80}(?:unknown|invalid|missing|not found|does not exist|could not find)").unwrap()
    });
    UNKNOWN.is_match(text)
}

/// Autonomous Claude runs through the shared workspace pipeline.
pub struct ClaudeProvider<'a> {
    pub runs: &'a ClaudeRuns,
    pub discovered: Option<Vec<String>>,
}

impl Provider for ClaudeProvider<'_> {
    fn name(&self) -> &'static str {
        "Claude"
    }

    async fn attempt(&self, env: &Env, ctx: &RunContext, layout: &Workspace, prompt: &str, session: Option<String>) -> Attempt {
        let request = &ctx.request;
        // Only Project runs ask; Full access never prompts.
        let approve = ctx.approve.as_ref().filter(|_| request.access == Access::Project);
        let instructions = autonomous::instructions(env, ctx, layout, prompt);
        let effort = effort(request.model.as_deref(), request.reasoning_effort.as_deref(), self.discovered.as_ref());
        let argv = claude_argv(&ClaudeArgv {
            schema: &env.prompts.result_schema,
            model: request.model.as_deref(),
            effort: effort.as_deref(),
            session_id: session.as_deref(),
            access: request.access,
            extensions_dir: Some(&layout.extensions_dir),
            instructions: Some(&instructions),
            testing_instructions: &env.prompts.testing_instructions,
            shell_network_instructions: &env.prompts.shell_network_instructions,
            tools: ctx.tools.as_ref(),
            asking: approve.is_some(),
        });
        let (exit, parser, captured) = self.runs.execute(env, ctx, argv, user_message(prompt, &layout.image_paths), &layout.root, approve).await;
        if let Some(id) = &captured {
            workspace::write_session(&layout.session_path, id).await;
        }
        if exit.success() && parser.error.is_none() {
            let output = parser.output.clone().or_else(|| serde_json::from_str(&parser.text).ok());
            return Attempt { exit, output, failure: None, retry_fresh: false };
        }
        let diagnostic = format!("{}\n{}", exit.stderr, parser.error.clone().unwrap_or_default());
        let retry_fresh = session.is_some() && unknown_session(&diagnostic);
        let failure = humanize_failure(&exit, &parser, "The Claude agent failed.");
        Attempt { exit, output: None, failure: Some(failure), retry_fresh }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn long_prompts_leave_the_command_line() {
        let directory = tempfile::tempdir().unwrap();
        let system = "x".repeat(40_000);
        let argv = vec!["--print".into(), "--mcp-config".into(), "{}".into(), "--system-prompt".into(), system.clone()];
        let argv = spill_long_arguments(argv, directory.path()).await.unwrap();
        assert_eq!(argv[3], "--system-prompt-file");
        assert!(argv.iter().map(String::len).sum::<usize>() < 1_000);
        assert_eq!(std::fs::read_to_string(&argv[4]).unwrap(), system);
        assert_eq!(std::fs::read_to_string(&argv[2]).unwrap(), "{}");
    }

    #[test]
    fn project_runs_keep_shells_behind_the_prompt() {
        let tools = NativeTools { url: "http://127.0.0.1:9/mcp".into(), token: "secret".into(), names: vec!["get_project_state".into()], timeout_sec: 120, outside_sandbox: false };
        let schema = json!({ "type": "object" });
        let argv = claude_argv(&ClaudeArgv {
            schema: &schema,
            model: Some("opus"),
            effort: Some("high"),
            session_id: Some("abc-1"),
            access: Access::Project,
            extensions_dir: Some(std::path::Path::new("ext")),
            instructions: Some("INSTR"),
            testing_instructions: "T",
            shell_network_instructions: "NET",
            tools: Some(&tools),
            asking: true,
        });
        let allowed = &argv[argv.iter().position(|arg| arg == "--allowedTools").unwrap() + 1];
        assert!(!allowed.split(',').any(|tool| tool == "Bash" || tool == "PowerShell"));
        assert!(allowed.contains("mcp__powermove__get_project_state"));
        let config = &argv[argv.iter().position(|arg| arg == "--mcp-config").unwrap() + 1];
        assert!(config.contains("${POWERMOVE_AGENT_TOOL_TOKEN}") && !config.contains("secret"));
        assert!(argv.windows(2).any(|pair| pair == ["--resume", "abc-1"]));
        assert!(argv.last().unwrap().contains("APPROVALS"));
    }

    #[test]
    fn picks_supported_efforts() {
        assert_eq!(effort(Some("opus"), Some("ultra"), None).as_deref(), Some("max"));
        assert_eq!(effort(Some("claude-haiku-4-5"), Some("high"), None), None);
        assert_eq!(effort(Some("x"), Some("xhigh"), Some(&vec!["low".into(), "high".into()])).as_deref(), Some("high"));
    }

    #[test]
    fn answers_reach_the_held_question() {
        let runs = ClaudeRuns::default();
        let live = Arc::new(Mutex::new(Live::default()));
        let (sender, mut inbox) = tokio::sync::mpsc::unbounded_channel();
        live.lock().stdin = Some(Stdin::from_sender(sender));
        live.lock().questions.insert("q1".into(), Held { control_id: "c1".into(), input: json!({ "questions": [] }), texts: vec![("q1-0".into(), "Which layout?".into())] });
        runs.live.lock().insert("run-1".into(), live);
        assert!(!runs.answer("run-2", "q1", &json!({})));
        assert!(runs.answer("run-1", "q1", &json!({ "q1-0": ["Grid"] })));
        let written: Value = serde_json::from_str(inbox.try_recv().unwrap().unwrap().trim()).unwrap();
        assert_eq!(written["response"]["request_id"], "c1");
        assert_eq!(written["response"]["response"]["updatedInput"]["answers"]["Which layout?"], "Grid");
    }
}
