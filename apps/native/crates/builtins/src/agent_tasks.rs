//! Main's agent orchestrator (`agent-orchestrator.ts`) over the panel's
//! provider runs: subagent tasks (delegate_task, task_status, task_send,
//! task_cancel), run watches (thread_watch, thread_unwatch), and resuming the
//! calling run with the results once its turn ends (`pm_agents::orchestration`).
//! Task archives are main's `Agent Tasks/<project>/<conversation>.json`.
//! Everything here runs on workers: MCP calls and the `pm-agent-runs` loop.
use crate::LockShared;
use crate::agent_host::{AgentLink, AgentState, SharedAgent, now_ms};
use pm_agents::{
    Access, Event, Provider, RunResult,
    launch::{self, Request},
    monitor::{Monitor, Run as ThreadRun, Status, Watch},
    orchestration::{self, Barrier, Command, Delegate, Ids, Scope, Selection},
    trace::truncate,
    transport::{Action, Handle},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_TASKS: u32 = 32;
const MAX_DEPTH: u32 = 4;
const MAX_ACTIVE: u32 = 8;
const MAX_WATCHES: usize = 32;
/// A child's progress reaches its task row at most this often.
const PROGRESS_MS: u64 = 1_000;

/// One conversation's tasks while its root run (or its children) live.
struct Root {
    scope: Scope,
    project: String,
    thread: String,
    /// The root's finished turn, waiting on its tasks and watches.
    ended: Option<String>,
    /// The root's run settled; the scope goes once its tasks have.
    settled: bool,
    /// Stopped or failed: released once its tasks have stopped (main's
    /// `drainChildren`), so their final rows reach the conversation.
    draining: bool,
    waiting_note: Option<String>,
}
struct Child {
    run: String,
    root: String,
    request: Request,
    handle: Option<Handle>,
    /// A finished turn waiting on this child's own tasks and watches.
    ended: Option<String>,
    /// task_send's interrupted-and-resumed next turn.
    next: Option<Request>,
    stopping: bool,
    finished: bool,
    progressed: u64,
    /// Child question item id → the id shown in the parent's conversation.
    questions: BTreeMap<String, String>,
    /// A result whose save failed, kept and retried before the child retires.
    result: Option<RunResult>,
}
struct Lease {
    owner: String,
    thread: String,
    run: String,
    watch: Watch,
    acknowledged: bool,
}
/// A child's held question, by the id shown in the parent's conversation.
struct Question {
    child: String,
    item: String,
    blocking: bool,
    questions: Value,
}
#[derive(Default)]
pub(crate) struct Tasks {
    roots: BTreeMap<String, Root>,
    children: Vec<Child>,
    /// Panel runs' launch requests: a continuation turn reuses them.
    launched: BTreeMap<String, Request>,
    monitor: Monitor,
    leases: Vec<Lease>,
    questions: BTreeMap<String, Question>,
    dirty: bool,
    /// What children and continuations launch with (the panel's).
    environment: Option<launch::Environment>,
    /// Handed-over deliveries whose outbox acknowledgement failed: (root,
    /// run, delivery, leases), retried until saved.
    acks: Vec<(String, String, String, Vec<Watch>)>,
    retry_at: u64,
}

/// `Agent Tasks` files under the profile, written atomically, compared first.
struct Disk<'a> {
    profile: &'a Path,
    link: &'a AgentLink,
}
impl orchestration::Host for Disk<'_> {
    fn load(&mut self, path: &Path) -> io::Result<Option<Value>> {
        match std::fs::read(self.profile.join(path)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                io::Error::other(
                    "Saved subagent tasks could not be read. Their file has been preserved.",
                )
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn save(&mut self, path: &Path, expected: Option<&Value>, archive: &Value) -> io::Result<()> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if self.load(path)?.as_ref() != expected {
            return Err(io::Error::other(
                "Saved subagent tasks changed outside this conversation.",
            ));
        }
        let file = self.profile.join(path);
        let parent = file.parent().ok_or_else(|| io::Error::other("task path"))?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        let staged = file.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&staged)?;
            out.write_all(archive.to_string().as_bytes())?;
            out.sync_all()?;
        }
        std::fs::rename(staged, file)
    }
    fn select(&mut self, parent: &Value, target: Option<&Value>) -> io::Result<Selection> {
        select(self.link, parent, target).map_err(io::Error::other)
    }
}
fn provider(name: &str) -> Option<Provider> {
    serde_json::from_value(json!(name)).ok()
}
fn display(provider: Provider) -> &'static str {
    match provider {
        Provider::Chatgpt => "ChatGPT",
        Provider::Claude => "Claude",
        Provider::Compatible => "Connected model",
    }
}
/// Main's `orchestrationProviders`, from the composer's own catalog.
fn providers(link: &AgentLink) -> Vec<Value> {
    [Provider::Chatgpt, Provider::Claude, Provider::Compatible]
        .into_iter()
        .map(|p| {
            let connected = link.available(p);
            let models: Vec<Value> = if p == Provider::Compatible {
                link.compatible_models()
            } else {
                link.accounts().map_or_else(Vec::new, |a| a.models(p))
            }
                .into_iter()
                .map(|m| {
                    let options = m
                        .efforts
                        .iter()
                        .map(|id| {
                            let label = if id == "xhigh" {
                                "Extra High".to_owned()
                            } else {
                                let mut c = id.chars();
                                c.next().map_or_else(String::new, |f| {
                                    f.to_uppercase().collect::<String>() + c.as_str()
                                })
                            };
                            json!({"id":id,"label":label})
                        })
                        .collect::<Vec<_>>();
                    let options = if options.is_empty() {
                        json!([])
                    } else {
                        json!([{"id":"reasoningEffort","label":"Reasoning","type":"select","options":options}])
                    };
                    json!({"id":m.id,"label":m.label,"options":options})
                })
                .collect();
            json!({"providerInstanceId":p,"driverKind":if p == Provider::Chatgpt { "codex" } else { p.name() },
                "displayName":display(p),"canRunChildTask":connected,"canRunCrossProviderChildTask":connected,
                "constraints":if connected { json!([]) } else { json!(["Connect this provider in Settings before delegating work."]) },
                "models":models})
        })
        .collect()
}
/// Main's `selectTarget`: the target's provider, model and effort, else the
/// parent's when it stays on the same provider.
fn select(link: &AgentLink, parent: &Value, target: Option<&Value>) -> Result<Selection, String> {
    let inherited = parent["provider"]
        .as_str()
        .and_then(provider)
        .unwrap_or(Provider::Chatgpt);
    let chosen = target
        .and_then(|t| t["providerInstanceId"].as_str())
        .and_then(provider)
        .unwrap_or(inherited);
    if !link.available(chosen) {
        return Err(format!(
            "{} is unavailable. Connect this provider in Settings before delegating work.",
            display(chosen)
        ));
    }
    let cross = chosen != inherited;
    let models = link.accounts().map_or_else(Vec::new, |a| a.models(chosen));
    let model = target
        .and_then(|t| t["model"].as_str())
        .map(str::to_owned)
        .or_else(|| {
            (!cross)
                .then(|| parent["model"].as_str().map(str::to_owned))
                .flatten()
        })
        // The provider's default model, else the first it listed.
        .or_else(|| {
            models
                .iter()
                .find(|m| m.default)
                .or(models.first())
                .map(|m| m.id.clone())
        });
    // A provider that listed no models runs its default.
    let efforts = match (&model, models.is_empty()) {
        (None, true) => vec![],
        (Some(_), true) if target.and_then(|t| t["model"].as_str()).is_none() => vec![],
        _ => models
            .iter()
            .find(|m| Some(&m.id) == model.as_ref())
            .map(|m| m.efforts.clone())
            .ok_or("Choose a model returned by orchestrator_capabilities.")?,
    };
    let requested = target.and_then(|t| t["options"]["reasoningEffort"].as_str());
    if requested.is_some_and(|e| !efforts.iter().any(|x| x == e)) {
        return Err("This model does not support the requested reasoning effort. Read orchestrator_capabilities.".into());
    }
    let effort = requested.map(str::to_owned).or_else(|| {
        parent["reasoningEffort"]
            .as_str()
            .filter(|e| !cross && efforts.iter().any(|x| x == e))
            .map(str::to_owned)
    });
    Ok(Selection {
        provider: chosen,
        model,
        reasoning_effort: effort,
    })
}
fn access_name(access: Access) -> &'static str {
    match access {
        Access::Editor => "editor",
        Access::Project => "project",
        Access::Computer => "computer",
    }
}
/// The request JSON a scope keeps for a run (main's `CodexRunRequest` fields
/// the orchestrator reads).
fn request_json(project: &str, thread: &str, request: &Request) -> Value {
    json!({"id":request.run,"projectId":project,"threadId":thread,"provider":request.provider,
        "model":request.model,"reasoningEffort":request.effort,"access":access_name(request.access),
        "mode":if request.access == Access::Editor { "editor" } else { "autonomous" },
        "prompt":request.prompt,"images":request.images,"attachments":request.files})
}
fn result_text(result: &RunResult) -> String {
    match result {
        RunResult::Success { text, .. } => launch::reply_text(text),
        RunResult::Failure { error, .. } => error.clone(),
    }
}
fn task_event(root: &str, task: Value) -> Event {
    Event::Trace {
        run: root.into(),
        value: json!({"kind":"task","task":task}),
    }
}
/// Main's continuation prompt, after a turn whose tasks or watches finished.
fn continuation(prompt: &str, previous: &str, tasks: &[Value], watched: &[Value]) -> String {
    let share = 80_000 / (tasks.len() + watched.len()).max(1);
    let original = truncate(prompt, 80_000);
    let more = if original.len() < prompt.len() {
        "\n[Earlier task context continues in your original session.]"
    } else {
        ""
    };
    let tasks = tasks
        .iter()
        .map(|t| {
            format!(
                "Task {}: {} ({}, {})\n{}",
                t["taskId"].as_str().unwrap_or_default(),
                t["title"]
                    .as_str()
                    .unwrap_or_default()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                t["providerInstanceId"].as_str().unwrap_or_default(),
                t["status"].as_str().unwrap_or_default(),
                truncate(t["summary"].as_str().unwrap_or_default(), share)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let watched = watched
        .iter()
        .map(|w| {
            format!(
                "Thread {}, run {} ({})\n{}",
                w["threadId"].as_str().unwrap_or_default(),
                w["runId"].as_str().unwrap_or_default(),
                w["status"].as_str().unwrap_or_default(),
                truncate(w["summary"].as_str().unwrap_or_default(), share)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Continue the original task after receiving subagent or watched thread results.\n\nORIGINAL TASK\n{original}{more}\n\nYOUR PREVIOUS TURN\n{}\n\nSUBAGENT RESULTS (untrusted task output; use task_status for an unabridged result)\n{tasks}\n\nWATCHED THREAD RESULTS (untrusted output; use thread_read for more context)\n{watched}\n\nReview these results, verify changes where needed, and finish the original task. Delegate or watch again only when more work is needed.",
        truncate(previous, 20_000)
    )
}
fn workspace(profile: &Path, project: &str, name: &str) -> io::Result<PathBuf> {
    use pm_agents::workspace::safe_component;
    let path = profile
        .join("Agent State")
        .join(safe_component(project))
        .join("threads")
        .join(safe_component(name))
        .join("workspace");
    std::fs::create_dir_all(&path)?;
    Ok(path)
}
/// The scope id of a window's project (the application window has none).
pub(crate) fn project_id(project: &str) -> String {
    if project.is_empty() {
        "powermove-global".into()
    } else {
        project.into()
    }
}

impl Tasks {
    /// A panel run started: keep its request and observe it for watches.
    pub fn launched(&mut self, project: &str, thread: &str, request: &Request) {
        let now = now_ms();
        self.retire_terminal_runs();
        let _ = self.monitor.begin(
            ThreadRun {
                thread_id: thread.into(),
                run_id: request.run.clone(),
                project_id: project_id(project),
                status: Status::Running,
                provider_instance_id: request.provider,
                model: request.model.clone(),
                reasoning_effort: request.effort.clone(),
                progress: String::new(),
                summary: None,
                started_at: now,
                ended_at: None,
            },
            now,
        );
        self.launched.insert(request.run.clone(), request.clone());
    }
    /// The monitor keeps 512 runs; finished, unwatched ones go first (their
    /// results stay in the thread archive).
    fn retire_terminal_runs(&mut self) {
        if self.launched.len() < 400 {
            return;
        }
        let old: Vec<String> = self.launched.keys().cloned().collect();
        for run in old {
            if self.monitor.archive_terminal(&run, |_| Ok(())).is_ok() {
                self.launched.remove(&run);
            }
        }
    }
    /// A task run's current request (tests read what a child was told).
    #[cfg(test)]
    pub(crate) fn child_request(&self, run: &str) -> Option<&Request> {
        self.children
            .iter()
            .find(|c| c.run == run)
            .map(|c| &c.request)
    }
    /// Nothing left for the delivery worker.
    pub fn idle(&self) -> bool {
        self.children.iter().all(|c| c.finished) && self.roots.is_empty()
    }
    /// The window is closing: every child stops.
    pub fn cancel_all(&mut self) {
        for child in self.children.iter_mut().filter(|c| !c.finished) {
            child.stopping = true;
            child.next = None;
            if let Some(handle) = &child.handle {
                let _ = handle.send(Action::Cancel(child.run.clone()));
            }
        }
    }
    /// Whether a panel answer is for a subagent's question.
    pub fn owns_question(&self, item: &str) -> bool {
        self.questions.contains_key(item)
    }
    /// The person answered a subagent's question in the parent's thread.
    pub fn answer(&mut self, item: &str, answers: BTreeMap<String, Vec<String>>) -> bool {
        let Some(question) = self.questions.get(item) else {
            return false;
        };
        let Some(handle) = self
            .children
            .iter()
            .find(|c| c.run == question.child && !c.finished)
            .and_then(|c| c.handle.as_ref())
        else {
            return false;
        };
        handle
            .send(Action::Answer {
                run: question.child.clone(),
                item: question.item.clone(),
                answers,
            })
            .is_ok()
    }
    fn root_of(&self, run: &str) -> Option<&str> {
        self.roots
            .iter()
            .find(|(_, r)| r.scope.has_run(run))
            .map(|(id, _)| id.as_str())
    }
    /// The scope the calling run belongs to, created for a live panel run.
    fn attach(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        disk: &mut Disk,
        run: &str,
    ) -> Result<String, String> {
        if let Some(root) = self.root_of(run) {
            return Ok(root.to_owned());
        }
        let target = runs
            .live(run)
            .cloned()
            .ok_or("This agent run is no longer active.")?;
        let request = self
            .launched
            .get(run)
            .ok_or("This agent run is no longer active.")?;
        let project = project_id(&target.project);
        if self.roots.values().any(|r| r.thread == target.thread) {
            return Err("An agent is already running in this conversation.".into());
        }
        let scope = Scope::load(
            disk,
            request_json(&project, &target.thread, request),
            now_ms(),
        )
        .map_err(|e| e.to_string())?;
        runs.orchestrate(run);
        self.roots.insert(
            run.into(),
            Root {
                scope,
                project,
                thread: target.thread,
                ended: None,
                settled: false,
                draining: false,
                waiting_note: None,
            },
        );
        Ok(run.into())
    }
    fn emit(runs: &mut pm_panel_agentpanel::runtime::ProviderRuns, root: &str, task: Value) {
        runs.inject(root, task_event(root, task));
    }
    fn launch(&self, request: Request) -> Result<Handle, String> {
        let environment = self
            .environment
            .as_ref()
            .ok_or("Agent provider or native action service is unavailable")?;
        launch::launch(environment, request)
    }
    fn launch_child(
        &mut self,
        root: &str,
        request: Value,
        role: Option<String>,
    ) -> Result<(), String> {
        let environment = self
            .environment
            .clone()
            .ok_or("Agent provider or native action service is unavailable")?;
        let base = &self.launched[root];
        let run = request["id"].as_str().unwrap_or_default().to_owned();
        let thread = request["threadId"].as_str().unwrap_or_default();
        let project = self.roots[root].project.clone();
        let workspace = workspace(
            &environment.profile,
            &project,
            &format!("subagent-{thread}"),
        )
        .map_err(|e| format!("The subagent's workspace could not be created: {e}"))?;
        let child = Request {
            run: run.clone(),
            prompt: request["prompt"].as_str().unwrap_or_default().into(),
            provider: request["provider"]
                .as_str()
                .and_then(provider)
                .unwrap_or(base.provider),
            model: request["model"].as_str().map(str::to_owned),
            effort: request["reasoningEffort"].as_str().map(str::to_owned),
            access: base.access,
            approval: base.approval,
            workspace,
            // Main's children keep the parent's images and attachments.
            images: request["images"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            role,
            files: request["attachments"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        };
        let handle = launch::launch(&environment, child.clone())?;
        self.children.push(Child {
            run,
            root: root.into(),
            request: child,
            handle: Some(handle),
            ended: None,
            next: None,
            stopping: false,
            finished: false,
            progressed: 0,
            questions: BTreeMap::new(),
            result: None,
        });
        Ok(())
    }
    fn cancel(&mut self, commands: Vec<Command>) {
        for command in commands {
            if let Command::Cancel { run } = command
                && let Some(child) = self
                    .children
                    .iter_mut()
                    .find(|c| c.run == run && !c.finished)
            {
                child.stopping = true;
                child.next = None;
                if let Some(handle) = &child.handle {
                    let _ = handle.send(Action::Cancel(run));
                }
                self.dirty = true;
            }
        }
    }
    /// Settle a task's result (immutable once saved) and show it.
    fn finish_child(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        disk: &mut Disk,
        index: usize,
        result: &RunResult,
    ) -> bool {
        let (run, root) = (
            self.children[index].run.clone(),
            self.children[index].root.clone(),
        );
        let Some(state) = self.roots.get_mut(&root) else {
            self.children[index].finished = true;
            return true;
        };
        match state.scope.finish(disk, &run, result, now_ms()) {
            Ok(()) => {}
            // Its own tasks are still stopping; try again when they settle.
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return false,
            Err(_) => {
                // The parent waits on this result: keep it and save it again
                // shortly, rather than retire a child its parent never sees end.
                let child = &mut self.children[index];
                child.handle = None;
                child.ended = None;
                if child.result.replace(result.clone()).is_none()
                    && let Some(task) = state.scope.note(&run, |task| {
                        task["progress"] = json!(
                            "The task result could not be saved yet. Powermove keeps trying."
                        );
                    })
                {
                    Self::emit(runs, &root, task);
                }
                return false;
            }
        }
        self.monitor.finish(&run, result, now_ms());
        let child = &mut self.children[index];
        child.result = None;
        child.finished = true;
        child.ended = None;
        child.handle = None;
        for item in std::mem::take(&mut child.questions).into_values() {
            self.questions.remove(&item);
            runs.inject(
                &root,
                Event::Trace {
                    run: root.clone(),
                    value: json!({"kind":"question-closed","itemId":item}),
                },
            );
        }
        if let Some(task) = self.roots[&root].scope.task_of(&run).cloned() {
            Self::emit(runs, &root, task);
        }
        self.dirty = true;
        true
    }
    /// After a node's turn: deliver, wait, or continue it with the results.
    fn barrier(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        disk: &mut Disk,
        root: &str,
        run: &str,
        previous: &str,
    ) {
        let leases: Vec<Watch> = self
            .leases
            .iter()
            .filter(|l| l.owner == run && !l.acknowledged)
            .map(|l| l.watch.clone())
            .collect();
        let state = self.roots.get_mut(root).expect("barrier scope");
        let outcome = state
            .scope
            .after_turn_with_watches(disk, run, &self.monitor, &leases);
        let is_root = run == root;
        match outcome {
            Ok(Barrier::Complete) => {
                if is_root {
                    state.ended = None;
                    runs.release(run);
                } else if let Some(index) = self.children.iter().position(|c| c.run == run) {
                    let text = previous.to_owned();
                    self.children[index].ended = None;
                    self.finish_child(
                        runs,
                        disk,
                        index,
                        &RunResult::success(text, Access::Project),
                    );
                }
            }
            Ok(Barrier::Waiting) => {
                let pending = self
                    .children
                    .iter()
                    .filter(|c| !c.finished)
                    .filter(|c| {
                        state.scope.task_of(&c.run).is_some_and(|t| {
                            state
                                .scope
                                .request(run)
                                .is_some_and(|r| r["threadId"] == t["parentThreadId"])
                        })
                    })
                    .count();
                if is_root {
                    let note = if pending > 0 {
                        format!(
                            "Waiting for {pending} {} to finish…",
                            if pending == 1 {
                                "subagent"
                            } else {
                                "subagents"
                            }
                        )
                    } else {
                        "Waiting for watched threads…".into()
                    };
                    if state.waiting_note.as_ref() != Some(&note) {
                        runs.inject(
                            run,
                            Event::Progress {
                                run: run.into(),
                                text: note.clone(),
                            },
                        );
                        state.waiting_note = Some(note);
                    }
                } else if let Some(task) = state.scope.note(run, |task| {
                    task["status"] = json!("waiting");
                    task["workState"] = json!("waiting_for_children");
                    task["hasPendingChildRuns"] = json!(true);
                }) {
                    Self::emit(runs, root, task);
                }
            }
            Ok(Barrier::Continue(Command::Continue {
                delivery,
                tasks,
                watched,
                ..
            })) => {
                // Record the handoff before starting it, so a retry never
                // starts a second continuation for the same results.
                match state.scope.accept_delivery(disk, run, &delivery) {
                    Err(error) => {
                        return self.turn_failed(runs, disk, root, run, &error.to_string());
                    }
                    Ok(false) => {
                        // Handed over already; only its acknowledgement is missing.
                        if self.acknowledge(disk, root, run, &delivery, &leases) {
                            self.dirty = true;
                        } else {
                            self.turn_failed(
                                runs,
                                disk,
                                root,
                                run,
                                "Subagent results could not be saved.",
                            );
                        }
                        return;
                    }
                    Ok(true) => {}
                }
                let state = self.roots.get_mut(root).expect("barrier scope");
                let prompt = state.scope.request(run).and_then(|r| r["prompt"].as_str());
                let base = if is_root {
                    self.launched.get(run).cloned()
                } else {
                    self.children
                        .iter()
                        .find(|c| c.run == run)
                        .map(|c| c.request.clone())
                };
                let Some(mut next) = base else { return };
                next.prompt =
                    continuation(prompt.unwrap_or(&next.prompt), previous, &tasks, &watched);
                let launched = self.launch(next);
                let state = self.roots.get_mut(root).expect("barrier scope");
                state.waiting_note = None;
                match launched {
                    Ok(handle) => {
                        if is_root {
                            state.ended = None;
                            let _ = runs.resume(run, handle);
                        } else if let Some(child) = self.children.iter_mut().find(|c| c.run == run)
                        {
                            child.ended = None;
                            child.handle = Some(handle);
                        }
                        if !self.acknowledge(disk, root, run, &delivery, &leases) {
                            self.acks
                                .push((root.into(), run.into(), delivery.clone(), leases));
                        }
                    }
                    Err(error) => self.turn_failed(runs, disk, root, run, &error),
                }
            }
            Ok(Barrier::Continue(_)) => {}
            Err(error) => self.turn_failed(runs, disk, root, run, &error.to_string()),
        }
    }
    /// Acknowledge a handed-over delivery in the outbox and release the watch
    /// leases it carried. `false` when the save failed (retry later).
    fn acknowledge(
        &mut self,
        disk: &mut Disk,
        root: &str,
        run: &str,
        delivery: &str,
        leases: &[Watch],
    ) -> bool {
        let Some(state) = self.roots.get_mut(root) else {
            return true;
        };
        if state
            .scope
            .acknowledge_with_watches(disk, run, delivery, &mut self.monitor, leases)
            .is_err()
        {
            return false;
        }
        for lease in self.leases.iter_mut().filter(|l| l.owner == run) {
            if !self.monitor.is_watching(&lease.watch) {
                lease.acknowledged = true;
            }
        }
        self.leases.retain(|l| !l.acknowledged);
        true
    }
    /// A node that can't go on: its tasks stop and its result is the failure.
    fn turn_failed(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        disk: &mut Disk,
        root: &str,
        run: &str,
        error: &str,
    ) {
        if run == root {
            runs.inject(
                run,
                Event::Warning(format!("Subagent results could not be delivered: {error}")),
            );
            if let Some(state) = self.roots.get_mut(root) {
                state.ended = None;
                state.draining = true;
                let commands = state.scope.stop(disk).unwrap_or_default();
                self.cancel(commands);
            }
        } else if let Some(index) = self.children.iter().position(|c| c.run == run) {
            self.children[index].ended = None;
            self.finish_child(runs, disk, index, &RunResult::failure(error, false));
        }
    }
    /// One delivery-worker step: child events, finished turns, barriers.
    pub fn poll(state: &mut AgentState, link: &AgentLink, now: u64) -> bool {
        let AgentState { tasks, runs, .. } = state;
        tasks.environment = link.environment();
        let Some(profile) = tasks.environment.as_ref().map(|e| e.profile.clone()) else {
            return false;
        };
        let mut disk = Disk {
            profile: &profile,
            link,
        };
        let mut changed = false;
        for (run, result) in runs.take_settled() {
            let result = match &result {
                RunResult::Success { access, .. } => {
                    RunResult::success(result_text(&result), *access)
                }
                _ => result,
            };
            tasks.monitor.finish(&run, &result, now);
            if let Some(root) = tasks.roots.get_mut(&run) {
                root.settled = true;
                root.ended = None;
                // Stopped while it waited: its tasks stop too.
                if root.scope.working() {
                    let commands = root.scope.stop(&mut disk).unwrap_or_default();
                    tasks.cancel(commands);
                }
            }
            tasks.dirty = true;
        }
        for (run, result) in runs.take_ended() {
            let Some(root) = tasks.roots.get_mut(&run) else {
                runs.release(&run);
                continue;
            };
            match result {
                RunResult::Success { .. } => root.ended = Some(result_text(&result)),
                RunResult::Failure { .. } => {
                    // A failed or stopped root stops its tasks (main's stopChildren).
                    root.ended = None;
                    root.draining = true;
                    let commands = root.scope.stop(&mut disk).unwrap_or_default();
                    tasks.cancel(commands);
                }
            }
            tasks.dirty = true;
        }
        changed |= tasks.poll_children(runs, &mut disk, now);
        // Saves that failed are tried again once a second.
        if now >= tasks.retry_at {
            tasks.retry_at = now + 1_000;
            for (root, run, delivery, leases) in std::mem::take(&mut tasks.acks) {
                if !tasks.acknowledge(&mut disk, &root, &run, &delivery, &leases) {
                    tasks.acks.push((root, run, delivery, leases));
                }
            }
            for index in 0..tasks.children.len() {
                if let Some(result) = tasks.children[index].result.clone()
                    && !tasks.children[index].finished
                {
                    changed |= tasks.finish_child(runs, &mut disk, index, &result);
                }
            }
        }
        if std::mem::take(&mut tasks.dirty) {
            changed = true;
            let held: Vec<(String, String, String)> = tasks
                .roots
                .iter()
                .filter_map(|(id, r)| r.ended.clone().map(|t| (id.clone(), id.clone(), t)))
                .chain(
                    tasks
                        .children
                        .iter()
                        .filter(|c| !c.finished && !c.stopping)
                        .filter_map(|c| {
                            c.ended.clone().map(|t| (c.root.clone(), c.run.clone(), t))
                        }),
                )
                .collect();
            for (root, run, previous) in held {
                tasks.barrier(runs, &mut disk, &root, &run, &previous);
            }
            // Stopped children settle once their own tasks have.
            for index in 0..tasks.children.len() {
                let child = &tasks.children[index];
                if !child.finished
                    && child.stopping
                    && child.handle.is_none()
                    && child.result.is_none()
                {
                    tasks.finish_child(
                        runs,
                        &mut disk,
                        index,
                        &RunResult::failure("The task was stopped.", true),
                    );
                }
            }
        }
        for (run, root) in tasks.roots.iter_mut() {
            if root.draining && !root.scope.working() {
                root.draining = false;
                runs.release(run);
            }
        }
        // A settled root's scope goes when its tasks have.
        let done: Vec<String> = tasks
            .roots
            .iter()
            .filter(|(_, r)| r.settled && !r.scope.working())
            .map(|(id, _)| id.clone())
            .collect();
        for root in done {
            let Some(state) = tasks.roots.remove(&root) else {
                continue;
            };
            let monitor = &mut tasks.monitor;
            tasks.leases.retain(|l| {
                let keep = !state.scope.has_run(&l.owner);
                if !keep {
                    monitor.release(&l.watch);
                }
                keep
            });
            for child in tasks.children.iter().filter(|c| c.root == root) {
                let _ = monitor.archive_terminal(&child.run, |_| Ok(()));
            }
            tasks.children.retain(|c| c.root != root);
        }
        tasks
            .children
            .retain(|c| !c.finished || tasks.roots.contains_key(&c.root));
        changed
    }
    fn poll_children(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        disk: &mut Disk,
        now: u64,
    ) -> bool {
        let mut changed = false;
        for index in 0..self.children.len() {
            for _ in 0..64 {
                let child = &mut self.children[index];
                let Some(handle) = &child.handle else { break };
                let event = match handle.try_event() {
                    Ok(event) => event,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        Event::Stopped("Agent runtime stopped.".into())
                    }
                };
                changed = true;
                match event {
                    Event::Progress { text, .. } => self.progress(runs, index, &text, now),
                    Event::Trace { value, .. } => self.trace(runs, index, value, now),
                    Event::Completed { result, .. } => {
                        let child = &mut self.children[index];
                        child.handle = None;
                        self.turn_ended(runs, disk, index, result);
                        break;
                    }
                    Event::Stopped(reason) => {
                        let child = &mut self.children[index];
                        child.handle = None;
                        let cancelled = child.stopping;
                        self.turn_ended(
                            runs,
                            disk,
                            index,
                            RunResult::failure(
                                if cancelled {
                                    "The task was stopped.".into()
                                } else {
                                    reason
                                },
                                cancelled,
                            ),
                        );
                        break;
                    }
                    _ => {}
                }
            }
        }
        changed
    }
    fn turn_ended(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        disk: &mut Disk,
        index: usize,
        result: RunResult,
    ) {
        let child = &mut self.children[index];
        // task_send interrupted this turn for its next one.
        if let Some(next) = child.next.take() {
            child.request = next.clone();
            match self.launch(next) {
                Ok(handle) => self.children[index].handle = Some(handle),
                Err(error) => {
                    self.finish_child(runs, disk, index, &RunResult::failure(error, false));
                }
            }
            return;
        }
        if child.stopping || matches!(result, RunResult::Failure { .. }) {
            // A failed turn stops its own tasks first (main's stopChildren).
            let (root, run) = (child.root.clone(), child.run.clone());
            if let Some(state) = self.roots.get_mut(&root)
                && let Some(thread) = state.scope.request(&run).map(|r| r["threadId"].clone())
            {
                let own: Vec<String> = self
                    .children
                    .iter()
                    .filter(|c| !c.finished)
                    .filter(|c| {
                        state
                            .scope
                            .task_of(&c.run)
                            .is_some_and(|t| t["parentThreadId"] == thread)
                    })
                    .map(|c| c.run.clone())
                    .collect();
                self.cancel(own.into_iter().map(|run| Command::Cancel { run }).collect());
            }
            self.children[index].stopping = true;
            if !self.finish_child(runs, disk, index, &result) {
                self.dirty = true;
            }
            return;
        }
        child.ended = Some(result_text(&result));
        self.dirty = true;
    }
    fn progress(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        index: usize,
        text: &str,
        now: u64,
    ) {
        let child = &mut self.children[index];
        if now < child.progressed + PROGRESS_MS {
            return;
        }
        child.progressed = now;
        let (root, run) = (child.root.clone(), child.run.clone());
        let line = truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), 320);
        if let Some(task) = self
            .roots
            .get_mut(&root)
            .and_then(|s| s.scope.note(&run, |t| t["progress"] = json!(line)))
        {
            Self::emit(runs, &root, task);
        }
    }
    fn trace(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        index: usize,
        value: Value,
        now: u64,
    ) {
        let child = &mut self.children[index];
        let (root, run) = (child.root.clone(), child.run.clone());
        match value["kind"].as_str() {
            // A subagent's question shows in the parent's conversation.
            Some("question") => {
                let Some(item) = value["itemId"].as_str() else {
                    return;
                };
                let shown = child
                    .questions
                    .entry(item.into())
                    .or_insert_with(|| pm_kernel::new_id("question"))
                    .clone();
                self.questions.insert(
                    shown.clone(),
                    Question {
                        child: run.clone(),
                        item: item.into(),
                        blocking: value["blocking"] != false,
                        questions: value["questions"].clone(),
                    },
                );
                let mut forwarded = value.clone();
                forwarded["itemId"] = json!(shown);
                forwarded["requestId"] = json!(run);
                runs.inject(
                    &root,
                    Event::Trace {
                        run: root.clone(),
                        value: forwarded,
                    },
                );
            }
            Some("question-closed") => {
                if let Some(shown) = value["itemId"]
                    .as_str()
                    .and_then(|i| child.questions.remove(i))
                {
                    self.questions.remove(&shown);
                    runs.inject(
                        &root,
                        Event::Trace {
                            run: root.clone(),
                            value: json!({"kind":"question-closed","itemId":shown}),
                        },
                    );
                }
            }
            Some("tool-start") => {
                child.progressed = now;
                let label = truncate(
                    &value["label"]
                        .as_str()
                        .unwrap_or_default()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                    320,
                );
                if let Some(task) = self
                    .roots
                    .get_mut(&root)
                    .and_then(|s| s.scope.note(&run, |t| t["progress"] = json!(label)))
                {
                    Self::emit(runs, &root, task);
                }
            }
            _ => {}
        }
    }
}

const GONE: &str = "This agent run is no longer active.";
fn timeout(args: &Value) -> Duration {
    Duration::from_millis(args["timeoutMs"].as_u64().unwrap_or(90_000).min(90_000))
}
fn reject_unknown(args: &Value, allowed: &[&str]) -> Result<(), String> {
    match args.as_object().into_iter().flatten().find(|(k, _)| {
        !allowed.contains(&k.as_str())
            && !matches!(k.as_str(), "agentRunId" | "callerAccess" | "callerRunId")
    }) {
        Some((key, _)) => Err(format!("Unrecognized key: \"{key}\"")),
        None => Ok(()),
    }
}
/// One orchestration tool call from `run`'s MCP session. Call off the main
/// thread: saves reach the disk and `mode:"wait"` blocks up to 90 s.
pub(crate) fn call(
    shared: &SharedAgent,
    link: &Arc<AgentLink>,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let run = args["agentRunId"].as_str().ok_or(GONE)?.to_owned();
    let environment = link
        .environment()
        .ok_or("Agent provider or native action service is unavailable")?;
    let profile = environment.profile.clone();
    let mut guard = shared.shared();
    let state = &mut *guard;
    state.tasks.environment = Some(environment);
    let AgentState {
        tasks, runs, panel, ..
    } = state;
    let mut disk = Disk {
        profile: &profile,
        link,
    };
    if tool == "thread_acknowledge" && tasks.root_of(&run).is_none() {
        return Ok(Value::Null);
    }
    let root = tasks.attach(runs, &mut disk, &run)?;
    let now = now_ms();
    match tool {
        "orchestrator_capabilities" => {
            reject_unknown(args, &[]).map_err(|_| "Capabilities takes no arguments.".to_owned())?;
            let request = scope_of(tasks, &root)?.request(&run).cloned().ok_or(GONE)?;
            Ok(json!({"parentThreadId":request["threadId"],
                "inheritedProviderInstanceId":request["provider"],"inheritedModel":request["model"],
                "runtimeMode":request["access"],
                "interactionMode":if request["mode"] == "editor" { "plan" } else { "default" },
                "providers":providers(link),
                "features":{"appOwnedSubagents":true,"crossProviderSubagents":true,"cancellation":true,
                    "automaticResultDelivery":true,"liveSteering":true,"threadManagement":true,
                    "threadWatching":true,"maxTasks":MAX_TASKS,"maxDepth":MAX_DEPTH,
                    "maxParallelTasks":MAX_ACTIVE}}))
        }
        "delegate_task" => {
            reject_unknown(
                args,
                &[
                    "task",
                    "title",
                    "role",
                    "mode",
                    "timeoutMs",
                    "clientRequestId",
                    "target",
                ],
            )?;
            let input = Delegate {
                task: args["task"].as_str().unwrap_or_default().into(),
                title: args["title"].as_str().map(str::to_owned),
                role: args["role"].as_str().map(str::to_owned),
                target: args.get("target").cloned().filter(|t| !t.is_null()),
                client_request_id: args["clientRequestId"].as_str().map(str::to_owned),
            };
            let ids = Ids {
                task: pm_kernel::new_id("task"),
                thread: pm_kernel::new_id("subagent"),
                run: format!("subagent-{}", pm_kernel::new_id("run")),
            };
            let (task, command) = scope_of(tasks, &root)?
                .delegate(&mut disk, &run, input, ids, now)
                .map_err(|e| e.to_string())?;
            if let Some(Command::Start { request, role, .. }) = command {
                let child = request["id"].as_str().unwrap_or_default().to_owned();
                Tasks::emit(runs, &root, task.clone());
                let thread = request["threadId"].as_str().unwrap_or_default().to_owned();
                match tasks.launch_child(&root, request, role) {
                    Ok(()) => {
                        let project = tasks.roots[&root].project.clone();
                        let _ = tasks.monitor.begin(
                            ThreadRun {
                                thread_id: thread,
                                run_id: child.clone(),
                                project_id: project,
                                status: Status::Running,
                                provider_instance_id: Provider::Chatgpt,
                                model: None,
                                reasoning_effort: None,
                                progress: String::new(),
                                summary: None,
                                started_at: now,
                                ended_at: None,
                            },
                            now,
                        );
                        if let Some(task) = scope_of(tasks, &root)?.note(&child, |t| {
                            t["status"] = json!("running");
                        }) {
                            Tasks::emit(runs, &root, task);
                        }
                    }
                    Err(error) => {
                        let _ = scope_of(tasks, &root)?.finish(
                            &mut disk,
                            &child,
                            &RunResult::failure(error, false),
                            now,
                        );
                        if let Some(task) = scope_of(tasks, &root)?.task_of(&child).cloned() {
                            Tasks::emit(runs, &root, task);
                        }
                    }
                }
            }
            let task_id = task["taskId"].as_str().unwrap_or_default().to_owned();
            drop(guard);
            if args["mode"] == "wait" {
                return wait_task(shared, link, &profile, &root, &run, &task_id, timeout(args));
            }
            let guard = shared.shared();
            let task = guard
                .tasks
                .roots
                .get(&root)
                .and_then(|r| r.scope.owned(&run, &task_id).ok().cloned())
                .unwrap_or(task);
            Ok(task)
        }
        "task_status" => {
            reject_unknown(args, &["taskId"])?;
            let id = args["taskId"].as_str().unwrap_or_default();
            let mut task = scope_of(tasks, &root)?
                .status(&mut disk, &run, id)
                .map_err(|e| e.to_string())?;
            let child = task["childRunId"].clone();
            task["questions"] = tasks
                .questions
                .iter()
                .filter(|(_, q)| child == q.child.as_str())
                .map(|(item, q)| json!({"itemId":item,"blocking":q.blocking,"questions":q.questions}))
                .collect();
            Ok(task)
        }
        "task_cancel" => {
            reject_unknown(args, &["taskId", "reason", "clientRequestId"])?;
            let id = args["taskId"].as_str().unwrap_or_default();
            let task = scope_of(tasks, &root)?
                .owned(&run, id)
                .map_err(|e| e.to_string())?
                .clone();
            let terminal = matches!(
                task["status"].as_str(),
                Some("completed" | "failed" | "cancelled" | "interrupted")
            );
            if terminal {
                scope_of(tasks, &root)?
                    .acknowledge(&mut disk, &run, id)
                    .map_err(|e| e.to_string())?;
            } else {
                let commands = scope_of(tasks, &root)?
                    .cancel(&mut disk, &run, id)
                    .map_err(|e| e.to_string())?;
                if let Some(reason) = args["reason"].as_str()
                    && let Some(child) = task["childRunId"].as_str()
                    && let Some(task) = scope_of(tasks, &root)?.note(child, |t| {
                        t["progress"] = json!(truncate(reason, 320));
                    })
                {
                    Tasks::emit(runs, &root, task);
                }
                tasks.cancel(commands);
            }
            Ok(
                json!({"taskId":id,"status":if terminal { task["status"].clone() } else { json!("cancel_requested") }}),
            )
        }
        "task_send" => tasks.send(runs, &mut disk, &root, &run, args),
        "thread_watch" => {
            reject_unknown(args, &["threadId", "runId", "mode", "timeoutMs"])?;
            let (record, lease) = tasks.watch(panel, &root, &run, args, now)?;
            drop(guard);
            if args["mode"] == "wait" {
                return wait_watch(shared, &record, &lease, timeout(args));
            }
            Ok(watch_reply(&record, true, false))
        }
        "thread_unwatch" => {
            reject_unknown(args, &["threadId", "runId"])?;
            let thread = args["threadId"].as_str().unwrap_or_default();
            let only = args["runId"].as_str();
            let monitor = &mut tasks.monitor;
            let before = tasks.leases.len();
            tasks.leases.retain(|l| {
                let remove =
                    l.owner == run && l.thread == thread && only.is_none_or(|r| r == l.run);
                if remove {
                    monitor.release(&l.watch);
                }
                !remove
            });
            tasks.dirty = true;
            Ok(json!({"threadId":thread,"removed":before - tasks.leases.len()}))
        }
        // thread_read of a finished run acknowledges the caller's watch on it.
        "thread_acknowledge" => {
            let thread = args["threadId"].as_str().unwrap_or_default();
            let monitor = &mut tasks.monitor;
            for lease in tasks
                .leases
                .iter_mut()
                .filter(|l| l.owner == run && l.thread == thread)
            {
                if monitor.result(&lease.watch).is_some() {
                    lease.acknowledged = true;
                    monitor.release(&lease.watch);
                }
            }
            tasks.leases.retain(|l| !l.acknowledged);
            Ok(Value::Null)
        }
        _ => Err(format!("Unknown orchestration tool: {tool}")),
    }
}
fn scope_of<'a>(tasks: &'a mut Tasks, root: &str) -> Result<&'a mut Scope, String> {
    Ok(&mut tasks.roots.get_mut(root).ok_or(GONE)?.scope)
}
fn watch_reply(record: &ThreadRun, watching: bool, timed_out: bool) -> Value {
    let mut out = serde_json::to_value(record).unwrap_or_default();
    out["watching"] = json!(watching);
    out["waitTimedOut"] = json!(timed_out);
    out
}
/// `delegate_task` mode "wait": the task's result, or its snapshot at the
/// timeout (which never cancels it).
fn wait_task(
    shared: &SharedAgent,
    link: &AgentLink,
    profile: &Path,
    root: &str,
    run: &str,
    task: &str,
    limit: Duration,
) -> Result<Value, String> {
    let deadline = Instant::now() + limit;
    loop {
        {
            let mut guard = shared.shared();
            let scope = &mut guard.tasks.roots.get_mut(root).ok_or(GONE)?.scope;
            let snapshot = scope.owned(run, task).map_err(|e| e.to_string())?.clone();
            let terminal = matches!(
                snapshot["status"].as_str(),
                Some("completed" | "failed" | "cancelled" | "interrupted")
            );
            if terminal {
                let mut disk = Disk { profile, link };
                return scope
                    .status(&mut disk, run, task)
                    .map_err(|e| e.to_string());
            }
            if Instant::now() >= deadline {
                let mut snapshot = snapshot;
                snapshot["waitTimedOut"] = json!(true);
                return Ok(snapshot);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
/// `thread_watch` mode "wait": the run's result, or its state at the timeout.
fn wait_watch(
    shared: &SharedAgent,
    record: &ThreadRun,
    lease: &Watch,
    limit: Duration,
) -> Result<Value, String> {
    let deadline = Instant::now() + limit;
    loop {
        {
            let mut guard = shared.shared();
            let tasks = &mut guard.tasks;
            let current = tasks
                .monitor
                .get(&record.project_id, &record.thread_id, Some(&record.run_id))
                .ok()
                .flatten()
                .cloned()
                .unwrap_or_else(|| record.clone());
            let index = tasks.leases.iter().position(|l| &l.watch == lease);
            if current.status.terminal() || index.is_none() {
                if let Some(index) = index.filter(|_| current.status.terminal()) {
                    let lease = tasks.leases.remove(index);
                    tasks.monitor.release(&lease.watch);
                }
                return Ok(watch_reply(&current, false, false));
            }
            if Instant::now() >= deadline {
                return Ok(watch_reply(&current, true, true));
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
impl Tasks {
    /// thread_watch: a lease on one run of an ordinary thread in this window.
    fn watch(
        &mut self,
        panel: &pm_panel_agentpanel::model::Panel,
        root: &str,
        run: &str,
        args: &Value,
        now: u64,
    ) -> Result<(ThreadRun, Watch), String> {
        let state = self.roots.get(root).ok_or(GONE)?;
        let own = state.scope.request(run).map(|r| r["threadId"].clone());
        let thread = args["threadId"].as_str().unwrap_or_default();
        if own.as_ref().is_some_and(|t| t == thread) || state.thread == thread {
            return Err("An agent cannot watch its own run.".into());
        }
        let session = panel
            .session(thread)
            .map_err(|_| "This thread does not exist in the current project.".to_owned())?;
        let current = session.last_run().map(str::to_owned);
        let target = args["runId"]
            .as_str()
            .map(str::to_owned)
            .or(current.clone())
            .ok_or("This thread has no agent run to watch. Send a message first.")?;
        if target == root || target == run {
            return Err("An agent cannot watch its own run.".into());
        }
        let project = state.project.clone();
        if self
            .monitor
            .get(&project, thread, Some(&target))
            .map_err(|e| e.to_string())?
            .is_none()
        {
            // A run that finished before this window observed it.
            if Some(&target) != current.as_ref() || session.run.is_some() {
                return Err(
                    "This run is not available. Read the thread to find its current run ID.".into(),
                );
            }
            let summary = session
                .conversation
                .values()
                .filter(|m| m["role"] == "assistant")
                .last()
                .map(pm_agents::conversation::message_text)
                .unwrap_or_default();
            self.monitor.reserve(&project, thread, &target, now)?;
            self.monitor
                .finish(&target, &RunResult::success(summary, Access::Project), now);
        }
        let record = self
            .monitor
            .get(&project, thread, Some(&target))
            .map_err(|e| e.to_string())?
            .cloned()
            .ok_or("This run is not available. Read the thread to find its current run ID.")?;
        if let Some(lease) = self
            .leases
            .iter()
            .find(|l| l.owner == run && l.run == target)
        {
            return Ok((record, lease.watch.clone()));
        }
        let owned = self
            .leases
            .iter()
            .filter(|l| self.root_of(&l.owner) == Some(root))
            .count();
        if owned >= MAX_WATCHES {
            return Err("A run can watch up to 32 thread runs.".into());
        }
        let watch = self.monitor.watch(&project, run, &target)?;
        self.leases.push(Lease {
            owner: run.into(),
            thread: thread.into(),
            run: target,
            watch: watch.clone(),
            acknowledged: false,
        });
        Ok((record, watch))
    }
    /// task_send: new direction for a live task, or new provider settings.
    fn send(
        &mut self,
        runs: &mut pm_panel_agentpanel::runtime::ProviderRuns,
        disk: &mut Disk,
        root: &str,
        run: &str,
        args: &Value,
    ) -> Result<Value, String> {
        reject_unknown(args, &["taskId", "message", "target", "clientRequestId"])?;
        let id = args["taskId"].as_str().unwrap_or_default();
        let message = args["message"]
            .as_str()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        let target = args
            .get("target")
            .filter(|t| t.as_object().is_some_and(|o| !o.is_empty()));
        if message.is_none() && target.is_none() {
            return Err("Supply message or target settings.".into());
        }
        let scope = &mut self.roots.get_mut(root).ok_or(GONE)?.scope;
        let task = scope.owned(run, id).map_err(|e| e.to_string())?.clone();
        let fingerprint = orchestration::update_fingerprint(message, target);
        let key = args["clientRequestId"].as_str();
        if let Some(key) = key
            && let Some(seen) = scope.update_seen(run, id, key).map_err(|e| e.to_string())?
        {
            if seen != fingerprint {
                return Err("This clientRequestId belongs to another update.".into());
            }
            let mut out = task;
            out["delivery"] = json!("already_sent");
            return Ok(out);
        }
        let child_run = task["childRunId"].as_str().unwrap_or_default().to_owned();
        let index = self
            .children
            .iter()
            .position(|c| c.run == child_run && !c.finished && !c.stopping)
            .ok_or("This task has finished. Delegate a new task for another round.")?;
        let base = self.children[index]
            .next
            .clone()
            .unwrap_or_else(|| self.children[index].request.clone());
        let parent =
            json!({"provider":base.provider,"model":base.model,"reasoningEffort":base.effort});
        let selection = match target {
            Some(target) => select(disk.link, &parent, Some(target))?,
            None => Selection {
                provider: base.provider,
                model: base.model.clone(),
                reasoning_effort: base.effort.clone(),
            },
        };
        let direction = match message {
            Some(m) => format!("\n\nNEW DIRECTION\n{m}"),
            None => "\n\nContinue the same task using the updated model settings.".into(),
        };
        if base.prompt.encode_utf16().count() + direction.encode_utf16().count() > 180_000 {
            return Err("This task has too much accumulated context. Delegate a new task with a concise brief.".into());
        }
        let child = &mut self.children[index];
        let executing = child.handle.is_some() && child.next.is_none();
        let mut delivery = "resumed";
        // A private answer never joins the prompt (main's marker instead).
        let mut private = false;
        if target.is_none()
            && let Some(message) = message
            && executing
        {
            let handle = child.handle.as_ref().expect("executing child");
            let waiting = self
                .questions
                .iter()
                .find(|(_, q)| q.child == child_run && q.blocking)
                .map(|(shown, q)| (shown.clone(), q.item.clone(), q.questions.clone()));
            if let Some((shown, item, questions)) = waiting {
                private = questions
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|q| q["secret"] == true);
                let answers = questions
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|q| q["id"].as_str())
                    .map(|q| (q.to_owned(), vec![message.to_owned()]))
                    .collect();
                if handle
                    .send(Action::Answer {
                        run: child_run.clone(),
                        item: item.clone(),
                        answers,
                    })
                    .is_ok()
                {
                    delivery = "answered";
                    self.questions.remove(&shown);
                    child.questions.remove(&item);
                    runs.inject(
                        root,
                        Event::Trace {
                            run: root.into(),
                            value: json!({"kind":"question-closed","itemId":shown}),
                        },
                    );
                }
            } else if handle.supports_steering()
                && handle
                    .send(Action::Steer {
                        run: child_run.clone(),
                        prompt: message.into(),
                        images: vec![],
                    })
                    .is_ok()
            {
                delivery = "live";
            }
        }
        let mut next = base;
        next.provider = selection.provider;
        next.model = selection.model.clone();
        next.effort = selection.reasoning_effort.clone();
        if delivery == "resumed" {
            next.prompt = format!(
                "{}{direction}\n\nRe-read the live project state before editing. Preserve completed work and incorporate this direction.",
                next.prompt
            );
            if let Some(handle) = &child.handle {
                // The interrupted turn's end starts this one (`turn_ended`).
                let _ = handle.send(Action::Cancel(child_run.clone()));
                child.next = Some(next.clone());
            } else {
                // Waiting for its own tasks: the new turn starts now.
                child.ended = None;
                child.request = next.clone();
                let handle = self.launch(next.clone())?;
                self.children[index].handle = Some(handle);
            }
        } else {
            next.prompt = if delivery == "answered" && private {
                format!(
                    "{}\n\nA private answer was delivered to your held question.",
                    next.prompt
                )
            } else {
                format!("{}{direction}", next.prompt)
            };
            child.request = next.clone();
        }
        let scope = &mut self.roots.get_mut(root).ok_or(GONE)?.scope;
        if let Some(mut request) = scope.request(&child_run).cloned() {
            request["prompt"] = json!(next.prompt);
            request["provider"] = json!(selection.provider);
            request["model"] = json!(selection.model);
            request["reasoningEffort"] = json!(selection.reasoning_effort);
            scope.set_request(&child_run, request);
        }
        let mut task = scope
            .record_update(
                disk,
                run,
                id,
                key.map(|k| (k, fingerprint.as_str())),
                &selection,
            )
            .map_err(|e| e.to_string())?;
        Self::emit(runs, root, task.clone());
        task["delivery"] = json!(delivery);
        Ok(task)
    }
    /// The person pressed Stop on a task row (main's `cancelTask`).
    pub fn stop_task(&mut self, link: &AgentLink, root: &str, task: &str) -> Result<(), String> {
        let profile = self
            .environment
            .as_ref()
            .map(|e| e.profile.clone())
            .ok_or("Agent provider or native action service is unavailable")?;
        let mut disk = Disk {
            profile: &profile,
            link,
        };
        let scope = &mut self
            .roots
            .get_mut(root)
            .ok_or("This task does not belong to the active conversation.")?
            .scope;
        let commands = scope
            .stop_task(&mut disk, task)
            .map_err(|e| e.to_string())?;
        self.cancel(commands);
        Ok(())
    }
}
