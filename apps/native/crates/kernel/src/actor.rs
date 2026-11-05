//! The kernel actor (plan v6 §3.2): the single writer, on its own thread.
//!
//! * [`KernelActor::spawn`] starts the thread that owns the [`Kernel`].
//! * [`KernelHandle`] (cheap to clone, `Send + Sync`) is how every host
//!   reaches it. **Nothing a caller does waits on the kernel's work**:
//!   [`call`](KernelHandle::call) returns a [`Pending`] future,
//!   [`call_with`](KernelHandle::call_with) takes a callback run on the
//!   kernel thread (post it to your executor), and
//!   [`call_blocking`](KernelHandle::call_blocking) exists for worker
//!   threads and tests only: it panics on the main thread.
//! * After every call the actor **publishes** a copy-on-write
//!   [`Published`] per project (snapshot, revision, recent changes, history
//!   and selection summaries, the open-entry indicator, workspace) and then
//!   sends [`KernelEvent`]s. Reading [`KernelHandle::published`] takes a
//!   mutex for a pointer swap only, never while the kernel works.
//! * [`QueryMirror`] is the UI's **synchronous query mirror**: a
//!   [`QueryContext`] the UI thread owns and fast-forwards from published
//!   changes, so evaluated values, bounds and picks never cross threads.
//! * Realms register a mailbox ([`KernelHandle::register_realm`]) and
//!   receive [`RealmMessage`]s (run an external undo closure, cleanups).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::task::{Context, Poll, Waker};
use std::thread;

use pm_model::json::Value;

use crate::document::{Change, Snapshot};
use crate::journal::Recovery;
use crate::kernel::{Kernel, KernelConfig, KernelFailure, OpenInfo, OpenSource};
use crate::queries::QueryContext;
use crate::selection::Selection;
use crate::tx::{Ack, Notice, RealmMessage, Request, Writer};

// ── a small oneshot future (no runtime dependency) ─────────────────────────

struct Slot<T> {
    value: Option<T>,
    waker: Option<Waker>,
    closed: bool,
}

struct OneShot<T> {
    slot: Mutex<Slot<T>>,
    ready: Condvar,
}

/// The answer to a kernel call: a `Future`, or poll it with
/// [`try_take`](Pending::try_take). Never wait for it on the main thread.
pub struct Pending<T> {
    inner: Arc<OneShot<T>>,
}

struct Resolver<T> {
    inner: Arc<OneShot<T>>,
}

fn oneshot<T>() -> (Resolver<T>, Pending<T>) {
    let inner = Arc::new(OneShot {
        slot: Mutex::new(Slot {
            value: None,
            waker: None,
            closed: false,
        }),
        ready: Condvar::new(),
    });
    (
        Resolver {
            inner: inner.clone(),
        },
        Pending { inner },
    )
}

impl<T> Resolver<T> {
    fn resolve(self, value: T) {
        let waker = {
            let mut slot = self.inner.slot.lock().expect("oneshot");
            slot.value = Some(value);
            slot.waker.take()
        };
        self.inner.ready.notify_all();
        if let Some(w) = waker {
            w.wake();
        }
    }
}

impl<T> Drop for Resolver<T> {
    fn drop(&mut self) {
        let waker = {
            let mut slot = self.inner.slot.lock().expect("oneshot");
            slot.closed = true;
            slot.waker.take()
        };
        self.inner.ready.notify_all();
        if let Some(w) = waker {
            w.wake();
        }
    }
}

impl<T> Pending<T> {
    /// The answer, if it arrived.
    pub fn try_take(&self) -> Option<T> {
        self.inner.slot.lock().expect("oneshot").value.take()
    }

