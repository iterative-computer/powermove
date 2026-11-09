use crate::{
    vm::{self, HEAP_FULL, HeapPlan, Vm},
    *,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

pub(crate) struct Completion<T> {
    value: Option<Result<T>>,
    waker: Option<Waker>,
}
pub(crate) type SharedCompletion<T> = Arc<Mutex<Completion<T>>>;
/// Nonblocking result: poll as a Future or inspect with try_take. No wait/join API.
pub struct Ticket<T> {
    pub execution: u64,
    shared: SharedCompletion<T>,
}
impl<T> Ticket<T> {
    pub fn try_take(&self) -> Option<Result<T>> {
        self.shared.lock().unwrap().value.take()
    }
}
impl<T> Future for Ticket<T> {
    type Output = Result<T>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut c = self.shared.lock().unwrap();
        if let Some(value) = c.value.take() {
            Poll::Ready(value)
        } else {
            c.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
pub(crate) fn ticket<T>(id: u64) -> (Ticket<T>, SharedCompletion<T>) {
    let shared = Arc::new(Mutex::new(Completion {
        value: None,
        waker: None,
    }));
    (
        Ticket {
            execution: id,
            shared: shared.clone(),
        },
        shared,
    )
}
pub(crate) fn complete<T>(shared: SharedCompletion<T>, value: Result<T>) {
    let waker = {
        let mut c = shared.lock().unwrap();
        c.value = Some(value);
        c.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub generation: Generation,
    pub value: String,
    pub cache_hits: usize,
}
#[derive(Debug, Clone)]
pub struct ActiveExecution {
    pub execution: u64,
    pub generation: Generation,
    pub elapsed: Duration,
}
#[derive(Clone)]
pub struct GenerationPin {
    pub generation: Generation,
    live: Option<Arc<()>>,
    notifier: Weak<SyncSender<Command>>,
}
impl std::fmt::Debug for GenerationPin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.generation.fmt(f)
    }
}
impl Drop for GenerationPin {
    fn drop(&mut self) {
        drop(self.live.take());
        if let Some(tx) = self.notifier.upgrade() {
            let _ = tx.try_send(Command::Wake);
        }
    }
}
struct Execution {
    id: u64,
    handle: v8::IsolateHandle,
    cancellation: Cancellation,
    generation: Generation,
    epoch: u64,
    started: Instant,
    /// Time blocked in engine calls, which the watchdog doesn't count.
    blocked: Arc<crate::vm::Blocked>,
    noticed: bool,
    next_prompt: Duration,
}
struct Control {
    active: Mutex<Option<Execution>>,
    closing: AtomicBool,
    disposed: AtomicBool,
    wake: Condvar,
    ids: AtomicU64,
    /// The extension the watchdog found running a task it prompts about.
    culprit: crate::vm::Culprit,
}
struct Inner {
    tx: Arc<SyncSender<Command>>,
    control: Arc<Control>,
    max_source: usize,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.control.closing.store(true, Ordering::Release);
        self.control.wake.notify_all();
        if let Some(e) = &*self.control.active.lock().unwrap() {
            e.cancellation.cancel();
            e.handle.terminate_execution();
        }
    }
}
#[derive(Clone)]
pub struct Realm {
    inner: Arc<Inner>,
}
enum Command {
    Measure {
        reply: SharedCompletion<Vec<GenerationMemory>>,
    },
    Wake,
    Eval {
        id: u64,
        source: String,
        expected: Option<Generation>,
        svelte: Option<(String, SvelteUnit)>,
        reply: SharedCompletion<Evaluation>,
    },
    Reload {
        id: u64,
        source: String,
        reply: SharedCompletion<Evaluation>,
    },
    Pin {
        reply: SharedCompletion<GenerationPin>,
    },
    Retained {
        id: u64,
        pin: GenerationPin,
        source: String,
        reply: SharedCompletion<Evaluation>,
    },
}
impl Realm {
    fn send(&self, command: Command) -> Result<()> {
        if self.inner.control.closing.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        self.inner.tx.try_send(command).map_err(|e| match e {
            TrySendError::Full(_) => Error::Busy,
            TrySendError::Disconnected(_) => Error::Closed,
        })
    }
    fn check_source(&self, source: &str) -> Result<()> {
        if source.len() > self.inner.max_source {
            Err(Error::Quota("source bytes"))
        } else {
            Ok(())
        }
    }
    pub fn eval(&self, source: impl Into<String>) -> Result<Ticket<Evaluation>> {
        self.eval_generation(source, None)
    }
    pub fn eval_generation(
        &self,
        source: impl Into<String>,
        expected: Option<Generation>,
    ) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Eval {
            id,
            source,
            expected,
            svelte: None,
            reply,
        })?;
        Ok(ticket)
    }
    pub(crate) fn eval_svelte(
        &self,
        source: String,
        filename: String,
        unit: SvelteUnit,
    ) -> Result<Ticket<Evaluation>> {
        self.check_source(&source)?;
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Eval {
            id,
            source,
            expected: None,
            svelte: Some((filename, unit)),
            reply,
        })?;
        Ok(ticket)
    }
    /// Source is a script expression, optionally yielding a Promise. The caller's
    /// compiler/bundler resolves ESM imports before submission. Only v4 may reload.
    pub fn reload(&self, source: impl Into<String>) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Reload { id, source, reply })?;
        Ok(ticket)
    }
    /// Sampling runs on the owner and includes every pinned retired generation.
    pub fn measure(&self) -> Result<Ticket<Vec<GenerationMemory>>> {
        let (ticket, reply) = ticket(0);
        self.send(Command::Measure { reply })?;
        Ok(ticket)
    }
    pub fn pin(&self) -> Result<Ticket<GenerationPin>> {
        let (ticket, reply) = ticket(0);
        self.send(Command::Pin { reply })?;
        Ok(ticket)
    }
    pub fn invoke_retained(
        &self,
        pin: &GenerationPin,
        source: impl Into<String>,
    ) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Retained {
            id,
            pin: pin.clone(),
            source,
            reply,
        })?;
        Ok(ticket)
    }
    /// Inspect a hung or active realm without entering JavaScript.
    pub fn active_execution(&self) -> Option<ActiveExecution> {
        self.inner
            .control
            .active
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| ActiveExecution {
                execution: e.id,
                generation: e.generation.clone(),
                elapsed: e.started.elapsed(),
            })
    }
    /// Fenced by execution id; an old Stop cannot terminate a later task.
    /// The extension (`pmext://<id>/…` module) the watchdog found running
    /// `execution` when it prompted, if any.
    pub fn culprit(&self, execution: u64) -> Option<String> {
        self.inner.control.culprit.get(execution)
    }
    pub fn stop(&self, execution: u64) -> bool {
        let active = self.inner.control.active.lock().unwrap();
        if let Some(active) = active.as_ref().filter(|a| a.id == execution) {
            active.cancellation.cancel();
            active.handle.terminate_execution();
            true
        } else {
            false
        }
    }
    /// Nonblocking shutdown; disposal and all revocations happen on the owner.
    pub fn close(&self) {
        self.inner.control.closing.store(true, Ordering::Release);
        self.inner.control.wake.notify_all();
        let _ = self.inner.tx.try_send(Command::Wake);
        if let Some(e) = &*self.inner.control.active.lock().unwrap() {
            e.cancellation.cancel();
            e.handle.terminate_execution();
        }
    }
    pub fn is_disposed(&self) -> bool {
        self.inner.control.disposed.load(Ordering::Acquire)
    }
}

