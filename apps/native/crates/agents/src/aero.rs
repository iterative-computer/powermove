//! In-process Aero transport. The runtime worker owns every libfx call.
use crate::{
    Access, Event, RunResult, approvals,
    delivery::Surface,
    launch::Request,
    transport::{self, Action, Handle},
};
use pm_aero_core::{
    Config, Hook, HookAnswer, HookKind, Host, JournalAppend, Runtime, StopReason, Tool, ToolLease,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

mod journal;
mod model;

pub use model::{Model, Reply, evaluate};

pub type Report = Arc<dyn Fn(Value) + Send + Sync>;

pub trait Session: Send {
    fn context(&mut self) -> Result<Value, String>;
    fn call(
        &mut self,
        tool: &str,
        input: &str,
        approve: &mut dyn FnMut(&str) -> bool,
    ) -> Result<ToolResult, String>;
    fn failures(&mut self) -> Vec<String>;
    fn finish(&mut self) -> Result<(), String>;
}

pub struct ToolResult {
    pub text: String,
    pub images: Vec<(String, Vec<u8>)>,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }
}

pub type Open =
    Arc<dyn Fn(&Request, &Path, &str) -> Result<Box<dyn Session>, String> + Send + Sync>;

#[derive(Clone)]
pub struct Launcher(pub Open);
impl std::fmt::Debug for Launcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Aero")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allow,
    Approve,
    Deny,
}

pub fn permission(access: Access, tool: &str, op: &str) -> Permission {
    use Permission::*;
    match tool {
        "project" | "docs" => Allow,
        "look" if op != "effect_lab" || access != Access::Editor => Allow,
        _ if access == Access::Editor => Deny,
        "extension" if matches!(op, "publish" | "remove") => Approve,
        "panel" if matches!(op, "begin" | "build" | "interact") => Approve,
        "undo" if op == "extension" => Approve,
        "extension" | "edit" | "files" | "panel" | "undo" | "plan" | "review" => Allow,
        _ => Deny,
    }
}

const INSTRUCTIONS: &str = include_str!("aero/instructions.md");
const INPUT_SCHEMA: &str = r#"{"type":"object","properties":{"op":{"type":"string"},"revision":{"type":"integer"},"args":{"type":"object"}},"required":["op"],"additionalProperties":false}"#;

fn tools(access: Access) -> Vec<Tool<'static>> {
    [
        (
            "project",
            "Inspect project: summary, timeline, layer, media, markers.",
            false,
        ),
        (
            "edit",
            "Edit at a named revision: commands, strips, video, 3d, selection.",
            true,
        ),
        (
            "look",
            "Render real pixels: frames, sample, contact_sheet, waveform, panel, effect_lab.",
            false,
        ),
        (
            "files",
            "Access extension stage: list, grep, read, edit, write, discard.",
            true,
        ),
        (
            "extension",
            "Check, publish, fork, rebase or remove an extension.",
            true,
        ),
        (
            "panel",
            "Open, read, interact, capture, begin or build a native panel.",
            true,
        ),
        (
            "undo",
            "Restore the session's edits or one extension change set.",
            true,
        ),
        (
            "docs",
            "List topics or read an API-pack topic or sample by name.",
            false,
        ),
        (
            "plan",
            "List, add or verify a task whose check determines completion.",
            true,
        ),
        (
            "review",
            "Send result, captures and rubric to an independent reviewer.",
            false,
        ),
    ]
    .into_iter()
    .filter(|(name, _, _)| access != Access::Editor || ["project", "look", "docs"].contains(name))
    .map(|(name, description, writes)| Tool {
        name,
        description,
        input_schema_json: INPUT_SCHEMA,
        idempotent: !writes,
        writes,
    })
    .collect()
}

pub struct Options {
    pub model: Model,
    pub wall: Duration,
    pub steps: u32,
    pub continuations: u32,
    pub skip_gate: bool,
    pub skip_topics: bool,
    pub skip_review: bool,
    pub report: Option<Report>,
}