    /// Blocks until the answer arrives (`None`: the kernel stopped). Worker
    /// threads and tests only.
    pub fn wait(self) -> Option<T> {
        assert_not_main_thread();
        let mut slot = self.inner.slot.lock().expect("oneshot");
        loop {
            if let Some(v) = slot.value.take() {
                return Some(v);
            }
            if slot.closed {
                return None;
            }
            slot = self.inner.ready.wait(slot).expect("oneshot");
        }
    }
}

impl<T> Future for Pending<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut slot = self.inner.slot.lock().expect("oneshot");
        if let Some(v) = slot.value.take() {
            return Poll::Ready(Some(v));
        }
        if slot.closed {
            return Poll::Ready(None);
        }
        slot.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_main_np() -> std::ffi::c_int;
}

/// Kernel waits are asynchronous in every host (§3.2): a blocking wait on
/// the main thread is a bug, caught here.
fn assert_not_main_thread() {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: a libSystem query with no arguments.
        let main = unsafe { pthread_main_np() } == 1;
        assert!(
            !main,
            "never block the main thread on the kernel; await `Pending` instead"
        );
    }
}

// ── publication ─────────────────────────────────────────────────────────────

/// Undo menu state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistorySummary {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub entries: usize,
    pub applied: usize,
}

/// The open entry ("gesture open" indicator).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenEntryInfo {
    pub id: u64,
    pub label: String,
    pub depth: u32,
    pub source_edit_live: bool,
}

/// One project's published state: immutable, swapped after every call.
#[derive(Clone, Debug)]
pub struct Published {
    /// Diagnostic spans; no document serialization or main-thread clock dependency.
    pub actor_work: std::time::Duration,
    pub publication_work: std::time::Duration,
    pub project: String,
    pub snapshot: Snapshot,
    /// Recent changes, oldest first (for incremental readers and mirrors).
    pub changes: Arc<[Arc<Change>]>,
    pub history: HistorySummary,
    pub entry: Option<OpenEntryInfo>,
    pub selection: Selection,
    pub workspace: Value,
    pub ui_state: crate::state::UiState,
    pub dirty: bool,
    pub saving: bool,
    pub session: Option<crate::session::SessionMetadata>,
    pub media: crate::project_io::MediaTable,
    pub path: Option<std::path::PathBuf>,
}

impl Published {
    pub fn revision(&self) -> u64 {
        self.snapshot.revision
    }
}

/// What subscribers hear, after publication.
#[derive(Clone, Debug)]
pub enum KernelEvent {
    StateChanged {
        revision: u64,
        change: crate::state::StateChange,
    },
    SelectionChanged {
        project: String,
        selection: Selection,
    },
    WorkspaceChanged {
        project: String,
        workspace: Value,
    },
    Published {
        project: String,
        revision: u64,
    },
    Notice {
        project: String,
        notice: Notice,
    },
    Opened {
        project: String,
    },
    Closed {
        project: String,
    },
    /// A background journal error (disk full, ...).
    Durability {
        message: String,
    },
}

/// A realm's mailbox entry.
#[derive(Clone, Debug, PartialEq)]
pub struct RealmEnvelope {
    pub project: String,
    pub message: RealmMessage,
}

#[derive(Default)]
struct Shared {
    published: Mutex<HashMap<String, Arc<Published>>>,
    state: Mutex<Arc<crate::state::StateSnapshot>>,
    subscribers: Mutex<Vec<mpsc::Sender<KernelEvent>>>,
    realms: Mutex<HashMap<Writer, mpsc::Sender<RealmEnvelope>>>,
}

type Reply<T> = Box<dyn FnOnce(T) + Send>;