/// A pool is scoped to one project window. Its compat key is unique in that scope.
/// Callers must resolve synchronous dependencies before routing to this pool.
pub struct RuntimePool {
    scope: String,
    realms: Mutex<HashMap<RealmKind, Weak<Inner>>>,
    config: Config,
}
#[derive(Clone)]
pub struct Config {
    pub limits: Limits,
    pub watchdog: WatchdogPolicy,
    pub kernel: Arc<dyn KernelHooks>,
    pub host: Arc<dyn HostProxy>,
    pub watchdog_hooks: Arc<dyn WatchdogHooks>,
    pub cache: CodeCache,
    pub engine: Option<Arc<dyn EngineProxy>>,
    /// Shared admission account must include all pools and retained generations.
    pub admission: Admission,
}
/// Heap admission: what every isolate's current heap limit adds up to. An
/// isolate reserves its initial slice at start, takes more from V8's
/// near-heap-limit callback only while the account has room, gives back what
/// it grew past its live size when it idles, and returns the rest when it is
/// disposed on its owner, so `used` is exact at every step.
#[derive(Clone)]
pub struct Admission {
    used: Arc<AtomicUsize>,
    /// The most `used` has been, and how many isolates hold a reservation.
    peak: Arc<AtomicUsize>,
    isolates: Arc<AtomicUsize>,
    budget: usize,
}
/// In-process isolates share one 4 GB pointer-compression cage (plan §3.5.1);
/// a quarter stays for V8's code and read-only spaces, the grace a failing
/// isolate unwinds in, and allocator overheads.
pub const CAGE_ADMISSION: usize = 3 * 1024 * 1024 * 1024;
impl Admission {
    pub fn new(budget: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            peak: Arc::new(AtomicUsize::new(0)),
            isolates: Arc::new(AtomicUsize::new(0)),
            budget,
        }
    }
    /// The process's one account: every window's built-ins, v4 extensions,
    /// compat realm and compiler reserve from it.
    pub fn cage() -> Self {
        static CAGE: std::sync::OnceLock<Admission> = std::sync::OnceLock::new();
        CAGE.get_or_init(|| Admission::new(CAGE_ADMISSION)).clone()
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
    pub fn budget(&self) -> usize {
        self.budget
    }
    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }
    pub fn isolates(&self) -> usize {
        self.isolates.load(Ordering::Acquire)
    }
    fn take(&self, bytes: usize) -> bool {
        let taken = self
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|n| *n <= self.budget)
            });
        if let Ok(before) = taken {
            self.peak.fetch_max(before + bytes, Ordering::AcqRel);
        }
        taken.is_ok()
    }
    pub(crate) fn reserve(&self, bytes: usize) -> Result<Reservation> {
        if !self.take(bytes) {
            return Err(Error::Quota("isolate admission"));
        }
        self.isolates.fetch_add(1, Ordering::AcqRel);
        Ok(Reservation {
            account: self.clone(),
            bytes: AtomicUsize::new(bytes),
        })
    }
}
/// One isolate's share of its account. Only its owner thread changes it.
pub(crate) struct Reservation {
    account: Admission,
    bytes: AtomicUsize,
}
impl Reservation {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
    }
    /// More heap, only while the account has room.
    pub(crate) fn grow(&self, bytes: usize) -> bool {
        let taken = self.account.take(bytes);
        if taken {
            self.bytes.fetch_add(bytes, Ordering::AcqRel);
        }
        taken
    }
    /// The grace a terminating isolate unwinds in: taken even past the
    /// budget (the cage keeps room for it), and given back at disposal.
    pub(crate) fn grace(&self, bytes: usize) {
        let before = self.account.used.fetch_add(bytes, Ordering::AcqRel);
        self.account
            .peak
            .fetch_max(before + bytes, Ordering::AcqRel);
        self.bytes.fetch_add(bytes, Ordering::AcqRel);
    }
    pub(crate) fn shrink(&self, bytes: usize) {
        let bytes = bytes.min(self.bytes());
        self.bytes.fetch_sub(bytes, Ordering::AcqRel);
        self.account.used.fetch_sub(bytes, Ordering::AcqRel);
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.account.isolates.fetch_sub(1, Ordering::AcqRel);
        self.account
            .used
            .fetch_sub(*self.bytes.get_mut(), Ordering::AcqRel);
    }
}
impl RuntimePool {
    pub fn new(scope: impl Into<String>, config: Config) -> Result<Self> {
        if config.limits.heap_bytes < 16 * 1024 * 1024
            || config.limits.initial_heap_bytes > config.limits.heap_bytes
            || config.limits.queue_capacity == 0
            || config.limits.max_host_ops == 0
            || config.watchdog.prompt < config.watchdog.notice
            || config.watchdog.repeat.is_zero()
        {
            return Err(Error::Invalid("Invalid runtime limits"));
        }
        Ok(Self {
            scope: scope.into(),
            realms: Default::default(),
            config,
        })
    }
    /// Starting is asynchronous: the first task observes startup/load failures.
    pub fn realm(&self, kind: RealmKind) -> Result<Realm> {
        if matches!(kind, RealmKind::Store(_)) {
            return Err(Error::Invalid(
                "Store realms require StorePool helper confinement",
            ));
        }
        self.start_realm(kind)
    }
    pub(crate) fn start_realm(&self, kind: RealmKind) -> Result<Realm> {
        let mut realms = self.realms.lock().unwrap();
        if let Some(inner) = realms
            .get(&kind)
            .and_then(Weak::upgrade)
            .filter(|i| !i.control.closing.load(Ordering::Acquire))
        {
            return Ok(Realm { inner });
        }
        let reservation = self
            .config
            .admission
            .reserve(HeapPlan::new(&self.config.limits).initial())?;
        let (tx, rx) = mpsc::sync_channel(self.config.limits.queue_capacity);
        let control = Arc::new(Control {
            active: Mutex::new(None),
            closing: AtomicBool::new(false),
            disposed: AtomicBool::new(false),
            wake: Condvar::new(),
            ids: AtomicU64::new(1),
            culprit: crate::vm::Culprit::default(),
        });
        let inner = Arc::new(Inner {
            tx: Arc::new(tx),
            control: control.clone(),
            max_source: self.config.limits.max_source_bytes,
        });
        let name = format!("{}:{kind:?}", self.scope);
        let config = self.config.clone();
        let worker_kind = kind.clone();
        let notifier = Arc::downgrade(&inner.tx);
        thread::Builder::new()
            .name("pm-v8-owner".into())
            .spawn(move || {
                worker(
                    rx,
                    control,
                    name,
                    worker_kind,
                    config,
                    reservation,
                    notifier,
                )
            })
            .map_err(|e| Error::Host(e.to_string()))?;
        realms.insert(kind.clone(), Arc::downgrade(&inner));
        Ok(Realm { inner })
    }
    /// Whole editor closure switches to Compat when any provider has a v1–3 fork.
    pub fn builtin(&self, id: &str, compat_editor: bool) -> Result<Realm> {
        let kind = builtin_cluster(id);
        self.realm(if compat_editor && kind == RealmKind::BuiltinEditor {
            RealmKind::Compat
        } else {
            kind
        })
    }
}