impl Options {
    pub fn development() -> Self {
        Self {
            model: Model::fake(model::demo),
            wall: Duration::from_secs(120),
            steps: 32,
            continuations: 3,
            skip_gate: false,
            skip_topics: false,
            skip_review: false,
            report: None,
        }
    }
}

pub fn start(
    request: Request,
    session_id: String,
    scope: PathBuf,
    options: Options,
    open: impl FnOnce(&Path) -> Result<Box<dyn Session>, String> + Send + 'static,
) -> Handle {
    let (handle, actions, events) = transport::worker(Surface::Aero);
    thread::spawn(move || {
        let run_id = request.run.clone();
        let access = request.access;
        let report = options.report.clone();
        let outcome = drive(
            request,
            &session_id,
            &scope,
            options,
            &actions,
            &events,
            open,
        );
        let (reason, text) = match outcome {
            Ok(outcome) => outcome,
            Err(error) => ("error", error),
        };
        if let Some(report) = report {
            report(json!({"result":{"runId":run_id,"finalText":text,
                "completed":reason == "completed","reason":reason}}));
        }
        let result = if reason == "completed" {
            RunResult::success(text, access)
        } else {
            RunResult::failure(text, reason == "cancelled")
        };
        let _ = events.send(Event::Completed {
            run: run_id,
            result,
        });
        let _ = events.send(Event::Stopped("Aero's turn ended.".into()));
    });
    handle
}

