//! The host half of subagents (the `executeProvider` / `threadControl` part
//! of `codex/index.ts`): what the orchestrator needs from `Agents`, the
//! `codex:cancel-task` channel and child artifact handling. A child module of
//! `service`, so it shares `Agents`' private state.

use std::path::Path;

use futures::future::BoxFuture;

use super::super::autonomous::NativeTools;
use super::super::orchestrator::{Backend, ExecContext, Orchestrator};
use super::super::orchestrator_providers::{self, ProviderInputs};
use super::*;

/// The orchestrator, journaling under `<userData>/Agent Tasks`.
pub(super) fn orchestrator(user_data: &Path, agents: Weak<Agents>) -> Arc<Orchestrator> {
    Orchestrator::new(user_data.join("Agent Tasks"), Arc::new(HostBackend(agents)))
}

/// Route the subagent tools of every run to the orchestrator.
pub(super) fn connect(agents: &Arc<Agents>) {
    let Some(bridge) = &agents.bridge else { return };
    let orchestrator = Arc::downgrade(&agents.orchestrator);
    bridge.set_orchestrator(Arc::new(move |run_id, tool, args| {
        let orchestrator = orchestrator.clone();
        async move {
            let Some(orchestrator) = orchestrator.upgrade() else { bail!("This agent run is no longer active.") };
            orchestrator.call(&run_id, &tool, args).await
        }
        .boxed()
    }));
}

/// The subagent protocol (and a child's role) for a run that can delegate.
pub(super) fn instructions(env: &Env, tools: Option<&NativeTools>, role: Option<&str>) -> Option<String> {
    let delegates = tools.is_some_and(|tools| tools.names.iter().any(|name| name == "delegate_task"));
    if !delegates || env.prompts.orchestration_instructions.is_empty() {
        return None;
    }
    let role = role.map(|role| format!("\n\nTask role: {role}")).unwrap_or_default();
    Some(format!("{}{role}", env.prompts.orchestration_instructions))
}

/// `copyChildArtifacts`: a child's deliverables become readable through the
/// parent project's artifact API. Links are never copied.
pub(super) async fn copy_child_artifacts(user_data: &Path, workspace_id: &str, project_id: &str, result: &str) -> Result<()> {
    if workspace_id == project_id {
        return Ok(());
    }
    let Ok(parsed) = serde_json::from_str::<Value>(result) else { return Ok(()) };
    let source = workspace::workspace_root(user_data, workspace_id).join("artifacts");
    let destination = workspace::workspace_root(user_data, project_id).join("artifacts");
    for relative in parsed["artifacts"].as_array().into_iter().flatten().filter_map(|item| item["path"].as_str()) {
        let file = workspace::validated_artifact_path(&source, relative)?;
        let target = destination.join(relative.split('/').collect::<std::path::PathBuf>());
        let parent = target.parent().unwrap_or(&destination);
        tokio::fs::create_dir_all(parent).await?;
        // The validated path is a regular file inside the child's artifacts; refuse to follow a link at the target.
        if dunce::canonicalize(parent)?.strip_prefix(dunce::canonicalize(&destination)?).is_err() {
            bail!("Subagent artifacts cannot be copied through a link.");
        }
        if tokio::fs::symlink_metadata(&target).await.is_ok() {
            continue;
        }
        tokio::fs::copy(&file, &target).await?;
    }
    Ok(())
}

impl Agents {
    /// Remove a project's workspace with its subagent journals and workspaces.
    pub(super) async fn forget_project(&self, project: &str) {
        match self.orchestrator.forget_project(project).await {
            Ok(children) => {
                for child in children {
                    workspace::forget(&self.user_data, &child).await;
                }
            }
            Err(error) => tracing::warn!("[agent-workspaces] {error:#}"),
        }
        workspace::forget(&self.user_data, project).await;
    }

