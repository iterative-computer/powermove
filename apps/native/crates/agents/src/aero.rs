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
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

mod journal;
mod model;

pub use model::{Live, Model, Reply, evaluate};

pub type Report = Arc<dyn Fn(Value) + Send + Sync>;

pub trait Session: Send {
    fn context(&mut self) -> Result<Value, String>;
    fn call(
        &mut self,
        tool: &str,
        input: &str,
        approve: &mut dyn FnMut(&str) -> bool,
    ) -> Result<ToolResult, String>;
    fn failures(&mut self) -> Vec<Failure>;
    fn finish(&mut self) -> Result<(), String>;
    /// The flag Stop sets: the session starts no new mutation once it is set.
    fn halt(&mut self, _flag: Arc<AtomicBool>) {}
    /// A fresh prompt starts in this session (a queued one after a finished
    /// turn): what the last turn's own edits introduced is no longer this
    /// turn's to repair. Gate continuations are the same turn.
    fn begin(&mut self) {}
}

/// Why the completion gate still fails. The person is told only `kind`,
/// counted; `detail` goes to the model, never to the panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub kind: Failing,
    pub detail: String,
}

/// The fixed categories a person sees when the gate gives up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Failing {
    HiddenLayer,
    BlankFrames,
    ProjectProblem,
    PlanCheck,
    Drafts,
    Extension,
    Unchecked,
}

impl Failing {
    fn named(self, count: usize) -> &'static str {
        let (one, many) = match self {
            Failing::HiddenLayer => ("hidden layer", "hidden layers"),
            Failing::BlankFrames => ("blank range", "blank ranges"),
            Failing::ProjectProblem => ("project problem", "project problems"),
            Failing::PlanCheck => ("plan check", "plan checks"),
            Failing::Drafts => ("unpublished draft", "unpublished drafts"),
            Failing::Extension => ("extension that didn't load", "extensions that didn't load"),
            Failing::Unchecked => ("check that couldn't run", "checks that couldn't run"),
        };
        if count == 1 { one } else { many }
    }
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
        f.write_str("Aerolite")
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

    /// A turn on a real model, as a person runs it: long edits with many tool
    /// calls are normal, so the limits are generous. A silent stream still stops
    /// after five minutes (the edge's idle limit), and Stop always works.
    pub fn live(model: Model) -> Self {
        Self {
            model,
            wall: Duration::from_secs(60 * 60),
            steps: 400,
            ..Self::development()
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
        let _ = events.send(Event::Stopped("Aerolite's turn ended.".into()));
    });
    handle
}

type End = (&'static str, String);

/// The core's error for a failed model reply, in words a person can act on:
/// it names Zig error codes (`IncompleteStream`), which the panel would
/// otherwise show as they are. Messages already in words pass through.
fn plain(message: &str) -> String {
    let code = message.trim();
    if code.is_empty() || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return message.to_owned();
    }
    match code {
        "IncompleteStream" | "StreamClosed" | "ReadFailed" | "HostStreamFailed" => {
            "The connection to the model dropped before its reply finished. Try again.".into()
        }
        "ContentFiltered" => "The provider's content filter stopped this reply.".into(),
        "Refused" => "The model refused this request.".into(),
        _ => format!(
            "The model's reply couldn't be used ({code}). Try again, or choose another model in the picker."
        ),
    }
}

/// The end of a turn whose completion checks still fail after the last
/// repair round, for the panel and Details: a fixed sentence with counts
/// of fixed categories. No check text, layer name or plan wording reaches
/// it, so nothing a project or model wrote can.
fn unfinished(failures: &[Failure]) -> String {
    let mut counts = BTreeMap::new();
    for failure in failures {
        *counts.entry(failure.kind).or_insert(0usize) += 1;
    }
    let kinds = counts
        .iter()
        .map(|(kind, count)| format!("{count} {}", kind.named(*count)))
        .collect::<Vec<_>>()
        .join(", ");
    let total = failures.len();
    format!(
        "Aerolite stopped before its checks passed: {total} {} ({kinds}). Review the result before you continue.",
        if total == 1 {
            "problem remains"
        } else {
            "problems remain"
        }
    )
}

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
/// How long Stop waits for an admitted tool call and the turn's stored end.
const SETTLE: Duration = Duration::from_secs(15);

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
    Begin,
}

