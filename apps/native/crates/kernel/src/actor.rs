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
    pub project: String,
    pub snapshot: Snapshot,
    /// Recent changes, oldest first (for incremental readers and mirrors).
    pub changes: Arc<[Arc<Change>]>,
    pub history: HistorySummary,
    pub entry: Option<OpenEntryInfo>,
    pub selection: Selection,
    pub workspace: Value,
    pub dirty: bool,
}

impl Published {
    pub fn revision(&self) -> u64 {
        self.snapshot.revision
    }
}

/// What subscribers hear, after publication.
#[derive(Clone, Debug)]
pub enum KernelEvent {
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
    subscribers: Mutex<Vec<mpsc::Sender<KernelEvent>>>,
    realms: Mutex<HashMap<Writer, mpsc::Sender<RealmEnvelope>>>,
}

type Reply<T> = Box<dyn FnOnce(T) + Send>;

enum Msg {
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
    let p = kernel.project(project)?;
    let store = p.store();
    let history = store.history();
    let published = Published {
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
        dirty: store.is_dirty(),
    };
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
    for msg in rx {
        match msg {
            Msg::Call {
                project,
                request,
                reply,
            } => {
                let handled = match kernel.handle(&project, request) {
                    Ok(h) => h,
                    Err(e) => {
                        reply(Err(e));
                        continue;
                    }
                };
                // Publication boundary: publish, then answer, then events.
                let revision = publish(&mut kernel, &shared, &project);
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
                shared.published.lock().expect("published").remove(&project);
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