    /// `codex:cancel-task`: the person stops one task of their conversation.
    pub(super) async fn cancel_task(self: Arc<Self>, call: Call) -> Result<Value> {
        let value = call.arg(0);
        let request = value["requestId"].as_str().filter(|id| is_request_id(id));
        let task = value["taskId"].as_str().filter(|id| !id.is_empty() && id.chars().count() <= 240);
        let (Some(request), Some(task)) = (request, task) else { bail!("codex:cancel-task: invalid task request") };
        if self.owned(request, call.client.id).is_none() {
            bail!("This task belongs to another conversation.");
        }
        self.orchestrator.cancel_task(request, task).await?;
        Ok(Value::Null)
    }
}

struct HostBackend(Weak<Agents>);

impl Backend for HostBackend {
    fn providers(&self) -> BoxFuture<'static, Vec<Value>> {
        let agents = self.0.clone();
        async move {
            let Some(agents) = agents.upgrade() else { return Vec::new() };
            let (chatgpt, claude, compatible, chatgpt_models, claude_models) = tokio::join!(agents.chatgpt.status(), agents.claude.status(), agents.compatible.status(), agents.chatgpt.models(), agents.claude.models());
            orchestrator_providers::providers(&ProviderInputs {
                chatgpt_connected: chatgpt["state"] == "connected",
                chatgpt_models: chatgpt_models.unwrap_or_default(),
                claude_connected: claude["state"] == "connected",
                claude_models: claude_models.unwrap_or_default(),
                compatible: compatible.ok(),
            })
        }
        .boxed()
    }

    fn execute(&self, request: RunRequest, ctx: ExecContext) -> BoxFuture<'static, RunResult> {
        let agents = self.0.clone();
        async move {
            match agents.upgrade() {
                Some(agents) => agents.execute(request, ctx).await,
                None => RunResult::cancelled("Powermove closed before the run finished."),
            }
        }
        .boxed()
    }

    fn steer(&self, run_id: &str, prompt: &str) -> BoxFuture<'static, bool> {
        let agents = self.0.clone();
        let (run_id, prompt) = (run_id.to_owned(), prompt.to_owned());
        async move {
            match agents.upgrade() {
                Some(agents) => agents.codex_app.steer(&run_id, prompt, Vec::new()).await,
                None => false,
            }
        }
        .boxed()
    }

    fn answer(&self, run_id: &str, item_id: &str, answers: &Value) -> bool {
        self.0.upgrade().is_some_and(|agents| agents.approvals.answer(run_id, item_id, answers) || agents.codex_app.answer(run_id, item_id, answers) || agents.claude_runs.answer(run_id, item_id, answers))
    }

    fn thread_control(&self, root_id: &str, tool: &str, mut args: Value) -> BoxFuture<'static, Result<Value>> {
        let agents = self.0.clone();
        let root_id = root_id.to_owned();
        args["tool"] = json!(tool);
        async move {
            let agents = agents.upgrade().ok_or_else(|| anyhow::anyhow!("Thread controls are unavailable in this connection."))?;
            let bridge = agents.bridge.clone().ok_or_else(|| anyhow::anyhow!("Thread controls are unavailable in this connection."))?;
            let session = bridge.session_for_run(&root_id).ok_or_else(|| anyhow::anyhow!("Thread controls are unavailable in this connection."))?;
            let response = bridge.call_renderer(&session, "__agent_thread_control", args).await?;
            if !response.ok {
                bail!("{}", response.error.unwrap_or_else(|| "Thread control failed.".into()));
            }
            let text = response.content.iter().find_map(|item| if let super::super::tools::Content::Text(text) = item { Some(text.clone()) } else { None });
            let text = text.ok_or_else(|| anyhow::anyhow!("Thread control returned no result."))?;
            Ok(serde_json::from_str(&text)?)
        }
        .boxed()
    }

    fn has_thread_control(&self) -> bool {
        self.0.upgrade().is_some_and(|agents| agents.bridge.is_some())
    }
}
