//! The autonomous run shared by every CLI provider (the common half of
//! `codex/runner.ts` and `claude/runner.ts`): prepare the workspace, resume
//! or start the CLI session, then check the completion report and ask the
//! agent to correct it before anything is published, and publish the
//! extension changes it reports (`changes`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::RunEvents;
use super::approvals::{ApprovalRequest, Decision};
use super::changes;
use super::process::Exit;
use super::prompts::{self, InstructionInputs, Prompts};
use super::request::{RunRequest, RunResult};
use super::runtime::Runtimes;
use super::workspace::{self, Inputs, Prepare, ValidationError, Workspace};
use crate::extensions::compiler::Compiler;

const REPAIRS: usize = 2;

/// Host-wide paths and services every run uses.
pub struct Env {
    pub user_data: PathBuf,
    pub resources_dir: PathBuf,
    pub live_extensions: PathBuf,
    pub runtimes: Runtimes,
    pub prompts: Arc<Prompts>,
}

/// Asks the person to approve an action the sandbox would block.
pub type Approver = Arc<dyn Fn(ApprovalRequest) -> BoxFuture<'static, Decision> + Send + Sync>;

/// How a CLI reaches this run's Powermove tools.
#[derive(Debug, Clone)]
pub struct NativeTools {
    pub url: String,
    pub token: String,
    /// Tool names the run may call (already filtered for the run).
    pub names: Vec<String>,
    pub timeout_sec: u64,
    pub outside_sandbox: bool,
}

pub const TOOL_TOKEN_ENV: &str = "POWERMOVE_AGENT_TOOL_TOKEN";

/// Everything one provider run needs.
pub struct RunContext {
    pub request: RunRequest,
    pub events: RunEvents,
    pub cancel: CancellationToken,
    pub tools: Option<NativeTools>,
    /// Supervised Project runs ask before leaving the sandbox.
    pub approve: Option<Approver>,
    pub timeout: Duration,
    /// Appended to the agent instructions (the subagent protocol and task role).
    pub extra_instructions: Option<String>,
}

/// One CLI attempt, as the pipeline sees it.
pub struct Attempt {
    pub exit: Exit,
    /// The structured completion report, when the CLI produced one.
    pub output: Option<Value>,
    /// A person-readable failure; `None` when the attempt succeeded.
    pub failure: Option<String>,
    /// The CLI failed before the turn began, so a fresh session is safe.
    pub retry_fresh: bool,
}

pub trait Provider: Sync {
    /// "Codex" or "Claude", for cancellation copy.
    fn name(&self) -> &'static str;
    fn attempt(&self, env: &Env, ctx: &RunContext, layout: &Workspace, prompt: &str, session: Option<String>) -> impl std::future::Future<Output = Attempt> + Send;
}

fn cancelled(provider: &impl Provider, exit: Option<&Exit>) -> RunResult {
    if exit.is_some_and(|exit| exit.timed_out) {
        return RunResult::failed("The coding agent took too long to respond. Your partial work was kept; retry to continue.");
    }
    RunResult::cancelled(format!("The {} run was cancelled.", provider.name()))
}

/// The run's instructions (`agentInstructions`) plus the provider's extras.
pub fn instructions(env: &Env, ctx: &RunContext, layout: &Workspace, prompt: &str) -> String {
    let instructions = env.prompts.agent_instructions(&InstructionInputs {
        project_name: &ctx.request.project_name,
        artifact_path: &format!("artifacts/{}", layout.run_id),
        access: ctx.request.access.authority(),
        extensions_dir: &layout.extensions_dir,
        app_context: ctx.request.app_context,
        request: prompt,
    });
    match &ctx.extra_instructions {
        Some(extra) => format!("{instructions}\n\n{extra}"),
        None => instructions,
    }
}

fn repair_prompt(message: &str) -> String {
    format!(
        "Correct the completion report for the current run. Validation failed before publishing extensions or importing artifacts:\n{message}\n\nContinue in the same staging and artifact directories. Inspect the existing files and return a complete corrected structured result. Preserve the previous commands and artifact import requests unless they caused this validation error; those returned commands and imports have not been applied yet. Do not repeat live project edits, completed shell operations, uploads, or other side effects. Read get_project_state before any new project edit. Report only extension changes made in this stage, with their actual actions; omit unchanged extensions. Do not claim that an unsuccessful operation completed."
    )
}

