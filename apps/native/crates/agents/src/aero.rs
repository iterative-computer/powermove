//! In-process Aero transport. The runtime worker owns every libfx call.
use crate::{
    Access, Event, RunResult, approvals,
    delivery::Surface,
    launch::Request,
    transport::{self, Action, Handle},
};
use pm_aero_core::{
    Config, HookAnswer, HookAnswerError, HookKind, HookKinds, HookLease, Host, JournalAppend,
    OpenArgs, Runtime, StopReason, Tool, ToolAnswerError, ToolLease,
    journal::{JournalWriter, SessionFolder, StoreError},
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
    Hook(HookLease),
}

/// The core's callbacks only enqueue (header rule): the drive loop answers.
struct Mailbox {
    calls: mpsc::Sender<Callback>,
    journal: Arc<JournalWriter>,
}
impl Host for Mailbox {
    fn tool_call(&self, tool: ToolLease) {
        let _ = self.calls.send(Callback::Tool(tool));
    }
    fn journaled(&self) -> bool {
        true
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        self.journal.enqueue(&append);
    }
    fn hook_kinds(&self) -> HookKinds {
        HookKinds {
            pre_tool: true,
            stop: true,
        }
    }
    fn hook(&self, question: HookLease) {
        let _ = self.calls.send(Callback::Hook(question));
    }
}

/// A busy or out-of-memory core changed nothing: answer again shortly. Any
/// other refusal drops the lease, and the wrapper's settler sends the
/// fallback (a blocked call, an ended turn), so the core is never stranded.
const ANSWER_TRIES: u32 = 200;

fn retryable(error: pm_aero_core::Error) -> bool {
    matches!(
        error,
        pm_aero_core::Error::Busy | pm_aero_core::Error::OutOfMemory
    )
}