type End = (&'static str, String);

enum Callback {
    Tool(ToolLease),
    Append(journal::Append),
    PreTool { question: u64, name: String },
    Stop { question: u64, continuations: u32 },
}

struct Mailbox(mpsc::Sender<Callback>);
impl Host for Mailbox {
    fn tool_call(&self, tool: ToolLease) {
        let _ = self.0.send(Callback::Tool(tool));
    }
    fn hook(&self, hook: Hook<'_>) -> bool {
        let ask = match hook.kind {
            HookKind::PreTool => Callback::PreTool {
                question: hook.question,
                name: hook.name.into(),
            },
            HookKind::Stop => Callback::Stop {
                question: hook.question,
                continuations: hook.continuations,
            },
        };
        self.0.send(ask).is_ok()
    }
    fn journaled(&self) -> bool {
        true
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        let _ = self.0.send(Callback::Append(journal::Append {
            id: append.append,
            start: append.first_seq,
            end: append.last_seq,
            events: append.events_json.into(),
        }));
    }
}

struct Approval {
    item: String,
    title: String,
    answer: SyncSender<bool>,
}
type WorkerAnswer = (ToolLease, Result<ToolResult, String>);

enum Work {
    Tool(ToolLease),
    Check,
}

fn wait_open(runtime: &mut Runtime, deadline: Instant) -> Result<(), String> {
    let remote = runtime.remote();
    while Instant::now() < deadline {
        // Opening with an effort looks the model up in the provider's
        // catalog. A 404 makes the core use its built-in capabilities.
        if let Some(fetch) = runtime.fetches.take().map_err(|e| e.to_string())? {
            if fetch.purpose != pm_aero_core::Purpose::Catalog {
                return Err("Aero's core asked for the model before its session opened.".into());
            }
            model::refuse_catalog(remote, fetch.fetch);
        }
        for event in runtime.events.drain().map_err(|e| e.to_string())? {
            match event {
                pm_aero_core::Event::SessionOpened { .. } => return Ok(()),
                pm_aero_core::Event::RequestDone {
                    message,
                    outcome: Err(error),
                    ..
                } => {
                    return Err(format!(
                        "Aero couldn't open the session: {error}. {message}"
                    ));
                }
                pm_aero_core::Event::Exited { message, .. } => return Err(message.into()),
                _ => {}
            }
        }
        wait_ready(runtime);
    }
    Err("Aero timed out opening the session.".into())
}

fn wait_ready(runtime: &Runtime) {
    use std::os::fd::AsRawFd;
    let Some(fd) = runtime.ready_fd() else {
        return;
    };
    let mut poll = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let mut bytes = [0u8; 256];
    // SAFETY: the runtime owns this nonblocking readiness fd and both buffers
    // are valid for their calls. One read suffices before each drain.
    unsafe {
        if libc::poll(&mut poll, 1, 4) > 0 {
            libc::read(poll.fd, bytes.as_mut_ptr().cast(), bytes.len());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    request: Request,
    id: &str,
    scope: &Path,
    options: Options,
    actions: &Receiver<transport::Queued>,
    events: &SyncSender<Event>,
    open: impl FnOnce(&Path) -> Result<Box<dyn Session>, String>,
) -> Result<End, String> {
    use pm_aero_core::Event as Core;
    let err = |e: pm_aero_core::Error| e.to_string();
    let run = request.run;
    let deadline = Instant::now() + options.wall;
    let mut journal = journal::Journal::open(scope)?;
    let mut session = open(scope)?;
    let context = session.context()?;
    let instructions = if options.skip_topics {
        INSTRUCTIONS
            .lines()
            .filter(|line| !line.starts_with("Read ") && !line.starts_with("Consult docs"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        INSTRUCTIONS.into()
    };
    let registry = tools(request.access);
    let (tx, callbacks) = mpsc::channel();
    let mut runtime = Runtime::create(
        &Config {
            model: request.model.as_deref().unwrap_or("fake/model"),
            effort: request.effort.as_deref().unwrap_or(""),
            endpoint_url: options.model.endpoint(),
            instructions: &instructions,
            workspace_root: scope.to_str().ok_or("Aero needs a UTF-8 session path.")?,
            home: scope.to_str().unwrap(),
            tools: &registry,
            max_agent_steps: options.steps,
            max_concurrent_tools: 1,
            max_stop_continuations: options.continuations,
            ..Config::default()
        },
        Arc::new(Mailbox(tx)),
    )
    .map_err(err)?;
    journal.open_core(&mut runtime, id)?;
    wait_open(&mut runtime, deadline)?;
    let emit = |event| {
        events
            .send(event)
            .map_err(|_| "Aero's run was closed.".to_owned())
    };
    let trace = |value| {
        emit(Event::Trace {
            run: run.clone(),
            value,
        })
    };
    let report = |value| {
        if let Some(report) = &options.report {
            report(value)
        }
    };
    let (tool_tx, tool_rx) = mpsc::channel::<Work>();
    let (answer_tx, answers) = mpsc::channel::<WorkerAnswer>();
    let (question_tx, questions) = mpsc::channel::<Approval>();
    let (checks_tx, checks) = mpsc::channel();
    let skip_gate = options.skip_gate;
    let skip_review = options.skip_review;
    let model = options.model.clone();
    let child_dir = scope.join("review");
    let session_worker = thread::spawn(move || {
        use sha2::{Digest, Sha256};
        let mut captures = BTreeMap::new();
        while let Ok(work) = tool_rx.recv() {
            let Work::Tool(tool) = work else {
                if checks_tx
                    .send(if skip_gate {
                        Vec::new()
                    } else {
                        session.failures()
                    })
                    .is_err()
                {
                    break;
                }
                continue;
            };
            let name = tool.name();
            let item = format!("aero-approval-{}", tool.call());
            let mut approve = |title: &str| {
                let (answer, rx) = mpsc::sync_channel(1);
                question_tx
                    .send(Approval {
                        item: item.clone(),
                        title: title.into(),
                        answer,
                    })
                    .is_ok()
                    && rx.recv().unwrap_or(false)
            };
            let result = if name == "review" {
                if skip_review {
                    Err("Independent review is disabled for this run.".into())
                } else {
                    model::review(&model, &child_dir, tool.input_json(), &captures)
                }
            } else {
                session.call(name, tool.input_json(), &mut approve)
            };
            let result = result.map(|mut result| {
                for (mime, bytes) in &result.images {
                    let id = format!("{:x}", Sha256::digest(bytes));
                    if captures.len() < 16 {
                        captures
                            .entry(id.clone())
                            .or_insert_with(|| (mime.clone(), bytes.clone()));
                        result.text.push_str(&format!("\nCapture id {id}"));
                    }
                }
                result
            });
            if answer_tx.send((tool, result)).is_err() {
                break;
            }
        }
        session.finish().map(|()| session)
    });
    emit(Event::Ready)?;
    emit(Event::Session {
        run: run.clone(),
        id: id.into(),
    })?;
    let prompt = journal.context_prompt(&context, &request.prompt)?;
    let id_prompt = format!("{run}-prompt");
    journal.store_input(&id_prompt, &prompt)?;
    let mut turn = runtime.prompt(&prompt, &id_prompt).map_err(err)?;
    let mut steer_count = 0u64;
    let mut steers = BTreeMap::new();
    let mut text = String::new();
    let mut diagnostic = None;
    let mut approval: Option<Approval> = None;
    let mut allowed = HashSet::new();
    let mut opened = HashSet::new();
    let mut stopped = None;
    let mut settled = None;
    let mut stop_hook = None;
    let mut hook_checked = false;
    let mut gate_exhausted = None;
    let mut outputs = BTreeMap::new();
    let mut continuations = 0;
    let mut fetch_workers = Vec::new();
    let remote = runtime.remote();
    let mut outcome = None;
    while outcome.is_none() {
        loop {
            let mut disconnected = false;
            let action = match actions.try_recv() {
                Ok((action, reply)) => Some((action, reply)),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    None
                }
            };
            if disconnected {
                stopped.get_or_insert("cancelled");
            }
            let Some((action, reply)) = action else {
                break;
            };
            let accepted = match action {
                Action::Cancel(target) if target == run => {
                    stopped.get_or_insert("cancelled");
                    true
                }
                Action::Shutdown => {
                    stopped.get_or_insert("cancelled");
                    true
                }
                Action::Steer {
                    run: target,
                    prompt,
                    images,
                } if target == run && stopped.is_none() && images.is_empty() => {
                    steer_count += 1;
                    let id = format!("{run}-steer-{steer_count}");
                    journal.store_input(&id, &prompt)?;
                    if matches!(
                        runtime.steer(turn, &prompt, &id).map_err(err)?,
                        pm_aero_core::Steered::Accepted(_)
                    ) {
                        steers.insert(id, prompt);
                        true
                    } else {
                        false
                    }
                }
                Action::Answer {
                    run: target,
                    item,
                    answers,
                } if target == run
                    && approval
                        .as_ref()
                        .is_some_and(|question| question.item == item) =>
                {
                    let question = approval.take().unwrap();
                    let choices = answers.get("decision").map_or(&[][..], Vec::as_slice);
                    let always = choices.iter().any(|choice| choice == "Allow for this run");
                    let yes = always || choices.iter().any(|choice| choice == "Allow once");
                    if always {
                        allowed.insert(question.title);
                    }
                    let _ = question.answer.try_send(yes);
                    trace(json!({"kind":"question-closed","itemId":item}))?;
                    true
                }
                _ => false,
            };
            if let Some(reply) = reply {
                let _ = reply.try_send(accepted);
            }
        }
        if Instant::now() >= deadline {
            stopped.get_or_insert("deadline");
        }
        if stopped.is_some() {
            remote.cancel(turn).map_err(err)?;
            if let Some(question) = approval.take() {
                let _ = question.answer.try_send(false);
            }
        }
        for callback in callbacks.try_iter() {
            match callback {
                Callback::Tool(tool) if stopped.is_none() => {
                    tool_tx
                        .send(Work::Tool(tool))
                        .map_err(|_| "Aero's tool worker stopped.")?;
                }
                Callback::Tool(tool) => {
                    let _ = tool.answer("The turn was stopped.", true);
                }
                Callback::Append(append) => journal.append(remote, append)?,
                Callback::PreTool { question, name } => {
                    let answer =
                        if registry.iter().any(|tool| tool.name == name) && stopped.is_none() {
                            HookAnswer::Allow
                        } else {
                            HookAnswer::Block(
                                "This tool is outside this run's Access or the turn was cancelled.",
                            )
                        };
                    remote.hook_answer(question, answer).map_err(err)?;
                }
                Callback::Stop {
                    question,
                    continuations,
                } => {
                    hook_checked = true;
                    if skip_gate || stopped.is_some() {
                        remote
                            .hook_answer(question, HookAnswer::Allow)
                            .map_err(err)?;
                    } else {
                        stop_hook = Some((question, continuations));
                        tool_tx
                            .send(Work::Check)
                            .map_err(|_| "Aero's completion check worker stopped.")?;
                    }
                }
            }
        }
        for question in questions.try_iter() {
            if stopped.is_some() || options.report.is_some() {
                let _ = question
                    .answer
                    .try_send(stopped.is_none() && options.report.is_some());
            } else if allowed.contains(&question.title) {
                let _ = question.answer.try_send(true);
            } else {
                trace(approvals::question_card(
                    &question.item,
                    &question.title,
                    "",
                    "",
                ))?;
                approval = Some(question);
            }
        }
        for (call, result) in answers.try_iter() {
            let (result, failed) = match result {
                Ok(value) => (value, false),
                Err(error) => (ToolResult::text(error), true),
            };
            let image_refs: Vec<_> = result
                .images
                .iter()
                .map(|(mime, data)| pm_aero_core::Image { mime, data })
                .collect();
            let id = format!("{}/{}", call.turn(), call.model_call_id());
            outputs.insert(id, crate::trace::truncate(&result.text, 1000));
            call.answer_images_or_text(&result.text, &image_refs, failed)
                .map_err(err)?;
        }
        if let Some(fetch) = runtime.fetches.take().map_err(err)? {
            if fetch.purpose == pm_aero_core::Purpose::Catalog {
                model::refuse_catalog(remote, fetch.fetch);
            } else {
                let model = options.model.clone();
                let id = fetch.fetch;
                let url = fetch.url.to_owned();
                let body = fetch.body.to_vec();
                fetch_workers.retain(|worker: &thread::JoinHandle<()>| !worker.is_finished());
                fetch_workers.push(thread::spawn(move || {
                    model::deliver(&model, remote, id, &url, &body)
                }));
            }
        }
        let mut follow_up = None;
        for event in runtime.events.drain().map_err(err)? {
            match event {
                Core::TextDelta { text: delta, .. } => {
                    text.push_str(delta);
                    trace(json!({"kind":"answer","text":delta}))?;
                }
                Core::ThoughtDelta { text, .. } => trace(json!({"kind":"thought","text":text}))?,
                Core::ToolStart {
                    name,
                    model_call_id,
                    ..
                } => {
                    let id = format!("{turn}/{model_call_id}");
                    if opened.insert(id.clone()) {
                        report(json!({"type":"tool-started","callId":id,"name":name}));
                    }
                    trace(
                        json!({"kind":"tool-start","itemId":id,"toolName":name,"label":crate::trace::label(name)}),
                    )?;
                }
                Core::ToolEnd {
                    model_call_id,
                    failed,
                    ..
                } => {
                    let id = format!("{turn}/{model_call_id}");
                    if opened.contains(&id) {
                        report(json!({"type":"tool-finished","callId":id,"ok":!failed}));
                    }
                    trace(json!({"kind":"tool-end","itemId":id,"isError":failed,
                        "output":outputs.remove(&id)}))?;
                }
                Core::SteerReturned { message_id, .. } => {
                    follow_up = steers.remove_entry(message_id);
                }
                Core::SteerJoined { message_id, .. } => {
                    steers.remove(message_id);
                }
                Core::TurnEnd { reason, usage, .. } => {
                    report(json!({"type":"usage","input":usage.input_tokens,
                        "cached":usage.cache_read_tokens,"output":usage.output_tokens}));
                    outcome = Some(match (stopped, reason) {
                        (Some("cancelled"), _) | (_, StopReason::Cancelled) => {
                            ("cancelled", "Stopped.".into())
                        }
                        (Some("deadline"), _) => {
                            ("deadline", "Aero reached its wall-time limit.".into())
                        }
                        (_, StopReason::EndTurn) => gate_exhausted.take().map_or_else(
                            || ("completed", std::mem::take(&mut text)),
                            |text| ("gate-exhausted", text),
                        ),
                        (_, StopReason::MaxSteps) => {
                            ("step-limit", "Aero reached its tool step limit.".into())
                        }
                        _ => (
                            "error",
                            diagnostic
                                .take()
                                .unwrap_or_else(|| format!("Aero ended the turn: {reason:?}")),
                        ),
                    });
                }
                Core::Error { message, .. } => diagnostic = Some(message.to_owned()),
                Core::Exited { message, .. } => {
                    outcome.get_or_insert_with(|| {
                        ("error", format!("Aero's core stopped. {message}"))
                    });
                }
                _ => {}
            }
        }
        if let Some((message, next)) = follow_up.filter(|_| {
            stopped.is_none()
                && outcome
                    .as_ref()
                    .is_some_and(|(reason, _)| *reason == "completed")
        }) {
            text.clear();
            turn = runtime.prompt(&next, &message).map_err(err)?;
            outcome = None;
        }
        if outcome
            .as_ref()
            .is_some_and(|(reason, _)| *reason == "completed")
            && !skip_gate
            && !hook_checked
        {
            settled = outcome.take();
            tool_tx
                .send(Work::Check)
                .map_err(|_| "Aero's completion check worker stopped.")?;
        }
        if let Ok(failures) = checks.try_recv() {
            let prompt = || {
                format!(
                    "Completion checks failed. Repair the following issues before finishing:\n{}",
                    failures.join("\n")
                )
            };
            let not_done = || format!("Not done: {}", failures.join(" "));
            if let Some((question, count)) = stop_hook.take() {
                if failures.is_empty() {
                    remote
                        .hook_answer(question, HookAnswer::Allow)
                        .map_err(err)?;
                } else if count < options.continuations {
                    report(json!({"type":"gate-continuation","failures":failures}));
                    text.clear();
                    remote
                        .hook_answer(question, HookAnswer::Continue(&prompt()))
                        .map_err(err)?;
                } else {
                    gate_exhausted = Some(not_done());
                    remote
                        .hook_answer(question, HookAnswer::Allow)
                        .map_err(err)?;
                }
            } else if let Some(candidate) = settled.take() {
                // ABI 2a does not call Stop; maintain the visible completion
                // check with a bounded follow-up until ABI 2b is linked.
                if failures.is_empty() {
                    outcome = Some(candidate);
                } else if continuations < options.continuations && stopped.is_none() {
                    continuations += 1;
                    report(json!({"type":"gate-continuation","failures":failures}));
                    emit(Event::Progress {
                        run: run.clone(),
                        text: "Checking and repairing the result…".into(),
                    })?;
                    let message = format!("{run}-gate-{continuations}");
                    let prompt = prompt();
                    journal.store_input(&message, &prompt)?;
                    turn = runtime.prompt(&prompt, &message).map_err(err)?;
                } else {
                    outcome = Some(("gate-exhausted", not_done()));
                }
            }
        }
        if let Some(reason) = stopped {
            outcome = Some((
                reason,
                if reason == "cancelled" {
                    "Stopped.".into()
                } else {
                    "Aero reached its wall-time limit.".into()
                },
            ));
        }
        if outcome.is_none() {
            wait_ready(&runtime);
        }
    }
    drop(tool_tx);
    drop(answers);
    drop(questions);
    drop(callbacks);
    let outcome = outcome.unwrap();
    if matches!(outcome.0, "cancelled" | "deadline") {
        // Dropping forwards teardown to a worker, allowing an in-flight
        // tool lease to retire without blocking the caller's cancel result.
        drop(runtime);
    } else {
        runtime.close();
        let _ = session_worker
            .join()
            .map_err(|_| "Aero's tool worker panicked.")??;
    }
    Ok(outcome)
}