/// Check the report and attach what the host owns. A `ValidationError` asks the agent for a corrected report.
fn finalize(request: &RunRequest, layout: &Workspace, output: Option<Value>) -> Result<Map<String, Value>, ValidationError> {
    let Some(Value::Object(mut parsed)) = output else {
        return Err(ValidationError("The autonomous agent returned an invalid result. Return a JSON object matching the result schema.".into()));
    };
    let commands = parsed.get("commands").and_then(Value::as_array).is_some_and(|commands| !commands.is_empty());
    let requested: Vec<Value> = parsed.get("artifacts").and_then(Value::as_array).cloned().unwrap_or_default();
    if request.app_context && (commands || requested.iter().any(|item| item["importToTimeline"] == true)) {
        return Err(ValidationError("No project is attached. Return commands: [] and do not import artifacts to a timeline.".into()));
    }
    parsed.insert("artifacts".into(), Value::Array(workspace::collect_artifacts(&layout.run_directory, &layout.run_id, &requested)?));
    parsed.insert("projectId".into(), json!(request.project_id));
    parsed.insert("access".into(), json!(request.access.authority()));
    Ok(parsed)
}

/// Finalize the report, then publish the extensions it reports: validate
/// and compile a snapshot of the stage, promote it with an undo record, and
/// reload the promoted mods. A `ValidationError` inside asks for a repair.
async fn complete(env: &Env, ctx: &RunContext, layout: &Workspace, output: Option<Value>) -> anyhow::Result<RunResult> {
    let parsed = finalize(&ctx.request, layout, output)?;
    let declared = changes::parse_changes(parsed.get("extensions").unwrap_or(&Value::Null));
    let stage = changes::Stage {
        live: env.live_extensions.clone(),
        staging: layout.extensions_dir.clone(),
        history: changes::history_root(&env.user_data, &ctx.request.project_id),
        project_id: ctx.request.project_id.clone(),
        run_id: layout.run_id.clone(),
        baseline: layout.baseline.clone(),
    };
    let (list, cancel, compiler_dir) = (declared.clone().unwrap_or_default(), ctx.cancel.clone(), env.resources_dir.join("compiler"));
    let record = tokio::task::spawn_blocking(move || changes::promote(&stage, &list, &Compiler::new(compiler_dir), || cancel.is_cancelled())).await??;
    workspace::discard_stage(layout).await;
    if let Some(declared) = declared.as_ref().filter(|declared| !declared.is_empty()) {
        changes::refresh(&env.user_data, declared.iter().map(|change| change.id.clone()).collect()).await;
    }
    Ok(RunResult::Done {
        text: Value::Object(parsed).to_string(),
        access: ctx.request.access.authority(),
        live_edits: false,
        live_history_id: None,
        extensions: declared.map(|declared| json!(declared)),
        extension_change_set_id: record.map(|record| record.id),
    })
}

pub async fn run(provider: &impl Provider, env: &Env, ctx: &RunContext) -> RunResult {
    let request = &ctx.request;
    let api_pack = prompts::api_pack_files(&env.resources_dir);
    let workspace_id = request.workspace_id();
    // A publish this project interrupted is rolled back before the live folder is copied.
    let (history, live) = (changes::history_root(&env.user_data, &request.project_id), env.live_extensions.clone());
    if let Err(error) = tokio::task::spawn_blocking(move || changes::recover(&history, &live)).await.map_err(anyhow::Error::from).and_then(|recovered| recovered) {
        return RunResult::failed(format!("Could not prepare the agent workspace: {error:#}"));
    }
    let prepared = workspace::prepare(
        &Prepare {
            user_data: &env.user_data,
            workspace_id: &workspace_id,
            authority: request.access.authority(),
            thread_id: request.thread_id.as_deref(),
            provider: if provider.name() == "Claude" { "claude" } else { "chatgpt" },
            schema: &env.prompts.result_schema,
            live_extensions: &env.live_extensions,
            api_pack: &api_pack,
        },
        &Inputs { project_json: request.project_json.as_deref().unwrap_or("{}"), images: &request.images, attachments: &request.attachments },
    )
    .await;
    let layout = match prepared {
        Ok(layout) => layout,
        Err(error) => return RunResult::failed(format!("Could not prepare the agent workspace: {error:#}")),
    };
    let _stage = changes::track_stage(&request.id, &env.user_data, &layout.extensions_dir, &env.live_extensions);
    let result = run_in(provider, env, ctx, &layout).await;
    workspace::remove_empty_run_directory(&layout).await;
    result
}

