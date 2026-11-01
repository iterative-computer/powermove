use crate::{
    vm::{self, Vm},
    *,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

struct Completion<T> {
    value: Option<Result<T>>,
    waker: Option<Waker>,
}
type SharedCompletion<T> = Arc<Mutex<Completion<T>>>;
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
fn ticket<T>(id: u64) -> (Ticket<T>, SharedCompletion<T>) {
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
fn complete<T>(shared: SharedCompletion<T>, value: Result<T>) {
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
#[derive(Clone)]
pub struct GenerationPin {
    pub generation: Generation,
    live: Arc<()>,
}
impl std::fmt::Debug for GenerationPin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.generation.fmt(f)
    }
}
struct Execution {
    id: u64,
    handle: v8::IsolateHandle,
    cancellation: Cancellation,
}
struct Control {
    active: Mutex<Option<Execution>>,
    closing: AtomicBool,
    disposed: AtomicBool,
}
struct Inner {
    tx: SyncSender<Command>,
    control: Arc<Control>,
    ids: AtomicU64,
    max_source: usize,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.control.closing.store(true, Ordering::Release);
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
    Eval {
        id: u64,
        source: String,
        expected: Option<Generation>,
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
        let id = self.inner.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Eval {
            id,
            source,
            expected,
            reply,
        })?;
        Ok(ticket)
    }
    /// Source is a script expression, optionally yielding a Promise. The caller's
    /// compiler/bundler resolves ESM imports before submission. Only v4 may reload.
    pub fn reload(&self, source: impl Into<String>) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        let id = self.inner.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Reload { id, source, reply })?;
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
        let id = self.inner.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Retained {
            id,
            pin: pin.clone(),
            source,
            reply,
        })?;
        Ok(ticket)
    }
    /// Fenced by execution id; an old Stop cannot terminate a later task.
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
    /// Shared admission account must include all pools and retained generations.
    pub admission: Admission,
}
#[derive(Clone)]
pub struct Admission {
    used: Arc<AtomicUsize>,
    budget: usize,
}
impl Admission {
    pub fn new(budget: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            budget,
        }
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
    fn reserve(&self, bytes: usize) -> Result<Reservation> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|n| *n <= self.budget)
            })
            .map_err(|_| Error::Quota("isolate admission"))?;
        Ok(Reservation {
            account: self.clone(),
            bytes,
        })
    }
}
struct Reservation {
    account: Admission,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.account.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
impl RuntimePool {
    pub fn new(scope: impl Into<String>, config: Config) -> Result<Self> {
        if config.limits.heap_bytes < 16 * 1024 * 1024
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
            .reserve(self.config.limits.heap_bytes)?;
        let (tx, rx) = mpsc::sync_channel(self.config.limits.queue_capacity);
        let control = Arc::new(Control {
            active: Mutex::new(None),
            closing: AtomicBool::new(false),
            disposed: AtomicBool::new(false),
        });
        let inner = Arc::new(Inner {
            tx,
            control: control.clone(),
            ids: AtomicU64::new(1),
            max_source: self.config.limits.max_source_bytes,
        });
        let name = format!("{}:{kind:?}", self.scope);
        let config = self.config.clone();
        let worker_kind = kind.clone();
        thread::Builder::new()
            .name("pm-v8-owner".into())
            .spawn(move || worker(rx, control, name, worker_kind, config, reservation))
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

struct Guard {
    done: Option<mpsc::Sender<()>>,
    join: Option<thread::JoinHandle<()>>,
    control: Arc<Control>,
    id: u64,
}
impl Guard {
    fn new(
        vm: &Vm,
        generation: &Generation,
        id: u64,
        config: &Config,
        control: &Arc<Control>,
    ) -> Self {
        let handle = vm.handle();
        let cancellation = Cancellation::default();
        *control.active.lock().unwrap() = Some(Execution {
            id,
            handle: handle.clone(),
            cancellation: cancellation.clone(),
        });
        if control.closing.load(Ordering::Acquire) {
            cancellation.cancel();
            handle.terminate_execution();
        }
        let (tx, rx) = mpsc::channel();
        let hooks = config.watchdog_hooks.clone();
        let policy = config.watchdog.clone();
        let generation = generation.clone();
        let join = thread::spawn(move || {
            let start = Instant::now();
            let mut noticed = false;
            let mut prompt = policy.prompt;
            loop {
                if rx.recv_timeout(Duration::from_millis(5)).is_ok() {
                    break;
                }
                handle.request_interrupt(vm::check_external, std::ptr::null_mut());
                if cancellation.is_cancelled() {
                    handle.terminate_execution();
                }
                let elapsed = start.elapsed();
                if !noticed && elapsed >= policy.notice {
                    noticed = true;
                    hooks.notice(&generation, id);
                }
                if elapsed >= prompt {
                    match hooks.prompt(&generation, id) {
                        WatchdogDecision::Stop => {
                            cancellation.cancel();
                            handle.terminate_execution();
                            break;
                        }
                        WatchdogDecision::Wait => prompt = elapsed + policy.repeat,
                    }
                }
            }
        });
        Self {
            done: Some(tx),
            join: Some(join),
            control: control.clone(),
            id,
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(tx) = self.done.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let mut active = self.control.active.lock().unwrap();
        if active.as_ref().is_some_and(|a| a.id == self.id) {
            *active = None;
        }
    }
}
struct Request {
    id: u64,
    job: Box<dyn HostRequest>,
    cancel: Cancellation,
    deadline: Instant,
}
struct Live {
    vm: Vm,
    generation: Generation,
    storage: Value,
    hot: Value,
    pins: Arc<()>,
    requests: Vec<Request>,
    _reservation: Reservation,
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
            config.limits.clone(),
            &storage,
            &hot,
            config.cache.clone(),
        )?;
        Ok(Self {
            vm,
            generation,
            storage,
            hot,
            pins: Arc::new(()),
            requests: vec![],
            _reservation: reservation,
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
        n.aborted.clear();
        n.in_flight = 0;
        n.reserved = 0;
    }
    fn pump(&mut self, config: &Config) -> Result<()> {
        let (queued, aborted, logs) = {
            let mut n = self.vm.native.lock().unwrap();
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
        let mut retired = 0;
        for (id, request) in queued {
            if aborted.contains(&id) {
                retired += 1;
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
                }),
                Err(error) => {
                    completed.push(json!([id,{"error":error.to_string()}]));
                    retired += 1;
                }
            }
        }
        self.requests.retain_mut(|request|{
            let result=if aborted.contains(&request.id)||Instant::now()>=request.deadline {request.cancel.cancel();request.job.cancel();Some(Err(Error::Host("Aborted or timed out".into())))}else{request.job.poll()};
            if let Some(result)=result {
                retired+=1;
                let data=match result {
                    Ok(response) if response.body.len()<=config.limits.max_response_bytes && response.headers.iter().map(|(a,b)|a.len()+b.len()).sum::<usize>()<=config.limits.max_op_bytes =>json!({"status":response.status,"url":response.url,"headers":response.headers,"body":response.body}),
                    Ok(_)=>{request.cancel.cancel();request.job.cancel();json!({"error":"Host response quota"})},
                    Err(error)=>json!({"error":error.to_string()})
                };completed.push(json!([request.id,data]));false
            }else{true}
        });
        {
            let mut n = self.vm.native.lock().unwrap();
            n.in_flight = n.in_flight.saturating_sub(retired);
            n.reserved = n
                .reserved
                .saturating_sub(retired * config.limits.max_response_bytes);
        }
        self.vm.pump(json!(completed))
    }
    fn run(
        &mut self,
        source: &str,
        _id: u64,
        config: &Config,
        control: &Arc<Control>,
    ) -> Result<String> {
        let value = self.vm.start(source)?;
        loop {
            if control.closing.load(Ordering::Acquire) {
                self.vm.handle().terminate_execution();
                return Err(Error::Closed);
            }
            if let Some(value) = self.vm.settle(&value)? {
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
    fn restore_storage(&mut self) -> Result<()> {
        self.vm.text(&format!(
            "__pm.resetOps(); storage={}; hot.data={};",
            self.storage, self.hot
        ))?;
        Ok(())
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
) {
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
    while !control.closing.load(Ordering::Acquire) {
        retained.retain(|_, live| Arc::strong_count(&live.pins) > 1);
        let command = match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(live) = &mut current {
                    // Timers and pending host operations run without a submitted task.
                    let guard = Guard::new(&live.vm, &live.generation, 0, &config, &control);
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
            Command::Pin { reply } => complete(
                reply,
                current
                    .as_ref()
                    .map(|live| GenerationPin {
                        generation: live.generation.clone(),
                        live: live.pins.clone(),
                    })
                    .map_err(Clone::clone),
            ),
            Command::Eval {
                id,
                source,
                expected,
                reply,
            } => {
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
                            | Error::Quota("heap" | "external memory")
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
                    let reservation = config.admission.reserve(config.limits.heap_bytes)?;
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
                            let retired = old.vm.text("__pm.retire()");
                            drop(guard);
                            config.kernel.revoke_generation(&old.generation);
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
                let live = if current.as_ref().is_ok_and(|v| {
                    v.generation == pin.generation && Arc::ptr_eq(&v.pins, &pin.live)
                }) {
                    current.as_mut().ok()
                } else {
                    retained.get_mut(&pin.generation.number).filter(|v| {
                        v.generation == pin.generation && Arc::ptr_eq(&v.pins, &pin.live)
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
                if result.is_err()
                    && let Some(mut live) = retained.remove(&pin.generation.number)
                {
                    fail_generation(&mut live, &config);
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
            Command::Pin { reply } => complete(reply, Err(Error::Closed)),
            Command::Eval { reply, .. }
            | Command::Reload { reply, .. }
            | Command::Retained { reply, .. } => complete(reply, Err(Error::Closed)),
        }
    }
    control.disposed.store(true, Ordering::Release);
}
fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
fn recover(name: &str, kind: &RealmKind, config: &Config) -> Result<Live> {
    let generation = Generation {
        realm: name.into(),
        number: next_generation(),
    };
    let reservation = config.admission.reserve(config.limits.heap_bytes)?;
    let storage = config.kernel.load_storage(name)?;
    Live::new(generation, kind, config, storage, json!({}), reservation)
}