fn answer_hook(mut question: HookLease, answer: HookAnswer<'_>) -> Result<(), String> {
    for _ in 0..ANSWER_TRIES {
        match question.answer(answer) {
            Ok(_) => return Ok(()),
            Err(HookAnswerError::Open(lease, error)) if retryable(error) => {
                question = lease;
                thread::sleep(Duration::from_millis(2));
            }
            Err(HookAnswerError::Open(..)) => return Ok(()),
            Err(HookAnswerError::Spent(error)) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn answer_tool(
    mut call: ToolLease,
    text: &str,
    images: &[pm_aero_core::Image<'_>],
    failed: bool,
) -> Result<(), String> {
    for _ in 0..ANSWER_TRIES {
        match call.answer_with(text, images, failed) {
            // A malformed image was answered with an error the model reads.
            Ok(_) | Err(ToolAnswerError::Spent(pm_aero_core::Error::Invalid)) => return Ok(()),
            Err(ToolAnswerError::Open(lease, error)) if retryable(error) => {
                call = lease;
                thread::sleep(Duration::from_millis(2));
            }
            Err(ToolAnswerError::Open(..)) => return Ok(()),
            Err(ToolAnswerError::Spent(error)) => return Err(error.to_string()),
        }
    }
    Ok(())
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

/// Waits for the session `request` opened; true when a turn the last host
/// left open can resume.
fn wait_open(
    runtime: &mut Runtime,
    request: u64,
    (model, effort): (&str, &str),
    deadline: Instant,
) -> Result<bool, String> {
    let remote = runtime.remote();
    while Instant::now() < deadline {
        // Opening looks the model up in the provider's catalog.
        if let Some(fetch) = runtime.fetches.take().map_err(|e| e.to_string())? {
            if fetch.purpose != pm_aero_core::Purpose::Catalog {
                return Err("Aero's core asked for the model before its session opened.".into());
            }
            model::answer_catalog(remote, fetch.fetch, model, effort);
        }
        for event in runtime.events.drain().map_err(|e| e.to_string())? {
            match event {
                pm_aero_core::Event::SessionOpened {
                    request: opened,
                    resumable,
                    ..
                } if opened == request => return Ok(resumable),
                pm_aero_core::Event::ImagesNeeded { .. } => {
                    return Err("Aero's saved session names images it no longer has.".into());
                }
                pm_aero_core::Event::RequestDone {
                    request: done,
                    message,
                    outcome: Err(error),
                } if done == request => {
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
    let journal = journal::Journal::new(scope);
    let folder = SessionFolder::open(scope).map_err(|error| match error {
        StoreError::Fenced => "This Aero session is already running elsewhere.".to_owned(),
        StoreError::TooLarge => {
            "This Aero session's history is too large to reopen. Start a new thread.".to_owned()
        }
        StoreError::Corrupt(detail) => {
            format!("This Aero session's saved history is damaged ({detail}). Start a new thread.")
        }
        StoreError::Io(error) => format!("Aero's session storage failed: {error}"),
    })?;
    let writer = JournalWriter::start(folder).map_err(|e| e.to_string())?;
    let stored = SessionFolder::load(scope).map_err(|e| e.to_string())?;
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
    let model_name = request.model.as_deref().unwrap_or("fake/model");
    let effort = request.effort.as_deref().unwrap_or("");
    let mut runtime = Runtime::create(
        &Config {
            model: model_name,
            effort,
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
        Arc::new(Mailbox {
            calls: tx,
            journal: writer.clone(),
        }),
    )
    .map_err(err)?;
    writer.bind(runtime.remote());
    let opening = runtime
        .open(&OpenArgs {
            session_id: if stored.journal_json.is_empty() {
                id
            } else {
                ""
            },
            journal_json: &stored.journal_json,
            checkpoint: &stored.checkpoint,
            inputs_durable: true,
            image_store: false,
        })
        .map_err(err)?;
    let resumable = wait_open(&mut runtime, opening, (model_name, effort), deadline)?;
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
    // The same run launched again (a retry after a crash) is the open turn.
    let retried = journal.has_input(&id_prompt);
    journal.store_input(&id_prompt, &prompt)?;
    // A turn the last host left open (a crash, a kill) finishes first; the
    // core tells the model about a call that may have half-run.
    let mut queued = None;
    let mut turn = if resumable {
        emit(Event::Progress {
            run: run.clone(),
            text: "Resuming the interrupted turn…".into(),
        })?;
        if !retried {
            queued = Some((id_prompt, prompt));
        }
        runtime.resume().map_err(err)?
    } else {
        runtime.prompt(&prompt, &id_prompt).map_err(err)?
    };
    let mut steer_count = 0u64;
    let mut steers = BTreeMap::new();
    let mut text = String::new();
    let mut diagnostic = None;
    let mut approval: Option<Approval> = None;
    let mut allowed = HashSet::new();
    let mut opened = HashSet::new();
    let mut stopped = None;
    let mut stop_hook: Option<HookLease> = None;
    let mut gate_exhausted = None;
    let mut outputs = BTreeMap::new();
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
                Callback::Hook(question) if question.kind() == HookKind::PreTool => {
                    // Access is enforced here, deterministically, not in the prompt.
                    let answer = if stopped.is_some() {
                        HookAnswer::Block("The turn was stopped.")
                    } else if registry.iter().any(|tool| tool.name == question.name()) {
                        HookAnswer::Allow
                    } else {
                        HookAnswer::Block(
                            "This tool is outside this run's Access. Ask the user to change Access.",
                        )
                    };
                    answer_hook(question, answer)?;
                }
                Callback::Hook(question) => {
                    if skip_gate || stopped.is_some() {
                        answer_hook(question, HookAnswer::Allow)?;
                    } else {
                        stop_hook = Some(question);
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
            answer_tool(call, &result.text, &image_refs, failed)?;
        }
        if let Some(fetch) = runtime.fetches.take().map_err(err)? {
            if fetch.purpose == pm_aero_core::Purpose::Catalog {
                model::answer_catalog(remote, fetch.fetch, model_name, effort);
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
        let completed = stopped.is_none()
            && outcome
                .as_ref()
                .is_some_and(|(reason, _)| *reason == "completed");
        // A returned steer, or the person's prompt after a resumed turn.
        let next = if completed {
            follow_up.or_else(|| queued.take())
        } else {
            None
        };
        if let Some((message, next)) = next {
            text.clear();
            turn = runtime.prompt(&next, &message).map_err(err)?;
            outcome = None;
        }
        if let Ok(failures) = checks.try_recv() {
            let prompt = || {
                format!(
                    "Completion checks failed. Repair the following issues before finishing:\n{}",
                    failures.join("\n")
                )
            };
            let not_done = || format!("Not done: {}", failures.join(" "));
            if let Some(question) = stop_hook.take() {
                if failures.is_empty() {
                    answer_hook(question, HookAnswer::Allow)?;
                } else if question.continuations() < options.continuations {
                    report(json!({"type":"gate-continuation","failures":failures}));
                    emit(Event::Progress {
                        run: run.clone(),
                        text: "Checking and repairing the result…".into(),
                    })?;
                    text.clear();
                    answer_hook(question, HookAnswer::Continue(&prompt()))?;
                } else {
                    // The core asks once more after the last continuation;
                    // that ask can only end the turn.
                    gate_exhausted = Some(not_done());
                    answer_hook(question, HookAnswer::Allow)?;
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