/// What a session open found: a turn the last host left open, and the
/// inputs queued behind it (message id, text), in order.
#[derive(Default)]
struct Opened {
    open_turn: Option<String>,
    follow_ups: Vec<(String, String)>,
}

/// Waits for the session `request` to open.
fn wait_open(
    runtime: &mut Runtime,
    edge: &Model,
    request: u64,
    (model, effort): (&str, &str),
    deadline: Instant,
) -> Result<Opened, String> {
    let remote = runtime.remote();
    let mut opened = Opened::default();
    while Instant::now() < deadline {
        // Opening looks the model up in the provider's catalog.
        if let Some(fetch) = runtime.fetches.take().map_err(|e| e.to_string())? {
            if fetch.purpose != pm_aero_core::Purpose::Catalog {
                return Err("Aero's core asked for the model before its session opened.".into());
            }
            model::catalog(edge, remote, fetch.fetch, fetch.url, model, effort);
        }
        for event in runtime.events.drain().map_err(|e| e.to_string())? {
            match event {
                pm_aero_core::Event::Recovery {
                    request: r,
                    kind,
                    text,
                    text2,
                    ..
                } if r == request => match kind {
                    pm_aero_core::sys::FX_RECOVERY_OPEN_TURN => {
                        opened.open_turn = Some(text.into());
                    }
                    pm_aero_core::sys::FX_RECOVERY_FOLLOW_UP => {
                        opened.follow_ups.push((text2.into(), text.into()));
                    }
                    _ => {}
                },
                pm_aero_core::Event::SessionOpened {
                    request: r,
                    resumable,
                    ..
                } if r == request => {
                    if !resumable {
                        opened.open_turn = None;
                    }
                    return Ok(opened);
                }
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
    // The thread's previous turn may still be letting go of the folder.
    let opened_folder = {
        let settle = Instant::now() + Duration::from_secs(2);
        loop {
            match SessionFolder::open(scope) {
                Err(StoreError::Fenced) if Instant::now() < settle => {
                    thread::sleep(Duration::from_millis(10))
                }
                other => break other,
            }
        }
    };
    let folder = opened_folder.map_err(|error| match error {
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
    let halt = Arc::new(AtomicBool::new(false));
    session.halt(halt.clone());
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
    let model_name = options.model.name(request.model.as_deref().unwrap_or("fake/model"));
    let effort = request.effort.as_deref().unwrap_or("");
    let mut runtime = Runtime::create(
        &Config {
            provider: options.model.provider(),
            configured_providers_json: options.model.configured(),
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
    let opened = wait_open(&mut runtime, &options.model, opening, (model_name, effort), deadline)?;
    // A turn the person stopped is never resumed, even when the host went
    // away before the core stored its end.
    let resumable = opened
        .open_turn
        .as_deref()
        .is_some_and(|turn| journal.stopped().as_deref() != Some(turn));
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
    let halted = halt.clone();
    let session_worker = thread::spawn(move || {
        use sha2::{Digest, Sha256};
        let mut captures = BTreeMap::new();
        while let Ok(work) = tool_rx.recv() {
            let tool = match work {
                Work::Tool(tool) => tool,
                Work::Begin => {
                    session.begin();
                    continue;
                }
                Work::Check => {
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
                }
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
            let result = if halted.load(Ordering::Acquire) {
                // Stop came after the call was dispatched: it never starts.
                Err("The turn was stopped before this tool ran.".into())
            } else if model.fenced() {
                // The Powermove account changed: nothing more of this turn runs.
                halted.store(true, Ordering::Release);
                Err(crate::inference::CHANGED.into())
            } else if name == "review" {
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
    // Inputs waiting to run, oldest first: what the core recovered, then what
    // this host kept (returned steers), then this run's prompt. Each runs
    // from its stored copy; none is dropped.
    let mut pending: VecDeque<(String, String)> = VecDeque::new();
    for (message, text) in opened.follow_ups.into_iter().chain(journal.pending()?) {
        if !pending.iter().any(|(id, _)| *id == message) {
            let text = journal.input(&message).unwrap_or(text);
            pending.push_back((message, text));
        }
    }
    let mut current;
    let mut turn = if resumable {
        // A turn the last host left open (a crash, a kill) finishes first;
        // the core tells the model about a call that may have half-run.
        emit(Event::Progress {
            run: run.clone(),
            text: "Resuming the interrupted turn…".into(),
        })?;
        if !retried {
            pending.push_back((id_prompt, prompt));
        }
        current = opened.open_turn.unwrap_or_default();
        runtime.resume().map_err(err)?
    } else {
        pending.push_back((id_prompt, prompt));
        let (message, text) = pending.pop_front().expect("this run's prompt");
        let turn = runtime.prompt(&text, &message).map_err(err)?;
        current = message;
        turn
    };
    journal.save_pending(&pending)?;
    let mut steer_count = 0u64;
    // Accepted steers not yet settled, and the requests that answer them.
    let mut steers: BTreeMap<String, String> = BTreeMap::new();
    let mut steer_requests: HashMap<u64, String> = HashMap::new();
    let mut in_flight = 0usize;
    let mut cancel_sent = false;
    let mut settle_by: Option<Instant> = None;
    let settled;
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
    loop {
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
                    match runtime.steer(turn, &prompt, &id).map_err(err)? {
                        pm_aero_core::Steered::Accepted(request) => {
                            steer_requests.insert(request, id.clone());
                            steers.insert(id, prompt);
                        }
                        // Between turns: it runs as the next prompt.
                        pm_aero_core::Steered::Stale => {
                            pending.push_back((id, prompt));
                            journal.save_pending(&pending)?;
                        }
                    }
                    true
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
            if !cancel_sent {
                // Nothing new starts; the stopped turn is never resumed.
                halt.store(true, Ordering::Release);
                journal.mark_stopped(&current)?;
                match remote.cancel(turn) {
                    Ok(pm_aero_core::Outcome::Busy) => {}
                    Ok(_) => cancel_sent = true,
                    Err(pm_aero_core::Error::OutOfMemory) => {}
                    Err(error) => return Err(err(error)),
                }
            }
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
                    in_flight += 1;
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
            in_flight -= 1;
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
                model::catalog(&options.model, remote, fetch.fetch, fetch.url, model_name, effort);
            } else {
                let model = options.model.clone();
                let id = fetch.fetch;
                let url = fetch.url.to_owned();
                let headers: Vec<_> = fetch.headers().map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
                let body = fetch.body.to_vec();
                fetch_workers.retain(|worker: &thread::JoinHandle<()>| !worker.is_finished());
                fetch_workers.push(thread::spawn(move || {
                    model::deliver(&model, remote, id, &url, &headers, &body)
                }));
            }
        }
        let pending_before = pending.len();
        for event in runtime.events.drain().map_err(err)? {
            match event {
                // After a request failed in words the edge wrote, the core's own
                // account of it ("HTTP 529: …", an auth notice) is not the
                // reply; the turn ends with the edge's words (P1-6). A request
                // sent again clears them before it streams.
                Core::TextDelta { .. } if options.model.failure_pending() => {}
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
                    // Text before a tool call narrates that step (the panel
                    // already shows it); the turn's answer is what the model
                    // says after its last tool call, as with Codex and Claude.
                    text.clear();
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
                    if let Some(entry) = steers.remove_entry(message_id) {
                        pending.push_back(entry);
                    }
                }
                Core::SteerJoined { message_id, .. } => {
                    steers.remove(message_id);
                }
                // A steer the turn never read (FX_STALE, or an error) comes
                // back here; one that came back already answers FX_OK.
                Core::RequestDone {
                    request, outcome, ..
                } => {
                    if let Some(id) = steer_requests.remove(&request)
                        && !matches!(outcome, Ok(pm_aero_core::Outcome::Applied))
                        && let Some(entry) = steers.remove_entry(&id)
                    {
                        pending.push_back(entry);
                    }
                }
                Core::TurnEnd { reason, usage, .. } => {
                    report(json!({"type":"usage","input":usage.input_tokens,
                        "cached":usage.cache_read_tokens,"output":usage.output_tokens}));
                    outcome = Some(match (stopped, reason) {
                        (Some("cancelled"), _) | (_, StopReason::Cancelled) => {
                            ("cancelled", "Stopped.".into())
                        }
                        (Some("deadline"), _) => {
                            ("deadline", "Aerolite reached its time limit for one turn. Send a message to continue.".into())
                        }
                        (_, StopReason::EndTurn) => gate_exhausted.take().map_or_else(
                            || ("completed", std::mem::take(&mut text)),
                            |text| ("gate-exhausted", text),
                        ),
                        (_, StopReason::MaxSteps) => {
                            ("step-limit", "Aerolite reached its tool-step limit for one turn. Send a message to continue.".into())
                        }
                        // The edge's own words for a failed request (a
                        // dropped or silent stream) before the core's code.
                        _ => (
                            "error",
                            options
                                .model
                                .take_failure()
                                .or_else(|| diagnostic.take().map(|code: String| plain(&code)))
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
        // The next waiting input: a returned steer, a recovered follow-up,
        // or the person's prompt after a resumed turn.
        if completed && let Some((message, next)) = pending.pop_front() {
            text.clear();
            tool_tx
                .send(Work::Begin)
                .map_err(|_| "Aero's tool worker stopped.")?;
            turn = runtime.prompt(&next, &message).map_err(err)?;
            current = message;
            outcome = None;
        }
        if pending.len() != pending_before || completed {
            journal.save_pending(&pending)?;
        }
        if let Ok(failures) = checks.try_recv() {
            let details: Vec<_> = failures.iter().map(|f| f.detail.as_str()).collect();
            let prompt = || {
                format!(
                    "Completion checks failed. Your own work in this turn left these problems. Adjust your own edits within the request; if a fix would change something the person did not ask about, ask them instead:\n- {}",
                    details.join("\n- ")
                )
            };
            if let Some(question) = stop_hook.take() {
                if failures.is_empty() || stopped.is_some() {
                    answer_hook(question, HookAnswer::Allow)?;
                } else if question.continuations() < options.continuations {
                    report(json!({"type":"gate-continuation","failures":details}));
                    emit(Event::Progress {
                        run: run.clone(),
                        text: "Checking and repairing the result…".into(),
                    })?;
                    text.clear();
                    answer_hook(question, HookAnswer::Continue(&prompt()))?;
                } else {
                    // The core asks once more after the last continuation;
                    // that ask can only end the turn.
                    gate_exhausted = Some(unfinished(&failures));
                    answer_hook(question, HookAnswer::Allow)?;
                }
            }
        }
        if let Some(reason) = stopped {
            // Stop counts once the core has stored the turn's end and every
            // admitted tool call has settled; a stuck one is let go of.
            let by = *settle_by.get_or_insert_with(|| Instant::now() + SETTLE);
            let ended = outcome.is_some() && in_flight == 0;
            if ended || Instant::now() >= by {
                settled = ended;
                outcome = Some((
                    reason,
                    if reason == "cancelled" {
                        "Stopped.".into()
                    } else {
                        "Aerolite reached its time limit for one turn. Send a message to continue.".into()
                    },
                ));
                break;
            }
        } else if outcome.is_some() {
            settled = true;
            break;
        }
        wait_ready(&runtime);
    }
    journal.save_pending(&pending)?;
    drop(tool_tx);
    drop(answers);
    drop(questions);
    drop(callbacks);
    let outcome = outcome.expect("the loop ends with an outcome");
    if !settled {
        // A tool that never settled: dropping forwards teardown to a worker,
        // so its lease retires without blocking the caller's Stop.
        drop(runtime);
    } else {
        runtime.close();
        let _ = session_worker
            .join()
            .map_err(|_| "Aero's tool worker panicked.")??;
    }
    Ok(outcome)
}

#[cfg(test)]
mod limits_tests {
    use super::*;

    #[test]
    fn a_turn_on_a_real_model_is_not_held_to_the_development_limits() {
        let dev = Options::development();
        let live = Options::live(Model::fake(model::demo));
        assert_eq!((dev.wall, dev.steps), (Duration::from_secs(120), 32));
        assert!(live.wall >= Duration::from_secs(60 * 60), "{:?}", live.wall);
        assert!(live.steps >= 400, "{}", live.steps);
        assert_eq!(live.continuations, dev.continuations);
        assert!(!live.skip_gate && !live.skip_review);
    }
}