// One sleeping watchdog per owner, never a new thread per task or timer tick.
struct WatchdogThread {
    control: Arc<Control>,
    join: Option<thread::JoinHandle<()>>,
}
impl WatchdogThread {
    fn new(control: &Arc<Control>, config: &Config) -> Self {
        let control = control.clone();
        let watched = control.clone();
        let hooks = config.watchdog_hooks.clone();
        let repeat = config.watchdog.repeat;
        let notice_after = config.watchdog.notice;
        let join = thread::spawn(move || {
            loop {
                let mut active = watched.active.lock().unwrap();
                while active.is_none() && !watched.closing.load(Ordering::Acquire) {
                    active = watched.wake.wait(active).unwrap();
                }
                if watched.closing.load(Ordering::Acquire) {
                    break;
                }
                let (mut active, _) = watched
                    .wake
                    .wait_timeout(active, Duration::from_millis(5))
                    .unwrap();
                let Some(execution) = active.as_mut() else {
                    continue;
                };
                if execution.cancellation.is_cancelled() {
                    execution.handle.terminate_execution();
                }
                execution
                    .handle
                    .request_interrupt(vm::check_external, std::ptr::null_mut());
                // JS time only: waiting on the engine or kernel is not a hang.
                let elapsed = execution
                    .started
                    .elapsed()
                    .saturating_sub(execution.blocked.total());
                let notice = !execution.noticed && elapsed >= notice_after;
                if notice {
                    execution.noticed = true;
                }
                let prompt = elapsed >= execution.next_prompt;
                if prompt {
                    execution.next_prompt = elapsed + repeat;
                }
                let (generation, id, epoch) =
                    (execution.generation.clone(), execution.id, execution.epoch);
                drop(active);
                if notice {
                    hooks.notice(&generation, id);
                }
                if prompt {
                    // Who is running: the innermost extension module on the
                    // stack, read inside the running JS (any hang: a raster
                    // override, an event, a timer, a command).
                    let handle = watched
                        .active
                        .lock()
                        .unwrap()
                        .as_ref()
                        .filter(|e| e.epoch == epoch)
                        .map(|e| e.handle.clone());
                    if let Some(handle) = handle {
                        crate::vm::capture_culprit(&handle, &watched.culprit, id);
                    }
                }
                if prompt && hooks.prompt(&generation, id) == WatchdogDecision::Stop {
                    let active = watched.active.lock().unwrap();
                    if let Some(execution) = active.as_ref().filter(|e| e.epoch == epoch) {
                        execution.cancellation.cancel();
                        execution.handle.terminate_execution();
                    }
                }
            }
        });
        Self {
            control,
            join: Some(join),
        }
    }
}
impl Drop for WatchdogThread {
    fn drop(&mut self) {
        self.control.closing.store(true, Ordering::Release);
        self.control.wake.notify_all();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
struct Guard {
    control: Arc<Control>,
    epoch: u64,
}
impl Guard {
    fn new(
        vm: &Vm,
        generation: &Generation,
        id: u64,
        config: &Config,
        control: &Arc<Control>,
    ) -> Self {
        static EPOCH: AtomicU64 = AtomicU64::new(1);
        let epoch = EPOCH.fetch_add(1, Ordering::Relaxed);
        let handle = vm.handle();
        let cancellation = Cancellation::default();
        vm.begin_task(cancellation.clone());
        *control.active.lock().unwrap() = Some(Execution {
            id,
            epoch,
            generation: generation.clone(),
            handle: handle.clone(),
            cancellation: cancellation.clone(),
            started: Instant::now(),
            blocked: vm.blocked(),
            noticed: false,
            next_prompt: config.watchdog.prompt,
        });
        control.wake.notify_all();
        if control.closing.load(Ordering::Acquire) {
            cancellation.cancel();
            handle.terminate_execution();
        }
        Self {
            control: control.clone(),
            epoch,
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let mut active = self.control.active.lock().unwrap();
        if active.as_ref().is_some_and(|a| a.epoch == self.epoch) {
            *active = None;
        }
        self.control.wake.notify_all();
    }
}
struct Request {
    id: u64,
    job: Box<dyn HostRequest>,
    cancel: Cancellation,
    deadline: Instant,
    /// External bytes it holds (response room and body), released on retire.
    reserved: usize,
}
struct Live {
    vm: Vm,
    generation: Generation,
    storage: Value,
    hot: Value,
    pins: Arc<()>,
    requests: Vec<Request>,
    next_wake: Option<Instant>,
}
impl Live {
    fn new(
        generation: Generation,
        kind: &RealmKind,
        config: &Config,
        storage: Value,
        hot: Value,
        reservation: Reservation,
    ) -> Result<Self> {
        let vm = Vm::new(
            kind.snapshot_kind(),
            config,
            &storage,
            &hot,
            generation.clone(),
            reservation,
        )?;
        Ok(Self {
            vm,
            generation,
            storage,
            hot,
            pins: Arc::new(()),
            requests: vec![],
            next_wake: None,
        })
    }
    fn cancel_io(&mut self) {
        for request in &mut self.requests {
            request.cancel.cancel();
            request.job.cancel();
        }
        self.requests.clear();
        let mut n = self.vm.native.lock().unwrap();
        n.queued.clear();
        n.deferred.clear();
        n.aborted.clear();
        n.in_flight = 0;
        n.reserved = 0;
    }
    /// AbortSignal rejects in JS immediately. Retire the corresponding native
    /// job before acknowledging that task, even if no future idle tick occurs.
    fn cancel_aborted(&mut self) {
        let (aborted, mut retired) = {
            let mut n = self.vm.native.lock().unwrap();
            let aborted = std::mem::take(&mut n.aborted);
            let (mut count, mut released) = (0, 0);
            n.queued.retain(|(id, r)| {
                let keep = !aborted.contains(id);
                if !keep {
                    count += 1;
                    released += crate::vm::held(r, true);
                }
                keep
            });
            // A waiting fetch holds its body only.
            n.deferred.retain(|(id, r)| {
                let keep = !aborted.contains(id);
                if !keep {
                    count += 1;
                    released += crate::vm::held(r, false);
                }
                keep
            });
            n.in_flight -= count;
            n.reserved -= released;
            (aborted, 0)
        };
        let mut released = 0;
        self.requests.retain_mut(|request| {
            if aborted.contains(&request.id) {
                request.cancel.cancel();
                request.job.cancel();
                retired += 1;
                released += request.reserved;
                false
            } else {
                true
            }
        });
        let mut n = self.vm.native.lock().unwrap();
        n.in_flight -= retired;
        n.reserved -= released;
    }
    fn pump(&mut self, config: &Config) -> Result<()> {
        let (queued, aborted, logs) = {
            let mut n = self.vm.native.lock().unwrap();
            // A waiting fetch that was aborted held its body only.
            let aborted = n.aborted.clone();
            let (mut dropped, mut released) = (0, 0);
            n.deferred.retain(|(id, r)| {
                let keep = !aborted.contains(id);
                if !keep {
                    dropped += 1;
                    released += crate::vm::held(r, false);
                }
                keep
            });
            n.in_flight = n.in_flight.saturating_sub(dropped);
            n.reserved = n.reserved.saturating_sub(released);
            n.admit_deferred();
            (
                std::mem::take(&mut n.queued),
                std::mem::take(&mut n.aborted),
                std::mem::take(&mut n.logs),
            )
        };
        for (level, message) in logs {
            config.host.console(&self.generation, &level, &message);
        }
        let mut completed = Vec::new();
        let (mut retired, mut released) = (0, 0);
        for (id, request) in queued {
            let reserved = crate::vm::held(&request, true);
            if aborted.contains(&id) {
                retired += 1;
                released += reserved;
                continue;
            }
            let cancellation = Cancellation::default();
            let deadline = request.deadline;
            match config
                .host
                .fetch(&self.generation, request, cancellation.clone())
            {
                Ok(job) => self.requests.push(Request {
                    id,
                    job,
                    cancel: cancellation,
                    deadline,
                    reserved,
                }),
                Err(error) => {
                    completed.push(json!([id,{"error":error.to_string()}]));
                    retired += 1;
                    released += reserved;
                }
            }
        }
        let mut bodies = Vec::new();
        self.requests.retain_mut(|request|{
            let result=if aborted.contains(&request.id)||Instant::now()>=request.deadline {request.cancel.cancel();request.job.cancel();Some(Err(Error::Host("Aborted or timed out".into())))}else{request.job.poll()};
            if let Some(result)=result {
                retired+=1;
                released+=request.reserved;
                let data=match result {
                    Ok(response) if response.body.len()<=config.limits.max_response_bytes && response.headers.iter().map(|(a,b)|a.len()+b.len()).sum::<usize>()<=config.limits.max_op_bytes =>{
                        // The body goes over as bytes (`__body`), not JSON.
                        bodies.push((request.id,response.body));
                        json!({"status":response.status,"url":response.url,"headers":response.headers,"body":request.id})
                    },
                    Ok(_)=>{request.cancel.cancel();request.job.cancel();json!({"error":"Host response quota"})},
                    Err(error)=>json!({"error":error.to_string()})
                };completed.push(json!([request.id,data]));false
            }else{true}
        });
        {
            let mut n = self.vm.native.lock().unwrap();
            n.in_flight = n.in_flight.saturating_sub(retired);
            n.reserved = n.reserved.saturating_sub(released);
            // Room that just came free goes to waiting fetches; the next pump starts them.
            n.admit_deferred();
            n.bodies.extend(bodies);
        }
        let pumped = self.vm.pump(json!(completed));
        // A body JS didn't take (its request was gone) is dropped here.
        self.vm.native.lock().unwrap().bodies.clear();
        pumped
    }
    fn run(
        &mut self,
        source: &str,
        _id: u64,
        config: &Config,
        control: &Arc<Control>,
    ) -> Result<String> {
        let started = self.vm.start(source);
        self.cancel_aborted();
        let value = started.map_err(|error| {
            if self.vm.task_cancelled() {
                Error::Terminated
            } else {
                error
            }
        })?;
        loop {
            if self.vm.task_cancelled() {
                self.vm.handle().terminate_execution();
                return Err(Error::Terminated);
            }
            if control.closing.load(Ordering::Acquire) {
                self.vm.handle().terminate_execution();
                return Err(Error::Closed);
            }
            let settled = self.vm.settle(&value);
            self.cancel_aborted();
            if let Some(value) = settled? {
                return Ok(value);
            }
            self.pump(config)?;
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn commit(
        &mut self,
        previous: Option<&Generation>,
        config: &Config,
        retained: bool,
    ) -> Result<()> {
        let effects = self.vm.effects()?;
        let ops = effects["ops"]
            .as_array()
            .ok_or(Error::Invalid("Invalid staged operations"))?;
        if ops.len() > config.limits.max_ops
            || serde_json::to_vec(ops)
                .map_err(|e| Error::JavaScript(e.to_string()))?
                .len()
                > config.limits.max_op_bytes
        {
            return Err(Error::Quota("staged operation bytes/count"));
        }
        // Finish every fallible JS step before the kernel's atomic publication.
        self.update_schedule()?;
        if retained {
            config.kernel.commit_retained(&self.generation, ops)?;
        } else {
            config.kernel.commit_activation(
                previous,
                &self.generation,
                ops,
                &effects["storage"],
            )?;
            self.storage = effects["storage"].clone();
            self.hot = effects["hot"].clone();
        }
        Ok(())
    }
    fn update_schedule(&mut self) -> Result<()> {
        let delay = self.vm.text("__pm.nextDelay(__now())")?;
        let delay = delay
            .parse::<f64>()
            .ok()
            .map(|n| Duration::from_secs_f64(n.max(0.) / 1000.));
        let native = self.vm.native.lock().unwrap();
        let io_delay = if !native.queued.is_empty() || !native.logs.is_empty() {
            Some(Duration::ZERO)
        } else if !self.requests.is_empty() || !native.deferred.is_empty() {
            Some(Duration::from_millis(5))
        } else {
            None
        };
        self.next_wake = delay
            .into_iter()
            .chain(io_delay)
            .min()
            .map(|d| Instant::now() + d);
        Ok(())
    }
    fn restore_storage(&mut self) -> Result<()> {
        self.vm.restore(&self.storage, &self.hot)?;
        self.update_schedule()
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        self.cancel_io();
    }
}
fn fail_generation(live: &mut Live, config: &Config) {
    live.cancel_io();
    config.kernel.revoke_generation(&live.generation);
    config.kernel.seal_open_gestures(&live.generation);
}
fn worker(
    rx: Receiver<Command>,
    control: Arc<Control>,
    name: String,
    kind: RealmKind,
    config: Config,
    reservation: Reservation,
    notifier: Weak<SyncSender<Command>>,
) {
    let _watchdog = WatchdogThread::new(&control, &config);
    let generation = Generation {
        realm: name.clone(),
        number: next_generation(),
    };
    let initial = config
        .kernel
        .load_storage(&name)
        .and_then(|storage| Live::new(generation, &kind, &config, storage, json!({}), reservation));
    let mut current = initial;
    let mut retained: HashMap<u64, Live> = HashMap::new();
    // Set after work that grew a heap: an isolate idle this long gives back
    // what it grew past its live size.
    let mut trim_at: Option<Instant> = None;
    while !control.closing.load(Ordering::Acquire) {
        retained.retain(|_, live| Arc::strong_count(&live.pins) > 1);
        let grown = current.as_ref().is_ok_and(|live| live.vm.grown())
            || retained.values().any(|live| live.vm.grown());
        if !grown {
            trim_at = None;
        } else if trim_at.is_none() {
            trim_at = Some(Instant::now() + IDLE_TRIM);
        }
        let wake = current.as_ref().ok().and_then(|live| live.next_wake);
        let deadline = match (wake, trim_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let received = match deadline {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        if received.is_ok() && trim_at.is_some() {
            trim_at = Some(Instant::now() + IDLE_TRIM);
        }
        let command = match received {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if trim_at.is_some_and(|at| at <= Instant::now()) {
                    trim_at = None;
                    if let Ok(live) = &mut current {
                        live.vm.trim();
                    }
                    for live in retained.values_mut() {
                        live.vm.trim();
                    }
                }
                if wake.is_none_or(|wake| wake > Instant::now()) {
                    continue;
                }
                if let Ok(live) = &mut current {
                    // Timers and pending host operations run without a submitted task.
                    let guard = Guard::new(
                        &live.vm,
                        &live.generation,
                        control.ids.fetch_add(1, Ordering::Relaxed),
                        &config,
                        &control,
                    );
                    let result = live
                        .pump(&config)
                        .and_then(|_| live.commit(Some(&live.generation.clone()), &config, false));
                    drop(guard);
                    if result.is_err() {
                        fail_generation(live, &config);
                        drop(std::mem::replace(&mut current, Err(Error::Closed)));
                        current = recover(&name, &kind, &config);
                    }
                }
                continue;
            }
        };
        match command {
            Command::Wake => continue,
            Command::Measure { reply } => {
                let mut measurements = Vec::new();
                if let Ok(live) = &current {
                    measurements.push(GenerationMemory {
                        generation: live.generation.clone(),
                        retired: false,
                        memory: live.vm.memory(),
                    });
                }
                for live in retained.values() {
                    measurements.push(GenerationMemory {
                        generation: live.generation.clone(),
                        retired: true,
                        memory: live.vm.memory(),
                    });
                }
                complete(reply, Ok(measurements));
            }
            Command::Pin { reply } => complete(
                reply,
                current
                    .as_ref()
                    .map(|live| GenerationPin {
                        generation: live.generation.clone(),
                        live: Some(live.pins.clone()),
                        notifier: notifier.clone(),
                    })
                    .map_err(Clone::clone),
            ),
            Command::Eval {
                id,
                source,
                expected,
                svelte,
                reply,
            } => {
                let source = if let Some((filename, unit)) = svelte {
                    match crate::compiler::compile_expression(&source, &filename, unit) {
                        Ok(source) => source,
                        Err(e) => {
                            complete(reply, Err(e));
                            continue;
                        }
                    }
                } else {
                    source
                };
                let result = match &mut current {
                    Ok(live) if expected.as_ref().is_none_or(|g| g == &live.generation) => {
                        // Keep the watchdog armed through effect serialization and commit.
                        let guard = Guard::new(&live.vm, &live.generation, id, &config, &control);
                        let result = live.run(&source, id, &config, &control).and_then(|value| {
                            live.commit(Some(&live.generation.clone()), &config, false)?;
                            Ok(Evaluation {
                                generation: live.generation.clone(),
                                value,
                                cache_hits: live.vm.cache_hits,
                            })
                        });
                        drop(guard);
                        result
                    }
                    Ok(_) => Err(Error::StaleGeneration),
                    Err(error) => Err(error.clone()),
                };
                if let Ok(live) = &mut current {
                    if matches!(
                        result,
                        Err(Error::Terminated
                            | Error::Quota(HEAP_FULL | "heap" | "external memory")
                            | Error::Closed)
                    ) {
                        fail_generation(live, &config);
                        drop(std::mem::replace(&mut current, Err(Error::Closed)));
                        current = recover(&name, &kind, &config);
                    } else if result.is_err() {
                        let guard = Guard::new(&live.vm, &live.generation, id, &config, &control);
                        let restored = live.restore_storage();
                        drop(guard);
                        if restored.is_err() {
                            fail_generation(live, &config);
                            drop(std::mem::replace(&mut current, Err(Error::Closed)));
                            current = recover(&name, &kind, &config);
                        }
                    }
                }
                complete(reply, result);
            }
            Command::Reload { id, source, reply } => {
                if !matches!(kind, RealmKind::V4Extension(_)) {
                    complete(
                        reply,
                        Err(Error::Invalid(
                            "Only v4 extensions hot reload in separate generations",
                        )),
                    );
                    continue;
                }
                let result = (|| -> Result<(Live, String)> {
                    let old = current.as_ref().map_err(Clone::clone)?;
                    let reservation = config
                        .admission
                        .reserve(HeapPlan::new(&config.limits).initial())?;
                    let generation = Generation {
                        realm: name.clone(),
                        number: next_generation(),
                    };
                    let mut candidate = Live::new(
                        generation,
                        &kind,
                        &config,
                        old.storage.clone(),
                        old.hot.clone(),
                        reservation,
                    )?;
                    let guard =
                        Guard::new(&candidate.vm, &candidate.generation, id, &config, &control);
                    let result = candidate
                        .run(&source, id, &config, &control)
                        .and_then(|value| {
                            candidate.commit(Some(&old.generation), &config, false)?;
                            Ok(value)
                        });
                    drop(guard);
                    match result {
                        Ok(value) => Ok((candidate, value)),
                        Err(error) => {
                            fail_generation(&mut candidate, &config);
                            Err(error)
                        }
                    }
                })();
                match result {
                    Ok((candidate, value)) => {
                        let result = Evaluation {
                            generation: candidate.generation.clone(),
                            value,
                            cache_hits: candidate.vm.cache_hits,
                        };
                        if let Ok(mut old) = std::mem::replace(&mut current, Ok(candidate)) {
                            old.cancel_io();
                            let guard = Guard::new(&old.vm, &old.generation, id, &config, &control);
                            let retired = old.vm.retire();
                            drop(guard);
                            config.kernel.revoke_generation(&old.generation);
                            config.kernel.seal_open_gestures(&old.generation);
                            if retired.is_ok() && Arc::strong_count(&old.pins) > 1 {
                                retained.insert(old.generation.number, old);
                            }
                        }
                        complete(reply, Ok(result));
                    }
                    Err(error) => complete(reply, Err(error)),
                }
            }
            Command::Retained {
                id,
                pin,
                source,
                reply,
            } => {
                let is_current = current.as_ref().is_ok_and(|v| {
                    v.generation == pin.generation
                        && pin.live.as_ref().is_some_and(|p| Arc::ptr_eq(&v.pins, p))
                });
                let live = if is_current {
                    current.as_mut().ok()
                } else {
                    retained.get_mut(&pin.generation.number).filter(|v| {
                        v.generation == pin.generation
                            && pin.live.as_ref().is_some_and(|p| Arc::ptr_eq(&v.pins, p))
                    })
                };
                let result = if let Some(live) = live {
                    let guard = Guard::new(&live.vm, &live.generation, id, &config, &control);
                    let result = live.run(&source, id, &config, &control).and_then(|value| {
                        live.commit(None, &config, true)?;
                        Ok(Evaluation {
                            generation: live.generation.clone(),
                            value,
                            cache_hits: live.vm.cache_hits,
                        })
                    });
                    drop(guard);
                    result
                } else {
                    Err(Error::StaleGeneration)
                };
                let mut failed = matches!(
                    result,
                    Err(Error::Terminated
                        | Error::Quota(HEAP_FULL | "heap" | "external memory")
                        | Error::Closed)
                );
                if result.is_err() && !failed {
                    let live = if is_current {
                        current.as_mut().ok()
                    } else {
                        retained.get_mut(&pin.generation.number)
                    };
                    if let Some(live) = live {
                        let guard = Guard::new(&live.vm, &live.generation, id, &config, &control);
                        failed = live.restore_storage().is_err();
                        drop(guard);
                    }
                }
                if failed {
                    if is_current {
                        if let Ok(live) = &mut current {
                            fail_generation(live, &config);
                        }
                        drop(std::mem::replace(&mut current, Err(Error::Closed)));
                        current = recover(&name, &kind, &config);
                    } else if let Some(mut live) = retained.remove(&pin.generation.number) {
                        fail_generation(&mut live, &config);
                    }
                }
                complete(reply, result);
            }
        }
    }
    if let Ok(live) = &mut current {
        fail_generation(live, &config);
    }
    for live in retained.values_mut() {
        fail_generation(live, &config);
    }
    drop(current);
    drop(retained);
    control.closing.store(true, Ordering::Release);
    for command in rx.try_iter() {
        match command {
            Command::Wake => continue,
            Command::Measure { reply } => complete(reply, Err(Error::Closed)),
            Command::Pin { reply } => complete(reply, Err(Error::Closed)),
            Command::Eval { reply, .. }
            | Command::Reload { reply, .. }
            | Command::Retained { reply, .. } => complete(reply, Err(Error::Closed)),
        }
    }
    control.disposed.store(true, Ordering::Release);
}
/// How long an isolate that grew waits without work before it trims.
const IDLE_TRIM: Duration = Duration::from_secs(5);
fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
fn recover(name: &str, kind: &RealmKind, config: &Config) -> Result<Live> {
    let generation = Generation {
        realm: name.into(),
        number: next_generation(),
    };
    let reservation = config
        .admission
        .reserve(HeapPlan::new(&config.limits).initial())?;
    let storage = config.kernel.load_storage(name)?;
    Live::new(generation, kind, config, storage, json!({}), reservation)
}