enum Msg {
    ProEditing {
        project: String,
        request: crate::pro_editing::ProRequest,
        reply: Reply<Result<crate::pro_editing::ProReply, KernelFailure>>,
    },
    ProClock {
        project: String,
        document: u64,
        comp_id: Option<String>,
        available: bool,
        reply: Reply<Option<u64>>,
    },
    Profile {
        call: crate::profile::ProfileCall,
        reply: Reply<Result<crate::profile::ProfileReply, KernelFailure>>,
    },
    PrepareSave {
        session: crate::session::SessionHandle,
        path: Option<std::path::PathBuf>,
        reply: Reply<Result<crate::session::SessionSave, KernelFailure>>,
    },
    FinishSave {
        completion: crate::session::SessionSaveCompletion,
        reply: Reply<Result<crate::session::ShellReply, KernelFailure>>,
    },
    State {
        tx: crate::state::StateTransaction,
        reply: Reply<Result<crate::state::StateAck, KernelFailure>>,
    },
    Shell {
        call: crate::session::ShellCall,
        reply: Reply<Result<crate::session::ShellReply, KernelFailure>>,
    },
    Call {
        project: String,
        request: Request,
        reply: Reply<Result<Ack, KernelFailure>>,
    },
    Open {
        project: String,
        source: Box<OpenSource>,
        reply: Reply<Result<OpenInfo, KernelFailure>>,
    },
    Close {
        project: String,
        reply: Reply<bool>,
    },
    Recover {
        project: String,
        current_base: Option<String>,
        base_document: Option<Value>,
        reply: Reply<Recovery>,
    },
    Sync {
        reply: Reply<()>,
    },
    Shutdown,
}

/// Owns the kernel thread. Dropping it (or [`shutdown`](Self::shutdown))
/// flushes every journal and stops the thread.
pub struct KernelActor {
    handle: KernelHandle,
    thread: Option<thread::JoinHandle<()>>,
}

/// The way in. See the module docs.
#[derive(Clone)]
pub struct KernelHandle {
    tx: mpsc::Sender<Msg>,
    shared: Arc<Shared>,
}

impl KernelActor {
    pub fn spawn(config: KernelConfig) -> KernelActor {
        let (tx, rx) = mpsc::channel::<Msg>();
        let shared = Arc::new(Shared::default());
        let thread = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("pm-kernel".into())
                .spawn(move || run(Kernel::new(config), rx, shared))
                .expect("spawn kernel thread")
        };
        KernelActor {
            handle: KernelHandle { tx, shared },
            thread: Some(thread),
        }
    }

    pub fn handle(&self) -> KernelHandle {
        self.handle.clone()
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            // Calls already queued run first; then journals sync and the
            // thread ends. Handles still held elsewhere get `None`.
            let _ = self.handle.tx.send(Msg::Shutdown);
            let _ = thread.join();
        }
    }
}

impl Drop for KernelActor {
    fn drop(&mut self) {
        self.stop();
    }
}

impl KernelHandle {
    /// Bind an immutable actor identity; cloned handles never change another caller's origin.
    /// Queue native UI/MCP pro actions. Never waits on the caller thread.
    pub fn pro_editing(
        &self,
        project: &str,
        request: crate::pro_editing::ProRequest,
    ) -> Pending<Result<crate::pro_editing::ProReply, KernelFailure>> {
        let (reply, pending) = oneshot();
        let _ = self.tx.send(Msg::ProEditing {
            project: project.into(),
            request,
            reply: Box::new(move |v| reply.resolve(v)),
        });
        pending
    }

    /// Native transport adapter reports real availability for this composition.
    /// Return a fresh token; observations from replaced bindings are refused.
    /// Default is unavailable; close/reopen fences an earlier clock binding.
    pub fn bind_pro_clock(
        &self,
        project: &str,
        document: u64,
        comp_id: Option<String>,
        available: bool,
    ) -> Pending<Option<u64>> {
        let (reply, pending) = oneshot();
        let _ = self.tx.send(Msg::ProClock {
            project: project.into(),
            document,
            comp_id,
            available,
            reply: Box::new(move |v| reply.resolve(v)),
        });
        pending
    }

