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

struct Resolver<T: 'static> {
    inner: Arc<OneShot<T>>,
}

/// Closers for answers a panicking turn dropped (`None` off the kernel thread).
type Closers = Option<Vec<Box<dyn FnOnce()>>>;

thread_local! {
    /// On the kernel thread: answers a panicking turn dropped, closed only
    /// once the panic is contained (`run`), so a caller that sees `None`
    /// also sees the project stopped.
    static UNANSWERED: std::cell::RefCell<Closers> = const { std::cell::RefCell::new(None) };
}

fn close<T>(inner: &OneShot<T>) {
    let waker = {
        let mut slot = inner.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.closed = true;
        slot.waker.take()
    };
    inner.ready.notify_all();
    if let Some(w) = waker {
        w.wake();
    }
}

/// A caller's reply callback: dropped unanswered by a panicking turn, it is
/// dropped only once the panic is contained, like a [`Resolver`].
struct Unanswered<F: Send + 'static>(Option<F>);

impl<F: Send + 'static> Drop for Unanswered<F> {
    fn drop(&mut self) {
        let Some(f) = self.0.take() else { return };
        if std::thread::panicking() {
            let mut f = Some(f);
            UNANSWERED.with(|u| {
                if let Some(v) = u.borrow_mut().as_mut() {
                    let f = f.take();
                    v.push(Box::new(move || drop(f)));
                }
            });
            drop(f);
        }
    }
}

/// Closes the answers the last turn's panic dropped.
fn close_unanswered() {
    let closers = UNANSWERED.with(|u| u.borrow_mut().as_mut().map(std::mem::take));
    for closer in closers.into_iter().flatten() {
        closer();
    }
}

fn oneshot<T: 'static>() -> (Resolver<T>, Pending<T>) {
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

impl<T: 'static> Resolver<T> {
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

impl<T: 'static> Drop for Resolver<T> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let inner = self.inner.clone();
            let deferred = UNANSWERED.with(|u| {
                u.borrow_mut()
                    .as_mut()
                    .map(|v| v.push(Box::new(move || close(&inner))))
            });
            if deferred.is_some() {
                return;
            }
        }
        close(&self.inner);
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
    /// Independent note publication sequence; does not change composition revision.
    pub metadata_revision: u64,
    pub saving: bool,
    pub session: Option<crate::session::SessionMetadata>,
    pub media: crate::project_io::MediaTable,
    pub path: Option<std::path::PathBuf>,
    /// Set when an internal error stopped this project
    /// ([`Kernel::quarantine`]): this publication is the last one before
    /// the error, and the project takes no edits or saves until reopened.
    pub fault: Option<String>,
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
    /// A background journal error (disk full, ...) after `project`'s edits
    /// were acknowledged: they are applied but not durable.
    Durability {
        project: String,
        message: String,
    },
    /// Something the person is not asked about but can look up: a line for
    /// the host's Error Log.
    Log {
        message: String,
    },
    /// A request failed with an internal error (a contained panic); the
    /// kernel keeps serving. Hosts show it in the Error Log.
    /// A panic was contained to one call. `projects` stopped
    /// ([`Kernel::quarantine`]); empty when nothing could be half applied.
    Fault {
        message: String,
        projects: Vec<String>,
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
    journal_sync: Mutex<Option<crate::journal::JournalSync>>,
    /// The actor's own mailbox, for a worker to hand back what it did.
    back: Mutex<Option<mpsc::Sender<Msg>>>,
}

type Reply<T> = Box<dyn FnOnce(T) + Send>;