async fn run_in(provider: &impl Provider, env: &Env, ctx: &RunContext, layout: &Workspace) -> RunResult {
    if ctx.cancel.is_cancelled() {
        return cancelled(provider, None);
    }
    let mut session = workspace::read_session(&layout.session_path).await;
    let mut attempt = provider.attempt(env, ctx, layout, &ctx.request.prompt, session.clone()).await;
    if attempt.failure.is_some() && attempt.retry_fresh && !ctx.cancel.is_cancelled() && !attempt.exit.timed_out {
        ctx.events.progress(if session.is_some() { "The saved agent thread could not be resumed — starting a fresh run…" } else { "The agent stopped before it could start — retrying…" });
        workspace::clear_session(&layout.session_path).await;
        session = None;
        attempt = provider.attempt(env, ctx, layout, &ctx.request.prompt, session).await;
    }
    if ctx.cancel.is_cancelled() || attempt.exit.timed_out {
        return cancelled(provider, Some(&attempt.exit));
    }
    if let Some(failure) = attempt.failure {
        workspace::clear_session(&layout.session_path).await;
        return RunResult::failed(failure);
    }
    let mut output = attempt.output;
    for repair in 0..=REPAIRS {
        let error = match complete(env, ctx, layout, output.take()).await {
            Ok(result) => return result,
            Err(_) if ctx.cancel.is_cancelled() => return cancelled(provider, None),
            Err(error) => error,
        };
        // The stage stays on disk when the run fails, so its files are not lost.
        let Some(ValidationError(message)) = error.downcast_ref::<ValidationError>() else { return RunResult::failed(format!("{error:#}")) };
        if repair == REPAIRS {
            return RunResult::failed(message.clone());
        }
        ctx.events.progress(&format!("Correcting the agent result ({} of {REPAIRS})…", repair + 1));
        let session = workspace::read_session(&layout.session_path).await;
        let repaired = provider.attempt(env, ctx, layout, &repair_prompt(message), session).await;
        if ctx.cancel.is_cancelled() || repaired.exit.timed_out {
            return cancelled(provider, Some(&repaired.exit));
        }
        if let Some(failure) = repaired.failure {
            return RunResult::failed(failure);
        }
        output = repaired.output;
    }
    unreachable!("the last repair returns")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_attaches_host_fields() {
        let directory = tempfile::tempdir().unwrap();
        let run_directory = directory.path().join("run-1");
        std::fs::create_dir_all(&run_directory).unwrap();
        let layout = Workspace {
            root: directory.path().into(),
            run_id: "run-1".into(),
            run_directory,
            extensions_dir: directory.path().join("stage"),
            baseline: Default::default(),
            schema_path: directory.path().join("schema.json"),
            output_path: directory.path().join("out.json"),
            session_path: directory.path().join("session.txt"),
            image_paths: vec![],
        };
        let request = RunRequest {
            id: "run-abcdefgh".into(),
            provider: super::super::request::Provider::ChatGpt,
            thread_id: None,
            mode: super::super::request::Mode::Autonomous,
            prompt: "x".into(),
            schema: None,
            images: vec![],
            model: None,
            reasoning_effort: None,
            access: super::super::request::Access::Project,
            project_id: "p1".into(),
            project_name: "P".into(),
            project_json: Some("{}".into()),
            app_context: false,
            attachments: vec![],
            consent_token: None,
            supervised: false,
        };
        let parsed = finalize(&request, &layout, Some(json!({ "summary": "s", "extensions": [{ "id": "glow", "action": "created" }] }))).unwrap();
        assert_eq!(parsed["extensions"], json!([{ "id": "glow", "action": "created" }]));
        assert_eq!(parsed["projectId"], "p1");
        assert_eq!(parsed["access"], "project");
        assert_eq!(parsed["artifacts"], json!([]));
        assert!(finalize(&request, &layout, Some(json!("text"))).is_err());
    }
}