    pub fn for_actor(&self, caller: crate::Caller) -> ActorHandle {
        ActorHandle {
            kernel: self.clone(),
            caller,
        }
    }
    pub fn shell(
        &self,
        call: crate::session::ShellCall,
    ) -> Pending<Result<crate::session::ShellReply, KernelFailure>> {
        let (r, p) = oneshot();
        self.send(Msg::Shell {
            call,
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }
    pub fn profile(
        &self,
        call: crate::profile::ProfileCall,
    ) -> Pending<Result<crate::profile::ProfileReply, KernelFailure>> {
        let (r, p) = oneshot();
        self.send(Msg::Profile {
            call,
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }
    pub fn prepare_save(
        &self,
        session: crate::session::SessionHandle,
        path: Option<std::path::PathBuf>,
    ) -> Pending<Result<crate::session::SessionSave, KernelFailure>> {
        let (r, p) = oneshot();
        self.send(Msg::PrepareSave {
            session,
            path,
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }
    pub fn finish_save(
        &self,
        completion: crate::session::SessionSaveCompletion,
    ) -> Pending<Result<crate::session::ShellReply, KernelFailure>> {
        let (r, p) = oneshot();
        self.send(Msg::FinishSave {
            completion,
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }
    pub fn state_transaction(
        &self,
        tx: crate::state::StateTransaction,
    ) -> Pending<Result<crate::state::StateAck, KernelFailure>> {
        let (r, p) = oneshot();
        self.send(Msg::State {
            tx,
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }
    pub fn state(&self) -> Arc<crate::state::StateSnapshot> {
        self.shared.state.lock().expect("state").clone()
    }
    fn send(&self, msg: Msg) {
        // A stopped kernel drops the message; the resolver's drop closes
        // the `Pending` (`None`).
        let _ = self.tx.send(msg);
    }

    /// Runs `request` on `project`; resolves with the acknowledgement.
    pub fn call(&self, project: &str, request: Request) -> Pending<Result<Ack, KernelFailure>> {
        let (r, p) = oneshot();
        self.call_with(project, request, move |a| r.resolve(a));
        p
    }

    /// Runs `request`; `reply` runs **on the kernel thread** (post to your
    /// own executor from it; never call back into the kernel synchronously).
    pub fn call_with(
        &self,
        project: &str,
        request: Request,
        reply: impl FnOnce(Result<Ack, KernelFailure>) + Send + 'static,
    ) {
        self.send(Msg::Call {
            project: project.to_owned(),
            request,
            reply: Box::new(reply),
        });
    }

    /// Worker threads and tests only (panics on the main thread).
    pub fn call_blocking(&self, project: &str, request: Request) -> Result<Ack, KernelFailure> {
        self.call(project, request)
            .wait()
            .unwrap_or_else(|| Err(KernelFailure::NoProject(project.to_owned())))
    }

    pub fn open(
        &self,
        project: &str,
        source: OpenSource,
    ) -> Pending<Result<OpenInfo, KernelFailure>> {
        let (r, p) = oneshot();
        self.send(Msg::Open {
            project: project.to_owned(),
            source: Box::new(source),
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }

    pub fn close(&self, project: &str) -> Pending<bool> {
        let (r, p) = oneshot();
        self.send(Msg::Close {
            project: project.to_owned(),
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }

    pub fn recover(
        &self,
        project: &str,
        current_base: Option<String>,
        base_document: Option<Value>,
    ) -> Pending<Recovery> {
        let (r, p) = oneshot();
        self.send(Msg::Recover {
            project: project.to_owned(),
            current_base,
            base_document,
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }

    /// Resolves once every earlier call ran and journals are synced.
    pub fn sync(&self) -> Pending<()> {
        let (r, p) = oneshot();
        self.send(Msg::Sync {
            reply: Box::new(move |v| r.resolve(v)),
        });
        p
    }

    /// The latest published state (pointer copy).
    pub fn published(&self, project: &str) -> Option<Arc<Published>> {
        self.shared
            .published
            .lock()
            .expect("published")
            .get(project)
            .cloned()
    }

    /// Events after every publication.
    pub fn subscribe(&self) -> mpsc::Receiver<KernelEvent> {
        let (tx, rx) = mpsc::channel();
        self.shared
            .subscribers
            .lock()
            .expect("subscribers")
            .push(tx);
        rx
    }

    /// A realm generation's mailbox.
    pub fn register_realm(&self, writer: Writer) -> mpsc::Receiver<RealmEnvelope> {
        let (tx, rx) = mpsc::channel();
        self.shared
            .realms
            .lock()
            .expect("realms")
            .insert(writer, tx);
        rx
    }
}

fn publish(kernel: &mut Kernel, shared: &Shared, project: &str) -> Option<u64> {
    publish_timed(kernel, shared, project, None)
}

fn publish_timed(
    kernel: &mut Kernel,
    shared: &Shared,
    project: &str,
    started: Option<std::time::Instant>,
) -> Option<u64> {
    for trimmed in std::mem::take(&mut kernel.trimmed_projects) {
        if trimmed != project {
            if let Some(revision) = publish(kernel, shared, &trimmed) {
                emit(
                    shared,
                    KernelEvent::Published {
                        project: trimmed.clone(),
                        revision,
                    },
                );
            }
            if let Some(p) = kernel.project_mut(&trimmed) {
                let realms = shared.realms.lock().expect("realms");
                for message in p.take_outbox() {
                    let writer = match &message {
                        RealmMessage::RunExternal { writer, .. }
                        | RealmMessage::Cleanup { writer, .. } => writer,
                    };
                    if let Some(mailbox) = realms.get(writer) {
                        let _ = mailbox.send(RealmEnvelope {
                            project: trimmed.clone(),
                            message,
                        });
                    }
                }
            }
        }
    }
    let publication_started = std::time::Instant::now();
    let p = kernel.project(project)?;
    let store = p.store();
    let history = store.history();
    let mut published = Published {
        actor_work: started.map_or(std::time::Duration::ZERO, |t| t.elapsed()),
        publication_work: Default::default(),
        project: project.to_owned(),
        snapshot: store.snapshot(),
        changes: store.recent_changes().into(),
        history: HistorySummary {
            can_undo: history.can_undo(),
            can_redo: history.can_redo(),
            undo_label: history.undo_label().map(str::to_owned),
            redo_label: history.redo_label().map(str::to_owned),
            entries: history.len(),
            applied: history.applied(),
        },
        entry: p.open_entry().map(|(id, label, depth)| OpenEntryInfo {
            id,
            label: label.to_owned(),
            depth,
            source_edit_live: p.source_edit_live(),
        }),
        selection: store.selection().clone(),
        workspace: p.workspace().clone(),
        ui_state: kernel.state().ui.get(project).cloned().unwrap_or_default(),
        dirty: store.is_dirty(),
        saving: store.is_saving(),
        session: kernel.session_metadata(project),
        media: store.media().clone(),
        path: store.path().map(std::path::Path::to_path_buf),
    };
    published.publication_work = publication_started.elapsed();
    let revision = published.revision();
    shared
        .published
        .lock()
        .expect("published")
        .insert(project.to_owned(), Arc::new(published));
    Some(revision)
}

fn emit(shared: &Shared, event: KernelEvent) {
    shared
        .subscribers
        .lock()
        .expect("subscribers")
        .retain(|s| s.send(event.clone()).is_ok());
}

fn run(mut kernel: Kernel, rx: mpsc::Receiver<Msg>, shared: Arc<Shared>) {
    // Deferred replies (external undo running in another realm).
    let mut deferred: HashMap<(String, u64), Reply<Result<Ack, KernelFailure>>> = HashMap::new();
    let mut pro_clocks: HashMap<String, (u64, Option<String>, bool, u64)> = HashMap::new();
    let mut pro_clock_token = 0u64;
    for msg in rx {
        // Prepare and execute in this same actor turn: no snapshot/read/write
        // race, and all writes retain the ordinary publication/event paths.
        let msg = match msg {
            Msg::ProClock {
                project,
                document,
                comp_id,
                available,
                reply,
            } => {
                let valid = kernel.project(&project).is_some_and(|p| {
                    p.store().doc_id() == document
                        && p.store()
                            .root()
                            .is_some_and(|root| crate::model::open_comp_id(root) == comp_id)
                });
                let token = pro_clock_token.checked_add(1).filter(|_| valid);
                if let Some(token) = token {
                    pro_clock_token = token;
                    pro_clocks.insert(project, (document, comp_id, available, token));
                }
                reply(token);
                continue;
            }
            Msg::ProEditing {
                project,
                request,
                reply,
            } => {
                let clock = pro_clocks
                    .get(&project)
                    .and_then(|(doc, comp, available, token)| {
                        let current_comp = kernel
                            .project(&project)
                            .and_then(|p| p.store().root())
                            .and_then(crate::model::open_comp_id);
                        (*doc == request.document && *available && *comp == current_comp)
                            .then_some(*token)
                    });
                match crate::pro_editing::prepare(&kernel, &project, request, clock) {
                    Ok(crate::pro_editing::Prepared::Call(request)) => Msg::Call {
                        project,
                        request,
                        reply: Box::new(move |v| {
                            reply(v.map(crate::pro_editing::ProReply::Document))
                        }),
                    },
                    Ok(crate::pro_editing::Prepared::State(tx)) => Msg::State {
                        tx,
                        reply: Box::new(move |v| reply(v.map(crate::pro_editing::ProReply::State))),
                    },
                    Ok(crate::pro_editing::Prepared::Inspection(v)) => {
                        reply(Ok(crate::pro_editing::ProReply::Inspection(v)));
                        continue;
                    }
                    Ok(crate::pro_editing::Prepared::Declined) => {
                        reply(Ok(crate::pro_editing::ProReply::Declined));
                        continue;
                    }
                    Err(e) => {
                        reply(Err(e));
                        continue;
                    }
                }
            }
            other => other,
        };
        match msg {
            Msg::ProEditing { .. } | Msg::ProClock { .. } => unreachable!("prepared above"),
            Msg::Profile { call, reply } => reply(kernel.profile(call)),
            Msg::PrepareSave {
                session,
                path,
                reply,
            } => {
                let id = session.project.clone();
                let result = kernel.prepare_session_save(session, path);
                let revision = publish(&mut kernel, &shared, &id);
                reply(result);
                if let Some(revision) = revision {
                    emit(
                        &shared,
                        KernelEvent::Published {
                            project: id,
                            revision,
                        },
                    );
                }
            }
            Msg::FinishSave { completion, reply } => {
                let result = kernel.finish_session_save(completion);
                let ids: Vec<_> = kernel.projects().map(str::to_owned).collect();
                for id in &ids {
                    publish(&mut kernel, &shared, id);
                }
                reply(result);
                for id in ids {
                    emit(
                        &shared,
                        KernelEvent::Published {
                            revision: kernel.project(&id).unwrap().store().revision(),
                            project: id,
                        },
                    );
                }
            }
            Msg::State { tx, reply } => {
                let result = kernel.state_transaction(tx);
                *shared.state.lock().expect("state") = Arc::new(kernel.state().clone());
                let ids: Vec<_> = kernel.projects().map(str::to_owned).collect();
                for id in ids {
                    publish(&mut kernel, &shared, &id);
                }
                let events = result
                    .as_ref()
                    .ok()
                    .map(|a| (a.revision, a.changes.clone()));
                reply(result);
                if let Some((revision, changes)) = events {
                    for change in changes {
                        emit(&shared, KernelEvent::StateChanged { revision, change });
                    }
                }
            }
            Msg::Shell { call, reply } => {
                let before: Vec<_> = kernel.projects().map(str::to_owned).collect();
                let result = kernel.shell(call);
                let ids: Vec<_> = kernel.projects().map(str::to_owned).collect();
                shared
                    .published
                    .lock()
                    .expect("published")
                    .retain(|id, _| ids.contains(id));
                *shared.state.lock().expect("state") = Arc::new(kernel.state().clone());
                for id in &ids {
                    publish(&mut kernel, &shared, id);
                }
                reply(result);
                for id in &ids {
                    if !before.contains(id) {
                        emit(
                            &shared,
                            KernelEvent::Opened {
                                project: id.clone(),
                            },
                        );
                    } else if let Some(p) = kernel.project(id) {
                        emit(
                            &shared,
                            KernelEvent::Published {
                                project: id.clone(),
                                revision: p.store().revision(),
                            },
                        );
                    }
                }
                for id in before {
                    if !ids.contains(&id) {
                        emit(&shared, KernelEvent::Closed { project: id });
                    }
                }
            }
            Msg::Call {
                project,
                request,
                reply,
            } => {
                let actor_started = std::time::Instant::now();
                let before_state = kernel.state().clone();
                let before_selection = kernel
                    .project(&project)
                    .map(|p| p.store().selection().clone());
                let handled = match kernel.handle(&project, request) {
                    Ok(h) => h,
                    Err(e) => {
                        reply(Err(e));
                        continue;
                    }
                };
                if before_state.revision != kernel.state().revision {
                    *shared.state.lock().expect("state") = Arc::new(kernel.state().clone());
                }
                let workspace_changed = kernel
                    .project(&project)
                    .map(|p| p.workspace().clone())
                    .filter(|ws| {
                        shared
                            .published
                            .lock()
                            .expect("published")
                            .get(&project)
                            .is_some_and(|p| &p.workspace != ws)
                    });
                // Publication boundary: publish, then answer, then events.
                let revision = publish_timed(&mut kernel, &shared, &project, Some(actor_started));
                let (outbox, notices) = match kernel.project_mut(&project) {
                    Some(p) => (p.take_outbox(), p.take_notices()),
                    None => (Vec::new(), Vec::new()),
                };
                match (handled.reply, handled.deferred) {
                    (Some(ack), _) => reply(Ok(ack)),
                    (None, Some(ticket)) => {
                        deferred.insert((project.clone(), ticket), reply);
                    }
                    (None, None) => {}
                }
                for (ticket, ack) in handled.resolved {
                    if let Some(r) = deferred.remove(&(project.clone(), ticket)) {
                        r(Ok(ack));
                    }
                }
                {
                    let realms = shared.realms.lock().expect("realms");
                    for message in outbox {
                        let writer = match &message {
                            RealmMessage::RunExternal { writer, .. }
                            | RealmMessage::Cleanup { writer, .. } => *writer,
                        };
                        if let Some(mailbox) = realms.get(&writer) {
                            let _ = mailbox.send(RealmEnvelope {
                                project: project.clone(),
                                message,
                            });
                        }
                    }
                }
                if let Some(revision) = revision {
                    emit(
                        &shared,
                        KernelEvent::Published {
                            project: project.clone(),
                            revision,
                        },
                    );
                }
                if let Some(selection) = kernel
                    .project(&project)
                    .map(|p| p.store().selection().clone())
                    && before_selection.as_ref() != Some(&selection)
                {
                    emit(
                        &shared,
                        KernelEvent::SelectionChanged {
                            project: project.clone(),
                            selection,
                        },
                    );
                }
                if let Some(workspace) = workspace_changed {
                    emit(
                        &shared,
                        KernelEvent::WorkspaceChanged {
                            project: project.clone(),
                            workspace,
                        },
                    );
                }
                if before_state.revision != kernel.state().revision {
                    for key in before_state
                        .settings
                        .keys()
                        .chain(kernel.state().settings.keys())
                        .collect::<std::collections::BTreeSet<_>>()
                    {
                        if before_state.settings.get(key) != kernel.state().settings.get(key) {
                            emit(
                                &shared,
                                KernelEvent::StateChanged {
                                    revision: kernel.state().revision,
                                    change: crate::state::StateChange::Setting {
                                        key: key.clone(),
                                        value: kernel.state().settings.get(key).cloned(),
                                    },
                                },
                            );
                        }
                    }
                }
                for notice in notices {
                    emit(
                        &shared,
                        KernelEvent::Notice {
                            project: project.clone(),
                            notice,
                        },
                    );
                }
                if let Some(d) = kernel.durability() {
                    for message in d.take_errors() {
                        emit(&shared, KernelEvent::Durability { message });
                    }
                }
            }
            Msg::Open {
                project,
                source,
                reply,
            } => {
                let result = kernel.open(&project, *source);
                if result.is_ok() {
                    publish(&mut kernel, &shared, &project);
                }
                reply(result.clone());
                if result.is_ok() {
                    emit(&shared, KernelEvent::Opened { project });
                }
            }
            Msg::Close { project, reply } => {
                let closed = kernel.close(&project);
                if closed {
                    shared.published.lock().expect("published").remove(&project);
                }
                *shared.state.lock().expect("state") = Arc::new(kernel.state().clone());
                reply(closed);
                if closed {
                    emit(&shared, KernelEvent::Closed { project });
                }
            }
            Msg::Recover {
                project,
                current_base,
                base_document,
                reply,
            } => reply(kernel.recover(&project, current_base.as_deref(), base_document.as_ref())),
            Msg::Sync { reply } => {
                kernel.sync_all();
                reply(());
            }
            Msg::Shutdown => break,
        }
    }
    kernel.sync_all();
}

// ── the UI's synchronous query mirror ───────────────────────────────────────

/// A [`QueryContext`] owned by the UI thread, following published changes.
pub struct QueryMirror {
    queries: QueryContext,
    doc_id: u64,
    revision: u64,
}

impl QueryMirror {
    pub fn new(published: &Published) -> QueryMirror {
        QueryMirror {
            queries: QueryContext::new(&published.snapshot.document),
            doc_id: published.snapshot.doc_id,
            revision: published.revision(),
        }
    }

    /// Fast-forwards to `published` (applying its changes in order, or
    /// adopting its snapshot when they no longer reach back). Returns the
    /// mirror's revision.
    pub fn sync(&mut self, published: &Published) -> u64 {
        if published.snapshot.doc_id != self.doc_id {
            *self = QueryMirror::new(published);
            return self.revision;
        }
        if published.revision() == self.revision {
            return self.revision;
        }
        let reaches = published
            .changes
            .iter()
            .any(|c| c.base_revision == self.revision);
        if reaches {
            let from = self.revision;
            for change in published.changes.iter().filter(|c| c.revision > from) {
                self.queries.apply(
                    &change.ops,
                    change.base_revision,
                    change.revision,
                    &published.snapshot.document,
                );
                self.revision = change.revision;
            }
        }
        if self.revision != published.revision() || self.queries.revision() != published.revision()
        {
            self.queries.resync(&published.snapshot.document);
            self.revision = published.revision();
        }
        self.revision
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn queries(&mut self) -> &mut QueryContext {
        &mut self.queries
    }
}

/// A realm/agent/MCP client's generation-bound view of the one kernel actor.
#[derive(Clone)]
pub struct ActorHandle {
    pub kernel: KernelHandle,
    pub caller: crate::Caller,
}
impl ActorHandle {
    pub fn call(
        &self,
        project: &str,
        call: impl Into<crate::Call>,
    ) -> Pending<Result<Ack, KernelFailure>> {
        self.kernel.call(project, Request::new(self.caller, call))
    }
}