enum Msg {
    /// The journal queued a background error: collect and publish it.
    DurabilityErrors,
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
    PrepareExport {
        session: crate::session::SessionHandle,
        document: u64,
        path: std::path::PathBuf,
        reply: Reply<Result<crate::SavePlan, KernelFailure>>,
    },
    PrepareSave {
        session: crate::session::SessionHandle,
        path: Option<std::path::PathBuf>,
        replace: bool,
        document: Option<u64>,
        /// The save's disk work, when a worker has done it
        /// ([`crate::session::SaveJob`]); `None` inside: the worker failed.
        prelude: Option<Option<Box<crate::session::SavePrelude>>>,
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
    /// A choice after a failed open whose disk work a worker has done
    /// ([`crate::session::DiskJob`]); `result` is `None` if it failed.
    DiskDone {
        path: Option<std::path::PathBuf>,
        result: Option<Box<Result<crate::session::ShellReply, KernelFailure>>>,
        reply: Reply<Result<crate::session::ShellReply, KernelFailure>>,
    },
    /// A file open whose disk work a worker has done
    /// ([`crate::session::OpenJob`]); `prelude` is `None` if it failed.
    Opened {
        call: crate::session::ShellCall,
        path: std::path::PathBuf,
        prelude: Option<Box<crate::session::OpenPrelude>>,
        reply: Reply<Result<crate::session::ShellReply, KernelFailure>>,
    },
    Call {
        project: String,
        request: Request,
        source_finish: Option<crate::pro_editing::SourceFinish>,
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
    /// `KernelHandle::arm_fault`: arms a panic on this actor only.
    #[cfg(any(test, feature = "fault-injection"))]
    ArmFault {
        stage: crate::fault_injection::Stage,
        project: String,
    },
    /// `KernelHandle::set_history_budget_later`.
    #[cfg(any(test, feature = "fault-injection"))]
    HistoryBudget(usize),
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
        *shared.state.lock().expect("state") = Arc::new(crate::state::StateSnapshot::seeded());
        *shared.back.lock().expect("back") = Some(tx.clone());
        let thread = {
            let shared = shared.clone();
            let wake = tx.clone();
            thread::Builder::new()
                .name("pm-kernel".into())
                .spawn(move || {
                    let idle_memory_relief = config.idle_memory_relief;
                    let kernel = Kernel::new(config);
                    // A background journal failure is reported when it
                    // happens, not at the next request (which may never
                    // come before the person walks away).
                    if let Some(durability) = kernel.durability() {
                        let wake = std::sync::Mutex::new(wake);
                        durability.on_error(move || {
                            let _ = wake.lock().expect("wake").send(Msg::DurabilityErrors);
                        });
                    }
                    *shared.journal_sync.lock().expect("journal sync") = kernel.journal_sync();
                    run(kernel, rx, shared, idle_memory_relief)
                })
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
    /// Makes every journal durable without waiting for the actor, which may
    /// be busy (Quit Now). Blocking: run it off the main thread.
    pub fn sync_journals(&self) -> std::io::Result<()> {
        let sync = self
            .shared
            .journal_sync
            .lock()
            .expect("journal sync")
            .clone();
        sync.map_or(Ok(()), |sync| sync.sync())
    }
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
    pub fn prepare_export(
        &self,
        session: crate::session::SessionHandle,
        document: u64,
        path: std::path::PathBuf,
    ) -> Pending<Result<crate::SavePlan, KernelFailure>> {
        let (reply, pending) = oneshot();
        self.send(Msg::PrepareExport {
            session,
            document,
            path,
            reply: Box::new(move |v| reply.resolve(v)),
        });
        pending
    }
    pub fn prepare_save(
        &self,
        session: crate::session::SessionHandle,
        path: Option<std::path::PathBuf>,
    ) -> Pending<Result<crate::session::SessionSave, KernelFailure>> {
        self.prepare_save_checked(session, path, false, None)
    }
    /// Save As after the save panel confirmed replacing an existing file.
    pub fn prepare_save_replacing(
        &self,
        session: crate::session::SessionHandle,
        path: std::path::PathBuf,
    ) -> Pending<Result<crate::session::SessionSave, KernelFailure>> {
        self.prepare_save_checked(session, Some(path), true, None)
    }
    /// A queued automation save remains bound to the observed document lifetime.
    pub fn prepare_document_save(
        &self,
        session: crate::session::SessionHandle,
        document: u64,
    ) -> Pending<Result<crate::session::SessionSave, KernelFailure>> {
        self.prepare_save_checked(session, None, false, Some(document))
    }
    fn prepare_save_checked(
        &self,
        session: crate::session::SessionHandle,
        path: Option<std::path::PathBuf>,
        replace: bool,
        document: Option<u64>,
    ) -> Pending<Result<crate::session::SessionSave, KernelFailure>> {
        let (r, p) = oneshot();
        self.send(Msg::PrepareSave {
            session,
            path,
            replace,
            document,
            prelude: None,
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

    /// Tests: this actor's next call on `project` that reaches `stage`
    /// panics, once (calls sent after this one see it armed).
    #[cfg(any(test, feature = "fault-injection"))]
    pub fn arm_fault(&self, stage: crate::fault_injection::Stage, project: &str) {
        self.send(Msg::ArmFault {
            stage,
            project: project.to_owned(),
        });
    }

    /// Tests: the shared undo budget in entries, applied by the next call
    /// that maintains it.
    #[cfg(any(test, feature = "fault-injection"))]
    pub fn set_history_budget_later(&self, max_entries: usize) {
        self.send(Msg::HistoryBudget(max_entries));
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
        // The callback is the caller's code: a panic in it is reported and
        // stays there. The call it answers has already committed, and the
        // rest of its turn (events, realm messages, deferred answers) runs.
        let shared = self.shared.clone();
        let mut reply = Unanswered(Some(reply));
        let reply = move |answer| {
            let reply = reply.0.take().expect("answered once");
            if let Err(panic) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reply(answer)))
            {
                emit(
                    &shared,
                    KernelEvent::Fault {
                        message: format!(
                            "A kernel reply handler failed: {}",
                            panic_detail(panic.as_ref())
                        ),
                        projects: Vec::new(),
                    },
                );
            }
        };
        self.send(Msg::Call {
            project: project.to_owned(),
            request,
            source_finish: None,
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

    /// A finished generation stops receiving; its mailbox reader ends.
    pub fn retire_realm(&self, writer: Writer) {
        self.shared.realms.lock().expect("realms").remove(&writer);
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
    if kernel.fault(project).is_some() {
        // A stopped project keeps its last publication: its live state may
        // be half applied.
        return shared
            .published
            .lock()
            .expect("published")
            .get(project)
            .map(|p| p.revision());
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
        metadata_revision: store.metadata_revision(),
        saving: store.is_saving(),
        session: kernel.session_metadata(project),
        media: store.media().clone(),
        path: store.path().map(std::path::Path::to_path_buf),
        fault: None,
    };
    published.publication_work = publication_started.elapsed();
    crate::fault_injection::hit(crate::fault_injection::Stage::InPublish);
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

/// The actor loop's own state between messages.
#[derive(Default)]
struct Turns {
    /// Deferred replies (external undo running in another realm).
    deferred: HashMap<(String, u64), Reply<Result<Ack, KernelFailure>>>,
    pro_clocks: HashMap<String, (u64, Option<String>, bool, u64)>,
    pro_clock_token: u64,
}

/// What a message can change, so a panic in it stops exactly that.
enum Scope {
    /// Reads, and state transactions (staged, swapped in at the end).
    Nothing,
    Project(String),
    /// Shell calls that are not one project's (the library, quit).
    Every,
}

/// What a shell call can change. A call that names a project (its open,
/// save, close, copies) can leave only that project half done: a panic in
/// it stops that one and every other open project keeps working. Calls on
/// the library as a whole can touch any of them.
fn shell_scope(call: &crate::session::ShellCall) -> Scope {
    use crate::session::ShellCall::*;
    match call {
        Open { id, .. } | PrepareOpen { id, .. } => Scope::Project(id.clone()),
        Save(session)
        | Close(session)
        | DeferClose(session)
        | DiscardClose(session)
        | CopiesOf(session)
        | KeptBeside(session)
        | KeptSeen { session, .. }
        | SaveAs { session, .. } => Scope::Project(session.project.clone()),
        _ => Scope::Every,
    }
}

fn scope(msg: &Msg) -> Scope {
    match msg {
        Msg::Call { project, .. }
        | Msg::ProEditing { project, .. }
        | Msg::Open { project, .. }
        | Msg::Close { project, .. } => Scope::Project(project.clone()),
        Msg::PrepareSave { session, .. } => Scope::Project(session.project.clone()),
        Msg::FinishSave { completion, .. } => Scope::Project(completion.project().to_owned()),
        Msg::Shell { call, .. } | Msg::Opened { call, .. } => shell_scope(call),
        // As the shell call it finishes (a choice after a failed open).
        Msg::DiskDone { .. } => Scope::Every,
        Msg::State { .. }
        | Msg::ProClock { .. }
        | Msg::Profile { .. }
        | Msg::PrepareExport { .. }
        | Msg::Recover { .. }
        | Msg::Sync { .. }
        | Msg::DurabilityErrors
        | Msg::Shutdown => Scope::Nothing,
        #[cfg(any(test, feature = "fault-injection"))]
        Msg::ArmFault { .. } | Msg::HistoryBudget(_) => Scope::Nothing,
    }
}

fn run(
    mut kernel: Kernel,
    rx: mpsc::Receiver<Msg>,
    shared: Arc<Shared>,
    idle_memory_relief: Option<fn()>,
) {
    let mut turns = Turns::default();
    UNANSWERED.with(|u| *u.borrow_mut() = Some(Vec::new()));
    loop {
        let received = if let Some(collect) = idle_memory_relief {
            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(msg) => Ok(msg),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    collect();
                    // Collect once per idle period, then sleep without polling.
                    rx.recv()
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            rx.recv()
        };
        let Ok(msg) = received else { break };
        let shutdown = matches!(msg, Msg::Shutdown);
        let scope = scope(&msg);
        crate::fault_injection::enter(match &scope {
            Scope::Project(id) => Some(id),
            _ => None,
        });
        // A panic in one call must not take the kernel (and every open
        // project's unsaved work) down with it, nor leave a project half
        // applied: the projects the call could change stop (`contain`), its
        // reply is dropped (its caller sees `None`) and the next message is
        // served.
        let turn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            step(&mut kernel, &shared, &mut turns, msg)
        }));
        if let Err(panic) = turn {
            contain(&mut kernel, &shared, &mut turns, scope, panic.as_ref());
        }
        close_unanswered();
        if shutdown {
            break;
        }
    }
    kernel.sync_all();
}

#[cfg(test)]
mod idle_memory_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static ON_OWNER: AtomicBool = AtomicBool::new(false);
    fn collect() {
        ON_OWNER.store(
            std::thread::current().name() == Some("pm-kernel"),
            Ordering::Release,
        );
        CALLS.fetch_add(1, Ordering::Release);
    }

    #[test]
    fn collects_on_the_allocating_actor_once_then_sleeps() {
        let actor = KernelActor::spawn(KernelConfig {
            idle_memory_relief: Some(collect),
            ..Default::default()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while CALLS.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < deadline, "idle owner did not collect");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ON_OWNER.load(Ordering::Acquire));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(CALLS.load(Ordering::Acquire), 1);
        actor.shutdown();
    }
}

fn panic_detail(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown cause".into())
}

/// After a contained panic: clears lock poison, stops every project the
/// call could have changed, answers its deferred callers with the failure,
/// flags its last publication, and reports the fault. A project the call
/// was opening (never published) is released so opening it again works;
/// its journal stays.
fn contain(
    kernel: &mut Kernel,
    shared: &Shared,
    turns: &mut Turns,
    scope: Scope,
    panic: &(dyn std::any::Any + Send),
) {
    shared.published.clear_poison();
    shared.state.clear_poison();
    shared.subscribers.clear_poison();
    shared.realms.clear_poison();
    let detail = panic_detail(panic);
    let ids: Vec<String> = match scope {
        Scope::Nothing => Vec::new(),
        Scope::Project(id) => vec![id],
        Scope::Every => kernel.projects().map(str::to_owned).collect(),
    };
    let mut stopped = Vec::new();
    for id in ids {
        if !kernel.quarantine(&id, &detail) {
            continue;
        }
        let failure = kernel.refuse_faulted(&id).unwrap_err();
        let waiting: Vec<_> = turns
            .deferred
            .keys()
            .filter(|(project, _)| *project == id)
            .cloned()
            .collect();
        for key in waiting {
            if let Some(reply) = turns.deferred.remove(&key) {
                reply(Err(failure.clone()));
            }
        }
        turns.pro_clocks.remove(&id);
        let mut published = shared.published.lock().expect("published");
        match published.get(&id) {
            Some(last) => {
                let mut flagged = Published::clone(last);
                flagged.fault = Some(detail.clone());
                let revision = flagged.revision();
                published.insert(id.clone(), Arc::new(flagged));
                drop(published);
                emit(
                    shared,
                    KernelEvent::Published {
                        project: id.clone(),
                        revision,
                    },
                );
            }
            None => {
                drop(published);
                let _ = kernel.flush_project(&id);
                kernel.remove_closed_project(&id);
            }
        }
        stopped.push(id);
    }
    let message = if stopped.is_empty() {
        format!("An internal error stopped one request: {detail}")
    } else {
        format!(
            "An internal error stopped one request: {detail}. Editing stopped in {} so nothing \
             half done is kept; reopen to recover every edit made before the error.",
            stopped.join(", ")
        )
    };
    emit(
        shared,
        KernelEvent::Fault {
            message,
            projects: stopped,
        },
    );
}

/// Runs `work` on a worker thread and posts `done(result)` back to the
/// actor (`None` if `work` panicked): the actor never waits on it.
fn off_actor<T: Send + 'static>(
    name: &str,
    back: mpsc::Sender<Msg>,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(Option<Box<T>>) -> Msg + Send + 'static,
) {
    let run = move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).ok();
        let _ = back.send(done(result.map(Box::new)));
    };
    // A thread that cannot be started leaves the caller unanswered (its
    // reply closes), as any call the kernel could not serve.
    let _ = thread::Builder::new().name(name.into()).spawn(run);
}

/// One shell call, with what it publishes and emits.
fn shell_step(
    kernel: &mut Kernel,
    shared: &Arc<Shared>,
    call: crate::session::ShellCall,
    reply: Reply<Result<crate::session::ShellReply, KernelFailure>>,
) {
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
        publish(kernel, shared, id);
    }
    reply(result);
    for id in &ids {
        if !before.contains(id) {
            emit(
                shared,
                KernelEvent::Opened {
                    project: id.clone(),
                },
            );
        } else if let Some(p) = kernel.project(id) {
            emit(
                shared,
                KernelEvent::Published {
                    project: id.clone(),
                    revision: p.store().revision(),
                },
            );
        }
    }
    for id in before {
        if !ids.contains(&id) {
            emit(shared, KernelEvent::Closed { project: id });
        }
    }
    for message in kernel.take_log() {
        emit(shared, KernelEvent::Log { message });
    }
}

fn step(kernel: &mut Kernel, shared: &Arc<Shared>, turns: &mut Turns, msg: Msg) {
    let Turns {
        deferred,
        pro_clocks,
        pro_clock_token,
    } = turns;
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
                *pro_clock_token = token;
                pro_clocks.insert(project, (document, comp_id, available, token));
            }
            reply(token);
            return;
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
            match crate::pro_editing::prepare(kernel, &project, request, clock) {
                Ok(crate::pro_editing::Prepared::Call(request)) => Msg::Call {
                    project,
                    request,
                    source_finish: None,
                    reply: Box::new(move |v| reply(v.map(crate::pro_editing::ProReply::Document))),
                },
                Ok(crate::pro_editing::Prepared::SourceEdit { request, finish }) => {
                    let state = shared.clone();
                    Msg::Call {
                        project,
                        request,
                        source_finish: Some(finish),
                        reply: Box::new(move |v| {
                            reply(v.map(|ack| crate::pro_editing::ProReply::SourceEdit {
                                ack,
                                state_revision: state.state.lock().expect("state").revision,
                            }))
                        }),
                    }
                }
                Ok(crate::pro_editing::Prepared::State(tx)) => Msg::State {
                    tx,
                    reply: Box::new(move |v| reply(v.map(crate::pro_editing::ProReply::State))),
                },
                Ok(crate::pro_editing::Prepared::Inspection(v)) => {
                    reply(Ok(crate::pro_editing::ProReply::Inspection(v)));
                    return;
                }
                Ok(crate::pro_editing::Prepared::Declined) => {
                    reply(Ok(crate::pro_editing::ProReply::Declined));
                    return;
                }
                Err(e) => {
                    reply(Err(e));
                    return;
                }
            }
        }
        other => other,
    };
    match msg {
        Msg::ProEditing { .. } | Msg::ProClock { .. } => unreachable!("prepared above"),
        Msg::Profile { call, reply } => reply(kernel.profile(call)),
        Msg::PrepareExport {
            session,
            document,
            path,
            reply,
        } => {
            reply(kernel.prepare_session_export(session, document, path));
        }
        Msg::PrepareSave {
            session,
            path,
            replace,
            document,
            prelude,
            reply,
        } => {
            let id = session.project.clone();
            // The disk work a save in place does first (settling what an
            // earlier save left, proving a renamed file) runs on a worker;
            // this message comes back with what it did, and every other
            // project is answered meanwhile.
            match prelude {
                None => match kernel.save_job(&session, path.as_deref()) {
                    Ok(Some(job)) => {
                        let back = shared.back.lock().expect("back").clone();
                        if let Some(back) = back {
                            kernel.preparing.push(id);
                            let again = move |prelude| Msg::PrepareSave {
                                session,
                                path,
                                replace,
                                document,
                                prelude: Some(prelude),
                                reply,
                            };
                            off_actor("pm-kernel-save", back, move || job.run(), again);
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => return reply(Err(e)),
                },
                Some(done) => {
                    kernel.preparing.retain(|p| *p != id);
                    kernel.save_prelude = done;
                }
            }
            let result = if document.is_some_and(|expected| {
                kernel
                    .project(&id)
                    .is_none_or(|p| p.store().doc_id() != expected)
            }) {
                Err(KernelFailure::Storage(
                    "The document was closed or replaced; the save was not applied".into(),
                ))
            } else {
                kernel.prepare_session_save(session, path, replace)
            };
            let revision = publish(kernel, shared, &id);
            reply(result);
            for message in kernel.take_log() {
                emit(shared, KernelEvent::Log { message });
            }
            if let Some(revision) = revision {
                emit(
                    shared,
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
                publish(kernel, shared, id);
            }
            reply(result);
            for id in ids {
                emit(
                    shared,
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
                publish(kernel, shared, &id);
            }
            let events = result
                .as_ref()
                .ok()
                .map(|a| (a.revision, a.changes.clone()));
            reply(result);
            if let Some((revision, changes)) = events {
                for change in changes {
                    emit(shared, KernelEvent::StateChanged { revision, change });
                }
            }
        }
        Msg::Shell { call, reply } => {
            // The disk work of a choice after a failed open (the versions a
            // record proves, read whole; the chosen one put in place) runs
            // on a worker; it comes back as `DiskDone`.
            match kernel.disk_job(&call) {
                Ok(Some(job)) => {
                    let back = shared.back.lock().expect("back").clone();
                    if let Some(back) = back {
                        let path = job.path().map(ToOwned::to_owned);
                        if let Some(path) = &path {
                            kernel.opening.push(path.clone());
                        }
                        let done = move |result| Msg::DiskDone {
                            path,
                            result,
                            reply,
                        };
                        off_actor("pm-kernel-disk", back, move || job.run(), done);
                        return;
                    }
                }
                Ok(None) => {}
                Err(e) => return reply(Err(e)),
            }
            // The disk work of opening a project file (its lock, what a
            // cut-short save left, the file itself read whole) runs on a
            // worker; the open comes back as `Opened` with what it did, and
            // every other project is answered meanwhile.
            match kernel.open_job(&call) {
                Ok(Some(job)) => {
                    let back = shared.back.lock().expect("back").clone();
                    if let Some(back) = back {
                        let path = job.path().to_owned();
                        kernel.opening.push(path.clone());
                        let opened = move |prelude: Option<Box<_>>| Msg::Opened {
                            call,
                            path,
                            prelude,
                            reply,
                        };
                        off_actor("pm-kernel-open", back, move || job.run(), opened);
                        return;
                    }
                }
                Ok(None) => {}
                Err(e) => return reply(Err(e)),
            }
            shell_step(kernel, shared, call, reply)
        }
        Msg::DiskDone {
            path,
            result,
            reply,
        } => {
            if let Some(path) = path {
                kernel.opening.retain(|p| *p != path);
            }
            kernel.disk_job_done();
            *shared.state.lock().expect("state") = Arc::new(kernel.state().clone());
            let ids: Vec<_> = kernel.projects().map(str::to_owned).collect();
            for id in &ids {
                publish(kernel, shared, id);
            }
            reply(result.map_or_else(
                || {
                    Err(KernelFailure::Storage(
                        "The disk work stopped unexpectedly; nothing more was done.".into(),
                    ))
                },
                |result| *result,
            ));
            for message in kernel.take_log() {
                emit(shared, KernelEvent::Log { message });
            }
        }
        Msg::Opened {
            call,
            path,
            prelude,
            reply,
        } => {
            kernel.opening.retain(|p| *p != path);
            kernel.open_prelude = prelude;
            shell_step(kernel, shared, call, reply)
        }
        Msg::Call {
            project,
            request,
            source_finish,
            reply,
        } => {
            let actor_started = std::time::Instant::now();
            let before_state = kernel.state().clone();
            let before_selection = kernel
                .project(&project)
                .map(|p| p.store().selection().clone());
            let mut handled = match kernel.handle(&project, request) {
                Ok(h) => h,
                Err(e) => {
                    reply(Err(e));
                    return;
                }
            };
            if let Some(finish) = source_finish {
                crate::pro_editing::finish_source(kernel, &project, finish, &mut handled);
            }
            crate::fault_injection::hit(crate::fault_injection::Stage::BeforePublish);
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
            let revision = publish_timed(kernel, shared, &project, Some(actor_started));
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
                    shared,
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
                    shared,
                    KernelEvent::SelectionChanged {
                        project: project.clone(),
                        selection,
                    },
                );
            }
            if let Some(workspace) = workspace_changed {
                emit(
                    shared,
                    KernelEvent::WorkspaceChanged {
                        project: project.clone(),
                        workspace,
                    },
                );
            }
            if before_state.revision != kernel.state().revision {
                if before_state.ui.get(&project) != kernel.state().ui.get(&project) {
                    emit(
                        shared,
                        KernelEvent::StateChanged {
                            revision: kernel.state().revision,
                            change: crate::state::StateChange::Ui {
                                project: project.clone(),
                            },
                        },
                    );
                }
                for key in before_state
                    .settings
                    .keys()
                    .chain(kernel.state().settings.keys())
                    .collect::<std::collections::BTreeSet<_>>()
                {
                    if before_state.settings.get(key) != kernel.state().settings.get(key) {
                        emit(
                            shared,
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
                    shared,
                    KernelEvent::Notice {
                        project: project.clone(),
                        notice,
                    },
                );
            }
            if let Some(d) = kernel.durability() {
                for (project, message) in d.take_errors() {
                    emit(shared, KernelEvent::Durability { project, message });
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
                publish(kernel, shared, &project);
            }
            reply(result.clone());
            if result.is_ok() {
                emit(shared, KernelEvent::Opened { project });
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
                emit(shared, KernelEvent::Closed { project });
                crate::memory::pressure_relief();
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
        #[cfg(any(test, feature = "fault-injection"))]
        Msg::ArmFault { stage, project } => crate::fault_injection::arm(stage, project),
        #[cfg(any(test, feature = "fault-injection"))]
        Msg::HistoryBudget(max_entries) => kernel.set_history_budget_later(max_entries),
        Msg::DurabilityErrors => {
            if let Some(d) = kernel.durability() {
                for (project, message) in d.take_errors() {
                    emit(shared, KernelEvent::Durability { project, message });
                }
            }
        }
        Msg::Shutdown => {}
    }
}

#[cfg(test)]
#[path = "actor_fault_tests.rs"]
mod fault_tests;

// ── the UI's synchronous query mirror ───────────────────────────────────────

/// A [`QueryContext`] owned by the UI thread, following published changes.
pub struct QueryMirror {
    queries: QueryContext,
    doc_id: u64,
    revision: u64,
    metadata_revision: u64,
}

impl QueryMirror {
    pub fn new(published: &Published) -> QueryMirror {
        QueryMirror {
            queries: QueryContext::new(&published.snapshot.document),
            doc_id: published.snapshot.doc_id,
            revision: published.revision(),
            metadata_revision: published.metadata_revision,
        }
    }

    /// Fast-forwards to `published` (applying its changes in order, or
    /// adopting its snapshot when they no longer reach back). Returns the
    /// mirror's revision.
    pub fn sync(&mut self, published: &Published) -> u64 {
        if published.snapshot.doc_id != self.doc_id
            || published.metadata_revision != self.metadata_revision
        {
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
